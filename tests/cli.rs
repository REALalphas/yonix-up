//! Exercise the CLI binary with isolated configuration and an inert HTTP fixture.
use axum::{
    Json, Router,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Command, Output, Stdio},
};
use tempfile::TempDir;
use yonix::config::Config;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const LOG: &str = "stdout: fixture\nstderr: fixture without newline";

struct Fixture {
    dir: TempDir,
    config: Config,
    config_file: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            config_path: dir.path().join("repo"),
            host: "local-fixture".into(),
            state_dir: dir.path().join("state"),
            ..Config::default()
        };
        let config_file = dir.path().join("config.json");
        fs::write(&config_file, serde_json::to_vec(&config).unwrap()).unwrap();
        Self {
            dir,
            config,
            config_file,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_yonix"));
        command
            .arg("--config")
            .arg(&self.config_file)
            .env_remove("NO_COLOR")
            .env_remove("FORCE_COLOR")
            .env("TERM", "xterm")
            .stdin(Stdio::null());
        command
    }

    fn write_token(&self, token: &str) {
        let path = self.config.token_path();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, token).unwrap();
    }

    async fn api(&self, app: Router) -> ApiFixture<'_> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let token_file = self.dir.path().join("tunnel-token");
        fs::write(&token_file, format!("{TOKEN}\n")).unwrap();
        ApiFixture {
            fixture: self,
            url: format!("http://{addr}/"),
            token_file,
            server,
        }
    }
}

struct ApiFixture<'a> {
    fixture: &'a Fixture,
    url: String,
    token_file: PathBuf,
    server: tokio::task::JoinHandle<()>,
}

impl ApiFixture<'_> {
    fn command(&self) -> Command {
        let mut command = self.fixture.command();
        command
            .args(["--url", &self.url, "--token-file"])
            .arg(&self.token_file);
        command
    }
}

impl Drop for ApiFixture<'_> {
    fn drop(&mut self) {
        self.server.abort();
    }
}

fn stdout(output: Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn help_and_init_work_without_a_config() {
    let fixture = Fixture::new();
    fs::remove_file(&fixture.config_file).unwrap();
    let help = stdout(fixture.command().arg("--help").output().unwrap());
    for option in ["--config", "--url", "--token-file", "--color"] {
        assert!(help.contains(option), "{help}");
    }
    let version = stdout(fixture.command().arg("--version").output().unwrap());
    assert_eq!(version.trim(), concat!("yonix ", env!("CARGO_PKG_VERSION")));

    let raw = stdout(
        fixture
            .command()
            .args(["init", "--color", "never"])
            .output()
            .unwrap(),
    );
    let config: Config = serde_json::from_str(&raw).unwrap();
    config.validate().unwrap();
    assert!(config.listen_address.is_loopback());
    assert!(config.privileged_helper.is_none());
    assert!(config.peers.is_empty());
    assert!(!fixture.config_file.exists());
    assert!(!fixture.config.state_dir.exists());
}

#[test]
fn argument_errors_exit_before_loading_configuration() {
    let fixture = Fixture::new();
    fs::remove_file(&fixture.config_file).unwrap();
    for (args, rejected) in [
        (&["invalid-subcommand"][..], "invalid-subcommand"),
        (&["history", "--limit", "many"], "many"),
        (&["status", "--color", "rainbow"], "rainbow"),
    ] {
        let output = fixture.command().args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains(rejected), "{error}");
        assert!(!error.contains("Cannot read"), "{error}");
    }
}

#[test]
fn token_command_trims_the_token_and_rejects_invalid_files() {
    let fixture = Fixture::new();
    fixture.write_token(&format!("  {TOKEN}\n"));
    assert_eq!(
        stdout(fixture.command().arg("token").output().unwrap()),
        format!("{TOKEN}\n")
    );

    fixture.write_token(&"g".repeat(64));
    let output = fixture.command().arg("token").output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("Invalid token file")
    );
}

