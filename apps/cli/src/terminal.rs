use std::path::PathBuf;

use antiburn_cli_runtime::{Installation, TerminalState};
use serde::Serialize;

use crate::{CliError, cli::TerminalCommand};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalResponse {
    #[serde(flatten)]
    state: TerminalState,
    note: &'static str,
}

pub async fn run(
    command: TerminalCommand,
    runtime_root: Option<PathBuf>,
) -> Result<TerminalResponse, CliError> {
    tokio::task::spawn_blocking(move || {
        let root = runtime_root
            .or_else(|| dirs::data_dir().map(|path| path.join("ai.antiburn.desktop/cli")))
            .ok_or_else(|| CliError::runtime("could not locate the per-user runtime directory"))?;
        let installation = Installation::for_terminal(root);
        let state = match command {
            TerminalCommand::Install { directory } => {
                let directory = directory
                    .or_else(default_directory)
                    .ok_or_else(|| CliError::runtime("could not locate a per-user command directory"))?;
                installation.terminal_install(&directory)
            }
            TerminalCommand::Status => installation.terminal_status(),
            TerminalCommand::Uninstall => installation.terminal_uninstall(),
        }
        .map_err(|error| CliError::runtime(format!("{error:#}")))?;
        Ok(TerminalResponse {
            note: if state.installed {
                "Command file installed. Ensure its directory is on your shell PATH, then check command resolution in a new terminal. PATH and shell profiles were not changed."
            } else if state.enabled {
                "Terminal setup needs attention. Existing or modified commands were preserved."
            } else {
                "Terminal command is not enabled. Agent integrations use the private runtime independently."
            },
            state,
        })
    })
    .await
    .map_err(|error| CliError::runtime(format!("terminal operation did not finish: {error}")))?
}

fn default_directory() -> Option<PathBuf> {
    if cfg!(windows) {
        dirs::data_local_dir().map(|path| path.join("antiburn/bin"))
    } else {
        dirs::home_dir().map(|path| path.join(".local/bin"))
    }
}
