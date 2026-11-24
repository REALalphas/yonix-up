use crate::{
    config::Config,
    engine::{Action, Engine, Job},
};
use anyhow::Result;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Request, State},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

mod lan;

include!(concat!(env!("OUT_DIR"), "/web_assets.rs"));

#[derive(Clone)]
struct App {
    engine: Engine,
    token: Arc<String>,
    lan: Arc<lan::Handshakes>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Start {
    action: Action,
    #[serde(default)]
    confirmed: bool,
}
type ApiError = (StatusCode, Json<serde_json::Value>);
fn error(code: StatusCode, message: impl ToString) -> ApiError {
    (
        code,
        Json(serde_json::json!({"error": message.to_string()})),
    )
}

pub fn token_matches(expected: &str, supplied: &str) -> bool {
    if expected.len() != supplied.len() {
        return false;
    }
    expected
        .bytes()
        .zip(supplied.bytes())
        .fold(0u8, |d, (a, b)| d | (a ^ b))
        == 0
}

pub fn valid_host(host: &str, address: std::net::IpAddr) -> bool {
    // IPv6 Host headers arrive bracketed (`[::1]:9847`); strip the brackets
    // before comparing or parsing. A bare address without a port is rejected.
    host.rsplit_once(':').is_some_and(|(name, port)| {
        let name = name
            .strip_prefix('[')
            .and_then(|inner| inner.strip_suffix(']'))
            .unwrap_or(name);
        (matches!(name, "localhost" | "127.0.0.1" | "::1")
            || name == address.to_string()
            || (address.is_unspecified()
                && name.parse::<std::net::IpAddr>().is_ok_and(|ip| {
                    !ip.is_unspecified()
                        && !matches!(ip, std::net::IpAddr::V4(v4) if v4.is_multicast() || v4.is_broadcast())
                        && !matches!(ip, std::net::IpAddr::V6(v6) if v6.is_multicast())
                })))
            && port.parse::<u16>().is_ok_and(|p| p != 0)
    })
}

async fn protect(State(app): State<App>, request: Request, next: Next) -> Response {
    let headers = request.headers();
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if !valid_host(host, app.engine.config.listen_address) {
        return error(StatusCode::FORBIDDEN, "Use the configured server address").into_response();
    }
    if let Some(origin) = headers.get(header::ORIGIN) {
        if origin.to_str().ok() != Some(format!("http://{host}").as_str()) {
            return error(
                StatusCode::FORBIDDEN,
                "Cross-origin requests are not allowed",
            )
            .into_response();
        }
    }
    if (request.uri().path().starts_with("/api/hosts")
        || request.uri().path().starts_with("/api/connections"))
        && headers.contains_key("X-Yonix-Peer-Request")
    {
        return error(StatusCode::FORBIDDEN, "Peer forwarding is single-hop").into_response();
    }
    // The redeem endpoint authenticates by pairing code (see `redeem_pairing`),
    // never by the login token: the joiner proves the out-of-band secret
    // without knowing any token yet. Minting and accepting stay
    // token-authenticated (accept runs on the joiner daemon and needs its
    // private token).
    if request.uri().path().starts_with("/api/")
        && !matches!(
            request.uri().path(),
            "/api/pairings/redeem" | "/api/pairings/hello" | "/api/pairings/finish"
        )
    {
        let token = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .unwrap_or("");
        if !token_matches(&app.token, token) {
            return error(StatusCode::UNAUTHORIZED, "Access token required").into_response();
        }
    }
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    headers.insert("X-Content-Type-Options", "nosniff".parse().unwrap());
    headers.insert("Referrer-Policy", "no-referrer".parse().unwrap());
    headers.insert("Content-Security-Policy", "default-src 'none'; script-src 'self'; style-src 'self'; style-src-attr 'unsafe-inline'; font-src 'self'; connect-src 'self'; img-src 'self' data:; frame-ancestors 'none'; base-uri 'none'; form-action 'none'".parse().unwrap());
    response
}

pub fn router(engine: Engine, token: String) -> Router {
    let app = App {
        engine,
        token: Arc::new(token),
        lan: Arc::default(),
    };
    Router::new()
        .route("/api/pairings", get(pairing_status).post(mint_pairing))
        .route("/api/pairings/settings", post(update_pairing))
        .route("/api/pairings/revoke", post(revoke_pairing))
        .route("/api/pairings/hello", post(lan::hello))
        .route("/api/pairings/finish", post(lan::finish))
        .route("/api/pairings/redeem", post(redeem_pairing))
        .route("/api/pairings/accept", post(accept_pairing))
        .route("/api/connections", get(connections).post(add_connection))
        .route("/api/connections/check", post(check_connection))
        .route("/api/connections/{id}/check", post(check_saved_connection))
        .route("/api/connections/{id}/remove", post(remove_connection))
        .route("/api/hosts", get(hosts))
        .route("/api/hosts/{host}/status", get(host_status))
        .route("/api/hosts/{host}/jobs", get(host_jobs).post(host_start))
        .route("/api/hosts/{host}/jobs/{id}", get(host_job))
        .route("/api/hosts/{host}/jobs/{id}/log", get(host_log))
        .route(
            "/locales.js",
            get(|| async {
                (
                    [(
                        header::CONTENT_TYPE,
                        "application/javascript; charset=utf-8",
                    )],
                    crate::i18n::web_bundle(),
                )
            }),
        )
        .route(
            "/api/status",
            get(|State(app): State<App>| async move { Json(app.engine.status()) }),
        )
        .route(
            "/api/jobs",
            get(|State(app): State<App>| async move { Json(app.engine.jobs()) }).post(start),
        )
        .route(
            "/api/jobs/{id}",
            get(
                |State(app): State<App>, Path(id): Path<String>| async move {
                    app.engine
                        .job(&id)
                        .map(Json)
                        .ok_or_else(|| error(StatusCode::NOT_FOUND, "Unknown job"))
                },
            ),
        )
        .route(
            "/api/jobs/{id}/log",
            get(
                |State(app): State<App>, Path(id): Path<String>| async move {
                    app.engine
                        .log(&id)
                        .map_err(|e| error(StatusCode::NOT_FOUND, e))
                },
            ),
        )
        .fallback(get(|request: Request| async move {
            let path = request.uri().path().trim_start_matches('/');
            match web_asset(if path.is_empty() { "index.html" } else { path }) {
                Some((mime, bytes)) => ([(header::CONTENT_TYPE, mime)], bytes).into_response(),
                None => StatusCode::NOT_FOUND.into_response(),
            }
        }))
        .layer(DefaultBodyLimit::max(4096))
        .layer(middleware::from_fn_with_state(app.clone(), protect))
        .with_state(app)
}

async fn start(State(app): State<App>, Json(body): Json<Start>) -> Result<Json<Job>, ApiError> {
    if (body.action.privileged() || body.action == Action::UpdateInputs) && !body.confirmed {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "This operation requires explicit confirmation",
        ));
    }
    app.engine
        .start(body.action)
        .map(Json)
        .map_err(|e| error(StatusCode::CONFLICT, e))
}

