use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use std::{
    io::{self, Write},
    path::PathBuf,
    time::Duration,
};
use yonix::{client::Client, config::Config, engine::Action};

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
    Status,
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
    /// Remove generations older than the configured retention period and collect garbage
    Clean {
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Show operation history
    History,
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
        /// Full link (base64url#code) or a bare 32-hex code (manual address top-up)
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
fn confirm(message: &str, yes: bool) -> Result<()> {
    if yes {
        return Ok(());
    }
    print!("{message} [y/N] ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    if !matches!(answer.trim(), "y" | "Y" | "yes") {
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
                "Bare code {}: pass --origin <url> to top up the initiator address by hand",
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
                    println!("Paired with {peer_host} via {url} (pair {pair_id})");
                    return Ok(());
                }
                Err(error) => failures.push(format!("{candidate}: {error:#}")),
            }
        }
    }
    bail!("No pairing path worked:\n  {}", failures.join("\n  "));
}

fn operation(client: Client, action: Action) -> Result<()> {
    let job = client.start(action, true)?;
    println!("Started {} · {}", action.name(), job.id);
    let mut last_log = String::new();
    loop {
        let status = client.job(&job.id)?;
        let log = client.log(&job.id)?;
        if let Some(extra) = log.strip_prefix(&last_log) {
            print!("{extra}");
        } else if log != last_log {
            print!("{log}");
        }
        io::stdout().flush()?;
        last_log = log;
        if status.status != "running" {
            if status.status != "succeeded" {
                bail!("{}", status.error.unwrap_or(status.status));
            }
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}
fn main() -> Result<()> {
    let args = Args::parse();
    let command = args.command.unwrap_or(Commands::Gui);
    if matches!(command, Commands::Init) {
        println!("{}", serde_json::to_string_pretty(&Config::default())?);
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
        return pair_command(&config, origin, extra_path, extra_path_role, json);
    }
    if let Commands::PairAccept {
        link,
        origin,
        label,
    } = command
    {
        return pair_accept_command(&config, link, origin, label);
    }
    let client = Client::new(&config, args.url.clone(), args.token_file)?;
    match command {
        Commands::Gui => {
            #[cfg(feature = "gui")]
            yonix::gui::run(client);
            #[cfg(not(feature = "gui"))]
            bail!("This package was built without the gui feature");
        }
        Commands::Web => {
            let url = args.url.unwrap_or_else(|| config.base_url());
            let status = std::process::Command::new("xdg-open").arg(url).status()?;
            if !status.success() {
                bail!("Cannot open browser");
            }
        }
        Commands::Status => println!("{}", serde_json::to_string_pretty(&client.status()?)?),
        Commands::History => println!("{}", serde_json::to_string_pretty(&client.jobs()?)?),
        Commands::Logs { id } => print!("{}", client.log(&id)?),
        Commands::Check => operation(client, Action::Check)?,
        Commands::Build => operation(client, Action::Build)?,
        Commands::Diff => operation(client, Action::Diff)?,
        Commands::UpdateInputs { yes } => {
            confirm("Update flake.lock in the configuration repository?", yes)?;
            operation(client, Action::UpdateInputs)?;
        }
        Commands::Switch { yes } => {
            confirm("Build and apply the configured system?", yes)?;
            operation(client, Action::Switch)?;
        }
        Commands::Rollback { yes } => {
            confirm("Activate the previous system generation?", yes)?;
            operation(client, Action::Rollback)?;
        }
        Commands::Clean { yes } => {
            confirm(
                "Delete old generations and collect unreferenced store paths?",
                yes,
            )?;
            operation(client, Action::Clean)?;
        }
        _ => unreachable!(),
    }
    Ok(())
}
