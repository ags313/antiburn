use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "antiburn",
    version,
    about = "Read Antiburn check updates for an agent session"
)]
pub struct Cli {
    #[arg(
        long,
        global = true,
        value_name = "PATH",
        help = "Path to a JSON config file"
    )]
    pub config: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Set up the optional terminal command without changing PATH or agent integrations.
    Terminal {
        #[arg(
            long,
            global = true,
            value_name = "PATH",
            help = "Private runtime root (for an isolated or debug profile)"
        )]
        runtime_root: Option<PathBuf>,
        #[command(subcommand)]
        command: TerminalCommand,
    },
    Capabilities {
        #[arg(long, required = true)]
        json: bool,
    },
    Checks {
        session_id: String,
        #[arg(long)]
        delivery_v1: bool,
        #[arg(long, requires = "delivery_v1")]
        consumer: Option<String>,
        #[arg(long, requires = "consumer")]
        ack: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum TerminalCommand {
    /// Install a per-user antiburn command; preserve existing commands.
    Install {
        #[arg(
            long,
            value_name = "PATH",
            help = "Per-user command directory; does not modify PATH"
        )]
        directory: Option<PathBuf>,
    },
    /// Inspect the optional command without changing installation files.
    Status,
    /// Disable terminal setup and remove only the owned command.
    Uninstall,
}