pub async fn run(config: Config) -> Result<()> {
    let engine = Engine::open(config.clone())?;
    let token = crate::engine::ensure_token(&config)?;
    let listener = tokio::net::TcpListener::bind((config.listen_address, config.port)).await?;
    let discovery_engine = engine.clone();
    tokio::spawn(async move {
        if let Err(error) = crate::discovery::respond(discovery_engine).await {
            eprintln!("Yonix LAN discovery unavailable: {error:#}");
        }
    });
    let display = match config.listen_address {
        std::net::IpAddr::V4(address) => address.to_string(),
        std::net::IpAddr::V6(address) => format!("[{address}]"),
    };
    eprintln!("Yonix listening on http://{display}:{}", config.port);
    let restart_engine = engine.clone();
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let server = async move {
        axum::serve(
            listener,
            router(engine, token).into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.await;
        })
        .await
    };
    tokio::pin!(server);
    let target = tokio::select! {
        biased;
        _ = interrupt.recv() => None,
        _ = terminate.recv() => None,
        target = wait_for_restart(restart_engine) => Some(target),
        result = &mut server => return result.map_err(Into::into),
    };
    let _ = shutdown_tx.send(());
    if let Some(target) = target {
        // An explicit stop also takes precedence while requests are draining.
        tokio::select! {
            biased;
            _ = interrupt.recv() => {},
            _ = terminate.recv() => {},
            result = &mut server => {
                result?;
                return self_update(&target);
            }
        }
    }
    server.await?;
    Ok(())
}

