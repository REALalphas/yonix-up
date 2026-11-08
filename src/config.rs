use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    net::{IpAddr, Ipv4Addr},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub config_path: PathBuf,
    pub host: String,
    pub source_mode: String,
    pub port: u16,
    pub listen_address: Ipv4Addr,
    pub allow_public_bind: bool,
    pub state_dir: PathBuf,
    pub privileged_helper: Option<PathBuf>,
    pub peers: Vec<crate::peers::Peer>,
    pub cache: Option<Cache>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Cache {
    pub url: String,
    pub public_key_file: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        let state = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/state"));
        Self {
            config_path: PathBuf::from("/etc/nixos"),
            host: fs::read_to_string("/proc/sys/kernel/hostname")
                .unwrap_or_else(|_| "nixos".into())
                .trim()
                .into(),
            source_mode: "git".into(),
            port: 9847,
            listen_address: Ipv4Addr::LOCALHOST,
            allow_public_bind: false,
            state_dir: state.join("yonix"),
            privileged_helper: None,
            peers: Vec::new(),
            cache: None,
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let config: Self = serde_json::from_slice(&fs::read(path).with_context(|| {
            format!(
                "Cannot read {}. Install the NixOS module or pass --config.",
                path.display()
            )
        })?)?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        if !self.config_path.is_absolute() || !self.state_dir.is_absolute() {
            bail!("config_path and state_dir must be absolute paths");
        }
        if self.host.is_empty()
            || !self
                .host
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "_-".contains(c))
        {
            bail!("host may only contain ASCII letters, digits, '-' and '_'");
        }
        if !matches!(self.source_mode.as_str(), "git" | "path") {
            bail!("source_mode must be 'git' or 'path'");
        }
        // 0.0.0.0 is neither loopback nor private, so wildcard/public
        // binds land here and need the explicit opt-in. Bearer-token auth
        // is then the only gate — enable only behind a trusted firewall/VPN.
        if !self.listen_address.is_loopback()
            && !self.listen_address.is_private()
            && !self.allow_public_bind
        {
            bail!("wildcard/public listen_address needs allow_public_bind = true");
        }
        if self.port == 0 {
            bail!("port must not be zero");
        }
        if self
            .privileged_helper
            .as_ref()
            .is_some_and(|p| !p.is_absolute())
        {
            bail!("privileged_helper must be an absolute path");
        }
        if self
            .cache
            .as_ref()
            .is_some_and(|cache| !cache.public_key_file.is_absolute())
        {
            bail!("cache public_key_file must be absolute");
        }
        let mut ids = std::collections::HashSet::new();
        if self.peers.len() > 32 {
            bail!("At most 32 peers are supported");
        }
        for peer in &self.peers {
            peer.validate()?;
            if !ids.insert(&peer.id) {
                bail!("Duplicate peer id");
            }
            if peer.url.trim_end_matches('/') == self.base_url() {
                bail!("A peer cannot point to this instance");
            }
        }
        Ok(())
    }

    pub fn base_url(&self) -> String {
        format!("http://{}:{}", self.listen_address, self.port)
    }
    pub fn flake_source(&self) -> String {
        format!(
            "{}{}",
            if self.source_mode == "path" {
                "path:"
            } else {
                ""
            },
            self.config_path.display()
        )
    }
    pub fn token_path(&self) -> PathBuf {
        self.state_dir.join("token")
    }
}

pub fn validate_base_url(url: &str) -> Result<()> {
    let parsed = reqwest::Url::parse(url)?;
    let host: IpAddr = parsed
        .host_str()
        .context("Missing host")?
        .parse()
        .context("Use a numeric loopback address and an SSH tunnel")?;
    if parsed.scheme() != "http"
        || !host.is_loopback()
        || parsed.username() != ""
        || parsed.password().is_some()
        || parsed.path() != "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        bail!("Only an HTTP loopback endpoint is supported; use an SSH tunnel for remote access");
    }
    Ok(())
}
