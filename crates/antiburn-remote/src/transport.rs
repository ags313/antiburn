use std::fmt;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::{Hello, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES, PROTOCOL_VERSION, Request};

const HELPER_COMMAND: &str = "~/.local/bin/antiburn-remote stdio";
/// The helper command for a Windows host. OpenSSH for Windows runs a remote
/// command in `cmd.exe`, which expands `%USERPROFILE%` but not `~`.
const WINDOWS_HELPER_COMMAND: &str = r"%USERPROFILE%\.local\bin\antiburn-remote.exe stdio";

/// The hosts that run remote commands in `cmd.exe`. The first request to a
/// host in this process finds out, and later requests use it at once.
static WINDOWS_SHELL_HOSTS: std::sync::Mutex<std::collections::BTreeSet<String>> =
    std::sync::Mutex::new(std::collections::BTreeSet::new());

fn uses_windows_shell(host: &str) -> bool {
    WINDOWS_SHELL_HOSTS
        .lock()
        .is_ok_and(|hosts| hosts.contains(host))
}

fn remember_windows_shell(host: &str) {
    if let Ok(mut hosts) = WINDOWS_SHELL_HOSTS.lock() {
        hosts.insert(host.to_owned());
    }
}

/// True when `cmd.exe` cannot run the Unix helper path. The Unix path starts
/// with `~`, which `cmd.exe` reads as the name of a command.
fn is_windows_shell_failure(diagnostic: &str) -> bool {
    diagnostic
        .to_ascii_lowercase()
        .contains("is not recognized as an internal or external command")
}
const MAX_DIAGNOSTIC_BYTES: usize = 8 * 1024;

pub fn validate_host(host: &str) -> Result<()> {
    ensure!(
        !host.is_empty() && host.len() <= 128,
        "Use an SSH config host alias of at most 128 characters"
    );
    ensure!(
        host.as_bytes()[0].is_ascii_alphanumeric()
            && host
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_')),
        "Use an SSH config alias containing only letters, numbers, dots, hyphens or underscores"
    );
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrerequisiteErrorCategory {
    InvalidAlias,
    SshUnavailable,
    HostKey,
    Authentication,
    HelperMissing,
    ProtocolMismatch,
    UnsupportedHost,
    InvalidResponse,
}

#[derive(Debug)]
pub struct PrerequisiteError {
    category: PrerequisiteErrorCategory,
    diagnostic: String,
}

impl PrerequisiteError {
    fn new(category: PrerequisiteErrorCategory, diagnostic: impl Into<String>) -> Self {
        Self {
            category,
            diagnostic: diagnostic.into(),
        }
    }

    pub fn category(&self) -> PrerequisiteErrorCategory {
        self.category
    }

    /// Returns a bounded local diagnostic. Do not expose it in UI or analytics.
    pub fn diagnostic(&self) -> &str {
        &self.diagnostic
    }
}

impl fmt::Display for PrerequisiteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self.category {
            PrerequisiteErrorCategory::InvalidAlias => "The SSH alias is invalid",
            PrerequisiteErrorCategory::SshUnavailable => "The SSH host is unavailable",
            PrerequisiteErrorCategory::HostKey => "SSH host-key verification failed",
            PrerequisiteErrorCategory::Authentication => "SSH authentication failed",
            PrerequisiteErrorCategory::HelperMissing => "The remote helper is not installed",
            PrerequisiteErrorCategory::ProtocolMismatch => {
                "The remote helper protocol is incompatible"
            }
            PrerequisiteErrorCategory::UnsupportedHost => "The remote host is unsupported",
            PrerequisiteErrorCategory::InvalidResponse => {
                "The remote helper returned an invalid response"
            }
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for PrerequisiteError {}

/// The installed helper does not accept the history-window field.
#[derive(Debug)]
pub struct HistoryWindowRejected;

impl fmt::Display for HistoryWindowRejected {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("The remote helper does not support longer history windows")
    }
}

impl std::error::Error for HistoryWindowRejected {}

fn request_failure(failure: CommandFailure) -> anyhow::Error {
    if let CommandFailure::Exit(diagnostic) = &failure
        && diagnostic.starts_with("antiburn-remote: unknown field `lookback_secs`")
    {
        return HistoryWindowRejected.into();
    }
    classify_failure(failure).into()
}

enum CommandFailure {
    Start(String),
    Timeout,
    Output(String),
    Exit(String),
}

fn ssh_command(host: &str) -> tokio::process::Command {
    let helper = if uses_windows_shell(host) {
        WINDOWS_HELPER_COMMAND
    } else {
        HELPER_COMMAND
    };
    ssh_command_with(host, helper)
}

fn ssh_command_with(host: &str, helper: &str) -> tokio::process::Command {
    let mut command = antiburn_local::platform::process::headless_tokio_command("ssh");
    command.args([
        "-T",
        "-o",
        "BatchMode=yes",
        "-o",
        "ConnectTimeout=10",
        "-o",
        "StrictHostKeyChecking=yes",
        "-o",
        "ServerAliveInterval=10",
        "-o",
        "ServerAliveCountMax=2",
        "--",
        host,
        helper,
    ]);
    command
}

/// The helper rejects this session without a transport failure.
#[derive(Debug)]
pub struct SessionRejected;

impl fmt::Display for SessionRejected {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("The remote helper could not export this session")
    }
}

