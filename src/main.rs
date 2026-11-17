use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use std::{
    io::{self, Write},
    path::PathBuf,
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
    /// Generate a two-way pairing link: base64url(payload)#PAIRING_CODE
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
        /// Full link (base64url#code) or a bare 6-digit code (provide initiator address via --origin)
        link: String,
        /// Initiator address when redeeming a bare code
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
    let code: String = client
        .post_json("/api/pairings", &serde_json::json!({}))?
        .get("code")
        .and_then(|code| code.as_str())
        .context("Pairing server did not return a code")?
        .into();
    let primary = origin.unwrap_or_else(|| config.base_url());
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
    let link = yonix::pairing::encode_link(&payload, &code)?;
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
    let mut paths: Vec<(String, PathRole)> = parsed
        .payload
        .paths
        .iter()
        .map(|path| (path.url.clone(), path.role))
        .collect();
    if paths.is_empty() {
        let Some(origin) = origin else {
            bail!(
                "Bare code {}: pass --origin <url> to specify the initiator address manually",
                parsed.code
            );
        };
        paths.push((origin, PathRole::Direct));
    }
    let own_token = yonix::engine::read_token(config)?;
    let own_base = config.base_url().trim_end_matches('/').to_string();
    let mut failures: Vec<String> = Vec::new();
    for (url, role) in &paths {
        // Relay paths parse for display but never carry pairing traffic until
        // a relay review lands; skip them instead of failing the whole link.
        if *role == PathRole::Relay {
            failures.push(format!("{url}: relay paths are not dialed yet"));
            continue;
        }
        // Public paths are reachable but untrusted: ask explicitly.
        if *role == PathRole::Public {
            confirm(
                &format!("Path {url} is a public address. Pair over it anyway?"),
                false,
                theme,
            )?;
        }
        let dial = match yonix::pairing::resolve_checked(url, *role) {
            Ok(dial) => dial,
            Err(error) => {
                failures.push(format!("{url}: {error:#}"));
                continue;
            }
        };
        for candidate in dial {
            match Client::post_no_auth(
                &candidate,
                "/api/pairings/redeem",
                &serde_json::json!({
                    "code": parsed.code,
                    "host": config.host,
                    "token": own_token,
                    "url": own_base,
                }),
            ) {
                Ok(reply) => {
                    let pair_id = reply
                        .get("pair_id")
                        .and_then(|id| id.as_str())
                        .context("Pairing server did not return a pair id")?;
                    let peer_host = reply
                        .get("host")
                        .and_then(|host| host.as_str())
                        .unwrap_or(parsed.payload.host.as_str());
                    let peer_token = reply
                        .get("token")
                        .and_then(|token| token.as_str())
                        .context("Pairing server did not return a token")?;
                    let name = label.clone().unwrap_or_else(|| {
                        if peer_host.is_empty() {
                            "peer".into()
                        } else {
                            peer_host.into()
                        }
                    });
                    let local = Client::new(config, None, None)?;
                    // Persist via the daemon so both sides agree on the
                    // pairing id and credentials live in the private store.
                    local.post_json(
                        "/api/connections",
                        &serde_json::json!({
                            "label": name,
                            "url": url,
                            "token": peer_token,
                            "pair_id": pair_id,
                        }),
                    )?;
                    println!(
                        "{} Paired with {} via {} (pair {})",
                        theme.check(),
                        theme.bold(peer_host),
                        theme.cyan(url),
                        theme.dim(pair_id)
                    );
                    return Ok(());
                }
                Err(error) => failures.push(format!("{candidate}: {error:#}")),
            }
        }
    }
    bail!("No pairing path worked:\n  {}", failures.join("\n  "));
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
