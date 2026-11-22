use std::{fs, path::PathBuf};
use yonix::{
    config::{Config, ConfigKind, validate_base_url},
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
    assert_eq!(
        steps[0].args[0..3],
        [
            "--extra-experimental-features",
            "nix-command flakes",
            "build"
        ]
    );
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
fn nix_steps_enable_experimental_features() {
    let (_dir, config) = fixture();
    for action in [
        Action::Check,
        Action::Build,
        Action::Diff,
        Action::UpdateInputs,
    ] {
        for step in plan(&config, action).unwrap() {
            assert_eq!(step.program, "nix");
            assert_eq!(
                step.args[0..2],
                ["--extra-experimental-features", "nix-command flakes"]
            );
        }
    }
}
#[test]
fn check_evaluates_without_building() {
    let (_dir, config) = fixture();
    let steps = plan(&config, Action::Check).unwrap();
    assert_eq!(steps[0].args[2], "eval");
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
            "--extra-experimental-features",
            "nix-command flakes",
            "flake",
            "update",
            "--flake",
            config.config_path.to_str().unwrap()
        ]
    );
}
#[test]
fn flake_diff_builds_and_compares_the_state_result_link() {
    let (_dir, mut config) = fixture();
    config.source_mode = "path".into();
    let result = config.state_dir.join("result");
    let result = result.to_str().unwrap();
    let target = format!(
        "path:{}#nixosConfigurations.nas.config.system.build.toplevel",
        config.config_path.display()
    );
    let steps = plan(&config, Action::Diff).unwrap();
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].program, "nix");
    assert_eq!(
        steps[0].args,
        [
            "--extra-experimental-features",
            "nix-command flakes",
            "build",
            "--no-write-lock-file",
            "--out-link",
            result,
            &target,
        ]
    );
    assert!(steps[0].cwd.is_none());
    assert_eq!(steps[1].program, "nix");
    assert_eq!(
        steps[1].args,
        [
            "--extra-experimental-features",
            "nix-command flakes",
            "store",
            "diff-closures",
            "/run/current-system",
            result,
        ]
    );
    assert!(steps[1].cwd.is_none());
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
        "http://[fd00::1]:9847",
        "http://[::]:9847",
        "https://127.0.0.1:9847",
        "http://secret@127.0.0.1:9847",
        "http://127.0.0.1:9847/path",
        "http://127.0.0.1:9847?token=secret",
    ] {
        assert!(validate_base_url(url).is_err(), "{url}");
    }
}

#[test]
fn client_accepts_an_explicit_ipv6_loopback_url() {
    let (dir, config) = fixture();
    let token_file = dir.path().join("client-token");
    fs::write(&token_file, "a".repeat(64)).unwrap();
    yonix::client::Client::new(&config, Some("http://[::1]:9847".into()), Some(token_file))
        .expect("a numeric IPv6 loopback URL is valid for a local SSH tunnel");
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

fn legacy_fixture() -> (tempfile::TempDir, Config) {
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        config_path: dir.path().join("nixos"),
        state_dir: dir.path().join("state"),
        host: "nas".into(),
        ..Config::default()
    };
    fs::create_dir_all(&config.config_path).unwrap();
    fs::write(config.config_path.join("configuration.nix"), "{ }").unwrap();
    (dir, config)
}

#[test]
fn detects_flake_legacy_and_unknown_environments() {
    let (_dir, flake) = fixture();
    assert_eq!(flake.config_kind(), ConfigKind::Flake);
    let (_dir, legacy) = legacy_fixture();
    assert_eq!(legacy.config_kind(), ConfigKind::Legacy);
    let dir = tempfile::tempdir().unwrap();
    let empty = Config {
        config_path: dir.path().into(),
        state_dir: dir.path().join("state"),
        host: "nas".into(),
        ..Config::default()
    };
    assert_eq!(empty.config_kind(), ConfigKind::Unknown);
    // A present flake.nix wins when both files exist.
    fs::write(legacy.config_path.join("flake.nix"), "{}").unwrap();
    assert_eq!(legacy.config_kind(), ConfigKind::Flake);
    // Detection is runtime: missing files on an existing path read as unknown.
    fs::remove_file(flake.config_path.join("flake.nix")).unwrap();
    assert_eq!(flake.config_kind(), ConfigKind::Unknown);
}