impl std::error::Error for SessionRejected {}

fn check_export_status(code: Option<i32>, diagnostics: &[u8]) -> Result<()> {
    match code {
        Some(0) => Ok(()),
        Some(crate::SESSION_REJECTED_EXIT_CODE) => Err(SessionRejected.into()),
        _ => Err(classify_failure(CommandFailure::Exit(
            String::from_utf8_lossy(diagnostics).trim().to_owned(),
        ))
        .into()),
    }
}

async fn request_bytes(
    host: &str,
    request: &Request,
) -> std::result::Result<Vec<u8>, CommandFailure> {
    request_with_shell_fallback(host, request, ssh_command_with, Duration::from_secs(60)).await
}

/// Send a request with the helper command for the host's shell. A Windows
/// host rejects the Unix helper path before the helper starts, so a second
/// attempt with the Windows path is safe.
async fn request_with_shell_fallback(
    host: &str,
    request: &Request,
    command_for: impl Fn(&str, &str) -> tokio::process::Command,
    deadline: Duration,
) -> std::result::Result<Vec<u8>, CommandFailure> {
    let run = |helper| {
        request_with_command(
            request,
            command_for(host, helper),
            MAX_RESPONSE_BYTES,
            deadline,
        )
    };
    let windows = uses_windows_shell(host);
    let helper = if windows {
        WINDOWS_HELPER_COMMAND
    } else {
        HELPER_COMMAND
    };
    match run(helper).await {
        Err(CommandFailure::Exit(diagnostic))
            if !windows && is_windows_shell_failure(&diagnostic) =>
        {
            remember_windows_shell(host);
            run(WINDOWS_HELPER_COMMAND).await
        }
        result => result,
    }
}

