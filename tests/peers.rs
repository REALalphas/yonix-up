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
    // The fake nix must be executable even when TMPDIR is mounted noexec.
    let test_binary = std::env::current_exe().unwrap();
    let temp = tempfile::tempdir_in(test_binary.parent().unwrap()).unwrap();
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
    let job = loop {
        let job: Value = get(&format!("hosts/web/jobs/{id}")).json().unwrap();
        if job["status"] != "running" {
            break job;
        }
        assert!(Instant::now() < deadline, "Remote check timed out: {job}");
        thread::sleep(Duration::from_millis(20));
    };
    let log = get(&format!("hosts/web/jobs/{id}/log")).text().unwrap();
    assert_eq!(
        job["status"], "succeeded",
        "Remote check failed: {}\n{log}",
        job["error"]
    );
    assert!(log.contains("remote-fixture-output"), "{log}");
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
fn pairing_links_exchange_tokens_burn_codes_and_reject_replay() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let initiator = config(temp.path(), "nas");
    let joiner = config(temp.path(), "web");
    // An unspent invitation from before the encrypted handshake remains usable
    // by the old protocol until its original expiry.
    fs::create_dir_all(&initiator.state_dir).unwrap();
    let old = initiator.state_dir.join("pairings.json");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    fs::write(&old, json!([{"code":"012345","created":now}]).to_string()).unwrap();
    fs::set_permissions(&old, fs::Permissions::from_mode(0o600)).unwrap();
    let _initiator_process = spawn(&initiator);
    let _joiner_process = spawn(&joiner);
    let initiator_token = engine::read_token(&initiator).unwrap();
    let joiner_token = engine::read_token(&joiner).unwrap();
    let http = reqwest::blocking::Client::builder()
        .no_proxy()
        .build()
        .unwrap();
    // Minting requires the login token and returns only a one-time code.
    assert_eq!(
        http.post(format!("{}/api/pairings", initiator.base_url()))
            .json(&json!({}))
            .send()
            .unwrap()
            .status(),
        401
    );
    let mint: Value = http
        .post(format!("{}/api/pairings", initiator.base_url()))
        .bearer_auth(&initiator_token)
        .json(&json!({}))
        .send()
        .unwrap()
        .json()
        .unwrap();
    let code = mint["code"].as_str().unwrap();
    assert_eq!(code.len(), 6);
    assert!(
        code.bytes()
            .all(|b| b.is_ascii_digit() || b"ABCDEFGHJKMNPQRSTVWXYZ".contains(&b))
    );
    assert!(!mint.to_string().contains(&initiator_token));
    // Server-side pairing state is a private file next to connections.json.
    assert_eq!(
        fs::metadata(initiator.state_dir.join("pairings.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    // Build the link the initiator would print: base64url(payload)#code.
    let payload = yonix::pairing::PairPayload {
        v: 1,
        host: "nas".into(),
        paths: vec![yonix::pairing::PathCandidate {
            url: format!("{}/", initiator.base_url()),
            role: yonix::pairing::PathRole::Direct,
        }],
    };
    let link = yonix::pairing::encode_link(&payload, code).unwrap();
    let parsed = yonix::pairing::parse_link(&link).unwrap();
    assert_eq!(parsed.code, code);
    // Redeem with no auth header: the code is the credential, the login
    // token is the payload. The server burns the code and returns its own
    // login token plus a stable id for both sides.
    let redeem = |body: Value| {
        http.post(format!("{}/api/pairings/redeem", initiator.base_url()))
            .json(&body)
            .send()
            .unwrap()
    };
    assert_eq!(
        redeem(json!({"code":code, "host":"web", "url":joiner.base_url(), "token":joiner_token}))
            .status(),
        401
    );
    let joiner_body = json!({
        "code": "012345",
        "host": "web",
        "token": joiner_token,
        "url": joiner.base_url(),
    });
    let reply: Value = redeem(joiner_body.clone()).json().unwrap();
    assert_eq!(reply["host"], "nas");
    assert_eq!(reply["token"], initiator_token);
    assert!(reply["pair_id"].as_str().unwrap().starts_with("pair-"));
    assert!(reply.get("code").is_none());
    // The initiator stored the joiner's entry with the shared pair id.
    let connections: Value = http
        .get(format!("{}/api/connections", initiator.base_url()))
        .bearer_auth(&initiator_token)
        .send()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(connections.as_array().unwrap().len(), 1);
    // Replay fails: the code burned on first use.
    assert_eq!(redeem(joiner_body).status(), 401);
    // Bad codes and payloads are rejected with 400/401, never 500.
    for bad in ["zz", "12345", "1234567", "I23456"] {
        assert_eq!(
            redeem(
                json!({"code": bad, "host": "web", "token": joiner_token, "url": joiner.base_url()})
            )
            .status(),
            400
        );
    }
    assert_eq!(
        redeem(json!({"code": "000000", "host": "web", "token": "zz", "url": joiner.base_url()}))
            .status(),
        400
    );
}

#[test]
fn failed_connection_save_can_be_retried_without_losing_the_peer() {
    use yonix::peers::{ConnectionInput, Connections};

    let temp = tempfile::tempdir().unwrap();
    let config = Config {
        state_dir: temp.path().to_owned(),
        ..Config::default()
    };
    let store = Connections::open(&config).unwrap();
    let input = || ConnectionInput {
        label: "Backup".into(),
        url: "http://192.168.1.8:9847".into(),
        token: "a".repeat(64),
        pair_id: None,
    };

    // A directory at the destination makes the atomic rename fail even as root.
    let registry = config.state_dir.join("connections.json");
    fs::create_dir(&registry).unwrap();
    assert!(store.add(input()).is_err());
    assert!(store.list().is_empty());
    fs::remove_dir(&registry).unwrap();
    let peer = store.add(input()).unwrap();

    let backup = config.state_dir.join("backup.json");
    fs::rename(&registry, &backup).unwrap();
    fs::create_dir(&registry).unwrap();
    assert!(store.remove(&peer.id).is_err());
    assert_eq!(
        store.target(&peer.id).unwrap(),
        Some((input().url, input().token))
    );
    fs::remove_dir(&registry).unwrap();
    fs::rename(backup, registry).unwrap();
    store.remove(&peer.id).unwrap();
    drop(store);
    assert!(Connections::open(&config).unwrap().list().is_empty());
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

#[test]
fn six_character_or_compact_invitation_pair_both_daemons() {
    for use_code in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let share = config(root.path(), "sender");
        let _sender = spawn(&share);
        let join = config(root.path(), "receiver");
        let _receiver = spawn(&join);
        let http = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap();
        let post = |c: &Config, path: &str, body: Value| {
            http.post(format!("{}/api/{path}", c.base_url()))
                .bearer_auth(engine::read_token(c).unwrap())
                .json(&body)
                .send()
                .unwrap()
        };
        let mint = || post(&share, "pairings", json!({})).json::<Value>().unwrap();
        let invite = mint();
        assert_eq!(invite["code"], mint()["code"]);
        let link = invite["link"].as_str().unwrap();
        assert_eq!(link.len(), 16);
        let code = invite["code"].as_str().unwrap();
        // A wrong PIN cannot create a peer or consume the original invitation.
        let wrong = format!(
            "{}{}",
            if code.starts_with('0') { "1" } else { "0" },
            &code[1..]
        );
        let wrong_link = format!("{}#{wrong}", link.split_once('#').unwrap().0);
        assert!(
            !post(&join, "pairings/accept", json!({"link":wrong_link}))
                .status()
                .is_success()
        );
        let entry = post(
            &join,
            "pairings/accept",
            json!({"link":if use_code { code } else {link}}),
        );
        let status = entry.status();
        let entry: Value = entry.json().unwrap();
        assert!(status.is_success(), "{entry}");
        for (from, to) in [(&join, &share), (&share, &join)] {
            let peers: Value = http
                .get(format!("{}/api/connections", from.base_url()))
                .bearer_auth(engine::read_token(from).unwrap())
                .send()
                .unwrap()
                .json()
                .unwrap();
            let peers = peers.as_array().unwrap();
            assert_eq!(peers.len(), 1);
            assert_eq!(peers[0]["url"], to.base_url());
            let checked = post(
                from,
                &format!("connections/{}/check", peers[0]["id"].as_str().unwrap()),
                json!({}),
            );
            assert_eq!(checked.json::<Value>().unwrap()["host"], to.host);
        }
        assert!(
            !post(&join, "pairings/accept", json!({"link":link}))
                .status()
                .is_success()
        );
    }
}

#[test]
fn interrupted_pairing_recovers_from_the_persisted_peer_record() {
    let root = tempfile::tempdir().unwrap();
    let cfg = config(root.path(), "sender");
    fs::create_dir_all(&cfg.state_dir).unwrap();
    let journal = cfg.state_dir.join("pairings.json");
    let pair = format!("pair-{}", "1".repeat(32));
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let claim =
        json!([{"code":"001234","created":now,"invite_id":"fixture","committing_pair":pair}]);
    let write = |path: &std::path::Path, body: &Value| {
        fs::write(path, body.to_string()).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    };
    for committed in [false, true] {
        write(&journal, &claim);
        if committed {
            write(
                &cfg.state_dir.join("connections.json"),
                &json!([{
                    "id":"pair-fixture","pair_id":pair,"label":"other","url":"http://127.0.0.2:9847",
                    "token":"a".repeat(64)
                }]),
            );
        }
        let store = yonix::peers::Connections::open(&cfg).unwrap();
        let pending: Value = serde_json::from_slice(&fs::read(&journal).unwrap()).unwrap();
        assert_eq!(pending.as_array().unwrap().len(), usize::from(!committed));
        if !committed {
            assert_eq!(store.mint_pairing().unwrap().0, "001234");
        }
    }
}

#[test]
fn expired_pairing_invites_are_removed_before_a_new_one_is_minted() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    let cfg = config(root.path(), "sender");
    fs::create_dir_all(&cfg.state_dir).unwrap();
    let journal = cfg.state_dir.join("pairings.json");
    fs::write(
        &journal,
        json!([{
            "code": "111111",
            "created": 0,
            "invite_id": "expired-invite",
            "committing_pair": null
        }])
        .to_string(),
    )
    .unwrap();
    fs::set_permissions(&journal, fs::Permissions::from_mode(0o600)).unwrap();

    let store = yonix::peers::Connections::open(&cfg).unwrap();
    let loaded: Value = serde_json::from_slice(&fs::read(&journal).unwrap()).unwrap();
    assert_eq!(loaded, json!([]));
    let (code, expires_in) = store.mint_pairing().unwrap();
    assert_eq!(expires_in, 300);

    let pending: Value = serde_json::from_slice(&fs::read(&journal).unwrap()).unwrap();
    let entries = pending.as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["code"], code);
    assert_ne!(entries[0]["invite_id"], "expired-invite");
}

#[test]
fn pairing_can_be_disabled_and_persists_across_restart() {
    let root = tempfile::tempdir().unwrap();
    let cfg = config(root.path(), "sender");
    fs::create_dir_all(&cfg.state_dir).unwrap();
    let store = yonix::peers::Connections::open(&cfg).unwrap();
    assert!(store.pairing_enabled());
    store.set_pairing_enabled(false).unwrap();
    assert!(!store.pairing_enabled());
    assert!(store.mint_pairing().is_err());

    let reopened = yonix::peers::Connections::open(&cfg).unwrap();
    assert!(!reopened.pairing_enabled());
    reopened.set_pairing_enabled(true).unwrap();
    let (code, expires_in) = reopened.mint_pairing().unwrap();
    assert_eq!(code.len(), 6);
    assert_eq!(expires_in, 300);
    reopened.revoke_pairing().unwrap();
    let pending: Value =
        serde_json::from_slice(&fs::read(cfg.state_dir.join("pairings.json")).unwrap()).unwrap();
    assert_eq!(pending, json!([]));
}
