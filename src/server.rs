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

include!(concat!(env!("OUT_DIR"), "/web_assets.rs"));

#[derive(Clone)]
struct App {
    engine: Engine,
    token: Arc<String>,
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
    // without knowing any token yet. Minting stays token-authenticated.
    if request.uri().path().starts_with("/api/") && request.uri().path() != "/api/pairings/redeem" {
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
    };
    Router::new()
        .route("/api/pairings", post(mint_pairing))
        .route("/api/pairings/redeem", post(redeem_pairing))
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
    let display = match config.listen_address {
        std::net::IpAddr::V4(address) => address.to_string(),
        std::net::IpAddr::V6(address) => format!("[{address}]"),
    };
    eprintln!("Yonix listening on http://{display}:{}", config.port);
    axum::serve(listener, router(engine, token))
        .with_graceful_shutdown(async {
            let mut terminate =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("signal handler");
            tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
        })
        .await?;
    Ok(())
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

/// Mint a one-time pairing code (token-authenticated). The response carries
/// only the code: the caller builds `base64url(payload)#code` locally, so the
/// login token never leaves the daemon.
async fn mint_pairing(State(app): State<App>) -> Result<Json<serde_json::Value>, ApiError> {
    let code = app
        .engine
        .connections
        .mint_pairing()
        .map_err(|e| error(StatusCode::CONFLICT, e))?;
    Ok(Json(serde_json::json!({"code": code})))
}

#[derive(Deserialize)]
struct RedeemRequest {
    code: String,
    /// Joiner-side facts the initiator stores with its own connection entry.
    host: String,
    token: String,
    url: String,
}

#[derive(Serialize)]
struct RedeemResponse {
    host: String,
    version: String,
    token: String,
    url: String,
    pair_id: String,
}

/// Redeem a pairing code. Authenticated by the one-time code, not by any
/// login token: the joiner proves the out-of-band secret and offers its own
/// login token in the same call. The server burns the code first, then
/// returns its own login token so both sides store the mirrored entry.
async fn redeem_pairing(
    State(app): State<App>,
    Json(body): Json<RedeemRequest>,
) -> Result<Json<RedeemResponse>, ApiError> {
    let code = body.code.trim().to_string();
    crate::pairing::validate_code(&code).map_err(|e| error(StatusCode::BAD_REQUEST, e))?;
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
    // Burn before doing anything else: a valid code works exactly once even
    // if the rest of the redeem fails afterwards.
    if !app
        .engine
        .connections
        .redeem_local(&code)
        .map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, e))?
    {
        return Err(error(
            StatusCode::UNAUTHORIZED,
            "Unknown or expired pairing code",
        ));
    }
    let pair_id = format!(
        "pair-{}",
        now_hex_id().map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, e))?
    );
    app.engine
        .connections
        .add_paired(
            joiner_host.clone(),
            joiner_url.clone(),
            body.token.clone(),
            pair_id.clone(),
        )
        .map_err(|e| error(StatusCode::CONFLICT, e))?;
    let token = app
        .engine
        .connections
        .local_token()
        .map_err(|e| error(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let status = app.engine.status();
    Ok(Json(RedeemResponse {
        host: status.host,
        version: status.version,
        token,
        url: app.engine.config.base_url(),
        pair_id,
    }))
}

fn now_hex_id() -> anyhow::Result<String> {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
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