async fn request_with_command(
    request: &Request,
    mut command: tokio::process::Command,
    response_limit: u64,
    deadline: Duration,
) -> std::result::Result<Vec<u8>, CommandFailure> {
    let bytes =
        serde_json::to_vec(request).map_err(|error| CommandFailure::Output(error.to_string()))?;
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err(CommandFailure::Output("Request exceeds 8 KiB".to_owned()));
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| CommandFailure::Start(error.to_string()))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| CommandFailure::Output("Missing SSH input".to_owned()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| CommandFailure::Output("Missing SSH output".to_owned()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| CommandFailure::Output("Missing SSH diagnostic stream".to_owned()))?;

    let operation = async {
        stdin
            .write_all(&bytes)
            .await
            .map_err(|error| CommandFailure::Output(error.to_string()))?;
        stdin
            .shutdown()
            .await
            .map_err(|error| CommandFailure::Output(error.to_string()))?;
        drop(stdin);
        let read_out = async {
            let mut output = Vec::new();
            stdout
                .take(response_limit + 1)
                .read_to_end(&mut output)
                .await
                .map_err(|error| CommandFailure::Output(error.to_string()))?;
            if output.len() as u64 > response_limit {
                return Err(CommandFailure::Output(
                    "Remote response exceeds 8 MiB".to_owned(),
                ));
            }
            Ok(output)
        };
        let read_err = async {
            let mut error = Vec::new();
            stderr
                .take(MAX_DIAGNOSTIC_BYTES as u64 + 1)
                .read_to_end(&mut error)
                .await
                .map_err(|read_error| CommandFailure::Output(read_error.to_string()))?;
            if error.len() > MAX_DIAGNOSTIC_BYTES {
                return Err(CommandFailure::Output(
                    "SSH diagnostic output exceeds 8 KiB".to_owned(),
                ));
            }
            Ok(error)
        };
        let (output, error) = tokio::try_join!(read_out, read_err)?;
        let status = child
            .wait()
            .await
            .map_err(|wait_error| CommandFailure::Output(wait_error.to_string()))?;
        if !status.success() {
            return Err(CommandFailure::Exit(
                String::from_utf8_lossy(&error).trim().to_owned(),
            ));
        }
        Ok(output)
    };
    tokio::time::timeout(deadline, operation)
        .await
        .map_err(|_| CommandFailure::Timeout)?
}

fn classify_failure(failure: CommandFailure) -> PrerequisiteError {
    match failure {
        CommandFailure::Start(diagnostic) => {
            PrerequisiteError::new(PrerequisiteErrorCategory::SshUnavailable, diagnostic)
        }
        CommandFailure::Timeout => PrerequisiteError::new(
            PrerequisiteErrorCategory::SshUnavailable,
            "SSH/helper request timed out after 60 seconds",
        ),
        CommandFailure::Output(diagnostic) => {
            PrerequisiteError::new(PrerequisiteErrorCategory::InvalidResponse, diagnostic)
        }
        CommandFailure::Exit(diagnostic) => {
            let lowercase = diagnostic.to_ascii_lowercase();
            let category = if lowercase.contains("host key verification failed")
                || lowercase.contains("remote host identification has changed")
            {
                PrerequisiteErrorCategory::HostKey
            } else if lowercase.contains("permission denied")
                || lowercase.contains("authentication failed")
                || lowercase.contains("no supported authentication methods")
            {
                PrerequisiteErrorCategory::Authentication
            } else if lowercase.contains("antiburn-remote: not found")
                || lowercase.contains("antiburn-remote: no such file")
                || lowercase.contains("no such file or directory")
                || is_windows_shell_failure(&lowercase)
            {
                PrerequisiteErrorCategory::HelperMissing
            } else if lowercase.contains("protocol mismatch")
                || lowercase.contains("unknown variant `hello`")
                || lowercase.contains("unknown variant \"hello\"")
            {
                PrerequisiteErrorCategory::ProtocolMismatch
            } else {
                PrerequisiteErrorCategory::SshUnavailable
            };
            PrerequisiteError::new(category, diagnostic)
        }
    }
}

pub async fn check(host: &str) -> std::result::Result<Hello, PrerequisiteError> {
    validate_host(host).map_err(|error| {
        PrerequisiteError::new(PrerequisiteErrorCategory::InvalidAlias, error.to_string())
    })?;
    let request = Request::Hello {
        version: PROTOCOL_VERSION,
    };
    let bytes = request_bytes(host, &request)
        .await
        .map_err(classify_failure)?;
    let hello: Hello = serde_json::from_slice(&bytes).map_err(|error| {
        PrerequisiteError::new(
            PrerequisiteErrorCategory::InvalidResponse,
            error.to_string(),
        )
    })?;
    if hello.version != PROTOCOL_VERSION {
        return Err(PrerequisiteError::new(
            PrerequisiteErrorCategory::ProtocolMismatch,
            format!(
                "helper protocol {}, client protocol {}",
                hello.version, PROTOCOL_VERSION
            ),
        ));
    }
    hello.validate_compatibility().map_err(|error| {
        PrerequisiteError::new(
            PrerequisiteErrorCategory::UnsupportedHost,
            error.to_string(),
        )
    })?;
    Ok(hello)
}