async fn wait_for_restart(engine: Engine) -> std::path::PathBuf {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        if let Some(target) = engine.restart_target() {
            return target;
        }
    }
}

fn self_update(target: &std::path::Path) -> Result<()> {
    use std::os::unix::process::CommandExt;
    eprintln!("Re-executing from {}", target.display());
    let err = std::process::Command::new(target)
        .args(std::env::args_os().skip(1))
        .exec();
    Err(anyhow::anyhow!("Cannot exec {}: {err}", target.display()))
}

async fn hosts(State(app): State<App>) -> Json<Vec<crate::peers::Host>> {
    let mut hosts = vec![crate::peers::Host {
        id: "local".into(),
        label: app.engine.config.host.clone(),
        local: true,
    }];
    hosts.extend(
        app.engine
            .connections
            .list()
            .iter()
            .map(|peer| crate::peers::Host {
                id: peer.id.clone(),
                label: peer.label.clone(),
                local: false,
            }),
    );
    Json(hosts)
}

async fn host_status(State(app): State<App>, Path(host): Path<String>) -> Response {
    if host == "local" {
        return Json(app.engine.status()).into_response();
    }
    forward(app, host, "status".into(), None).await
}
async fn host_jobs(State(app): State<App>, Path(host): Path<String>) -> Response {
    if host == "local" {
        return Json(app.engine.jobs()).into_response();
    }
    forward(app, host, "jobs".into(), None).await
}
async fn host_start(
    State(app): State<App>,
    Path(host): Path<String>,
    Json(body): Json<Start>,
) -> Response {
    if host == "local" {
        return start(State(app), Json(body)).await.into_response();
    }
    forward(app, host, "jobs".into(), Some(body)).await
}
async fn host_job(State(app): State<App>, Path((host, id)): Path<(String, String)>) -> Response {
    if !valid_job_id(&id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    if host == "local" {
        return match app.engine.job(&id) {
            Some(job) => Json(job).into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        };
    }
    forward(app, host, format!("jobs/{id}"), None).await
}
async fn host_log(State(app): State<App>, Path((host, id)): Path<(String, String)>) -> Response {
    if !valid_job_id(&id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    if host == "local" {
        return match app.engine.log(&id) {
            Ok(log) => log.into_response(),
            Err(_) => StatusCode::NOT_FOUND.into_response(),
        };
    }
    forward(app, host, format!("jobs/{id}/log"), None).await
}
fn valid_job_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
async fn forward(app: App, host: String, path: String, body: Option<Start>) -> Response {
    let (peer_url, token) = match app.engine.connections.target(&host) {
        Ok(Some(target)) => target,
        Ok(None) => return error(StatusCode::NOT_FOUND, "Unknown host").into_response(),
        Err(_) => {
            return error(StatusCode::BAD_GATEWAY, "Cannot read host credentials").into_response();
        }
    };
    let result =
        tokio::task::spawn_blocking(move || -> anyhow::Result<(StatusCode, String, Vec<u8>)> {
            use std::io::Read;
            let http = reqwest::blocking::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(std::time::Duration::from_secs(2))
                .timeout(std::time::Duration::from_secs(6))
                .build()?;
            let url = format!("{}/api/{path}", peer_url.trim_end_matches('/'));
            let request = if let Some(body) = body {
                http.post(url).json(&body)
            } else {
                http.get(url)
            };
            let response = request
                .bearer_auth(token)
                .header("X-Yonix-Peer-Request", "1")
                .send()?;
            let status = response.status();
            if status.is_redirection() || matches!(status.as_u16(), 401 | 403) {
                anyhow::bail!("Peer refused connection");
            }
            let mime = if path.ends_with("/log") && status.is_success() {
                "text/plain; charset=utf-8"
            } else {
                "application/json"
            };
            let mut bytes = Vec::new();
            response.take(2 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
            if bytes.len() > 2 * 1024 * 1024 {
                anyhow::bail!("Peer response too large");
            }
            Ok((status, mime.into(), bytes))
        })
        .await;
    match result {
        Ok(Ok((status, mime, bytes))) => {
            (status, [(header::CONTENT_TYPE, mime)], bytes).into_response()
        }
        _ => error(
            StatusCode::BAD_GATEWAY,
            "Cannot reach host. Check its service, network and private token file.",
        )
        .into_response(),
    }
}

fn advertised_address(config: &Config, headers: &axum::http::HeaderMap) -> std::net::SocketAddr {
    use std::net::{IpAddr, SocketAddr};
    let port = config.port;
    if !config.listen_address.is_unspecified() {
        return SocketAddr::new(config.listen_address, port);
    }
    let family_v4 = config.listen_address.is_ipv4();
    let eligible = |addr: &IpAddr| {
        addr.is_ipv4() == family_v4
            && !addr.is_loopback()
            && crate::peers::validate_peer_url(&format!("http://{}", SocketAddr::new(*addr, port)))
                .is_ok()
    };
    let interfaces = if_addrs::get_if_addrs().unwrap_or_default();
    let mut addresses = interfaces.iter().map(|i| i.ip()).filter(eligible);
    let browser_host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<SocketAddr>().ok())
        .map(|s| s.ip());
    let ip = browser_host
        .filter(|ip| addresses.clone().any(|a| a == *ip))
        .or_else(|| addresses.next())
        .unwrap_or_else(|| {
            if family_v4 {
                std::net::Ipv4Addr::LOCALHOST.into()
            } else {
                std::net::Ipv6Addr::LOCALHOST.into()
            }
        });
    SocketAddr::new(ip, port)
}

/// Mint a one-time pairing code (token-authenticated). The response carries
/// the code plus a compact address link built from this
/// daemon's own reachable origin. The login token never leaves the daemon.
async fn mint_pairing(
    State(app): State<App>,
    headers: axum::http::HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (code, expires_in) = app
        .engine
        .connections
        .mint_pairing()
        .map_err(|e| error(StatusCode::CONFLICT, e))?;
    let config = &app.engine.config;
    let address = advertised_address(config, &headers);
    let link = crate::pairing::compact_link(address, &code)
        .map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(
        serde_json::json!({"code": code, "link": link, "expires_in": expires_in}),
    ))
}

async fn pairing_status(State(app): State<App>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "enabled": app.engine.connections.pairing_enabled(),
        "active": app.engine.connections.current_invite().is_some(),
    }))
}

