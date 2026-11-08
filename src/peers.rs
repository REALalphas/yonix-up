//! Explicitly configured, single-hop peer connections. Credentials never leave the daemon.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{net::Ipv4Addr, os::unix::fs::PermissionsExt, path::PathBuf};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Peer {
    pub id: String,
    pub label: String,
    pub url: String,
    pub token_file: PathBuf,
}
impl Peer {
    pub fn validate(&self) -> Result<()> {
        if self.id == "local"
            || self.id.is_empty()
            || self.id.len() > 64
            || !self
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        {
            bail!("Invalid peer id (local is reserved)");
        }
        if self.label.trim().is_empty() || self.label.len() > 128 {
            bail!("Invalid peer label");
        }
        if !self.token_file.is_absolute() {
            bail!("Peer token_file must be absolute");
        }
        let url = reqwest::Url::parse(&self.url)?;
        let address: Ipv4Addr = url
            .host_str()
            .context("Missing peer address")?
            .parse()
            .context("Peer URL requires a numeric private or loopback IPv4 address")?;
        if url.scheme() != "http"
            || !(address.is_loopback() || address.is_private())
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || url.port_or_known_default() == Some(0)
        {
            bail!(
                "Peer URL must be a private/loopback HTTP origin; use a trusted LAN/VPN or local SSH tunnel"
            );
        }
        Ok(())
    }
    pub fn token(&self) -> Result<String> {
        let metadata = std::fs::metadata(&self.token_file)?;
        if !metadata.is_file() || metadata.permissions().mode() & 0o077 != 0 {
            bail!("Peer token file must be private (0600 or 0400)");
        }
        let value = std::fs::read_to_string(&self.token_file)?.trim().to_owned();
        if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("Invalid peer token file");
        }
        Ok(value)
    }
}
#[derive(Serialize)]
pub struct Host {
    pub id: String,
    pub label: String,
    pub local: bool,
}

/// Public connection details. Credentials are only serialized to the private state file.
#[derive(Clone, Serialize)]
pub struct Connection {
    pub id: String,
    pub label: String,
    pub url: String,
    pub managed_by_nix: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionInput {
    pub label: String,
    pub url: String,
    pub token: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SavedConnection {
    id: String,
    label: String,
    url: String,
    token: String,
}

pub struct Connections {
    declared: Vec<Peer>,
    saved: std::sync::Mutex<Vec<SavedConnection>>,
    file: PathBuf,
    local_url: String,
}

fn random_id() -> Result<String> {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

impl Connections {
    pub fn open(config: &crate::config::Config) -> Result<Self> {
        let file = config.state_dir.join("connections.json");
        let saved: Vec<SavedConnection> = if file.exists() {
            let metadata = std::fs::metadata(&file)?;
            if !metadata.is_file()
                || metadata.permissions().mode() & 0o077 != 0
                || metadata.len() > 65536
            {
                bail!("Connection state must be a private file of at most 64 KiB");
            }
            serde_json::from_slice(&std::fs::read(&file)?)?
        } else {
            Vec::new()
        };
        if saved.len() + config.peers.len() > 32 {
            bail!("At most 32 peers are supported");
        }
        let store = Self {
            declared: config.peers.clone(),
            saved: std::sync::Mutex::new(saved),
            file,
            local_url: config.base_url(),
        };
        let entries = store.saved.lock().unwrap();
        let mut ids = std::collections::HashSet::new();
        for entry in entries.iter() {
            store.validate(&ConnectionInput {
                label: entry.label.clone(),
                url: entry.url.clone(),
                token: entry.token.clone(),
            })?;
            Peer {
                id: entry.id.clone(),
                label: entry.label.clone(),
                url: entry.url.clone(),
                token_file: store.file.clone(),
            }
            .validate()?;
            if !ids.insert(&entry.id) {
                bail!("Duplicate saved connection id");
            }
        }
        drop(entries);
        Ok(store)
    }

    pub fn validate(&self, input: &ConnectionInput) -> Result<()> {
        Peer {
            id: "candidate".into(),
            label: input.label.clone(),
            url: input.url.clone(),
            token_file: self.file.clone(),
        }
        .validate()?;
        if input.token.len() != 64 || !input.token.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!("Enter the 64-character access token from the other Yonix host");
        }
        if input.url.trim_end_matches('/') == self.local_url {
            bail!("This instance is already available as the local host");
        }
        Ok(())
    }

    pub fn list(&self) -> Vec<Connection> {
        let mut entries: Vec<_> = self
            .declared
            .iter()
            .map(|peer| Connection {
                id: peer.id.clone(),
                label: peer.label.clone(),
                url: peer.url.clone(),
                managed_by_nix: true,
            })
            .collect();
        entries.extend(
            self.saved
                .lock()
                .unwrap()
                .iter()
                .filter(|saved| !self.declared.iter().any(|peer| peer.id == saved.id))
                .map(|peer| Connection {
                    id: peer.id.clone(),
                    label: peer.label.clone(),
                    url: peer.url.clone(),
                    managed_by_nix: false,
                }),
        );
        entries
    }

    pub fn target(&self, id: &str) -> Result<Option<(String, String)>> {
        if let Some(peer) = self.declared.iter().find(|peer| peer.id == id) {
            return Ok(Some((peer.url.clone(), peer.token()?)));
        }
        Ok(self
            .saved
            .lock()
            .unwrap()
            .iter()
            .find(|peer| peer.id == id)
            .map(|peer| (peer.url.clone(), peer.token.clone())))
    }

    pub fn add(&self, input: ConnectionInput) -> Result<Connection> {
        self.validate(&input)?;
        let mut saved = self.saved.lock().unwrap();
        if saved.len() + self.declared.len() >= 32 {
            bail!("At most 32 peers are supported");
        }
        if self
            .declared
            .iter()
            .any(|p| p.url.trim_end_matches('/') == input.url.trim_end_matches('/'))
            || saved
                .iter()
                .any(|p| p.url.trim_end_matches('/') == input.url.trim_end_matches('/'))
        {
            bail!("This host is already connected");
        }
        let entry = SavedConnection {
            id: format!("peer-{}", random_id()?),
            label: input.label,
            url: input.url,
            token: input.token,
        };
        let view = Connection {
            id: entry.id.clone(),
            label: entry.label.clone(),
            url: entry.url.clone(),
            managed_by_nix: false,
        };
        let mut next = saved.clone();
        next.push(entry);
        self.save(&next)?;
        *saved = next;
        Ok(view)
    }

    pub fn remove(&self, id: &str) -> Result<()> {
        if self.declared.iter().any(|p| p.id == id) {
            bail!("This connection is managed by Nix; remove it from programs.yonix.peers");
        }
        let mut saved = self.saved.lock().unwrap();
        let next: Vec<_> = saved.iter().filter(|p| p.id != id).cloned().collect();
        if next.len() == saved.len() {
            bail!("Unknown connection");
        }
        self.save(&next)?;
        *saved = next;
        Ok(())
    }

    fn save(&self, entries: &[SavedConnection]) -> Result<()> {
        use std::{io::Write, os::unix::fs::OpenOptionsExt};
        let temporary = self.file.with_extension(format!("{}.tmp", random_id()?));
        let result = (|| -> Result<()> {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)?;
            file.write_all(&serde_json::to_vec(entries)?)?;
            file.sync_all()?;
            std::fs::rename(&temporary, &self.file)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temporary);
        }
        result
    }
}
