//! End-to-end tests run the real daemon and CLI against fake Nix executables.
//! No system build, sudo or activation is performed.
use serde_json::{Value, json};
use std::{
    fs,
    net::TcpListener,
    os::unix::fs::PermissionsExt,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use yonix::{client::Client, config::Config, engine::Action};
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn await_job(client: &Client, id: &str) -> yonix::engine::Job {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let job = client.job(id).unwrap();
        if job.status != "running" {
            return job;
        }
        assert!(Instant::now() < deadline, "Job timed out");
        thread::sleep(Duration::from_millis(30));
    }
}
#[test]
fn authenticated_end_to_end_workflow() {
    authenticated_workflow(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
}

#[test]
fn authenticated_workflow_on_all_interfaces() {
    authenticated_workflow(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
}

fn authenticated_workflow(listen_address: std::net::IpAddr) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let repo = root.join("repo with spaces");
    let bin = root.join("bin");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir(&bin).unwrap();
    fs::write(repo.join("flake.nix"), "{}").unwrap();
    let shell = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|p| p.join("sh"))
        .find(|p| p.is_file())
        .unwrap();
    fs::write(
        bin.join("nix"),
        format!(
            r#"#!{}
printf 'stdout: %s\n' "$*"
printf 'stderr: fixture\n' >&2
printf '%s\n' "$@" >> arguments
if [ "$1" = build ]; then sleep 0.4; fi
if [ -f fail ]; then exit 7; fi
"#,
            shell.display()
        ),
    )
    .unwrap();
    fs::set_permissions(bin.join("nix"), fs::Permissions::from_mode(0o755)).unwrap();
    let socket = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = socket.local_addr().unwrap().port();
    drop(socket);
    let config = Config {
        config_path: repo.clone(),
        state_dir: root.join("state"),
        host: "nas".into(),
        port,
        privileged_helper: None,
        peers: Vec::new(),
        cache: None,
        source_mode: "git".into(),
        listen_address,
    };
    let config_file = root.join("config.json");
    fs::write(&config_file, serde_json::to_vec(&config).unwrap()).unwrap();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let spawn = || {
        Process(
            Command::new(env!("CARGO_BIN_EXE_yonix"))
                .args(["--config", config_file.to_str().unwrap(), "serve"])
                .env("PATH", &path)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        )
    };
    let mut process = spawn();
    let http = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = config.base_url();
    let ready = || {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if http
                .get(format!("{base}/api/status"))
                .send()
                .is_ok_and(|r| r.status() == 401)
            {
                break;
            }
            assert!(Instant::now() < deadline, "Daemon did not start");
            thread::sleep(Duration::from_millis(40));
        }
    };
    ready();
    let token = yonix::engine::read_token(&config).unwrap();
    if listen_address.is_unspecified() {
        for host in [format!("10.0.2.142:{port}"), format!("8.8.8.8:{port}")] {
            assert_eq!(
                http.get(format!("{base}/api/status"))
                    .header("Host", &host)
                    .send()
                    .unwrap()
                    .status(),
                401
            );
            assert_eq!(
                http.get(format!("{base}/api/status"))
                    .header("Host", &host)
                    .header("Origin", format!("http://{host}"))
                    .bearer_auth(&token)
                    .send()
                    .unwrap()
                    .status(),
                200
            );
        }
    }
    assert_eq!(
        http.get(format!("{base}/api/status"))
            .bearer_auth("wrong")
            .send()
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        http.get(format!("{base}/api/status"))
            .header("Host", "evil.example:9847")
            .bearer_auth(&token)
            .send()
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        http.post(format!("{base}/api/jobs"))
            .header("Origin", "https://evil.example")
            .bearer_auth(&token)
            .json(&json!({"action":"build"}))
            .send()
            .unwrap()
            .status(),
        403
    );
    let page = http.get(&base).send().unwrap();
    assert_eq!(page.status(), 200);
    assert!(page.headers().contains_key("Content-Security-Policy"));
    let html = page.text().unwrap();
    assert!(html.contains("Yonix"));
    assert!(html.contains("type=\"module\""));
    let asset = html
        .split("src=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    let asset_response = http.get(format!("{base}{asset}")).send().unwrap();
    assert_eq!(asset_response.status(), 200);
    assert!(
        asset_response.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("application/javascript")
    );
    assert_eq!(
        http.get(format!("{base}/assets/missing.js"))
            .send()
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        http.get(format!("{base}/app.js")).send().unwrap().status(),
        404
    );
    let locale_script = http.get(format!("{base}/locales.js")).send().unwrap();
    assert_eq!(locale_script.status(), 200);
    assert!(
        locale_script.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("application/javascript")
    );
    let script = locale_script.text().unwrap();
    let json = script
        .strip_prefix("const translations = ")
        .unwrap()
        .trim_end()
        .strip_suffix(';')
        .unwrap();
    let catalogs: serde_json::Value = serde_json::from_str(json).unwrap();
    for (code, _) in yonix::i18n::LOCALES {
        assert!(
            catalogs[*code]["confirmClean"]
                .as_str()
                .is_some_and(|s| !s.is_empty()),
            "missing {code}"
        );
    }
    let unconfirmed = http
        .post(format!("{base}/api/jobs"))
        .bearer_auth(&token)
        .json(&json!({"action":"update-inputs"}))
        .send()
        .unwrap();
    assert_eq!(unconfirmed.status(), 400);
    let unknown = http
        .post(format!("{base}/api/jobs"))
        .bearer_auth(&token)
        .json(&json!({"action":"shell","confirmed":true}))
        .send()
        .unwrap();
    assert_eq!(unknown.status(), 422);
    let client = Client::new(&config, None, None).unwrap();
    assert_eq!(client.status().unwrap().host, "nas");
    assert!(client.start(Action::Switch, true).is_err());
    let job = client.start(Action::Build, false).unwrap();
    assert!(
        client.start(Action::Check, false).is_err(),
        "Parallel operations must be rejected"
    );
    assert_eq!(await_job(&client, &job.id).status, "succeeded");
    let log = client.log(&job.id).unwrap();
    assert!(log.contains("stdout:"));
    assert!(log.contains("stderr: fixture"));
    assert!(
        !fs::read_to_string(repo.join("arguments"))
            .unwrap()
            .contains("switch")
    );
    fs::write(repo.join("fail"), "").unwrap();
    let failure = client.start(Action::Check, false).unwrap();
    let failed = await_job(&client, &failure.id);
    assert_eq!(failed.status, "failed");
    assert!(failed.error.unwrap().contains("7"));
    fs::remove_file(repo.join("fail")).unwrap();
    let cli = Command::new(env!("CARGO_BIN_EXE_yonix"))
        .args(["--config", config_file.to_str().unwrap(), "check"])
        .output()
        .unwrap();
    assert!(
        cli.status.success(),
        "{}",
        String::from_utf8_lossy(&cli.stderr)
    );
    fs::write(repo.join("fail"), "").unwrap();
    let cli = Command::new(env!("CARGO_BIN_EXE_yonix"))
        .args(["--config", config_file.to_str().unwrap(), "check"])
        .output()
        .unwrap();
    assert!(!cli.status.success());
    fs::remove_file(repo.join("fail")).unwrap();
    let diff = client.start(Action::Diff, false).unwrap();
    assert_eq!(await_job(&client, &diff.id).status, "succeeded");
    assert!(client.log(&diff.id).unwrap().contains("diff-closures"));
    let update = client.start(Action::UpdateInputs, true).unwrap();
    assert_eq!(await_job(&client, &update.id).status, "succeeded");
    let before = client.jobs().unwrap().len();
    process.0.kill().unwrap();
    process.0.wait().unwrap();
    process = spawn();
    ready();
    assert_eq!(client.jobs().unwrap().len(), before);
    assert!(
        client
            .log(&job.id)
            .unwrap()
            .contains("completed successfully")
    );
    let cli = Command::new(env!("CARGO_BIN_EXE_yonix"))
        .args(["--config", config_file.to_str().unwrap(), "history"])
        .output()
        .unwrap();
    let history: Value = serde_json::from_slice(&cli.stdout).unwrap();
    assert_eq!(history.as_array().unwrap().len(), before);
    drop(process);
}