#[derive(Deserialize)]
struct PairingSettingsRequest {
    enabled: bool,
}

async fn update_pairing(
    State(app): State<App>,
    Json(body): Json<PairingSettingsRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    app.engine
        .connections
        .set_pairing_enabled(body.enabled)
        .map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(serde_json::json!({ "enabled": body.enabled })))
}

async fn revoke_pairing(State(app): State<App>) -> Result<Json<serde_json::Value>, ApiError> {
    app.engine
        .connections
        .revoke_pairing()
        .map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(serde_json::json!({ "revoked": true })))
}

#[derive(Deserialize, Serialize)]
struct RedeemRequest {
    code: String,
    /// Joiner-side facts the initiator stores with its own connection entry.
    host: String,
    token: String,
    url: String,
}

#[derive(Deserialize, Serialize)]
struct RedeemResponse {
    host: String,
    version: String,
    token: String,
    url: String,
    pair_id: String,
}

/// Redeem a legacy invitation that was already outstanding before the upgrade.
async fn redeem_pairing(
    State(app): State<App>,
    Json(body): Json<RedeemRequest>,
) -> Result<Json<RedeemResponse>, ApiError> {
    app.lan.admit_attempt()?;
    complete_redeem(&app, body, None).map(Json)
}

fn complete_redeem(
    app: &App,
    body: RedeemRequest,
    invite_id: Option<&str>,
) -> Result<RedeemResponse, ApiError> {
    let code = crate::pairing::normalize_code(&body.code)
        .map_err(|e| error(StatusCode::BAD_REQUEST, e))?;
    let joiner_host = body.host.trim().to_string();
    if joiner_host.is_empty()
        || joiner_host.len() > 64
        || !joiner_host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        return Err(error(StatusCode::BAD_REQUEST, "Invalid joiner host label"));
    }
    if body.token.len() != 64 || !body.token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(error(StatusCode::BAD_REQUEST, "Invalid joiner token"));
    }
    let joiner_url = body.url.trim().trim_end_matches('/').to_string();
    crate::peers::validate_peer_url(&joiner_url).map_err(|e| error(StatusCode::BAD_REQUEST, e))?;
    let pair_id = format!(
        "pair-{}",
        now_hex_id().map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, e))?
    );
    let token = app
        .engine
        .connections
        .local_token()
        .map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let input = crate::peers::ConnectionInput {
        label: joiner_host,
        url: joiner_url,
        token: body.token,
        pair_id: Some(pair_id.clone()),
    };
    if !app
        .engine
        .connections
        .redeem_paired(&code, invite_id, input)
        .map_err(|e| error(StatusCode::CONFLICT, e))?
    {
        return Err(error(
            StatusCode::UNAUTHORIZED,
            "Unknown or expired pairing code",
        ));
    }
    let status = app.engine.status();
    Ok(RedeemResponse {
        host: status.host,
        version: status.version,
        token,
        url: app.engine.config.base_url(),
        pair_id,
    })
}

