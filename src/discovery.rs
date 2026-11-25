//! Local multicast rendezvous. Discovery never sends a pairing code.
use crate::{config::Config, engine::Engine};
use anyhow::{Context, Result};
use if_addrs::IfAddr;
use socket2::{Domain, Protocol, Socket, Type};
use std::{
    io::Read,
    net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4},
    time::Duration,
};
use tokio::{
    net::UdpSocket,
    task::JoinSet,
    time::{Instant, timeout_at},
};

const GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 89, 1);
const PORT: u16 = 9847;
const MAGIC: [u8; 8] = *b"YONIXD01";
const QUERY_LEN: usize = 24;
const REPLY_LEN: usize = QUERY_LEN + 2;
const MAX_RESULTS: usize = 16;
const WAIT: Duration = Duration::from_millis(700);

#[derive(Clone, Copy)]
struct Interface {
    ip: Ipv4Addr,
    mask: Ipv4Addr,
}

fn local_ip(ip: Ipv4Addr) -> bool {
    ip.is_loopback() || ip.is_private() || ip.is_link_local()
}

impl Interface {
    fn matches(self, ip: Ipv4Addr) -> bool {
        local_ip(ip)
            && self.ip.is_loopback() == ip.is_loopback()
            && u32::from(self.ip) & u32::from(self.mask) == u32::from(ip) & u32::from(self.mask)
    }
}

fn interfaces(config: &Config) -> Result<Vec<Interface>> {
    let IpAddr::V4(listen) = config.listen_address else {
        return Ok(Vec::new());
    };
    let mut interfaces = Vec::<Interface>::new();
    for entry in if_addrs::get_if_addrs()? {
        let IfAddr::V4(addr) = entry.addr else {
            continue;
        };
        if local_ip(addr.ip)
            && (listen.is_unspecified() || listen == addr.ip)
            && !interfaces.iter().any(|entry| entry.ip == addr.ip)
        {
            interfaces.push(Interface {
                ip: addr.ip,
                mask: addr.netmask,
            });
        }
    }
    // Linux routes the full 127/8 block to loopback even if the interface
    // only lists 127.0.0.1. Explicit listeners can use other loopback IPs.
    if listen.is_loopback() && interfaces.is_empty() {
        interfaces.push(Interface {
            ip: listen,
            mask: Ipv4Addr::new(255, 0, 0, 0),
        });
    }
    Ok(interfaces)
}

fn interface_socket(interface: Interface) -> Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_multicast_if_v4(&interface.ip)?;
    socket.set_multicast_ttl_v4(1)?;
    socket.set_multicast_loop_v4(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&SocketAddrV4::new(interface.ip, 0).into())?;
    Ok(UdpSocket::from_std(socket.into())?)
}

fn receiver_socket() -> Result<UdpSocket> {
    let receiver = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    receiver.set_reuse_address(true)?;
    receiver.set_nonblocking(true)?;
    receiver.bind(&SocketAddrV4::new(GROUP, PORT).into())?;
    Ok(UdpSocket::from_std(receiver.into())?)
}

async fn query_interface(
    interface: Interface,
    query: [u8; QUERY_LEN],
    deadline: Instant,
    own_port: u16,
    local_ips: Vec<Ipv4Addr>,
) -> Result<Vec<SocketAddr>> {
    let socket = interface_socket(interface)?;
    let receiver = receiver_socket()?;
    receiver.join_multicast_v4(GROUP, interface.ip)?;
    let mut results = Vec::new();
    let mut buffer = [0; REPLY_LEN + 1];
    let received = timeout_at(deadline, async {
        socket
            .send_to(&query, SocketAddrV4::new(GROUP, PORT))
            .await?;
        while results.len() < MAX_RESULTS {
            let (size, addr) = receiver.recv_from(&mut buffer).await?;
            let SocketAddr::V4(addr) = addr else {
                continue;
            };
            if size != REPLY_LEN || buffer[..QUERY_LEN] != query || !interface.matches(*addr.ip()) {
                continue;
            }
            let port = u16::from_be_bytes([buffer[QUERY_LEN], buffer[QUERY_LEN + 1]]);
            if port == 0 || (own_port == port && local_ips.contains(addr.ip())) {
                continue;
            }
            let candidate = SocketAddrV4::new(*addr.ip(), port).into();
            if !results.contains(&candidate) {
                results.push(candidate);
            }
        }
        Ok::<(), std::io::Error>(())
    })
    .await;
    if let Ok(result) = received {
        result?;
    }
    Ok(results)
}