#[test]
fn legacy_workflow_uses_nixos_rebuild() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let repo = root.join("repo with spaces");
    let bin = root.join("bin");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir(&bin).unwrap();
    fs::write(repo.join("configuration.nix"), "{ }").unwrap();
    let shell = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|p| p.join("sh"))
        .find(|p| p.is_file())
        .unwrap();
    // `nix store diff-closures` still goes through `nix`; the rebuild itself
    // through `nixos-rebuild`, each logging its own invocation.
    for tool in ["nix", "nixos-rebuild"] {
        fs::write(
            bin.join(tool),
            format!(
                r#"#!{}
printf 'stdout: %s\n' "$*"
printf '%s\n' "$@" >> {tool}-arguments
if [ -f fail ]; then exit 7; fi
"#,
                shell.display()
            ),
        )
        .unwrap();
        fs::set_permissions(bin.join(tool), fs::Permissions::from_mode(0o755)).unwrap();
    }
    let socket = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = socket.local_addr().unwrap().port();
    drop(socket);
    let config = Config {
        config_path: repo.clone(),
        state_dir: root.join("state"),
        host: "nas".into(),
        port,
        privileged_helper: None,
        peers: Vec::new(),
        cache: None,
        source_mode: "git".into(),
        listen_address: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
    };
    let config_file = root.join("config.json");
    fs::write(&config_file, serde_json::to_vec(&config).unwrap()).unwrap();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let process = Process(
        Command::new(env!("CARGO_BIN_EXE_yonix"))
            .args(["--config", config_file.to_str().unwrap(), "serve"])
            .env("PATH", &path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let http = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = config.base_url();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if http
            .get(format!("{base}/api/status"))
            .send()
            .is_ok_and(|r| r.status() == 401)
        {
            break;
        }
        assert!(Instant::now() < deadline, "Daemon did not start");
        thread::sleep(Duration::from_millis(40));
    }
    let client = Client::new(&config, None, None).unwrap();
    let status = client.status().unwrap();
    assert_eq!(status.host, "nas");
    assert_eq!(status.config_kind, yonix::config::ConfigKind::Legacy);
    // No flake.lock exists for legacy environments.
    assert!(client.start(Action::UpdateInputs, true).is_err());
    let build = client.start(Action::Build, false).unwrap();
    assert_eq!(await_job(&client, &build.id).status, "succeeded");
    // Legacy builds run in `state_dir` (nixos-rebuild drops `result` in cwd).
    let rebuild_args =
        fs::read_to_string(config.state_dir.join("nixos-rebuild-arguments")).unwrap();
    assert!(rebuild_args.contains("--no-flake"));
    assert!(rebuild_args.contains(&format!(
        "nixos-config={}/configuration.nix",
        repo.display()
    )));
    assert!(!rebuild_args.contains("--flake"));
    let log = client.log(&build.id).unwrap();
    assert!(log.contains("$ nixos-rebuild"));
    let diff = client.start(Action::Diff, false).unwrap();
    assert_eq!(await_job(&client, &diff.id).status, "succeeded");
    assert!(client.log(&diff.id).unwrap().contains("diff-closures"));
    let check = client.start(Action::Check, false).unwrap();
    assert_eq!(await_job(&client, &check.id).status, "succeeded");
    assert!(client.log(&check.id).unwrap().contains("dry-build"));
    drop(process);
}