pub async fn request(host: &str, request: &Request) -> Result<Vec<u8>> {
    validate_host(host)?;
    request.validate()?;
    ensure!(
        !matches!(request, Request::Export { .. }),
        "Use export_to for bundle requests"
    );
    request_bytes(host, request).await.map_err(request_failure)
}

/// Streams one bounded bundle into an exclusively created staging file.
pub async fn export_to(host: &str, request: &Request, destination: &Path) -> Result<()> {
    validate_host(host)?;
    request.validate()?;
    ensure!(
        matches!(request, Request::Export { .. }),
        "Expected export request"
    );
    export_with_command(
        request,
        destination,
        ssh_command(host),
        Duration::from_secs(180),
        crate::export::MAX_BUNDLE_BYTES + crate::export::MAX_MANIFEST_BYTES as u64 + 12,
    )
    .await
}

async fn export_with_command(
    request: &Request,
    destination: &Path,
    mut command: tokio::process::Command,
    deadline: Duration,
    limit: u64,
) -> Result<()> {
    let bytes = serde_json::to_vec(request)?;
    ensure!(bytes.len() <= MAX_REQUEST_BYTES, "Request exceeds 8 KiB");
    let mut file = create_private_destination(destination).await?;
    let operation = async {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context("Could not start SSH")?;
        let mut stdin = child.stdin.take().context("Missing SSH input")?;
        let stdout = child.stdout.take().context("Missing SSH output")?;
        let stderr = child
            .stderr
            .take()
            .context("Missing SSH diagnostic stream")?;
        stdin.write_all(&bytes).await?;
        stdin.shutdown().await?;
        drop(stdin);
        let copy = async {
            let copied = tokio::io::copy(&mut stdout.take(limit + 1), &mut file).await?;
            ensure!(copied <= limit, "Bundle exceeds transfer limit");
            file.sync_all().await?;
            Ok::<_, anyhow::Error>(())
        };
        let diagnostics = async {
            let mut bytes = Vec::new();
            stderr
                .take(MAX_DIAGNOSTIC_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
                .await?;
            ensure!(
                bytes.len() <= MAX_DIAGNOSTIC_BYTES,
                "SSH diagnostic output exceeds 8 KiB"
            );
            Ok::<_, anyhow::Error>(bytes)
        };
        let (_, diagnostics) = tokio::try_join!(copy, diagnostics)?;
        check_export_status(child.wait().await?.code(), &diagnostics)?;
        Ok(())
    };
    let result = tokio::time::timeout(deadline, operation)
        .await
        .context("Transcript transfer timed out after 180 seconds")
        .and_then(|result| result);
    if result.is_err() {
        let _ = tokio::fs::remove_file(destination).await;
    }
    result
}

async fn create_private_destination(path: &Path) -> Result<tokio::fs::File> {
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }
    Ok(options.open(path).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_rejection_requires_the_helpers_unknown_field_diagnostic() {
        let error = request_failure(CommandFailure::Exit(
            "antiburn-remote: unknown field `lookback_secs`, expected `version`".into(),
        ));
        assert!(error.is::<HistoryWindowRejected>());
        for failure in [
            CommandFailure::Timeout,
            CommandFailure::Exit("Permission denied".into()),
            CommandFailure::Exit("Connection refused".into()),
            CommandFailure::Output("unknown field `lookback_secs`".into()),
        ] {
            assert!(!request_failure(failure).is::<HistoryWindowRejected>());
        }
    }

    #[test]
    fn ssh_keeps_fixed_security_options_and_remote_command() {
        let command = ssh_command("build-box");
        let args: Vec<_> = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_str().unwrap())
            .collect();
        assert_eq!(
            args,
            [
                "-T",
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=10",
                "-o",
                "StrictHostKeyChecking=yes",
                "-o",
                "ServerAliveInterval=10",
                "-o",
                "ServerAliveCountMax=2",
                "--",
                "build-box",
                HELPER_COMMAND
            ]
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_windows_host_gets_the_windows_helper_path_once_detected() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let host = "windows-box-under-test";
        let unix_attempts = AtomicUsize::new(0);
        let command_for = |_: &str, helper: &str| {
            if helper == HELPER_COMMAND {
                unix_attempts.fetch_add(1, Ordering::SeqCst);
                // The message that cmd.exe prints for the Unix helper path.
                fake_ssh(
                    "cat >/dev/null; echo \"'~' is not recognized as an internal or external command,\" >&2; exit 1",
                )
            } else {
                assert_eq!(helper, WINDOWS_HELPER_COMMAND);
                fake_ssh("cat")
            }
        };
        let request = Request::Hello {
            version: PROTOCOL_VERSION,
        };
        for _ in 0..2 {
            let output =
                request_with_shell_fallback(host, &request, command_for, Duration::from_secs(2))
                    .await
                    .unwrap_or_else(|_| panic!("fallback request failed"));
            assert_eq!(output, serde_json::to_vec(&request).unwrap());
        }
        // The second request uses the remembered Windows path at once.
        assert_eq!(unix_attempts.load(Ordering::SeqCst), 1);
        assert!(uses_windows_shell(host));
        assert!(!uses_windows_shell("build-box"));
    }

    #[test]
    fn a_missing_windows_helper_is_reported_as_missing() {
        let error = classify_failure(CommandFailure::Exit(
            r"'C:\Users\me\.local\bin\antiburn-remote.exe' is not recognized as an internal or external command,".to_owned(),
        ));
        assert_eq!(error.category(), PrerequisiteErrorCategory::HelperMissing);
        let args: Vec<_> = ssh_command_with("win-box", WINDOWS_HELPER_COMMAND)
            .as_std()
            .get_args()
            .map(|arg| arg.to_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            args.last().map(String::as_str),
            Some(r"%USERPROFILE%\.local\bin\antiburn-remote.exe stdio")
        );
    }

    #[test]
    fn only_explicit_session_rejection_is_recoverable() {
        let private = b"/private/transcript secret prompt";
        let rejected =
            check_export_status(Some(crate::SESSION_REJECTED_EXIT_CODE), private).unwrap_err();
        assert!(rejected.is::<SessionRejected>());
        assert!(!rejected.to_string().contains("secret"));
        for code in [Some(1), Some(127), Some(255), None] {
            let error = check_export_status(code, private).unwrap_err();
            assert!(!error.is::<SessionRejected>());
            assert!(!error.to_string().contains("secret"));
        }
        assert!(check_export_status(Some(0), private).is_ok());
    }

    #[test]
    fn rejects_shell_and_option_injection() {
        for host in [
            "-oProxyCommand=bad",
            "host;id",
            "user@host",
            "host\ncommand",
            "$(id)",
            "../host",
            "",
        ] {
            assert!(validate_host(host).is_err(), "{host}");
        }
        assert!(validate_host("build-box.example").is_ok());
    }

    #[test]
    fn classifies_bounded_prerequisite_failures() {
        assert_eq!(
            classify_failure(CommandFailure::Exit(
                "Host key verification failed.".to_owned()
            ))
            .category(),
            PrerequisiteErrorCategory::HostKey
        );
        assert_eq!(
            classify_failure(CommandFailure::Exit(
                "bash: ~/.local/bin/antiburn-remote: No such file or directory".to_owned()
            ))
            .category(),
            PrerequisiteErrorCategory::HelperMissing
        );
        assert_eq!(
            classify_failure(CommandFailure::Exit(
                "Permission denied (publickey).".to_owned()
            ))
            .category(),
            PrerequisiteErrorCategory::Authentication
        );
    }

    fn export_request() -> Request {
        Request::Export {
            version: PROTOCOL_VERSION,
            agent: "codex".to_owned(),
            session_id: "session-1".to_owned(),
            known: None,
            lookback_secs: None,
        }
    }

    #[cfg(unix)]
    fn fake_ssh(script: &str) -> tokio::process::Command {
        let mut command = antiburn_local::platform::process::headless_tokio_command("/bin/sh");
        command.args(["-c", script]);
        command
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn json_transport_preserves_request_bytes_and_enforces_bounds() {
        for request in [
            Request::Hello {
                version: PROTOCOL_VERSION,
            },
            Request::List {
                version: PROTOCOL_VERSION,
                agents: None,
                lookback_secs: None,
            },
            Request::List {
                version: PROTOCOL_VERSION,
                agents: Some(vec!["claude-code".to_owned(), "cursor".to_owned()]),
                lookback_secs: None,
            },
        ] {
            let output =
                request_with_command(&request, fake_ssh("cat"), 1024, Duration::from_secs(2))
                    .await
                    .unwrap_or_else(|_| panic!("fake SSH request failed"));
            assert_eq!(output, serde_json::to_vec(&request).unwrap());
        }
        for script in [
            "cat >/dev/null; printf '12345'",
            "cat >/dev/null; head -c 9000 /dev/zero >&2",
        ] {
            assert!(matches!(
                request_with_command(
                    &Request::List {
                        version: PROTOCOL_VERSION,
                        agents: None,
                        lookback_secs: None,
                    },
                    fake_ssh(script),
                    4,
                    Duration::from_secs(2)
                )
                .await,
                Err(CommandFailure::Output(_))
            ));
        }
        assert!(matches!(
            request_with_command(
                &Request::List {
                    version: PROTOCOL_VERSION,
                    agents: None,
                    lookback_secs: None,
                },
                fake_ssh("cat >/dev/null; exec sleep 2"),
                1024,
                Duration::from_millis(50)
            )
            .await,
            Err(CommandFailure::Timeout)
        ));
        assert!(matches!(
            request_with_command(
                &Request::List {
                    version: PROTOCOL_VERSION,
                    agents: None,
                    lookback_secs: None,
                },
                fake_ssh("cat >/dev/null; exit 255"),
                1024,
                Duration::from_secs(2)
            )
            .await,
            Err(CommandFailure::Exit(_))
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn streaming_transport_preserves_output_and_cleans_failed_destinations() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("bundle");
        export_with_command(
            &export_request(),
            &destination,
            fake_ssh("cat >/dev/null; printf 'ABR2DATA'"),
            Duration::from_secs(2),
            64,
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"ABR2DATA");
        std::fs::remove_file(&destination).unwrap();
        for (script, deadline) in [
            (
                "cat >/dev/null; head -c 65 /dev/zero",
                Duration::from_secs(2),
            ),
            (
                "cat >/dev/null; head -c 9000 /dev/zero >&2",
                Duration::from_secs(2),
            ),
            (
                "cat >/dev/null; printf partial; exit 65",
                Duration::from_secs(2),
            ),
            (
                "cat >/dev/null; printf partial; exit 255",
                Duration::from_secs(2),
            ),
            ("cat >/dev/null; exec sleep 2", Duration::from_millis(50)),
        ] {
            let result = export_with_command(
                &export_request(),
                &destination,
                fake_ssh(script),
                deadline,
                64,
            )
            .await;
            assert!(result.is_err(), "{script}");
            assert!(!destination.exists(), "{script}");
        }
    }

    #[tokio::test]
    async fn export_never_removes_a_preexisting_destination() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("existing.bundle");
        tokio::fs::write(&destination, b"keep").await.unwrap();

        assert!(
            export_to("build-box", &export_request(), &destination)
                .await
                .is_err()
        );
        assert_eq!(tokio::fs::read(destination).await.unwrap(), b"keep");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn export_destination_is_private() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("private.bundle");
        drop(create_private_destination(&destination).await.unwrap());
        assert_eq!(
            std::fs::metadata(destination).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