fn now_hex_id() -> anyhow::Result<String> {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Accept a pairing link on this daemon (the joiner side). Parses the link,
/// tries the initiator paths in order against `/api/pairings/redeem` on the
/// other host, and saves the mirrored connection here. Runs server-side
/// because it needs this daemon's private login token; the browser never
/// sees either token.
#[derive(Deserialize)]
struct AcceptRequest {
    link: String,
    origin: Option<String>,
    label: Option<String>,
    #[serde(default)]
    allow_public: bool,
}

async fn accept_pairing(
    State(app): State<App>,
    Json(body): Json<AcceptRequest>,
) -> Result<Json<crate::peers::Connection>, ApiError> {
    use crate::pairing::PathRole;
    let parsed =
        crate::pairing::parse_link(&body.link).map_err(|e| error(StatusCode::BAD_REQUEST, e))?;
    let mut paths: Vec<(String, PathRole)> = parsed
        .payload
        .paths
        .iter()
        .map(|path| (path.url.clone(), path.role))
        .collect();
    if paths.is_empty() {
        if let Some(origin) = body.origin.filter(|o| !o.trim().is_empty()) {
            match crate::pairing::validate_candidate(origin.trim(), PathRole::Direct) {
                Ok(()) => paths.push((origin.trim().to_string(), PathRole::Direct)),
                Err(e) => return Err(error(StatusCode::BAD_REQUEST, e)),
            }
        } else {
            return lan::accept(&app, &parsed.code, body.label).await;
        }
    }
    // Only unversioned, explicitly shared invitations from the old release
    // use its legacy HTTP exchange. All new routes require a pairing-code proof.
    if body.link.trim().starts_with(['~', '!']) || parsed.payload.paths.is_empty() {
        let mut candidates = Vec::new();
        for (url, role) in &paths {
            if *role == PathRole::Relay || (*role == PathRole::Public && !body.allow_public) {
                continue;
            }
            for origin in crate::pairing::resolve_checked(url, *role).unwrap_or_default() {
                let parsed =
                    reqwest::Url::parse(&origin).map_err(|e| error(StatusCode::BAD_REQUEST, e))?;
                if let Ok(addresses) = parsed.socket_addrs(|| Some(80)) {
                    for addr in addresses {
                        if candidates.len() < 16 && !candidates.contains(&addr) {
                            candidates.push(addr);
                        }
                    }
                }
            }
        }
        return lan::accept_candidates(&app, &parsed.code, body.label, candidates).await;
    }
    let own_token = app
        .engine
        .connections
        .local_token()
        .map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let own_base = app
        .engine
        .config
        .base_url()
        .trim_end_matches('/')
        .to_string();
    let mut failures: Vec<String> = Vec::new();
    for (url, role) in &paths {
        if *role == PathRole::Relay {
            failures.push(format!("{url}: relay paths are not dialed yet"));
            continue;
        }
        if *role == PathRole::Public && !body.allow_public {
            failures.push(format!("{url}: public path needs explicit confirmation"));
            continue;
        }
        let dial = match crate::pairing::resolve_checked(url, *role) {
            Ok(dial) => dial,
            Err(e) => {
                failures.push(format!("{url}: {e:#}"));
                continue;
            }
        };
        for candidate in dial {
            let reply = redeem_against(
                &candidate,
                &parsed.code,
                &app.engine.config.host,
                &own_token,
                &own_base,
            )
            .await;
            match reply {
                Ok(reply) => {
                    let name = body
                        .label
                        .clone()
                        .filter(|l| !l.trim().is_empty())
                        .unwrap_or_else(|| {
                            if reply.host.is_empty() {
                                "peer".into()
                            } else {
                                reply.host.clone()
                            }
                        });
                    let saved = app
                        .engine
                        .connections
                        .add(crate::peers::ConnectionInput {
                            label: name,
                            url: url.clone(),
                            token: reply.token,
                            pair_id: Some(reply.pair_id),
                        })
                        .map_err(|e| error(StatusCode::CONFLICT, e))?;
                    return Ok(Json(saved));
                }
                Err(message) => failures.push(format!("{candidate}: {message}")),
            }
        }
    }
    Err(error(
        StatusCode::BAD_GATEWAY,
        format!("No pairing path worked: {}", failures.join("; ")),
    ))
}

struct RedeemReply {
    host: String,
    token: String,
    pair_id: String,
}

async fn redeem_against(
    base: &str,
    code: &str,
    host: &str,
    token: &str,
    url: &str,
) -> Result<RedeemReply, String> {
    let (base, code, host, token, url) = (
        base.to_string(),
        code.to_string(),
        host.to_string(),
        token.to_string(),
        url.to_string(),
    );
    tokio::task::spawn_blocking(move || -> anyhow::Result<RedeemReply> {
        let http = reqwest::blocking::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(2))
            .timeout(std::time::Duration::from_secs(10))
            .build()?;
        let response = http
            .post(format!(
                "{}/api/pairings/redeem",
                base.trim_end_matches('/')
            ))
            .json(&serde_json::json!({
                "code": code,
                "host": host,
                "token": token,
                "url": url,
            }))
            .send()
            .map_err(|e| anyhow::anyhow!("{e:#}"))?;
        if !response.status().is_success() {
            let detail: serde_json::Value = response.json().unwrap_or(serde_json::Value::Null);
            let message = detail
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("Pairing host refused the redeem");
            anyhow::bail!("{message}");
        }
        let reply: serde_json::Value = response.json().map_err(|e| anyhow::anyhow!("{e:#}"))?;
        Ok(RedeemReply {
            host: reply
                .get("host")
                .and_then(|h| h.as_str())
                .unwrap_or_default()
                .into(),
            token: reply
                .get("token")
                .and_then(|t| t.as_str())
                .ok_or_else(|| anyhow::anyhow!("Pairing server did not return a token"))?
                .into(),
            pair_id: reply
                .get("pair_id")
                .and_then(|id| id.as_str())
                .ok_or_else(|| anyhow::anyhow!("Pairing server did not return a pair id"))?
                .into(),
        })
    })
    .await
    .map_err(|e| format!("{e:#}"))?
    .map_err(|e| format!("{e:#}"))
}

