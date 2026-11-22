//! Authenticate candidates before sending peer credentials. Discovery replies
//! supply addresses only. A successful SPAKE2 exchange identifies the invite.
use super::{ApiError, App, RedeemRequest, RedeemResponse, complete_redeem, error};
use crate::{
    config::Config,
    pairing_crypto::{self, Channel},
    peers::{Connection, ConnectionInput},
};
use anyhow::{Context, Result, ensure};
use axum::{Json, extract::State, http::StatusCode};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    net::{SocketAddr, UdpSocket},
    sync::Mutex,
    time::{Duration, Instant},
};

const WINDOW: Duration = Duration::from_secs(60);
const ATTEMPTS: usize = 12;

#[derive(Default)]
pub(super) struct Handshakes {
    attempts: Mutex<VecDeque<Instant>>,
    pending: Mutex<HashMap<String, Session>>,
}

struct Session {
    created: Instant,
    code: String,
    invite: String,
    channel: Channel,
}

impl Handshakes {
    pub(super) fn admit_attempt(&self) -> Result<(), ApiError> {
        let mut attempts = self.attempts.lock().unwrap();
        attempts.retain(|time| time.elapsed() < WINDOW);
        if attempts.len() >= ATTEMPTS {
            return Err(error(
                StatusCode::TOO_MANY_REQUESTS,
                "Too many pairing attempts. Try again in one minute.",
            ));
        }
        attempts.push_back(Instant::now());
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
pub(super) struct Hello {
    message: String,
}
#[derive(Serialize, Deserialize)]
pub(super) struct Challenge {
    invite: String,
    session: String,
    message: String,
    proof: String,
}
#[derive(Serialize, Deserialize)]
pub(super) struct Finish {
    session: String,
    sealed: String,
}
#[derive(Serialize, Deserialize)]
struct Credentials {
    host: String,
    url: String,
    token: String,
    target: SocketAddr,
}

fn invalid() -> ApiError {
    error(StatusCode::FORBIDDEN, "Invalid or expired pairing exchange")
}

pub(super) async fn hello(
    State(app): State<App>,
    Json(hello): Json<Hello>,
) -> Result<Json<Challenge>, ApiError> {
    let (code, invite) = app
        .engine
        .connections
        .current_invite()
        .ok_or_else(invalid)?;
    app.lan.admit_attempt()?;
    let (channel, message, proof) =
        pairing_crypto::start_share(&code, &hello.message, &invite).map_err(|_| invalid())?;
    let session = super::now_hex_id().map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let mut pending = app.lan.pending.lock().unwrap();
    pending.retain(|_, entry| entry.created.elapsed() < WINDOW);
    pending.insert(
        session.clone(),
        Session {
            created: Instant::now(),
            code,
            invite: invite.clone(),
            channel,
        },
    );
    Ok(Json(Challenge {
        invite,
        session,
        message,
        proof,
    }))
}

fn serves(config: &Config, addr: SocketAddr) -> bool {
    if config.port != addr.port() {
        return false;
    }
    if config.listen_address == addr.ip() {
        return true;
    }
    config.listen_address.is_unspecified()
        && config.listen_address.is_ipv4() == addr.is_ipv4()
        && if_addrs::get_if_addrs().is_ok_and(|ifs| ifs.iter().any(|iface| iface.ip() == addr.ip()))
}

pub(super) async fn finish(
    State(app): State<App>,
    axum::extract::ConnectInfo(remote): axum::extract::ConnectInfo<SocketAddr>,
    Json(body): Json<Finish>,
) -> Result<Json<String>, ApiError> {
    let mut session = app
        .lan
        .pending
        .lock()
        .unwrap()
        .remove(&body.session)
        .ok_or_else(invalid)?;
    if session.created.elapsed() >= WINDOW {
        return Err(invalid());
    }
    let bytes = session
        .channel
        .open_request(&body.sealed)
        .map_err(|_| invalid())?;
    let creds: Credentials = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    // Bind the saved address to the authenticated host, even if a discovery
    // response was forwarded through a different listener on the LAN.
    let callback = creds
        .url
        .strip_prefix("http://")
        .and_then(|s| s.parse::<SocketAddr>().ok())
        .ok_or_else(invalid)?;
    if callback.ip() != remote.ip() || !serves(&app.engine.config, creds.target) {
        return Err(invalid());
    }
    let mut reply = complete_redeem(
        &app,
        RedeemRequest {
            code: session.code,
            host: creds.host,
            url: creds.url,
            token: creds.token,
        },
        Some(&session.invite),
    )?;
    reply.url = format!("http://{}", creds.target);
    let payload =
        serde_json::to_vec(&reply).map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    session
        .channel
        .seal_reply(&payload)
        .map(Json)
        .map_err(|_| invalid())
}

pub(super) fn callback(config: &Config, peer: SocketAddr) -> Result<String> {
    let address = if config.listen_address.is_unspecified() {
        let route = UdpSocket::bind(SocketAddr::new(config.listen_address, 0))?;
        route.connect(peer)?;
        route.local_addr()?.ip()
    } else {
        config.listen_address
    };
    ensure!(
        peer.ip().is_loopback() == address.is_loopback(),
        "The hosts need mutually reachable addresses"
    );
    let url = format!("http://{}", SocketAddr::new(address, config.port));
    crate::peers::validate_peer_url(&url)?;
    Ok(url)
}

fn http_client(listen: std::net::IpAddr) -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .local_address(listen)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(3))
        .build()?)
}