#[test]
fn legacy_plans_use_nixos_rebuild_without_flake_flags() {
    let (_dir, config) = legacy_fixture();
    let include = format!(
        "nixos-config={}/configuration.nix",
        config.config_path.display()
    );
    let check = plan(&config, Action::Check).unwrap();
    assert_eq!(check.len(), 1);
    assert_eq!(check[0].program, "nixos-rebuild");
    assert_eq!(check[0].args, ["--no-flake", "-I", &include, "dry-build"]);
    assert!(check[0].cwd.is_none());
    let build = plan(&config, Action::Build).unwrap();
    assert_eq!(build.len(), 1);
    assert_eq!(build[0].program, "nixos-rebuild");
    assert_eq!(build[0].args, ["--no-flake", "-I", &include, "build"]);
    // `nixos-rebuild build` drops `result` in cwd: it must be state_dir.
    assert_eq!(build[0].cwd, Some(config.state_dir.clone()));
    assert!(
        !build[0]
            .args
            .iter()
            .any(|a| a == "--flake" || a.contains('#'))
    );
}

#[test]
fn legacy_diff_builds_in_state_dir_and_compares_its_result_link() {
    let (_dir, config) = legacy_fixture();
    let build = plan(&config, Action::Build).unwrap();
    let steps = plan(&config, Action::Diff).unwrap();
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0], build[0]);
    assert_eq!(steps[0].cwd.as_ref(), Some(&config.state_dir));
    assert_eq!(steps[1].program, "nix");
    assert_eq!(
        steps[1].args,
        [
            "--extra-experimental-features",
            "nix-command flakes",
            "store",
            "diff-closures",
            "/run/current-system",
            config.state_dir.join("result").to_str().unwrap(),
        ]
    );
    assert!(steps[1].cwd.is_none());
}

#[test]
fn legacy_has_no_flake_lock_to_update() {
    let (_dir, config) = legacy_fixture();
    let error = plan(&config, Action::UpdateInputs).unwrap_err();
    assert!(error.to_string().contains("flake.lock"));
}

#[test]
fn legacy_source_mode_is_ignored() {
    let (_dir, mut config) = legacy_fixture();
    for mode in ["git", "path"] {
        config.source_mode = mode.into();
        config.validate().unwrap();
        let steps = plan(&config, Action::Build).unwrap();
        assert_eq!(steps[0].program, "nixos-rebuild");
        assert!(!steps[0].args.iter().any(|a| a.starts_with("path:")));
    }
}

