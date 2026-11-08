use anyhow::{Result, bail};
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