#[test]
fn client_rejects_non_loopback_url_overrides() {
    let fixture = Fixture::new();
    fixture.write_token(TOKEN);
    let output = fixture
        .command()
        .args(["status", "--url", "http://192.0.2.1:9847/"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("Only an HTTP loopback endpoint is supported")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_selects_json_or_pretty_output_and_authenticates_with_token_override() {
    let fixture = Fixture::new();
    let status = json!({
        "version": "0.1.0",
        "host": "remote-fixture",
        "config_path": "/fixture/config",
        "current_system": null,
        "system_changes_enabled": false,
        "active": "job-1",
        "active_action": "build",
        "cache": null,
        "config_kind": "legacy",
        "needs_restart": false,
    });
    let response = status.clone();
    let app = Router::new().route(
        "/api/status",
        get(move |headers: HeaderMap| async move {
            if headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                != Some(&format!("Bearer {TOKEN}"))
            {
                return StatusCode::UNAUTHORIZED.into_response();
            }
            Json(response).into_response()
        }),
    );
    let api = fixture.api(app).await;
    let raw = stdout(
        api.command()
            .args(["status", "--json", "--color", "always"])
            .output()
            .unwrap(),
    );
    assert_eq!(serde_json::from_str::<Value>(&raw).unwrap(), status);

    let pretty = stdout(
        api.command()
            .args(["status", "--pretty", "--color", "never"])
            .output()
            .unwrap(),
    );
    assert!(pretty.contains("remote-fixture [legacy]"), "{pretty}");
    assert!(pretty.contains("build (job-1)"), "{pretty}");
    assert!(
        pretty.contains("disabled (no privileged helper)"),
        "{pretty}"
    );
    assert!(!pretty.contains('\x1b'));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn logs_write_plain_text_verbatim() {
    let fixture = Fixture::new();
    let app = Router::new().route("/api/jobs/log-fixture/log", get(|| async { LOG }));
    let api = fixture.api(app).await;
    assert_eq!(
        stdout(
            api.command()
                .args(["logs", "log-fixture", "--color", "never"])
                .output()
                .unwrap()
        ),
        LOG
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mutating_commands_decline_by_default_before_posting() {
    let fixture = Fixture::new();
    let app = Router::new().route(
        "/api/jobs",
        post(|| async { (StatusCode::BAD_REQUEST, "unexpected API call") }),
    );
    let api = fixture.api(app).await;
    for action in ["switch", "update", "update-inputs", "rollback", "clean"] {
        let output = api
            .command()
            .args([action, "--color", "never"])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{action}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains("[y/N]"), "{action}: {text}");
        assert!(text.contains("Operation cancelled."), "{action}: {text}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn switch_alias_posts_confirmation_and_surfaces_api_error() {
    let fixture = Fixture::new();
    let app = Router::new().route(
        "/api/jobs",
        post(|Json(request): Json<Value>| async move {
            if request != json!({"action": "switch", "confirmed": true}) {
                return (StatusCode::BAD_REQUEST, "unexpected job request");
            }
            (StatusCode::CONFLICT, "An operation is already running")
        }),
    );
    let api = fixture.api(app).await;
    let run = |assume_yes| {
        let mut child = api.command();
        child.arg("update");
        if assume_yes {
            child.arg("-y");
        }
        let mut child = child
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        if !assume_yes {
            child.stdin.take().unwrap().write_all(b"yes\n").unwrap();
        }
        child.wait_with_output().unwrap()
    };
    for assume_yes in [false, true] {
        let output = run(assume_yes);
        assert_eq!(output.status.code(), Some(1));
        let text = String::from_utf8(output.stdout).unwrap();
        assert_eq!(text.contains("[y/N]"), !assume_yes, "{text}");
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("An operation is already running")
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pretty_history_limits_rows_and_only_shows_the_first_error_line() {
    let fixture = Fixture::new();
    let jobs = json!([
        {
            "id": "failed-job",
            "action": "build",
            "status": "failed",
            "started": 10,
            "finished": 12,
            "error": "fixture build failed\nadditional diagnostics",
        },
        {
            "id": "completed-job",
            "action": "check",
            "status": "succeeded",
            "started": 1,
            "finished": 2,
            "error": null,
        },
    ]);
    let response = jobs.clone();
    let api = fixture
        .api(Router::new().route("/api/jobs", get(move || async move { Json(response) })))
        .await;
    let pretty = stdout(
        api.command()
            .args(["history", "--pretty", "-n", "1"])
            .output()
            .unwrap(),
    );
    assert!(pretty.contains("failed-job"), "{pretty}");
    assert!(pretty.contains("fixture build failed"), "{pretty}");
    assert!(!pretty.contains("additional diagnostics"), "{pretty}");
    assert!(!pretty.contains("completed-job"), "{pretty}");

    let raw = stdout(api.command().args(["history", "--json"]).output().unwrap());
    assert_eq!(serde_json::from_str::<Value>(&raw).unwrap(), jobs);
}
