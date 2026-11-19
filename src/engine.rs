use crate::config::Config;
use anyhow::{Context, Result, bail};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, clap::ValueEnum, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    Check,
    Build,
    Diff,
    UpdateInputs,
    Switch,
    Rollback,
    Clean,
}

impl Action {
    pub fn privileged(self) -> bool {
        matches!(self, Self::Switch | Self::Rollback | Self::Clean)
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Build => "build",
            Self::Diff => "diff",
            Self::UpdateInputs => "update-inputs",
            Self::Switch => "switch",
            Self::Rollback => "rollback",
            Self::Clean => "clean",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Job {
    pub id: String,
    pub action: Action,
    pub status: String,
    pub started: u64,
    pub finished: Option<u64>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Status {
    pub version: String,
    pub host: String,
    pub config_path: PathBuf,
    pub current_system: Option<PathBuf>,
    pub system_changes_enabled: bool,
    pub active: Option<String>,
    #[serde(default)]
    pub active_action: Option<Action>,
    #[serde(default)]
    pub cache: Option<CacheStatus>,
    /// Environment type detected from `config_path`. Defaults to flake so
    /// that statuses from older (flake-only) peers still deserialize.
    #[serde(default)]
    pub config_kind: crate::config::ConfigKind,
    /// A switch installed a new version and the server will restart itself
    /// once the current operation finishes.
    #[serde(default)]
    pub needs_restart: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CacheStatus {
    pub url: String,
    pub public_key: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Step {
    pub program: String,
    pub args: Vec<String>,
    /// Working directory override. `None` keeps the historical behavior
    /// (run inside `config_path` when it is a directory).
    /// Legacy `nixos-rebuild build` has no `--out-link` flag: it always
    /// leaves a `result` symlink in the current directory, so legacy
    /// builds run in `state_dir` to keep the GC root next to the flake one.
    pub cwd: Option<PathBuf>,
}
impl Step {
    fn new(program: &str, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
            cwd: None,
        }
    }
    /// A `nix` invocation that works regardless of the user's nix.conf.
    /// The NixOS module sets NIX_CONFIG for the systemd service, but the
    /// daemon/CLI can also run elsewhere, where `nix-command` and `flakes`
    /// may be disabled (the default on plain Nix installs).
    fn nix(args: Vec<String>) -> Self {
        let mut full = vec![
            "--extra-experimental-features".into(),
            "nix-command flakes".into(),
        ];
        full.extend(args);
        Self::new("nix", full)
    }
}

pub fn plan(config: &Config, action: Action) -> Result<Vec<Step>> {
    if action.privileged() {
        let helper = config.privileged_helper.as_ref().context(
            "System changes are disabled. Enable programs.yonix.allowSystemChanges in NixOS.",
        )?;
        return Ok(vec![Step::new(
            "sudo",
            vec![
                "-n".into(),
                "--".into(),
                helper.to_string_lossy().into_owned(),
                action.name().into(),
            ],
        )]);
    }
    if config.config_kind() == crate::config::ConfigKind::Legacy {
        return legacy_plan(config, action);
    }
    let target = format!(
        "{}#nixosConfigurations.{}.config.system.build.toplevel",
        config.flake_source(),
        config.host
    );
    let result = config
        .state_dir
        .join("result")
        .to_string_lossy()
        .into_owned();
    let build = || {
        Step::nix(vec![
            "build".into(),
            "--no-write-lock-file".into(),
            "--out-link".into(),
            result.clone(),
            target.clone(),
        ])
    };
    Ok(match action {
        Action::Check => vec![Step::nix(vec![
            "eval".into(),
            "--raw".into(),
            "--no-write-lock-file".into(),
            format!("{target}.drvPath"),
        ])],
        Action::Build => vec![build()],
        Action::Diff => vec![
            build(),
            Step::nix(vec![
                "store".into(),
                "diff-closures".into(),
                "/run/current-system".into(),
                result,
            ]),
        ],
        Action::UpdateInputs => vec![Step::nix(vec![
            "flake".into(),
            "update".into(),
            "--flake".into(),
            config.config_path.to_string_lossy().into_owned(),
        ])],
        _ => unreachable!(),
    })
}

/// Plans for legacy (non-flake) configurations. `source_mode` does not
/// apply here: there is no flake input for it to describe. The `--no-flake`
/// flag guards against a stray `flake.nix` appearing mid-flight (detection
/// says legacy, but `nixos-rebuild` would otherwise auto-switch to it).
fn legacy_plan(config: &Config, action: Action) -> Result<Vec<Step>> {
    if action == Action::UpdateInputs {
        bail!("Legacy configurations have no flake.lock; update channels outside Yonix");
    }
    let include = format!(
        "nixos-config={}",
        config
            .config_path
            .join("configuration.nix")
            .to_string_lossy()
    );
    let rebuild = |args: Vec<String>| {
        let mut full = vec!["--no-flake".into(), "-I".into(), include.clone()];
        full.extend(args);
        Step::new("nixos-rebuild", full)
    };
    let result = config
        .state_dir
        .join("result")
        .to_string_lossy()
        .into_owned();
    // `nixos-rebuild build` has no `--out-link`: the `result` symlink lands
    // in the working directory, so run it in `state_dir` (see `Step::cwd`).
    let build = || {
        let mut step = rebuild(vec!["build".into()]);
        step.cwd = Some(config.state_dir.clone());
        step
    };
    Ok(match action {
        Action::Check => vec![rebuild(vec!["dry-build".into()])],
        Action::Build => vec![build()],
        Action::Diff => vec![
            build(),
            Step::nix(vec![
                "store".into(),
                "diff-closures".into(),
                "/run/current-system".into(),
                result,
            ]),
        ],
        _ => unreachable!(),
    })
}

struct Inner {
    jobs: Vec<Job>,
    active: Option<String>,
    restart_target: Option<PathBuf>,
}
#[derive(Clone)]
pub struct Engine {
    pub config: Config,
    pub connections: Arc<crate::peers::Connections>,
    inner: Arc<Mutex<Inner>>,
    _lock: Arc<File>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl Engine {
    pub fn open(config: Config) -> Result<Self> {
        config.validate()?;
        fs::create_dir_all(&config.state_dir)?;
        fs::set_permissions(&config.state_dir, fs::Permissions::from_mode(0o700))?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(config.state_dir.join("daemon.lock"))?;
        lock.try_lock_exclusive()
            .context("Another yonix server is already using this state directory")?;
        fs::create_dir_all(config.state_dir.join("jobs"))?;
        let history = config.state_dir.join("history.json");
        let mut jobs: Vec<Job> = if history.exists() {
            serde_json::from_slice(&fs::read(history)?)?
        } else {
            vec![]
        };
        for job in &mut jobs {
            if job.status == "running" {
                job.status = "interrupted".into();
                job.finished = Some(now());
                job.error = Some("Server stopped before recording the result; inspect the log and system state before retrying.".into());
            }
        }
        let engine = Self {
            connections: Arc::new(crate::peers::Connections::open(&config)?),
            config,
            inner: Arc::new(Mutex::new(Inner {
                jobs,
                active: None,
                restart_target: None,
            })),
            _lock: Arc::new(lock),
        };
        engine.save(&engine.inner.lock().unwrap())?;
        Ok(engine)
    }

    fn save(&self, inner: &Inner) -> Result<()> {
        let path = self.config.state_dir.join("history.json.tmp");
        fs::write(&path, serde_json::to_vec_pretty(&inner.jobs)?)?;
        fs::rename(path, self.config.state_dir.join("history.json"))?;
        Ok(())
    }

    pub fn status(&self) -> Status {
        let inner = self.inner.lock().unwrap();
        let active_action = inner
            .active
            .as_ref()
            .and_then(|id| inner.jobs.iter().find(|j| &j.id == id).map(|j| j.action));
        Status {
            cache: self.config.cache.as_ref().map(|cache| CacheStatus {
                url: cache.url.clone(),
                public_key: fs::read_to_string(&cache.public_key_file)
                    .ok()
                    .map(|key| key.trim().to_string()),
            }),
            version: env!("CARGO_PKG_VERSION").into(),
            host: self.config.host.clone(),
            config_path: self.config.config_path.clone(),
            current_system: fs::read_link("/run/current-system").ok(),
            system_changes_enabled: self.config.privileged_helper.is_some(),
            active: inner.active.clone(),
            active_action,
            config_kind: self.config.config_kind(),
            needs_restart: inner.restart_target.is_some(),
        }
    }

    pub(crate) fn restart_target(&self) -> Option<PathBuf> {
        self.inner.lock().unwrap().restart_target.clone()
    }
    pub fn jobs(&self) -> Vec<Job> {
        self.inner
            .lock()
            .unwrap()
            .jobs
            .iter()
            .rev()
            .cloned()
            .collect()
    }
    pub fn job(&self, id: &str) -> Option<Job> {
        self.inner
            .lock()
            .unwrap()
            .jobs
            .iter()
            .find(|j| j.id == id)
            .cloned()
    }
    pub fn log(&self, id: &str) -> Result<String> {
        if self.job(id).is_none() {
            bail!("Unknown job");
        }
        let mut file = File::open(self.config.state_dir.join("jobs").join(format!("{id}.log")))?;
        let len = file.metadata()?.len();
        file.seek(SeekFrom::Start(len.saturating_sub(128 * 1024)))?;
        let mut bytes = Vec::new();
        file.take(128 * 1024).read_to_end(&mut bytes)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
    pub fn start(&self, action: Action) -> Result<Job> {
        let steps = plan(&self.config, action)?;
        if self.config.config_kind() == crate::config::ConfigKind::Unknown
            && action != Action::Clean
            && action != Action::Rollback
        {
            bail!(
                "No flake.nix or configuration.nix in {}",
                self.config.config_path.display()
            );
        }
        let mut inner = self.inner.lock().unwrap();
        if inner.active.is_some() {
            bail!("Another operation is running");
        }
        if inner.restart_target.is_some() {
            bail!("Yonix is restarting after the system switch; retry when it is available");
        }
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_nanos()
            .to_string();
        let job = Job {
            id: id.clone(),
            action,
            status: "running".into(),
            started: now(),
            finished: None,
            error: None,
        };
        let log = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(self.config.state_dir.join("jobs").join(format!("{id}.log")))?;
        inner.jobs.push(job.clone());
        inner.active = Some(id.clone());
        if let Err(error) = self.save(&inner) {
            inner.jobs.pop();
            inner.active = None;
            return Err(error);
        }
        drop(inner);
        let engine = self.clone();
        std::thread::spawn(move || {
            let result = execute(&engine.config, steps, log);
            let mut inner = engine.inner.lock().unwrap();
            let job = inner.jobs.iter_mut().find(|j| j.id == id).unwrap();
            let succeeded = result.is_ok();
            job.status = if succeeded { "succeeded" } else { "failed" }.into();
            job.error = result.err().map(|e| format!("{e:#}"));
            job.finished = Some(now());
            inner.active = None;
            if let Err(error) = engine.save(&inner) {
                eprintln!("Cannot persist job result: {error:#}");
                return;
            }
            if succeeded && action == Action::Switch {
                inner.restart_target = updated_launcher();
                if inner.restart_target.is_some() {
                    eprintln!("Switch installed a new Yonix launcher; scheduling restart");
                }
            }
        });
        Ok(job)
    }
}

/// The Nix package records its outer launcher because `/proc/self/exe`
/// points to the wrapped ELF, while the system profile points to its launcher.
fn updated_launcher() -> Option<PathBuf> {
    let running = std::env::var_os("YONIX_LAUNCHER")
        .and_then(|launcher| fs::canonicalize(launcher).ok())
        .or_else(|| fs::canonicalize("/proc/self/exe").ok())?;
    let activated = fs::canonicalize("/run/current-system/sw/bin/yonix").ok()?;
    (running != activated).then_some(activated)
}

fn execute(config: &Config, steps: Vec<Step>, mut log: File) -> Result<()> {
    for step in steps {
        writeln!(
            log,
            "$ {} {}",
            step.program,
            step.args
                .iter()
                .map(|s| format!("{s:?}"))
                .collect::<Vec<_>>()
                .join(" ")
        )?;
        let dir;
        let status = Command::new(&step.program)
            .args(&step.args)
            .current_dir(if let Some(cwd) = &step.cwd {
                dir = cwd.clone();
                &dir
            } else if config.config_path.is_dir() {
                &config.config_path
            } else {
                &config.state_dir
            })
            .env("NO_COLOR", "1")
            .env("TERM", "dumb")
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log.try_clone()?)
            .status()
            .with_context(|| format!("Cannot launch {}", step.program))?;
        if !status.success() {
            writeln!(log, "\nOperation failed: {status}")?;
            bail!(
                "{} exited with {status}; see the operation log",
                step.program
            );
        }
    }
    writeln!(log, "\nOperation completed successfully.")?;
    Ok(())
}

pub fn read_token(config: &Config) -> Result<String> {
    read_token_path(&config.token_path())
}

pub fn read_token_path(path: &std::path::Path) -> Result<String> {
    let token = fs::read_to_string(path)
        .context("Cannot read access token. Is the yonix service running for this user?")?;
    let token = token.trim();
    if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("Invalid token file");
    }
    Ok(token.into())
}

pub fn ensure_token(config: &Config) -> Result<String> {
    if config.token_path().exists() {
        return read_token(config);
    }
    let mut random = [0u8; 32];
    File::open("/dev/urandom")?.read_exact(&mut random)?;
    let token: String = random.iter().map(|b| format!("{b:02x}")).collect();
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(config.token_path())
    {
        Ok(mut file) => {
            file.write_all(token.as_bytes())?;
            Ok(token)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => read_token(config),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_restart_rejects_new_jobs() {
        let temp = tempfile::tempdir().unwrap();
        let config = Config {
            config_path: temp.path().join("config"),
            state_dir: temp.path().join("state"),
            ..Config::default()
        };
        fs::create_dir(&config.config_path).unwrap();
        fs::write(config.config_path.join("flake.nix"), "{}").unwrap();
        let engine = Engine::open(config).unwrap();
        engine.inner.lock().unwrap().restart_target = Some(temp.path().join("yonix"));

        assert!(engine.status().needs_restart);
        let error = engine.start(Action::Check).unwrap_err();
        assert!(error.to_string().contains("Yonix is restarting"));
        assert!(engine.jobs().is_empty());
    }
}