pub async fn discover(config: &Config) -> Result<Vec<SocketAddr>> {
    let interfaces = interfaces(config)?;
    let local_ips: Vec<_> = interfaces.iter().map(|interface| interface.ip).collect();
    let mut query = [0; QUERY_LEN];
    query[..MAGIC.len()].copy_from_slice(&MAGIC);
    std::fs::File::open("/dev/urandom")?.read_exact(&mut query[MAGIC.len()..])?;
    let deadline = Instant::now() + WAIT;
    let mut tasks = JoinSet::new();
    for interface in interfaces {
        tasks.spawn(query_interface(
            interface,
            query,
            deadline,
            config.port,
            local_ips.clone(),
        ));
    }
    let mut results = Vec::new();
    let mut first_error = None;
    let mut any_success = false;
    while let Some(result) = tasks.join_next().await {
        match result? {
            Ok(candidates) => {
                any_success = true;
                for candidate in candidates {
                    if !results.contains(&candidate) {
                        results.push(candidate);
                    }
                    if results.len() == MAX_RESULTS {
                        return Ok(results);
                    }
                }
            }
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    if let Some(error) = first_error.filter(|_| !any_success) {
        return Err(error);
    }
    Ok(results)
}

pub async fn respond(engine: Engine) -> Result<()> {
    let interfaces = interfaces(&engine.config)?;
    if interfaces.is_empty() {
        return Ok(());
    }
    let receiver = receiver_socket()?;
    let mut responders = Vec::new();
    for interface in interfaces {
        let Ok(socket) = interface_socket(interface) else {
            continue;
        };
        if receiver.join_multicast_v4(GROUP, interface.ip).is_ok() {
            responders.push((interface, socket));
        }
    }
    anyhow::ensure!(
        !responders.is_empty(),
        "No local interface joined Yonix discovery"
    );
    let mut packet = [0; REPLY_LEN];
    loop {
        let (size, source) = receiver
            .recv_from(&mut packet)
            .await
            .context("Receive discovery query")?;
        let SocketAddr::V4(source) = source else {
            continue;
        };
        if size != QUERY_LEN || packet[..MAGIC.len()] != MAGIC || !engine.connections.has_invite() {
            continue;
        }
        let Some((_, socket)) = responders
            .iter()
            .filter(|(interface, _)| interface.matches(*source.ip()))
            .max_by_key(|(interface, _)| u32::from(interface.mask).count_ones())
        else {
            continue;
        };
        packet[QUERY_LEN..].copy_from_slice(&engine.config.port.to_be_bytes());
        let _ = socket.send_to(&packet, (GROUP, PORT)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interface_matches_only_local_ipv4_addresses_in_its_subnet() {
        for (address, mask, accepted, rejected) in [
            ("10.28.16.1", "255.255.240.0", "10.28.31.254", "10.28.32.1"),
            ("127.0.0.1", "255.0.0.0", "127.0.0.2", "10.0.0.1"),
            ("169.254.1.2", "255.255.0.0", "169.254.3.4", "169.253.1.2"),
        ] {
            let interface = Interface {
                ip: address.parse().unwrap(),
                mask: mask.parse().unwrap(),
            };
            assert!(interface.matches(accepted.parse().unwrap()), "{accepted}");
            assert!(!interface.matches(rejected.parse().unwrap()), "{rejected}");
        }
        // A broad netmask must not turn public or multicast addresses into peers.
        let interface = Interface {
            ip: Ipv4Addr::new(10, 0, 0, 1),
            mask: Ipv4Addr::UNSPECIFIED,
        };
        assert!(!interface.matches(Ipv4Addr::new(192, 0, 2, 1)));
        assert!(!interface.matches(GROUP));
        assert!(!interface.matches(Ipv4Addr::LOCALHOST));
    }

    #[tokio::test]
    async fn ipv6_listener_skips_ipv4_multicast_discovery() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            state_dir: dir.path().join("state"),
            host: "discovery-test".into(),
            listen_address: std::net::Ipv6Addr::LOCALHOST.into(),
            ..Config::default()
        };
        assert!(discover(&config).await.unwrap().is_empty());
        tokio::time::timeout(
            Duration::from_secs(1),
            respond(Engine::open(config).unwrap()),
        )
        .await
        .unwrap()
        .unwrap();
    }
}
