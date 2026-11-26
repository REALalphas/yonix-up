use anyhow::{Context, Result, bail, ensure};
use clap::{Parser, Subcommand};
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use yonix::{client::Client, config::Config, engine::Action, terminal::Theme};

#[derive(Parser)]
#[command(version, about = "Your NixOS, from the desktop, terminal or browser")]
struct Args {
    #[arg(long, default_value = "/etc/yonix/config.json", global = true)]
    config: PathBuf,
    /// Loopback URL of a local server or SSH tunnel
    #[arg(long, global = true)]
    url: Option<String>,
    #[arg(long, global = true)]
    token_file: Option<PathBuf>,
    /// Color output mode: auto, always, never
    #[arg(long, global = true, value_parser = ["auto", "always", "never"])]
    color: Option<String>,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Open the native libadwaita application (default)
    Gui,
    /// Start the local API and web panel
    Serve,
    /// Show host, configuration and active operation
    Status {
        /// Print machine-readable JSON
        #[arg(long)]
        json: bool,
        /// Force human-friendly formatted terminal output
        #[arg(long)]
        pretty: bool,
    },
    /// Evaluate the system derivation without building or applying
    Check,
    /// Build without applying or changing hostname
    Build,
    /// Build and compare closures with the running system
    Diff,
    /// Update flake.lock without applying the system
    UpdateInputs {
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Build and activate the configured NixOS system
    #[command(alias = "update")]
    Switch {
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Activate the previous system generation
    Rollback {
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Check and clean system leftovers, old generations, journals and the Nix store
    Clean {
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Show operation history
    History {
        /// Print machine-readable JSON
        #[arg(long)]
        json: bool,
        /// Force human-friendly formatted terminal output
        #[arg(long)]
        pretty: bool,
        /// Limit the number of past operations to display
        #[arg(long, short = 'n')]
        limit: Option<usize>,
    },
    /// Show the last 128 KiB of an operation log
    Logs { id: String },
    /// Print the private API token for browser sign-in
    Token,
    /// Generate a short invitation for two-way pairing
    Pair {
        /// Explicit initiator origin, e.g. http://10.0.0.1:9847
        #[arg(long)]
        origin: Option<String>,
        /// Additional fallback path (repeatable): --extra-path <url> --extra-path-role <role>
        #[arg(long)]
        extra_path: Vec<String>,
        /// Role for each --extra-path, in order (default: direct)
        #[arg(long)]
        extra_path_role: Vec<String>,
        /// Print machine-readable JSON instead of the bare link
        #[arg(long)]
        json: bool,
    },
    /// Redeem a pairing link or bare code and link both hosts
    PairAccept {
        /// Full invitation or the six-character code from a host on the local network
        link: String,
        /// Optional initiator address when multicast discovery is unavailable
        #[arg(long)]
        origin: Option<String>,
        /// Label for the saved connection
        #[arg(long)]
        label: Option<String>,
    },
    /// Open the browser panel
    Web,
    /// Print an example configuration, without changing any files
    Init,
}

fn confirm(message: &str, yes: bool, theme: &Theme) -> Result<()> {
    if yes {
        return Ok(());
    }
    print!(
        "{} {} {} ",
        theme.question(),
        theme.bold(message),
        theme.dim("[y/N]")
    );
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    if !matches!(answer.trim(), "y" | "Y" | "yes") {
        println!("{}", theme.yellow("Operation cancelled."));
        bail!("Cancelled");
    }
    Ok(())
}

fn pairing_role(text: &str) -> Result<yonix::pairing::PathRole> {
    match text.to_ascii_lowercase().as_str() {
        "direct" => Ok(yonix::pairing::PathRole::Direct),
        "dns" => Ok(yonix::pairing::PathRole::Dns),
        "ssh" => Ok(yonix::pairing::PathRole::Ssh),
        "public" => Ok(yonix::pairing::PathRole::Public),
        "relay" => Ok(yonix::pairing::PathRole::Relay),
        other => bail!("Unknown path role '{other}': direct, dns, ssh, public or relay"),
    }
}

fn pair_command(
    config: &Config,
    origin: Option<String>,
    extra_paths: Vec<String>,
    extra_roles: Vec<String>,
    json: bool,
    theme: &Theme,
) -> Result<()> {
    use yonix::pairing::{PairPayload, PathCandidate, PathRole};
    if extra_roles.len() > extra_paths.len() {
        bail!("More --extra-path-role values than --extra-path values");
    }
    // Mint a one-time server-side code: the daemon owns expiry and single
    // use, so a copied link cannot outlive or replay the pairing.
    let client = Client::new(config, None, None)?;
    client.post_json(
        "/api/pairings/settings",
        &serde_json::json!({ "enabled": true }),
    )?;
    let invitation = client.post_json("/api/pairings", &serde_json::json!({}))?;
    let code: String = invitation
        .get("code")
        .and_then(|code| code.as_str())
        .context("Pairing server did not return a code")?
        .into();
    let default_link = invitation["link"]
        .as_str()
        .context("Missing pairing invitation")?;
    let advertised = yonix::pairing::parse_link(default_link)?;
    let use_default = origin.is_none() && extra_paths.is_empty();
    let primary = origin
        .or_else(|| {
            advertised
                .payload
                .paths
                .first()
                .map(|path| path.url.clone())
        })
        .context("Missing initiator address in invitation")?;
    let mut paths = vec![PathCandidate {
        url: primary,
        role: PathRole::Direct,
    }];
    for (index, url) in extra_paths.iter().enumerate() {
        let role = extra_roles
            .get(index)
            .map(|role| pairing_role(role))
            .transpose()?
            .unwrap_or(PathRole::Direct);
        paths.push(PathCandidate {
            url: url.clone(),
            role,
        });
    }
    let payload = PairPayload {
        v: 1,
        host: config.host.clone(),
        paths,
    };
    let link = if use_default {
        default_link.to_owned()
    } else {
        format!("!{}", yonix::pairing::encode_link(&payload, &code)?)
    };
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "link": link,
                "code": code,
                "host": payload.host,
                "paths": payload.paths,
            }))?
        );
    } else if theme.is_terminal {
        yonix::terminal::print_pair_info(&link, &code, &payload.host, theme);
    } else {
        println!("{link}");
    }
    Ok(())
}

