use serde_json::{Value, json};
use std::{
    fs,
    net::TcpListener,
    os::unix::fs::PermissionsExt,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use yonix::{config::Config, engine, peers::Peer};
struct Daemon(Child);
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn config(root: &std::path::Path, host: &str) -> Config {
    let repo = root.join(format!("{host}-repo"));
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("flake.nix"), "{}").unwrap();
    let socket = TcpListener::bind("127.0.0.1:0").unwrap();
    Config {
        host: host.into(),
        config_path: repo,
        state_dir: root.join(format!("{host}-state")),
        port: socket.local_addr().unwrap().port(),
        ..Config::default()
    }
}
fn spawn(config: &Config) -> Daemon {
    let file = config.state_dir.with_extension("json");
    fs::write(&file, serde_json::to_vec(config).unwrap()).unwrap();
    let daemon = Daemon(
        Command::new(env!("CARGO_BIN_EXE_yonix"))
            .args(["--config", file.to_str().unwrap(), "serve"])
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    config.state_dir.parent().unwrap().join("bin").display(),
                    std::env::var("PATH").unwrap()
                ),
            )
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while reqwest::blocking::get(format!("{}/api/status", config.base_url())).is_err() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(30));
    }
    daemon
}
#[test]
fn peer_configuration_and_credentials_are_restricted() {
    let temp = tempfile::tempdir().unwrap();
    let mut peer = Peer {
        id: "web".into(),
        label: "Web server".into(),
        url: "http://127.0.0.1:9847".into(),
        token_file: temp.path().join("token"),
    };
    peer.validate().unwrap();
    for url in [
        "http://8.8.8.8:9847",
        "http://0.0.0.0:9847",
        "http://localhost:9847",
        "http://10.0.0.1/api",
        "http://user:secret@10.0.0.1",
        "http://10.0.0.1?url=x",
    ] {
        peer.url = url.into();
        assert!(peer.validate().is_err(), "{url}");
    }
    peer.url = "http://10.0.0.1:9847".into();
    for id in ["local", "../web", "", "web/other"] {
        peer.id = id.into();
        assert!(peer.validate().is_err());
    }
    peer.id = "web".into();
    fs::write(&peer.token_file, "a".repeat(64)).unwrap();
    fs::set_permissions(&peer.token_file, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(peer.token().is_err());
    fs::set_permissions(&peer.token_file, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(peer.token().is_ok());
    let mut c = config(temp.path(), "nas");
    c.peers = vec![peer.clone(), peer];
    assert!(c.validate().is_err());
}
#[test]
fn gateway_keeps_credentials_local_and_routes_single_hop() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let shell = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|path| path.join("sh"))
        .find(|path| path.is_file())
        .unwrap();
    fs::write(
        bin.join("nix"),
        format!("#!{}\nprintf 'remote-fixture-output\\n'\n", shell.display()),
    )
    .unwrap();
    fs::set_permissions(bin.join("nix"), fs::Permissions::from_mode(0o755)).unwrap();
    let remote = config(temp.path(), "web");
    let remote_engine = engine::Engine::open(remote.clone()).unwrap();
    drop(remote_engine);
    let remote_token = engine::ensure_token(&remote).unwrap();
    let mut remote_process = spawn(&remote);
    let mut gateway = config(temp.path(), "nas");
    gateway.peers = vec![Peer {
        id: "web".into(),
        label: "Web server".into(),
        url: remote.base_url(),
        token_file: remote.token_path(),
    }];
    let _gateway_process = spawn(&gateway);
    let gateway_token = engine::read_token(&gateway).unwrap();
    let http = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let base = gateway.base_url();
    let get = |path: &str| {
        http.get(format!("{base}/api/{path}"))
            .bearer_auth(&gateway_token)
            .send()
            .unwrap()
    };
    assert_eq!(
        http.get(format!("{base}/api/hosts"))
            .send()
            .unwrap()
            .status(),
        401
    );
    let hosts = get("hosts").text().unwrap();
    assert!(!hosts.contains(&remote_token));
    assert!(!hosts.contains("token_file"));
    assert!(!hosts.contains(&remote.base_url()));
    let hosts: Value = serde_json::from_str(&hosts).unwrap();
    assert_eq!(hosts[0]["id"], "local");
    assert_eq!(hosts[1]["id"], "web");
    assert_eq!(
        get("hosts/local/status").json::<Value>().unwrap()["host"],
        "nas"
    );
    assert_eq!(
        get("hosts/web/status").json::<Value>().unwrap()["host"],
        "web"
    );
    assert_eq!(get("hosts/unknown/status").status(), 404);
    assert_eq!(get("hosts/web/jobs").json::<Value>().unwrap(), json!([]));
    let post = |body: Value| {
        http.post(format!("{base}/api/hosts/web/jobs"))
            .bearer_auth(&gateway_token)
            .json(&body)
            .send()
            .unwrap()
    };
    assert_eq!(post(json!({"action":"update-inputs"})).status(), 400);
    assert_eq!(
        post(json!({"action":"switch","confirmed":true})).status(),
        409
    );
    assert_eq!(post(json!({"action":"shell"})).status(), 422);
    assert_eq!(
        http.get(format!("{}/api/hosts", remote.base_url()))
            .bearer_auth(&remote_token)
            .header("X-Yonix-Peer-Request", "1")
            .send()
            .unwrap()
            .status(),
        403
    );
    let job: Value = post(json!({"action":"check"})).json().unwrap();
    let id = job["id"].as_str().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let job: Value = get(&format!("hosts/web/jobs/{id}")).json().unwrap();
        if job["status"] != "running" {
            assert_eq!(job["status"], "succeeded");
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(20));
    }
    assert!(
        get(&format!("hosts/web/jobs/{id}/log"))
            .text()
            .unwrap()
            .contains("remote-fixture-output")
    );
    assert_eq!(get("hosts/local/jobs").json::<Value>().unwrap(), json!([]));
    // Rotated credentials are read per request; neither a failure nor a disconnect selects the local host.
    fs::write(remote.token_path(), "b".repeat(64)).unwrap();
    assert_eq!(get("hosts/web/status").status(), 502);
    assert_eq!(
        get("hosts/local/status").json::<Value>().unwrap()["host"],
        "nas"
    );
    fs::write(remote.token_path(), &remote_token).unwrap();
    assert_eq!(get("hosts/web/status").status(), 200);
    remote_process.0.kill().unwrap();
    remote_process.0.wait().unwrap();
    assert_eq!(get("hosts/web/status").status(), 502);
}

