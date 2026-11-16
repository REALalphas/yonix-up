//! Pairing links: `base64url(payload)#PAIRING_CODE`.
//!
//! The payload carries the initiator's reachable paths; the code after `#`
//! is a one-time pairing code (never the login token). Everything before
//! `#` decodes to JSON:
//!
//! ```json
//! {"v":1,"host":"nas","paths":[{"url":"...","role":"direct"}]}
//! ```
//!
//! A bare code (no `#`, no payload) is also accepted: the operator then
//! types the initiator's address by hand (manual address entry).
//! Self-contained base64url encode/decode, no extra crates.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, ToSocketAddrs};

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Roles order payload paths by reachability preference. The acceptor tries
/// them in order and uses the first working one (priority + fallback).
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PathRole {
    /// Numeric IP suitable for a direct connection.
    Direct,
    /// DNS name; resolved by the acceptor, every result checked by policy.
    Dns,
    /// Loopback origin via a pre-established SSH tunnel (no SSH automation).
    Ssh,
    /// Global address; allowed but must be confirmed explicitly as untrusted.
    Public,
    /// Via a third-party peer. Parsed and shown, but traffic use requires a
    /// separate secure review of relaying; until then it is skipped.
    Relay,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PathCandidate {
    pub url: String,
    pub role: PathRole,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PairPayload {
    pub v: u32,
    pub host: String,
    #[serde(default)]
    pub paths: Vec<PathCandidate>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PairLink {
    pub payload: PairPayload,
    pub code: String,
}

/// Encode bytes as base64url without padding.
pub fn encode_b64url(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16
            | (chunk.get(1).copied().unwrap_or(0) as u32) << 8
            | chunk.get(2).copied().unwrap_or(0) as u32;
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[(n >> 6) as usize & 63] as char);
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[n as usize & 63] as char);
        }
    }
    out
}

/// Decode unpadded base64url. Rejects `=` padding and foreign alphabets.
pub fn decode_b64url(text: &str) -> Result<Vec<u8>> {
    if text.is_empty() {
        bail!("Empty pairing payload");
    }
    if text.bytes().any(|b| b == b'=') {
        bail!("Pairing payload must be unpadded base64url");
    }
    let mut sextets = Vec::with_capacity(text.len());
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => bail!("Invalid character in pairing payload"),
        };
        sextets.push(value as u32);
    }
    if sextets.len() % 4 == 1 {
        bail!("Invalid pairing payload length");
    }
    let mut out = Vec::with_capacity(sextets.len() / 4 * 3);
    for chunk in sextets.chunks(4) {
        let n = (chunk[0] << 18)
            | (chunk[1] << 12)
            | (chunk.get(2).copied().unwrap_or(0) << 6)
            | chunk.get(3).copied().unwrap_or(0);
        out.push((n >> 16) as u8);
        if chunk.len() > 2 {
            out.push((n >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(n as u8);
        }
    }
    Ok(out)
}

/// Build `base64url(payload)#code`. The payload must already be validated
/// (see [`validate_payload`]) so generated links never carry bad paths.
pub fn encode_link(payload: &PairPayload, code: &str) -> Result<String> {
    validate_code(code)?;
    validate_payload(payload)?;
    Ok(format!(
        "{}#{}",
        encode_b64url(&serde_json::to_vec(payload)?),
        code
    ))
}

/// Parse either a full link or a bare code. A bare code has no payload:
/// the caller must ask the operator for the initiator's address by hand.
pub fn parse_link(text: &str) -> Result<PairLink> {
    let text = text.trim();
    if text.is_empty() {
        bail!("Enter a pairing link or code");
    }
    match text.split_once('#') {
        Some((head, code)) => {
            let bytes = decode_b64url(head.trim()).context("Invalid pairing link")?;
            let payload: PairPayload =
                serde_json::from_slice(&bytes).context("Invalid pairing link payload")?;
            validate_code(code.trim())?;
            validate_payload(&payload)?;
            Ok(PairLink {
                payload,
                code: code.trim().into(),
            })
        }
        None => {
            validate_code(text)?;
            Ok(PairLink {
                payload: PairPayload {
                    v: 1,
                    host: String::new(),
                    paths: Vec::new(),
                },
                code: text.into(),
            })
        }
    }
}

/// A short pairing code: exactly 6 ASCII digits, easy to read aloud or type.
pub fn validate_code(code: &str) -> Result<()> {
    if code.len() != 6 || !code.bytes().all(|b| b.is_ascii_digit()) {
        bail!("Pairing code must be 6 digits");
    }
    Ok(())
}

pub fn validate_payload(payload: &PairPayload) -> Result<()> {
    if payload.v != 1 {
        bail!("Unsupported pairing payload version {}", payload.v);
    }
    if payload.paths.len() > 16 {
        bail!("At most 16 pairing paths are supported");
    }
    if payload.host.len() > 64
        || (!payload.host.is_empty()
            && !payload
                .host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)))
    {
        bail!("Invalid pairing host label");
    }
    for path in &payload.paths {
        validate_candidate(&path.url, path.role)?;
    }
    Ok(())
}