async fn connections(State(app): State<App>) -> Json<Vec<crate::peers::Connection>> {
    Json(app.engine.connections.list())
}

fn connection_input(
    app: &App,
    mut input: crate::peers::ConnectionInput,
) -> Result<crate::peers::ConnectionInput, ApiError> {
    input.label = input.label.trim().into();
    input.url = input.url.trim().trim_end_matches('/').into();
    input.token = input.token.trim().into();
    app.engine
        .connections
        .validate(&input)
        .map_err(|e| error(StatusCode::BAD_REQUEST, e))?;
    Ok(input)
}

#[derive(Serialize)]
struct ConnectionCheck {
    host: String,
    version: String,
}

async fn probe_connection(url: String, token: String) -> Result<Json<ConnectionCheck>, ApiError> {
    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<ConnectionCheck> {
        use std::io::Read;
        let http = reqwest::blocking::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(2))
            .timeout(std::time::Duration::from_secs(6))
            .build()?;
        let response = http
            .get(format!("{}/api/status", url.trim_end_matches('/')))
            .bearer_auth(token)
            .header("X-Yonix-Peer-Request", "1")
            .send()?;
        if matches!(response.status().as_u16(), 401 | 403) {
            anyhow::bail!("The host rejected the access token");
        }
        if !response.status().is_success() {
            anyhow::bail!("The host did not return a Yonix status");
        }
        let mut bytes = Vec::new();
        response.take(65537).read_to_end(&mut bytes)?;
        if bytes.len() > 65536 {
            anyhow::bail!("Host response is too large");
        }
        let status: crate::engine::Status = serde_json::from_slice(&bytes)?;
        Ok(ConnectionCheck {
            host: status.host,
            version: status.version,
        })
    })
    .await;
    match result {
        Ok(Ok(status)) => Ok(Json(status)),
        _ => Err(error(
            StatusCode::BAD_GATEWAY,
            "Connection failed. Check the host address, service and access token.",
        )),
    }
}

