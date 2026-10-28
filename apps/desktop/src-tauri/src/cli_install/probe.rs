use std::{process::Stdio, time::Duration};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use tokio::{io::AsyncReadExt, process::Command};

use super::CliRuntime;

const OUTPUT_LIMIT: u64 = 8192;
const DEADLINE: Duration = Duration::from_secs(3);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Capabilities {
    version: String,
    delivery_protocols: Vec<String>,
}

pub async fn test(runtime: &CliRuntime) -> Result<()> {
    run(runtime, DEADLINE).await
}

async fn read_output(stream: impl tokio::io::AsyncRead + Unpin) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    stream
        .take(OUTPUT_LIMIT + 1)
        .read_to_end(&mut bytes)
        .await?;
    ensure!(
        bytes.len() as u64 <= OUTPUT_LIMIT,
        "CLI test output is too large"
    );
    Ok(bytes)
}

async fn run(runtime: &CliRuntime, deadline: Duration) -> Result<()> {
    ensure!(
        runtime.executable.is_absolute(),
        "CLI test requires an absolute path"
    );
    let mut command = Command::new(&runtime.executable);
    command
        .args(["capabilities", "--json"])
        .env("PATH", "")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command
        .spawn()
        .context("Could not start the installed CLI")?;
    let stdout = child
        .stdout
        .take()
        .context("CLI test stdout is unavailable")?;
    let stderr = child
        .stderr
        .take()
        .context("CLI test stderr is unavailable")?;
    let result = tokio::time::timeout(deadline, async {
        tokio::try_join!(
            async { child.wait().await.context("Cannot wait for CLI test") },
            read_output(stdout),
            read_output(stderr),
        )
    })
    .await;
    let (status, stdout, stderr) = match result {
        Ok(Ok(output)) => output,
        other => {
            let _ = child.kill().await;
            match other {
                Ok(Err(error)) => return Err(error),
                _ => bail!("CLI test timed out"),
            }
        }
    };
    ensure!(
        status.success(),
        "CLI test exited unsuccessfully ({status})"
    );
    ensure!(
        stderr.is_empty(),
        "CLI test returned unexpected diagnostics"
    );
    let capabilities: Capabilities =
        serde_json::from_slice(&stdout).context("CLI test returned invalid capabilities")?;
    ensure!(
        capabilities.version == runtime.version,
        "CLI version does not match the installed version"
    );
    ensure!(
        capabilities.delivery_protocols == runtime.delivery_protocols,
        "CLI protocols do not match the installed protocols"
    );
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt};

    async fn fixture(script: &str, deadline: Duration) -> Result<()> {
        let temp = tempfile::tempdir()?;
        let executable = temp.path().join("probe");
        fs::write(&executable, format!("#!/bin/sh\n{script}\n"))?;
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))?;
        run(
            &CliRuntime {
                executable,
                version: "0.1.0".into(),
                sha256: "fixture".into(),
                delivery_protocols: vec!["delivery-v1".into()],
            },
            deadline,
        )
        .await
    }

    #[tokio::test]
    async fn executes_capabilities_with_an_empty_path() {
        fixture("[ -z \"$PATH\" ] && [ \"$1\" = capabilities ] && [ \"$2\" = --json ] || exit 1\nprintf '%s' '{\"version\":\"0.1.0\",\"delivery_protocols\":[\"delivery-v1\"]}'", DEADLINE).await.unwrap();
    }

    #[tokio::test]
    async fn rejects_failed_malformed_and_mismatched_responses() {
        for script in [
            "exit 2",
            "printf invalid",
            "printf '%s' '{\"version\":\"old\",\"delivery_protocols\":[\"delivery-v1\"]}'",
            "printf '%s' '{\"version\":\"0.1.0\",\"delivery_protocols\":[]}'",
            "printf diagnostics >&2",
        ] {
            assert!(fixture(script, DEADLINE).await.is_err());
        }
    }

    #[tokio::test]
    async fn bounds_output_and_execution_time() {
        let output = fixture("while :; do printf '0123456789'; done", DEADLINE)
            .await
            .unwrap_err();
        assert!(output.to_string().contains("too large"));
        let timeout = fixture("while :; do :; done", Duration::from_millis(50))
            .await
            .unwrap_err();
        assert!(timeout.to_string().contains("timed out"));
    }
}
