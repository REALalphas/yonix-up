//! Explicitly configured, single-hop peer connections. Credentials never leave the daemon.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{net::IpAddr, os::unix::fs::PermissionsExt, path::PathBuf};

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
        validate_peer_url(&self.url)?;
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
/// Peer URLs stay conservative: numeric private/loopback/link-local IPv4 or
/// loopback/unique-local/link-local IPv6 HTTP origins. Public, DNS and relay
/// pairing paths widen reachability only through the pairing handshake, where
/// each resolved address is re-checked before dialing.
pub fn validate_peer_url(text: &str) -> Result<()> {
    let url = reqwest::Url::parse(text)?;
    if url.scheme() != "http"
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
    let name = url.host_str().context("Missing peer address")?;
    // `Url::host_str` keeps IPv6 brackets; strip them before `IpAddr` parsing.
    let numeric = name
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(name);
    let address: IpAddr = numeric
        .parse()
        .context("Peer URL requires a numeric private or loopback IP address")?;
    let allowed = match address {
        IpAddr::V4(address) => {
            address.is_loopback() || address.is_private() || address.is_link_local()
        }
        IpAddr::V6(address) => {
            address.is_loopback()
                || address.is_unique_local()
                || (address.segments()[0] & 0xffc0) == 0xfe80
        }
    };
    if !allowed {
        bail!(
            "Peer URL must be a private/loopback HTTP origin; use a trusted LAN/VPN or local SSH tunnel"
        );
    }
    Ok(())
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
    /// Stable pairing id for connections created via the pairing handshake.
    /// `None` for legacy manually added connections. The daemon accepts it
    /// only on exact re-pairings, never as a free-form client field.
    #[serde(default)]
    pub pair_id: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SavedConnection {
    id: String,
    label: String,
    url: String,
    token: String,
    /// Stable two-way pairing identifier. `None` for legacy manually added
    /// connections; `Some` for connections created via the pairing handshake.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pair_id: Option<String>,
    /// `true` while an unlink request from either side awaits the other side.
    /// Re-pairing with the same peer is blocked until it clears.
    #[serde(default, skip_serializing_if = "is_false")]
    unlink_pending: bool,
}

fn is_false(value: &bool) -> bool {
    !value
}

/// One outstanding pairing code minted by `POST /api/pairings`. The code is
/// separate from the login token: it authorizes a single redeem that swaps
/// freshly read login tokens, then burns itself.
#[derive(Clone, Deserialize, Serialize)]
struct PendingPairing {
    code: String,
    created: u64,
}

pub struct Connections {
    declared: Vec<Peer>,
    saved: std::sync::Mutex<Vec<SavedConnection>>,
    pending: std::sync::Mutex<Vec<PendingPairing>>,
    pairings_file: PathBuf,
    file: PathBuf,
    local_url: String,
    token_path: PathBuf,
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
        let pairings_file = config.state_dir.join("pairings.json");
        let pending: Vec<PendingPairing> = if pairings_file.exists() {
            let metadata = std::fs::metadata(&pairings_file)?;
            if !metadata.is_file() || metadata.permissions().mode() & 0o077 != 0 {
                bail!("Pairing state must be a private file");
            }
            let stored: Vec<PendingPairing> =
                serde_json::from_slice(&std::fs::read(&pairings_file)?)?;
            // Restarting the daemon must not resurrect spent or stale codes:
            // drop expired entries so only live codes survive.
            stored
                .into_iter()
                .filter(|pairing| !pairing_code_expired(pairing.created))
                .collect()
        } else {
            Vec::new()
        };
        let store = Self {
            declared: config.peers.clone(),
            saved: std::sync::Mutex::new(saved),
            pending: std::sync::Mutex::new(pending),
            pairings_file,
            file,
            local_url: config.base_url(),
            token_path: config.token_path(),
        };
        store.save_pending(&store.pending.lock().unwrap())?;
        let entries = store.saved.lock().unwrap();
        let mut ids = std::collections::HashSet::new();
        for entry in entries.iter() {
            store.validate(&ConnectionInput {
                label: entry.label.clone(),
                url: entry.url.clone(),
                token: entry.token.clone(),
                pair_id: entry.pair_id.clone(),
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
        if let Some(pair_id) = &input.pair_id {
            validate_pair_id(pair_id)?;
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

    /// Pairing codes live 10 minutes, burn on first successful redeem and
    /// never return the login token.
    pub fn mint_pairing(&self) -> Result<String> {
        let mut pending = self.pending.lock().unwrap();
        let live: Vec<PendingPairing> = pending
            .iter()
            .filter(|pairing| !pairing_code_expired(pairing.created))
            .cloned()
            .collect();
        *pending = live;
        if pending.len() >= 8 {
            bail!("Too many outstanding pairing codes; redeem or wait for expiry");
        }
        let code = random_code()?;
        pending.push(PendingPairing {
            code: code.clone(),
            created: now_secs(),
        });
        self.save_pending(&pending)?;
        Ok(code)
    }

    /// Claim a code minted by this daemon. Returns `true` on first use and
    /// burns the code; expired or unknown codes return `false`.
    pub fn redeem_local(&self, code: &str) -> Result<bool> {
        let mut pending = self.pending.lock().unwrap();
        let mut kept = Vec::with_capacity(pending.len());
        let mut redeemed = false;
        for pairing in pending.iter() {
            if pairing_code_expired(pairing.created) {
                continue;
            }
            if crate::server::token_matches(&pairing.code, code) {
                // Burn on first successful redeem: never reuse a code.
                redeemed = true;
                continue;
            }
            kept.push(pairing.clone());
        }
        if redeemed {
            *pending = kept;
            self.save_pending(&pending)?;
        } else if kept.len() != pending.len() {
            // Prune expired codes even on a miss so they cannot accumulate.
            *pending = kept;
            self.save_pending(&pending)?;
        }
        Ok(redeemed)
    }

    /// Save the acceptor side of a completed pairing. Blocks only exact
    /// re-pairing while an unlink is pending, not the whole peer address.
    pub fn add_paired(
        &self,
        label: String,
        url: String,
        token: String,
        pair_id: String,
    ) -> Result<Connection> {
        self.validate(&ConnectionInput {
            label: label.clone(),
            url: url.clone(),
            token: token.clone(),
            pair_id: Some(pair_id.clone()),
        })?;
        let mut saved = self.saved.lock().unwrap();
        if saved.len() + self.declared.len() >= 32 {
            bail!("At most 32 peers are supported");
        }
        let entry = SavedConnection {
            id: format!("pair-{}", random_id()?),
            label,
            url,
            token,
            pair_id: Some(pair_id),
            unlink_pending: false,
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

    /// Read this daemon's own login token for the initiator side of a redeem.
    pub fn local_token(&self) -> Result<String> {
        crate::engine::read_token_path(&self.token_path)
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
        if let Some(pair_id) = &input.pair_id {
            // Exact pair ids are daemon-minted during redeem; they disambiguate
            // re-pairing from adding a different host at the same address.
            if saved.iter().any(|peer| {
                peer.pair_id.as_deref() == Some(pair_id.as_str()) && peer.unlink_pending
            }) {
                bail!("This pairing is awaiting unlink confirmation; revert it first");
            }
        }
        let entry = SavedConnection {
            id: format!("peer-{}", random_id()?),
            label: input.label,
            url: input.url,
            token: input.token,
            pair_id: input.pair_id,
            unlink_pending: false,
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
        write_private_json(&self.file, entries)
    }

    fn save_pending(&self, entries: &[PendingPairing]) -> Result<()> {
        write_private_json(&self.pairings_file, entries)
    }
}

fn write_private_json(path: &PathBuf, value: &(impl serde::Serialize + ?Sized)) -> Result<()> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let temporary = path.with_extension(format!("{}.tmp", random_id()?));
    let result = (|| -> Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&serde_json::to_vec(value)?)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Pairing codes are one-time and short-lived: 10 minutes from minting.
/// Expiry is checked on load, on mint and on redeem; spent codes are dropped
/// immediately so a captured code cannot be replayed later.
fn pairing_code_expired(created: u64) -> bool {
    now_secs().saturating_sub(created) > 600
}

fn random_code() -> Result<String> {
    // Six decimal digits from rejection-sampled /dev/urandom bytes so every
    // 000000..=999999 value is equally likely.
    use std::io::Read;
    let mut digits = String::with_capacity(6);
    let mut byte = [0u8; 1];
    let mut random = std::fs::File::open("/dev/urandom")?;
    while digits.len() < 6 {
        random.read_exact(&mut byte)?;
        // 250 is the largest multiple of 10 below 256: values above it are
        // discarded to keep the digit distribution uniform.
        if byte[0] < 250 {
            digits.push((b'0' + byte[0] % 10) as char);
        }
    }
    Ok(digits)
}

fn validate_pair_id(pair_id: &str) -> Result<()> {
    if !pair_id.starts_with("pair-")
        || pair_id.len() != 37
        || !pair_id[5..].bytes().all(|b| b.is_ascii_hexdigit())
    {
        bail!("Invalid pairing id");
    }
    Ok(())
}