async fn check_connection(
    State(app): State<App>,
    Json(input): Json<crate::peers::ConnectionInput>,
) -> Result<Json<ConnectionCheck>, ApiError> {
    let input = connection_input(&app, input)?;
    probe_connection(input.url, input.token).await
}

async fn add_connection(
    State(app): State<App>,
    Json(input): Json<crate::peers::ConnectionInput>,
) -> Result<Json<crate::peers::Connection>, ApiError> {
    let input = connection_input(&app, input)?;
    let _ = probe_connection(input.url.clone(), input.token.clone()).await?;
    app.engine
        .connections
        .add(input)
        .map(Json)
        .map_err(|e| error(StatusCode::CONFLICT, e))
}

async fn check_saved_connection(
    State(app): State<App>,
    Path(id): Path<String>,
) -> Result<Json<ConnectionCheck>, ApiError> {
    let (url, token) = app
        .engine
        .connections
        .target(&id)
        .map_err(|_| error(StatusCode::BAD_GATEWAY, "Cannot read host credentials"))?
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "Unknown host"))?;
    probe_connection(url, token).await
}

async fn remove_connection(
    State(app): State<App>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    app.engine
        .connections
        .remove(&id)
        .map_err(|e| error(StatusCode::CONFLICT, e))?;
    Ok(Json(serde_json::json!({"removed": true})))
}