/// Policy for a single path URL. Direct paths accept numeric loopback,
/// private, ULA and link-local IPs. DNS names are accepted syntactically;
/// every resolved address is re-checked by this policy before dialing
/// (see `resolve_checked`). Public IPs parse but are flagged by role.
pub fn validate_candidate(url: &str, role: PathRole) -> Result<()> {
    let parsed = reqwest::Url::parse(url).context("Invalid pairing path URL")?;
    if parsed.scheme() != "http"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.path() != "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.port_or_known_default().is_none_or(|p| p == 0)
    {
        bail!("Pairing path must be an http origin with a non-zero port");
    }
    let name = parsed.host_str().context("Missing pairing path host")?;
    // `Url::host_str` keeps IPv6 brackets; strip them before `IpAddr` parsing.
    let numeric_host = name
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(name);
    match numeric_host.parse::<IpAddr>() {
        Ok(IpAddr::V4(address)) => {
            let numeric = address.is_loopback() || address.is_private() || address.is_link_local();
            match role {
                PathRole::Direct => {
                    if !numeric {
                        bail!(
                            "Direct paths require a private, loopback or link-local IPv4 address"
                        );
                    }
                }
                PathRole::Ssh => {
                    if !address.is_loopback() {
                        bail!(
                            "SSH paths must use a loopback address with a pre-established tunnel"
                        );
                    }
                }
                PathRole::Public | PathRole::Dns | PathRole::Relay => {}
            }
        }
        Ok(IpAddr::V6(address)) => {
            // `is_global` is unstable; treat unique-local, link-local and
            // loopback as directly dialable, everything else as public.
            let numeric = address.is_loopback()
                || address.is_unique_local()
                || (address.segments()[0] & 0xffc0) == 0xfe80;
            match role {
                PathRole::Direct => {
                    if !numeric {
                        bail!(
                            "Direct paths require a loopback, unique-local or link-local IPv6 address"
                        );
                    }
                }
                PathRole::Ssh => {
                    if !address.is_loopback() {
                        bail!(
                            "SSH paths must use a loopback address with a pre-established tunnel"
                        );
                    }
                }
                PathRole::Public | PathRole::Dns | PathRole::Relay => {}
            }
        }
        Err(_) => {
            if role != PathRole::Dns {
                bail!("Only DNS-role paths may use a hostname; use a numeric IP otherwise");
            }
            // A bracketed literal that failed to parse is not a DNS name.
            if name.starts_with('[') {
                bail!("Invalid IPv6 literal in pairing path");
            }
            if name.len() > 253
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-.".contains(&b))
                || name.starts_with('-')
                || name.starts_with('.')
                || name.ends_with('-')
                || name.ends_with('.')
                || name.contains("..")
            {
                bail!("Invalid DNS name in pairing path");
            }
        }
    }
    Ok(())
}

/// Resolve a DNS-role path and re-check every address against the pairing
/// policy before dialing. Hostnames are never dialed blindly: each resolved
/// IP must satisfy the same per-role rules as a numeric path, so a hostile
/// DNS answer cannot smuggle the acceptor onto an unexpected network.
pub fn resolve_checked(url: &str, role: PathRole) -> Result<Vec<String>> {
    let parsed = reqwest::Url::parse(url).context("Invalid pairing path URL")?;
    validate_candidate(url, role)?;
    let port = parsed
        .port_or_known_default()
        .context("Missing pairing path port")?;
    let host = parsed.host_str().context("Missing pairing path host")?;
    // Numeric hosts need no DNS: return the origin itself.
    let numeric = host
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(host);
    if numeric.parse::<IpAddr>().is_ok() {
        return Ok(vec![url.to_string()]);
    }
    if role != PathRole::Dns {
        bail!("Only DNS-role paths may use a hostname; use a numeric IP otherwise");
    }
    let mut dialable: Vec<String> = Vec::new();
    for resolved in (host, port).to_socket_addrs()?.map(|addr| addr.ip()) {
        let candidate = match resolved {
            IpAddr::V4(address) => format!("http://{address}:{port}/"),
            IpAddr::V6(address) => format!("http://[{address}]:{port}/"),
        };
        // Untrusted DNS must never widen what a numeric path would allow.
        if validate_candidate(&candidate, role).is_ok() {
            dialable.push(candidate);
        }
    }
    if dialable.is_empty() {
        bail!("DNS name resolved to no dialable addresses");
    }
    dialable.sort();
    dialable.dedup();
    dialable.truncate(16);
    Ok(dialable)
}