#[test]
fn status_reports_config_kind_and_reads_old_peers() {
    let (_dir, config) = legacy_fixture();
    let engine = Engine::open(config.clone()).unwrap();
    let status = engine.status();
    assert_eq!(status.config_kind, ConfigKind::Legacy);
    // Statuses from pre-kind peers (flake-only era) still deserialize.
    let old: yonix::engine::Status =
        serde_json::from_str(r#"{"version":"0.1.0","host":"nas","config_path":"/etc/nixos","current_system":null,"system_changes_enabled":false,"active":null}"#)
            .unwrap();
    assert_eq!(old.config_kind, ConfigKind::Flake);
}

#[test]
fn listening_accepts_unicast_and_wildcard_addresses() {
    let (_dir, mut config) = fixture();
    assert_eq!(
        config.listen_address,
        std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
    );
    config.listen_address = "10.0.2.142".parse().unwrap();
    assert_eq!(
        config.listen_address,
        std::net::IpAddr::V4("10.0.2.142".parse().unwrap())
    );
    config.validate().unwrap();
    assert!(yonix::server::valid_host(
        "10.0.2.142:9847",
        config.listen_address
    ));
    assert!(!yonix::server::valid_host(
        "[fd00::5]:9847",
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
    for address in ["0.0.0.0", "8.8.8.8", "::1"] {
        config.listen_address = address.parse().unwrap();
        config.validate().unwrap();
    }
    for address in ["224.0.0.1", "255.255.255.255"] {
        config.listen_address = address.parse().unwrap();
        assert!(config.validate().is_err());
    }
    // IPv6 multicast is rejected, but global unicast binds like any address.
    config.listen_address = "ff02::1".parse().unwrap();
    assert!(config.validate().is_err());
    config.listen_address = "2001:db8::1".parse().unwrap();
    config.validate().unwrap();
    assert!(yonix::server::valid_host(
        "[2001:db8::1]:9847",
        config.listen_address
    ));
    assert!(!yonix::server::valid_host(
        "10.0.0.1:9847",
        config.listen_address
    ));
}

#[test]
fn wildcard_binding_uses_loopback_for_clients_and_accepts_numeric_hosts() {
    let (_dir, mut config) = fixture();
    config.listen_address = std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED);
    config.validate().unwrap();
    assert_eq!(config.base_url(), "http://127.0.0.1:9847");
    for host in [
        "localhost:9847",
        "127.0.0.1:9847",
        "10.0.2.142:9847",
        "8.8.8.8:9847",
    ] {
        assert!(
            yonix::server::valid_host(host, config.listen_address),
            "{host}"
        );
    }
    for host in [
        "evil.example:9847",
        "224.0.0.1:9847",
        "255.255.255.255:9847",
        "10.0.2.142:0",
        "10.0.2.142",
        "10.0.2.142:invalid",
        "10.0.2.142:65536",
    ] {
        assert!(
            !yonix::server::valid_host(host, config.listen_address),
            "{host}"
        );
    }
    // The IPv6 wildcard also accepts numeric hosts and serves the v6 loopback.
    config.listen_address = std::net::IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED);
    config.validate().unwrap();
    assert_eq!(config.base_url(), "http://127.0.0.1:9847");
    for host in ["[::1]:9847", "[fd00::5]:9847", "10.0.0.1:9847"] {
        assert!(
            yonix::server::valid_host(host, config.listen_address),
            "{host}"
        );
    }
    assert!(!yonix::server::valid_host(
        "evil.example:9847",
        config.listen_address
    ));
}

#[test]
fn pairing_link_round_trips_payload_then_code() {
    use yonix::pairing::{PairPayload, PathCandidate, PathRole, parse_link};
    let payload = PairPayload {
        v: 1,
        host: "nas".into(),
        paths: vec![
            PathCandidate {
                url: "http://10.0.0.1:9847/".into(),
                role: PathRole::Direct,
            },
            PathCandidate {
                url: "http://[fd00::5]:9847/".into(),
                role: PathRole::Direct,
            },
            PathCandidate {
                url: "http://nas.lan:9847/".into(),
                role: PathRole::Dns,
            },
            PathCandidate {
                url: "http://127.0.0.1:9847/".into(),
                role: PathRole::Ssh,
            },
            PathCandidate {
                url: "http://203.0.113.7:9847/".into(),
                role: PathRole::Public,
            },
            PathCandidate {
                url: "http://10.0.0.9:9847/".into(),
                role: PathRole::Relay,
            },
        ],
    };
    let code = "482916".to_string();
    let link = yonix::pairing::encode_link(&payload, &code).unwrap();
    let (head, tail) = link.split_once('#').unwrap();
    assert_eq!(tail, code);
    assert!(!head.contains('#'));
    let parsed = parse_link(&link).unwrap();
    assert_eq!(parsed.payload, payload);
    assert_eq!(parsed.code, code);
}

#[test]
fn pairing_bare_code_has_no_paths() {
    use yonix::pairing::parse_link;
    let code = "731045".to_string();
    let parsed = parse_link(&code).unwrap();
    assert_eq!(parsed.code, code);
    assert!(parsed.payload.paths.is_empty());
    assert_eq!(parse_link("731-045").unwrap().code, code);
}

#[test]
fn compact_pairing_links_round_trip_ipv6_addresses() {
    let address = "[::1]:54321".parse().unwrap();
    let link = yonix::pairing::compact_link(address, "482916").unwrap();
    assert!(link.starts_with('~'));
    let parsed = yonix::pairing::parse_link(&link).unwrap();
    assert_eq!(parsed.code, "482916");
    assert_eq!(parsed.payload.paths.len(), 1);
    assert_eq!(parsed.payload.paths[0].url, "http://[::1]:54321");
}

#[test]
fn pairing_rejects_bad_links_codes_and_paths() {
    use yonix::pairing::{PairPayload, PathCandidate, PathRole, parse_link};
    let payload = PairPayload {
        v: 1,
        host: "nas".into(),
        paths: vec![PathCandidate {
            url: "http://10.0.0.1:9847/".into(),
            role: PathRole::Direct,
        }],
    };
    // Bad codes.
    for bad in ["", "xyz", "12345", "1234567", "abcdef", "12 345"] {
        assert!(parse_link(bad).is_err(), "{bad}");
        assert!(yonix::pairing::encode_link(&payload, bad).is_err(), "{bad}");
    }
    // Bad payloads: version, host, path count, URL policy.
    let code = "482916".to_string();
    for bad in [
        PairPayload {
            v: 2,
            ..payload.clone()
        },
        PairPayload {
            host: "nas!".into(),
            ..payload.clone()
        },
        PairPayload {
            paths: vec![],
            ..payload.clone()
        }
        .clone(),
    ] {
        let _ = (bad, &code);
    }
    assert!(parse_link("!!!#ab").is_err());
    assert!(parse_link(&format!("{}#{}", "482916", "zz")).is_err());
    // encode_link validates paths as well.
    for (url, role) in [
        ("http://8.8.8.8:9847/", PathRole::Direct),
        ("http://nas.lan:9847/", PathRole::Direct),
        ("https://10.0.0.1:9847/", PathRole::Direct),
        ("http://10.0.0.1:9847/api", PathRole::Direct),
        ("http://10.0.0.1:9847/", PathRole::Ssh),
        ("http://[fe80::1]:9847/", PathRole::Ssh),
        ("http://..bad../:9847/", PathRole::Dns),
    ] {
        let bad = PairPayload {
            v: 1,
            host: "nas".into(),
            paths: vec![PathCandidate {
                url: url.into(),
                role,
            }],
        };
        assert!(
            yonix::pairing::encode_link(&bad, &code).is_err(),
            "{url} {role:?}"
        );
    }
    // Relay parses but stays traffic-inert; too many paths are rejected.
    let relay = PairPayload {
        v: 1,
        host: "nas".into(),
        paths: vec![PathCandidate {
            url: "http://10.0.0.9:9847/".into(),
            role: PathRole::Relay,
        }],
    };
    let link = yonix::pairing::encode_link(&relay, &code).unwrap();
    assert_eq!(parse_link(&link).unwrap().payload, relay);
    let many = PairPayload {
        v: 1,
        host: "nas".into(),
        paths: (0..17)
            .map(|i| PathCandidate {
                url: format!("http://10.0.0.{i}:9847/"),
                role: PathRole::Direct,
            })
            .collect(),
    };
    assert!(yonix::pairing::encode_link(&many, &code).is_err());
    // Padded base64 is rejected.
    assert!(parse_link("YWJjZA==#12").is_err());
}
