use std::{fs, path::PathBuf, sync::OnceLock};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use jester_core::{config::Config, proxy::Proxy};
use jester_plugin_sdk::PluginManifest;
use regex::Regex;
use tracing_subscriber::{fmt, EnvFilter};

#[derive(Parser, Debug)]
#[command(name = "jester", author, version, about = "Programmable reverse proxy")]
struct Cli {
    /// Sets the log level (error, warn, info, debug, trace).
    #[arg(long, default_value = "info", global = true)]
    log_level: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Run the proxy using the supplied configuration file.
    Run {
        #[arg(
            short,
            long,
            value_name = "FILE",
            default_value = "examples/config/minimal.jester.toml"
        )]
        config: PathBuf,
    },
    /// Interact with configuration files (validate, sample output, etc.)
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
    /// Inspect installed plugins (placeholder)
    Plugins {
        #[command(subcommand)]
        command: PluginCommands,
    },
    /// Placeholder command for future live log tailing.
    Tap {
        #[arg(long, value_name = "ROUTE")]
        route: String,
    },
    /// Dump the resolved configuration as JSON.
    Diag {
        #[arg(
            short,
            long,
            value_name = "FILE",
            default_value = "examples/config/minimal.jester.toml"
        )]
        config: PathBuf,
    },
}

#[derive(Subcommand, Debug)]
enum ConfigCommands {
    /// Validates the provided configuration file.
    Validate {
        #[arg(value_name = "FILE")]
        config: PathBuf,
    },
    /// Performs semantic linting (not yet implemented, returns TODO).
    Lint {
        #[arg(value_name = "FILE")]
        config: PathBuf,
    },
    /// Prints the bundled minimal example configuration.
    Example,
}

#[derive(Subcommand, Debug)]
enum PluginCommands {
    /// Lists discovered plugins (currently stubbed).
    List {
        #[arg(long, value_name = "DIR", default_value = "plugins")]
        dir: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(&cli.log_level)?;
    match cli.command {
        Commands::Run { config } => handle_run(config).await,
        Commands::Config { command } => handle_config(command),
        Commands::Plugins { command } => handle_plugins(command),
        Commands::Tap { route } => handle_tap(route),
        Commands::Diag { config } => handle_diag(config),
    }
}

fn init_tracing(level: &str) -> Result<()> {
    let filter = EnvFilter::try_new(level).unwrap_or_else(|_| EnvFilter::new("info"));
    fmt().with_env_filter(filter).try_init().ok();
    Ok(())
}

async fn handle_run(config_path: PathBuf) -> Result<()> {
    let config = load_config(&config_path)?;
    let proxy = Proxy::new(config)?;
    proxy.run().await
}

fn handle_config(command: ConfigCommands) -> Result<()> {
    match command {
        ConfigCommands::Validate { config } => {
            let cfg = load_config(&config)?;
            cfg.validate()?;
            println!("configuration OK: {}", config.display());
        }
        ConfigCommands::Lint { config } => {
            let cfg = load_config(&config)?;
            if let Err(err) = cfg.validate() {
                println!("lint failed: {err}");
            } else {
                println!("lint pass: no additional issues detected (future release will add more checks)");
            }
        }
        ConfigCommands::Example => {
            println!(
                "{}",
                include_str!("../../../examples/config/minimal.jester.toml")
            );
        }
    }
    Ok(())
}

fn handle_plugins(command: PluginCommands) -> Result<()> {
    match command {
        PluginCommands::List { dir } => {
            let manifests = discover_plugins(&dir)?;
            if manifests.is_empty() {
                println!("no plugin manifests found under {}", dir.display());
            } else {
                for manifest in manifests {
                    println!(
                        "- {} v{}{}",
                        manifest.name,
                        manifest.version,
                        manifest
                            .description
                            .as_ref()
                            .map(|d| format!(" — {d}"))
                            .unwrap_or_default()
                    );
                }
            }
        }
    }
    Ok(())
}

fn handle_tap(route: String) -> Result<()> {
    println!(
        "tap is not yet implemented; use `RUST_LOG=jester=trace cargo run -p jester-cli -- run --config <file>` \
         and filter logs for route `{}` in the meantime.",
        route
    );
    Ok(())
}

fn handle_diag(path: PathBuf) -> Result<()> {
    let cfg = load_config(&path)?;
    let json = serde_json::to_string_pretty(&cfg)?;
    println!("{json}");
    Ok(())
}

fn load_config(path: &PathBuf) -> Result<Config> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read config file {}", path.display()))?;
    let expanded = interpolate_env(&raw)?;
    let cfg = toml::from_str::<Config>(&expanded)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(cfg)
}

fn interpolate_env(input: &str) -> Result<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let regex = RE.get_or_init(|| Regex::new(r"\$\{([A-Z0-9_]+)(?::([^}]+))?\}").unwrap());
    let result = regex.replace_all(input, |caps: &regex::Captures| {
        let key = &caps[1];
        let default = caps.get(2).map(|m| m.as_str()).unwrap_or("");
        std::env::var(key).unwrap_or_else(|_| default.to_string())
    });
    Ok(result.into_owned())
}

fn discover_plugins(dir: &PathBuf) -> Result<Vec<PluginManifest>> {
    let mut manifests = Vec::new();
    if !dir.exists() {
        return Ok(manifests);
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let data = fs::read_to_string(&path)?;
        let manifest: PluginManifest = serde_json::from_str(&data)
            .with_context(|| format!("failed to parse manifest {}", path.display()))?;
        manifests.push(manifest);
    }
    Ok(manifests)
}
