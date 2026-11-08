use std::{fs, path::PathBuf};
use yonix::{
    config::{Config, validate_base_url},
    engine::{self, Action, Engine, plan},
};

fn fixture() -> (tempfile::TempDir, Config) {
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        config_path: dir.path().join("repo with spaces"),
        state_dir: dir.path().join("state"),
        host: "nas".into(),
        ..Config::default()
    };
    fs::create_dir(&config.config_path).unwrap();
    fs::write(config.config_path.join("flake.nix"), "{}").unwrap();
    (dir, config)
}

#[test]
fn build_never_switches_or_elevates() {
    let (_dir, config) = fixture();
    let steps = plan(&config, Action::Build).unwrap();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].program, "nix");
    assert_eq!(steps[0].args[0], "build");
    assert!(steps[0].args.contains(&"--no-write-lock-file".into()));
    assert!(
        steps[0]
            .args
            .last()
            .unwrap()
            .starts_with(&config.config_path.to_string_lossy().to_string())
    );
    assert!(!steps[0].args.iter().any(|a| a == "switch" || a == "sudo"));
}
#[test]
fn check_evaluates_without_building() {
    let (_dir, config) = fixture();
    let steps = plan(&config, Action::Check).unwrap();
    assert_eq!(steps[0].args[0], "eval");
    assert!(steps[0].args.last().unwrap().ends_with(".drvPath"));
}
#[test]
fn inputs_update_does_not_apply() {
    let (_dir, config) = fixture();
    let steps = plan(&config, Action::UpdateInputs).unwrap();
    assert_eq!(steps.len(), 1);
    assert_eq!(
        steps[0].args,
        [
            "flake",
            "update",
            "--flake",
            config.config_path.to_str().unwrap()
        ]
    );
}
#[test]
fn diff_builds_then_compares_running_system() {
    let (_dir, config) = fixture();
    let steps = plan(&config, Action::Diff).unwrap();
    assert_eq!(steps.len(), 2);
    assert_eq!(
        steps[1].args[0..3],
        ["store", "diff-closures", "/run/current-system"]
    );
}
#[test]
fn changes_require_explicit_helper() {
    let (_dir, mut config) = fixture();
    for action in [Action::Switch, Action::Rollback, Action::Clean] {
        assert!(plan(&config, action).is_err());
    }
    config.privileged_helper = Some(PathBuf::from("/nix/store/example-yonix-system"));
    let steps = plan(&config, Action::Switch).unwrap();
    assert_eq!(steps[0].program, "sudo");
    assert_eq!(
        steps[0].args,
        ["-n", "--", "/nix/store/example-yonix-system", "switch"]
    );
}
#[test]
fn validates_config_and_rejects_typos() {
    let (_dir, mut config) = fixture();
    config.host = "nas; reboot".into();
    assert!(config.validate().is_err());
    config.host = "nas".into();
    config.state_dir = "relative".into();
    assert!(config.validate().is_err());
    assert!(serde_json::from_str::<Config>(r#"{"config_pth":"/tmp"}"#).is_err());
}
#[test]
fn client_tokens_cannot_be_sent_to_remote_http() {
    for url in ["http://127.0.0.1:9847", "http://127.0.0.1:1234"] {
        assert!(validate_base_url(url).is_ok());
    }
    for url in [
        "http://example.org",
        "http://192.168.1.5:9847",
        "https://127.0.0.1:9847",
        "http://secret@127.0.0.1:9847",
        "http://127.0.0.1:9847/path",
        "http://127.0.0.1:9847?token=secret",
    ] {
        assert!(validate_base_url(url).is_err(), "{url}");
    }
}
#[test]
fn token_is_private_and_stable() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, config) = fixture();
    let engine = Engine::open(config.clone()).unwrap();
    let token = engine::ensure_token(&config).unwrap();
    assert_eq!(token.len(), 64);
    assert_eq!(engine::ensure_token(&config).unwrap(), token);
    assert_eq!(
        fs::metadata(config.token_path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(&config.state_dir)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    drop(engine);
}
#[test]
fn daemon_is_exclusive_and_history_survives() {
    let (_dir, config) = fixture();
    let engine = Engine::open(config.clone()).unwrap();
    assert!(Engine::open(config.clone()).is_err());
    drop(engine);
    assert!(Engine::open(config).is_ok());
}
#[test]
fn startup_marks_incomplete_operations_interrupted() {
    let (_dir, config) = fixture();
    fs::create_dir_all(&config.state_dir).unwrap();
    fs::write(config.state_dir.join("history.json"), r#"[{"id":"123","action":"build","status":"running","started":1,"finished":null,"error":null}]"#).unwrap();
    let engine = Engine::open(config).unwrap();
    let job = engine.job("123").unwrap();
    assert_eq!(job.status, "interrupted");
    assert!(job.error.is_some());
    assert!(engine.status().active.is_none());
}
#[test]
fn rejects_unknown_log_ids_including_path_traversal() {
    let (_dir, config) = fixture();
    let engine = Engine::open(config).unwrap();
    assert!(engine.log("../../token").is_err());
}
#[test]
fn token_comparison_rejects_prefix_and_suffix() {
    use yonix::server::token_matches;
    assert!(token_matches("0123", "0123"));
    for bad in ["", "012", "01234", "1123"] {
        assert!(!token_matches("0123", bad));
    }
}

#[test]
fn path_mode_explicitly_includes_local_files() {
    let (_dir, mut config) = fixture();
    config.source_mode = "path".into();
    config.validate().unwrap();
    let steps = plan(&config, Action::Check).unwrap();
    assert!(steps[0].args.last().unwrap().starts_with("path:/"));
    config.source_mode = "unexpected".into();
    assert!(config.validate().is_err());
}

#[test]
fn lan_listening_is_explicit_and_rejects_public_or_wildcard_addresses() {
    let (_dir, mut config) = fixture();
    assert_eq!(config.listen_address, std::net::Ipv4Addr::LOCALHOST);
    config.listen_address = "10.0.2.142".parse().unwrap();
    config.validate().unwrap();
    assert!(yonix::server::valid_host(
        "10.0.2.142:9847",
        config.listen_address
    ));
    assert!(!yonix::server::valid_host(
        "evil.example:9847",
        config.listen_address
    ));
    assert!(!yonix::server::valid_host(
        "10.0.2.143:9847",
        config.listen_address
    ));
    for address in ["0.0.0.0", "8.8.8.8", "224.0.0.1", "255.255.255.255"] {
        config.listen_address = address.parse().unwrap();
        assert!(config.validate().is_err());
    }
}