// Bound unauthenticated replies before decoding or deriving keys.
async fn exchange<T: serde::de::DeserializeOwned>(
    request: reqwest::RequestBuilder,
    body: &impl Serialize,
) -> Result<T> {
    let mut response = request.json(body).send().await?.error_for_status()?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            bytes.len() + chunk.len() <= 4096,
            "Oversize pairing response"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(serde_json::from_slice(&bytes)?)
}

async fn identify(
    http: reqwest::Client,
    peer: SocketAddr,
    code: String,
) -> Result<(String, SocketAddr, String, Channel)> {
    let (join, message) = pairing_crypto::start_join(&code)?;
    let answer: Challenge = exchange(
        http.post(format!("http://{peer}/api/pairings/hello")),
        &Hello { message },
    )
    .await?;
    let channel = join.finish(&answer.message, &answer.proof, &answer.invite)?;
    Ok((answer.invite, peer, answer.session, channel))
}

pub(super) async fn accept(
    app: &App,
    code: &str,
    label: Option<String>,
) -> Result<Json<Connection>, ApiError> {
    let candidates = crate::discovery::discover(&app.engine.config)
        .await
        .map_err(|e| error(StatusCode::BAD_GATEWAY, e))?;
    accept_candidates(app, code, label, candidates).await
}

pub(super) async fn accept_candidates(
    app: &App,
    code: &str,
    label: Option<String>,
    candidates: Vec<SocketAddr>,
) -> Result<Json<Connection>, ApiError> {
    let result: anyhow::Result<_> = async {
        let http = http_client(app.engine.config.listen_address)?;
        let mut probes = tokio::task::JoinSet::new();
        for peer in candidates {
            if !serves(&app.engine.config, peer) {
                probes.spawn(identify(http.clone(), peer, code.to_owned()));
            }
        }
        let mut matching: Option<(String, SocketAddr, String, Channel)> = None;
        while let Some(attempt) = probes.join_next().await {
            if let Ok(found) = attempt? {
                ensure!(
                    matching
                        .as_ref()
                        .is_none_or(|previous| previous.0 == found.0),
                    "Multiple hosts have this code. Paste the host's pairing link."
                );
                matching = Some(found);
            }
        }
        let (_, target, session, mut channel) = matching.context(
            "No reachable host has this code. Check the code or paste the pairing link.",
        )?;
        let peer_url = format!("http://{target}");
        let entries = app.engine.connections.list();
        ensure!(entries.len() < 32, "At most 32 peers are supported");
        ensure!(
            !entries
                .iter()
                .any(|entry| entry.url.trim_end_matches('/') == peer_url),
            "This host is already connected"
        );
        ensure!(
            label.as_ref().is_none_or(|s| s.trim().len() <= 128),
            "Invalid peer label"
        );
        let credentials = Credentials {
            host: app.engine.config.host.clone(),
            url: callback(&app.engine.config, target)?,
            token: app.engine.connections.local_token()?,
            target,
        };
        let sealed = channel.seal_request(&serde_json::to_vec(&credentials)?)?;
        let wire: String = exchange(
            // The authenticated host flushes its pairing journal and peer
            // credentials to disk before replying.
            http.post(format!("{peer_url}/api/pairings/finish"))
                .timeout(Duration::from_secs(6)),
            &Finish { session, sealed },
        )
        .await?;
        let reply: RedeemResponse = serde_json::from_slice(&channel.open_reply(&wire)?)?;
        ensure!(
            reply.url == peer_url,
            "Pairing host address changed during authentication"
        );
        app.engine.connections.add(ConnectionInput {
            label: label
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .unwrap_or(reply.host),
            url: peer_url,
            token: reply.token,
            pair_id: Some(reply.pair_id),
        })
    }
    .await;
    result
        .map(Json)
        .map_err(|e| error(StatusCode::BAD_GATEWAY, e))
}