#[test]
fn browser_connections_are_verified_private_persistent_and_removable() {
    let temp = tempfile::tempdir().unwrap();
    let remote = config(temp.path(), "web");
    let _remote_process = spawn(&remote);
    let remote_token = engine::read_token(&remote).unwrap();
    let gateway = config(temp.path(), "nas");
    let mut gateway_process = spawn(&gateway);
    let gateway_token = engine::read_token(&gateway).unwrap();
    let http = reqwest::blocking::Client::builder()
        .no_proxy()
        .build()
        .unwrap();
    let base = gateway.base_url();
    let input = json!({"label":"Web server", "url":remote.base_url(), "token":remote_token});
    let post = |path: &str, body: Value| {
        http.post(format!("{base}/api/{path}"))
            .bearer_auth(&gateway_token)
            .json(&body)
            .send()
            .unwrap()
    };
    let get = |path: &str| {
        http.get(format!("{base}/api/{path}"))
            .bearer_auth(&gateway_token)
            .send()
            .unwrap()
    };
    assert_eq!(
        http.post(format!("{base}/api/connections"))
            .json(&input)
            .send()
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        post(
            "connections/check",
            json!({"label":"Web", "url":"http://8.8.8.8", "token":remote_token})
        )
        .status(),
        400
    );
    assert_eq!(
        post(
            "connections",
            json!({"label":"Web", "url":remote.base_url(), "token":"a".repeat(64)})
        )
        .status(),
        502
    );
    assert_eq!(
        post(
            "connections",
            json!({"label":"Self", "url":base, "token":gateway_token})
        )
        .status(),
        400
    );
    assert_eq!(get("connections").json::<Value>().unwrap(), json!([]));
    assert_eq!(
        post("connections/check", input.clone())
            .json::<Value>()
            .unwrap()["host"],
        "web"
    );
    let response = post("connections", input.clone()).text().unwrap();
    assert!(!response.contains(&remote_token));
    assert!(!response.contains("token"));
    let created: Value = serde_json::from_str(&response).unwrap();
    let id = created["id"].as_str().unwrap();
    assert_eq!(created["managed_by_nix"], false);
    assert_eq!(post("connections", input).status(), 409);
    assert_eq!(
        post(&format!("connections/{id}/check"), json!({})).status(),
        200
    );
    let registry = gateway.state_dir.join("connections.json");
    assert_eq!(
        fs::metadata(&registry).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        get(&format!("hosts/{id}/status")).json::<Value>().unwrap()["host"],
        "web"
    );
    for path in ["connections", "hosts"] {
        let public = get(path).text().unwrap();
        assert!(!public.contains(&remote_token));
        assert!(!public.contains("token_file"));
    }
    assert_eq!(
        http.get(format!("{base}/api/connections"))
            .bearer_auth(&gateway_token)
            .header("X-Yonix-Peer-Request", "1")
            .send()
            .unwrap()
            .status(),
        403
    );
    gateway_process.0.kill().unwrap();
    gateway_process.0.wait().unwrap();
    gateway_process = spawn(&gateway);
    assert_eq!(get("connections").json::<Value>().unwrap()[0]["id"], id);
    assert_eq!(
        get(&format!("hosts/{id}/status")).json::<Value>().unwrap()["host"],
        "web"
    );
    assert_eq!(
        post(&format!("connections/{id}/remove"), json!({})).status(),
        200
    );
    assert_eq!(get("connections").json::<Value>().unwrap(), json!([]));
    assert_eq!(get(&format!("hosts/{id}/status")).status(), 404);
    assert!(
        !fs::read_to_string(registry)
            .unwrap()
            .contains(&remote_token)
    );
    drop(gateway_process);
}