fn pair_accept_command(
    config: &Config,
    link: String,
    origin: Option<String>,
    label: Option<String>,
    theme: &Theme,
) -> Result<()> {
    use yonix::pairing::PathRole;
    let parsed = yonix::pairing::parse_link(&link)?;
    let public = parsed
        .payload
        .paths
        .iter()
        .any(|p| p.role == PathRole::Public);
    if public {
        confirm("Allow public pairing paths?", false, theme)?;
    }
    let saved = Client::new(config, None, None)?.post_json(
        "/api/pairings/accept",
        &serde_json::json!({
            "link":link, "origin":origin, "label":label, "allow_public":public,
        }),
    )?;
    println!(
        "{} Paired with {} via {}",
        theme.check(),
        theme.bold(saved["label"].as_str().context("Missing peer label")?),
        theme.cyan(saved["url"].as_str().context("Missing peer address")?)
    );
    Ok(())
}

fn operation(client: Client, action: Action, theme: &Theme) -> Result<()> {
    let start_time = std::time::Instant::now();
    let job = client.start(action, true)?;
    let priv_badge = if action.privileged() {
        format!(" {}", theme.yellow("[privileged]"))
    } else {
        String::new()
    };
    println!(
        "{} Starting {} · {}{priv_badge}",
        theme.bullet(),
        theme.bold(action.name()),
        theme.dim(&job.id)
    );
    let mut last_log = String::new();
    loop {
        let status = client.job(&job.id)?;
        let log = client.log(&job.id)?;
        let extra = if let Some(diff) = log.strip_prefix(&last_log) {
            diff
        } else if log != last_log {
            &log
        } else {
            ""
        };
        if !extra.is_empty() {
            if theme.enabled {
                for line in extra.split_inclusive('\n') {
                    print!("{}", yonix::terminal::highlight_log_line(line, theme));
                }
            } else {
                print!("{extra}");
            }
            io::stdout().flush()?;
        }
        last_log = log;
        if status.status != "running" {
            let elapsed_str = yonix::terminal::format_duration(start_time.elapsed().as_secs());
            if status.status != "succeeded" {
                let err_msg = status.error.unwrap_or(status.status);
                eprintln!(
                    "{} Operation '{}' failed after {} · job {}: {}",
                    theme.cross(),
                    theme.bold(action.name()),
                    theme.dim(&elapsed_str),
                    theme.dim(&job.id),
                    theme.red(&err_msg)
                );
                eprintln!(
                    "  {} Run '{}' to view the full operation log.",
                    theme.arrow(),
                    theme.bold(&format!("yonix logs {}", job.id))
                );
                bail!("Operation failed");
            }
            println!(
                "{} Operation '{}' completed successfully in {} · job {}",
                theme.check(),
                theme.bold(action.name()),
                theme.green(&elapsed_str),
                theme.dim(&job.id)
            );
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn check_service_user(config_path: &Path) -> Result<()> {
    let systemd_pid = std::env::var("SYSTEMD_EXEC_PID")
        .ok()
        .and_then(|pid| pid.parse().ok());
    if systemd_pid != Some(std::process::id()) {
        return Ok(());
    }

    // A self-update exec keeps its original credentials. Read the account
    // from the activated generation so systemd can restart us with its User=.
    let path = config_path.with_file_name("service-user");
    let expected = match std::fs::read_to_string(&path) {
        Ok(user) => user,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).with_context(|| format!("Cannot read {}", path.display())),
    };
    let expected = expected.trim();
    let identity = std::process::Command::new("id").arg("-un").output()?;
    ensure!(
        identity.status.success(),
        "Cannot identify the Yonix service user"
    );
    let running = String::from_utf8_lossy(&identity.stdout);
    let running = running.trim();
    ensure!(
        running == expected,
        "Yonix is running as {running} after a service account change to {expected}. Restart yonix.service to apply the new credentials."
    );
    Ok(())
}

fn run() -> Result<()> {
    let args = Args::parse();
    let theme = Theme::detect(args.color.as_deref());
    let command = args.command.unwrap_or(Commands::Gui);
    if matches!(command, Commands::Init) {
        let sample = serde_json::to_string_pretty(&Config::default())?;
        yonix::terminal::print_json_colored(&sample, &theme);
        return Ok(());
    }
    let config = Config::load(&args.config)?;
    if matches!(command, Commands::Serve) {
        check_service_user(&args.config)?;
        return tokio::runtime::Runtime::new()?.block_on(yonix::server::run(config));
    }
    if matches!(command, Commands::Token) {
        println!("{}", yonix::engine::read_token(&config)?);
        return Ok(());
    }
    if let Commands::Pair {
        origin,
        extra_path,
        extra_path_role,
        json,
    } = command
    {
        return pair_command(&config, origin, extra_path, extra_path_role, json, &theme);
    }
    if let Commands::PairAccept {
        link,
        origin,
        label,
    } = command
    {
        return pair_accept_command(&config, link, origin, label, &theme);
    }
    #[cfg(feature = "gui")]
    if matches!(command, Commands::Gui)
        && (std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some())
    {
        let endpoint = Client::endpoint(&config, args.url.clone())?;
        let endpoint = if let Some(file) = args.token_file.clone() {
            endpoint.with_token(&std::fs::read_to_string(file)?)?
        } else {
            endpoint
        };
        yonix::gui::run(endpoint);
        return Ok(());
    }
    let client = Client::new(&config, args.url.clone(), args.token_file)?;
    match command {
        Commands::Gui => {
            #[cfg(feature = "gui")]
            {
                if std::env::var_os("DISPLAY").is_none()
                    && std::env::var_os("WAYLAND_DISPLAY").is_none()
                {
                    eprintln!(
                        "{}",
                        theme.yellow(
                            "No graphical display detected ($DISPLAY and $WAYLAND_DISPLAY unset)."
                        )
                    );
                    eprintln!(
                        "Showing system status instead. Run 'yonix --help' for available commands.\n"
                    );
                    let status = client.status()?;
                    yonix::terminal::print_status(&status, &theme);
                    return Ok(());
                }
                yonix::gui::run(client);
            }
            #[cfg(not(feature = "gui"))]
            {
                let status = client.status()?;
                yonix::terminal::print_status(&status, &theme);
            }
        }
        Commands::Web => {
            let url = args.url.unwrap_or_else(|| config.base_url());
            if theme.is_terminal {
                println!(
                    "{} Opening browser panel at {}...",
                    theme.bullet(),
                    theme.cyan(&url)
                );
            }
            let status = std::process::Command::new("xdg-open").arg(url).status()?;
            if !status.success() {
                bail!("Cannot open browser");
            }
        }
        Commands::Status { json, pretty } => {
            let status = client.status()?;
            let should_format =
                pretty || (theme.is_terminal && !json) || args.color.as_deref() == Some("always");
            if should_format && !json {
                yonix::terminal::print_status(&status, &theme);
            } else {
                println!("{}", serde_json::to_string_pretty(&status)?);
            }
        }
        Commands::History {
            json,
            pretty,
            limit,
        } => {
            let jobs = client.jobs()?;
            let should_format =
                pretty || (theme.is_terminal && !json) || args.color.as_deref() == Some("always");
            if should_format && !json {
                yonix::terminal::print_history(&jobs, limit, &theme);
            } else {
                println!("{}", serde_json::to_string_pretty(&jobs)?);
            }
        }
        Commands::Logs { id } => {
            let log = client.log(&id)?;
            yonix::terminal::print_logs(&id, &log, &theme);
        }
        Commands::Check => operation(client, Action::Check, &theme)?,
        Commands::Build => operation(client, Action::Build, &theme)?,
        Commands::Diff => operation(client, Action::Diff, &theme)?,
        Commands::UpdateInputs { yes } => {
            confirm(
                "Update flake.lock in the configuration repository?",
                yes,
                &theme,
            )?;
            operation(client, Action::UpdateInputs, &theme)?;
        }
        Commands::Switch { yes } => {
            confirm("Build and apply the configured system?", yes, &theme)?;
            operation(client, Action::Switch, &theme)?;
        }
        Commands::Rollback { yes } => {
            confirm("Activate the previous system generation?", yes, &theme)?;
            operation(client, Action::Rollback, &theme)?;
        }
        Commands::Clean { yes } => {
            confirm(
                "Clean expired system files, old journals and Nix generations, and run system checks?",
                yes,
                &theme,
            )?;
            operation(client, Action::Clean, &theme)?;
        }
        _ => unreachable!(),
    }
    Ok(())
}

fn main() {
    if let Err(err) = run() {
        if err.to_string() == "Cancelled" {
            std::process::exit(1);
        }
        let theme = Theme::detect(None);
        eprintln!("{} {:#}", theme.bold_red("error:"), err);
        std::process::exit(1);
    }
}
