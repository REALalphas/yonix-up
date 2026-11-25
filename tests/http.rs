//! Exercise the actual daemon router on an ephemeral loopback socket.
use axum::{
    Json, Router,
    http::{StatusCode, header},
    response::{IntoResponse, Redirect},
    routing::get,
};
use reqwest::{Client, Method, RequestBuilder};
use serde_json::{Value, json};
use std::{
    fs,
    net::SocketAddr,
    os::unix::fs::PermissionsExt,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tempfile::TempDir;
use tokio::{net::TcpListener, task::JoinHandle};
use yonix::{
    config::Config,
    engine::{Engine, ensure_token},
    peers::Peer,
    server::router,
};

struct Listener {
    address: SocketAddr,
    task: JoinHandle<()>,
}

impl Listener {
    fn spawn(listener: TcpListener, router: Router) -> Self {
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(
                listener,
                router.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        Self { address, task }
    }

    fn base(&self) -> String {
        format!("http://{}", self.address)
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct Api {
    server: Listener,
    http: Client,
    token: String,
    _dir: TempDir,
}

impl Api {
    async fn new(setup: impl FnOnce(&mut Config)) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut config = Config {
            config_path: dir.path().join("config"),
            state_dir: dir.path().join("state"),
            host: "router-test".into(),
            port: listener.local_addr().unwrap().port(),
            ..Config::default()
        };
        fs::create_dir_all(&config.state_dir).unwrap();
        setup(&mut config);
        let engine = Engine::open(config.clone()).unwrap();
        let token = ensure_token(&config).unwrap();
        let server = Listener::spawn(listener, router(engine, token.clone()));
        let http = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        Self {
            server,
            http,
            token,
            _dir: dir,
        }
    }

    fn request(&self, method: Method, path: &str) -> RequestBuilder {
        self.http
            .request(method, format!("{}/api/{path}", self.server.base()))
    }

    fn get(&self, path: &str) -> RequestBuilder {
        self.request(Method::GET, path).bearer_auth(&self.token)
    }

    fn post(&self, path: &str) -> RequestBuilder {
        self.request(Method::POST, path).bearer_auth(&self.token)
    }
}

#[tokio::test]
async fn browser_requests_require_consistent_host_origin_and_authorization() {
    let api = Api::new(|_| {}).await;
    let response = api
        .get("status")
        .header(header::ORIGIN, api.server.base())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert_eq!(response.headers()["referrer-policy"], "no-referrer");
    let csp = response.headers()["content-security-policy"]
        .to_str()
        .unwrap();
    assert!(csp.contains("frame-ancestors 'none'"));
    assert!(csp.contains("connect-src 'self'"));

    for (path, name, value, status) in [
        (
            "status",
            header::HOST,
            "example.invalid:9847",
            StatusCode::FORBIDDEN,
        ),
        ("status", header::ORIGIN, "null", StatusCode::FORBIDDEN),
        (
            "status",
            header::AUTHORIZATION,
            "Basic invalid",
            StatusCode::UNAUTHORIZED,
        ),
        (
            "status",
            header::AUTHORIZATION,
            "Bearer invalid",
            StatusCode::UNAUTHORIZED,
        ),
    ] {
        let request = api.request(Method::GET, path);
        let request = if name == header::AUTHORIZATION {
            request
        } else {
            request.bearer_auth(&api.token)
        };
        assert_eq!(
            request.header(name, value).send().await.unwrap().status(),
            status
        );
    }

    for path in ["hosts", "connections"] {
        assert_eq!(
            api.get(path)
                .header("X-Yonix-Peer-Request", "1")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN,
            "{path}"
        );
    }
    assert_eq!(
        api.get("status")
            .header("X-Yonix-Peer-Request", "1")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn rejected_job_requests_do_not_create_history_entries() {
    let api = Api::new(|_| {}).await;
    for path in ["jobs", "hosts/local/jobs"] {
        for action in ["switch", "rollback", "clean", "update-inputs"] {
            let response = api
                .post(path)
                .json(&json!({"action": action}))
                .send()
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::BAD_REQUEST,
                "{path}: {action}"
            );
            assert!(
                response.json::<Value>().await.unwrap()["error"]
                    .as_str()
                    .unwrap()
                    .contains("explicit confirmation")
            );
        }
        for (body, expected) in [
            ("{".to_owned(), StatusCode::BAD_REQUEST),
            (
                json!({"action": "check", "unexpected": true}).to_string(),
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                json!({"action": "check", "confirmed": "yes"}).to_string(),
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                json!({"action": "x".repeat(4096)}).to_string(),
                StatusCode::PAYLOAD_TOO_LARGE,
            ),
        ] {
            assert_eq!(
                api.post(path)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(body)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                expected,
                "{path}"
            );
        }
        assert_eq!(
            api.post(path).body("{}").send().await.unwrap().status(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE
        );
        let response = api
            .post(path)
            .json(&json!({"action": "clean", "confirmed": true}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert!(
            response.json::<Value>().await.unwrap()["error"]
                .as_str()
                .unwrap()
                .contains("System changes are disabled")
        );
        assert_eq!(
            api.get(path)
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap(),
            json!([])
        );
    }
}

#[tokio::test]
async fn local_job_routes_find_the_same_job_and_serve_logs_as_text() {
    let log = "<b>fixture log</b>\n";
    let job = json!({"id": "fixture-job_1", "action": "build", "status": "succeeded", "started": 1, "finished": 2, "error": null});
    let api = Api::new(|config| {
        fs::create_dir_all(config.state_dir.join("jobs")).unwrap();
        fs::write(config.state_dir.join("jobs/fixture-job_1.log"), log).unwrap();
        fs::write(
            config.state_dir.join("history.json"),
            json!([job]).to_string(),
        )
        .unwrap();
    })
    .await;

    for prefix in ["jobs", "hosts/local/jobs"] {
        let path = format!("{prefix}/fixture-job_1");
        assert_eq!(
            api.get(&path)
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap(),
            job
        );
        let response = api.get(&format!("{path}/log")).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/plain; charset=utf-8"
        );
        assert_eq!(response.text().await.unwrap(), log);

        for id in ["missing".into(), "nested%2Fid".into(), "a".repeat(129)] {
            for suffix in ["", "/log"] {
                assert_eq!(
                    api.get(&format!("{prefix}/{id}{suffix}"))
                        .send()
                        .await
                        .unwrap()
                        .status(),
                    StatusCode::NOT_FOUND,
                    "{prefix}/{id}{suffix}"
                );
            }
        }
    }
}

#[tokio::test]
async fn forwarded_peer_responses_are_bounded_and_redirects_are_not_followed() {
    let redirects = Arc::new(AtomicUsize::new(0));
    let hits = redirects.clone();
    let upstream = Router::new()
        .route(
            "/api/jobs/final",
            get(move || async move {
                hits.fetch_add(1, Ordering::Relaxed);
                Json(json!({"unexpected": true}))
            }),
        )
        .route(
            "/api/jobs/redirect",
            get(|| async { Redirect::temporary("/api/jobs/final") }),
        )
        .route(
            "/api/jobs/large",
            get(|| async { "x".repeat(2 * 1024 * 1024 + 1) }),
        )
        .route(
            "/api/jobs/missing",
            get(|| async {
                (
                    StatusCode::NOT_FOUND,
                    Json(json!({"error": "fixture job missing"})),
                )
            }),
        )
        .route(
            "/api/jobs/demo/log",
            get(|| async {
                ([(header::CONTENT_TYPE, "text/html")], "<pre>peer log</pre>").into_response()
            }),
        )
        .route("/api/status", get(|| async { "x".repeat(65537) }));
    let upstream = Listener::spawn(TcpListener::bind("127.0.0.1:0").await.unwrap(), upstream);
    let api = Api::new(|config| {
        let token_file = config.state_dir.join("peer-token");
        fs::write(&token_file, "a".repeat(64)).unwrap();
        fs::set_permissions(&token_file, fs::Permissions::from_mode(0o600)).unwrap();
        config.peers.push(Peer {
            id: "peer".into(),
            label: "Fixture peer".into(),
            url: upstream.base(),
            token_file,
        });
    })
    .await;

    for path in ["redirect", "large"] {
        let response = api
            .get(&format!("hosts/peer/jobs/{path}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert!(
            response.json::<Value>().await.unwrap()["error"]
                .as_str()
                .unwrap()
                .starts_with("Cannot reach host")
        );
    }
    assert_eq!(redirects.load(Ordering::Relaxed), 0);
    let response = api.get("hosts/peer/jobs/missing").send().await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"error": "fixture job missing"})
    );
    let response = api.get("hosts/peer/jobs/demo/log").send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/plain; charset=utf-8"
    );
    assert_eq!(response.text().await.unwrap(), "<pre>peer log</pre>");

    assert_eq!(
        api.post("connections/peer/check")
            .json(&json!({}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_GATEWAY
    );
}

#[tokio::test]
async fn pairing_routes_mint_reuse_disable_revoke_and_throttle_per_daemon() {
    let api = Api::new(|_| {}).await;
    let hello = json!({"message": "fixture"});
    let response = api
        .request(Method::POST, "pairings/hello")
        .json(&hello)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    assert_eq!(
        api.request(Method::POST, "pairings")
            .json(&json!({}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let invite = api
        .post("pairings")
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(invite["code"].as_str().unwrap().len(), 6);
    let reused = api
        .post("pairings")
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(reused["link"], invite["link"]);
    let status = api
        .get("pairings")
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(status, json!({"enabled": true, "active": true}));

    for _ in 0..12 {
        assert_eq!(
            api.request(Method::POST, "pairings/hello")
                .json(&hello)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        api.request(Method::POST, "pairings/hello")
            .json(&hello)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    let response = api
        .post("pairings/revoke")
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"revoked": true})
    );
    assert_eq!(
        api.get("pairings")
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap(),
        json!({"enabled": true, "active": false})
    );

    let response = api
        .post("pairings/settings")
        .json(&json!({"enabled": false}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        api.post("pairings")
            .json(&json!({}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        api.get("connections")
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap(),
        json!([])
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_joiners_can_commit_only_one_connection_for_an_invitation() {
    let sharer = Api::new(|config| config.host = "sharer".into()).await;
    let first = Api::new(|config| config.host = "first".into()).await;
    let second = Api::new(|config| config.host = "second".into()).await;
    let invite = sharer
        .post("pairings")
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let body = json!({"link": invite["link"]});
    let (first_response, second_response) = tokio::join!(
        first.post("pairings/accept").json(&body).send(),
        second.post("pairings/accept").json(&body).send(),
    );
    let first_response = first_response.unwrap();
    let second_response = second_response.unwrap();
    let (winner, winner_name, accepted, loser, rejected) =
        match (first_response.status(), second_response.status()) {
            (StatusCode::OK, StatusCode::BAD_GATEWAY) => {
                (&first, "first", first_response, &second, second_response)
            }
            (StatusCode::BAD_GATEWAY, StatusCode::OK) => {
                (&second, "second", second_response, &first, first_response)
            }
            (first, second) => panic!("Expected one accepted invitation, got {first} and {second}"),
        };
    let accepted = accepted.json::<Value>().await.unwrap();
    assert_eq!(accepted["label"], "sharer");
    assert_eq!(accepted["url"], sharer.server.base());
    assert!(rejected.json::<Value>().await.unwrap()["error"].is_string());

    let saved = sharer
        .get("connections")
        .send()
        .await
        .unwrap()
        .json::<Vec<Value>>()
        .await
        .unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0]["label"], winner_name);
    assert_eq!(saved[0]["url"], winner.server.base());
    let status = sharer
        .get("pairings")
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(status, json!({"enabled": true, "active": false}));
    for (joiner, expected) in [(winner, json!([accepted])), (loser, json!([]))] {
        let saved = joiner
            .get("connections")
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap();
        assert_eq!(saved, expected);
    }
}
