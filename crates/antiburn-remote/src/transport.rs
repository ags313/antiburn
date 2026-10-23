use std::fmt;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::{Hello, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES, PROTOCOL_VERSION, Request};

const HELPER_COMMAND: &str = "~/.local/bin/antiburn-remote stdio";
/// The helper command for a Windows host. OpenSSH for Windows runs a remote
/// command in `cmd.exe`, which expands `%USERPROFILE%` but not `~`.
const WINDOWS_HELPER_COMMAND: &str = r#""%USERPROFILE%\.local\bin\antiburn-remote.exe" stdio"#;

fn helper_command(platform: Option<&str>) -> &'static str {
    if platform.is_some_and(|value| value.eq_ignore_ascii_case("windows")) {
        WINDOWS_HELPER_COMMAND
    } else {
        HELPER_COMMAND
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

fn ssh_command(host: &str, platform: Option<&str>) -> tokio::process::Command {
    ssh_command_with(host, helper_command(platform))
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

/// The transfer of one session failed while the host stayed reachable. A scan
/// counts this failure against the session and continues with the next one.
#[derive(Debug)]
pub enum SessionTransferFailed {
    /// No bytes arrived within the progress window.
    NoProgress(Duration),
    /// The whole transfer ran longer than the total cap.
    TotalCapExceeded(Duration),
    StreamBroken(String),
}

impl fmt::Display for SessionTransferFailed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoProgress(window) => write!(
                formatter,
                "Transcript transfer received no data for {} seconds",
                window.as_secs()
            ),
            Self::TotalCapExceeded(cap) => write!(
                formatter,
                "Transcript transfer exceeded the total limit of {} seconds",
                cap.as_secs()
            ),
            Self::StreamBroken(reason) => {
                write!(formatter, "Transcript transfer stream broke: {reason}")
            }
        }
    }
}

impl std::error::Error for SessionTransferFailed {}

/// An export failed while it wrote the local destination file.
#[derive(Debug)]
pub struct ExportDestinationFailed;

impl fmt::Display for ExportDestinationFailed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Could not write the transcript transfer")
    }
}

/// An export failed on the SSH side: start, input, diagnostics, exit, or an
/// oversize bundle. An error with its own type keeps it.
#[derive(Debug)]
pub struct ExportTransportFailed;

impl fmt::Display for ExportTransportFailed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Transcript transfer over SSH failed")
    }
}

fn tag_export_failure(error: anyhow::Error) -> anyhow::Error {
    if error.is::<SessionRejected>()
        || error.is::<SessionTransferFailed>()
        || error.is::<ExportDestinationFailed>()
    {
        error
    } else {
        error.context(ExportTransportFailed)
    }
}

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
    platform: Option<&str>,
    request: &Request,
) -> std::result::Result<Vec<u8>, CommandFailure> {
    request_with_shell_fallback(
        host,
        platform,
        request,
        ssh_command_with,
        Duration::from_secs(60),
    )
    .await
}

/// Send a request with the helper command for the host's shell. A Windows
/// host rejects the Unix helper path before the helper starts, so a second
/// attempt with the Windows path is safe.
async fn request_with_shell_fallback(
    host: &str,
    platform: Option<&str>,
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
    let helper = helper_command(platform);
    match run(helper).await {
        Err(CommandFailure::Exit(diagnostic))
            if platform.is_none() && is_windows_shell_failure(&diagnostic) =>
        {
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

pub async fn check(
    host: &str,
    platform: Option<&str>,
) -> std::result::Result<Hello, PrerequisiteError> {
    validate_host(host).map_err(|error| {
        PrerequisiteError::new(PrerequisiteErrorCategory::InvalidAlias, error.to_string())
    })?;
    let request = Request::Hello {
        version: PROTOCOL_VERSION,
    };
    let bytes = request_bytes(host, platform, &request)
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

pub async fn request(host: &str, platform: Option<&str>, request: &Request) -> Result<Vec<u8>> {
    validate_host(host)?;
    request.validate()?;
    ensure!(
        !matches!(request, Request::Export { .. }),
        "Use export_to for bundle requests"
    );
    request_bytes(host, platform, request)
        .await
        .map_err(request_failure)
}

/// Streams one bounded bundle into an exclusively created staging file.
pub async fn export_to(
    host: &str,
    platform: Option<&str>,
    request: &Request,
    destination: &Path,
) -> Result<()> {
    validate_host(host)?;
    request.validate()?;
    ensure!(
        matches!(request, Request::Export { .. }),
        "Expected export request"
    );
    export_with_command(
        request,
        destination,
        ssh_command(host, platform),
        EXPORT_NO_PROGRESS_TIMEOUT,
        EXPORT_TOTAL_TIMEOUT,
        crate::export::MAX_BUNDLE_BYTES + crate::export::MAX_MANIFEST_BYTES as u64 + 12,
    )
    .await
    .map_err(tag_export_failure)
}

/// An export fails when no bytes arrive for this long.
const EXPORT_NO_PROGRESS_TIMEOUT: Duration = Duration::from_secs(60);
/// An export fails after this long even if bytes keep arriving.
const EXPORT_TOTAL_TIMEOUT: Duration = Duration::from_secs(30 * 60);

async fn export_with_command(
    request: &Request,
    destination: &Path,
    mut command: tokio::process::Command,
    no_progress: Duration,
    total: Duration,
    limit: u64,
) -> Result<()> {
    let bytes = serde_json::to_vec(request)?;
    ensure!(bytes.len() <= MAX_REQUEST_BYTES, "Request exceeds 8 KiB");
    let mut file = create_private_destination(destination)
        .await
        .context(ExportDestinationFailed)?;
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
            let mut last_progress = tokio::time::Instant::now();
            let copied = copy_stream(
                stdout.take(limit + 1),
                &mut file,
                no_progress,
                &mut last_progress,
            )
            .await?;
            ensure!(copied <= limit, "Bundle exceeds transfer limit");
            file.sync_all().await.context(ExportDestinationFailed)?;
            Ok::<_, anyhow::Error>(last_progress)
        };
        let diagnostics = tokio::spawn(async move {
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
        });
        let abort = diagnostics.abort_handle();
        let finish = async {
            let last_progress = copy.await?;
            // The waits that follow share the deadline of the last
            // transcript bytes. End of stream does not extend it.
            let deadline = last_progress + no_progress;
            let diagnostics = tokio::time::timeout_at(deadline, diagnostics)
                .await
                .map_err(|_| SessionTransferFailed::NoProgress(no_progress))??;
            let status = tokio::time::timeout_at(deadline, child.wait())
                .await
                .map_err(|_| SessionTransferFailed::NoProgress(no_progress))??;
            check_export_status(status.code(), &diagnostics?)
        };
        let result = finish.await;
        abort.abort();
        result
    };
    let result = tokio::time::timeout(total, operation)
        .await
        .map_err(|_| anyhow::Error::from(SessionTransferFailed::TotalCapExceeded(total)))
        .and_then(|result| result);
    if result.is_err() {
        let _ = tokio::fs::remove_file(destination).await;
    }
    result
}

/// Copy a stream. A read error, or no bytes for `no_progress` since
/// `last_progress`, is a per-session transfer failure. A write error is a local
/// storage failure and stays a host-level error. Only a nonempty read moves
/// `last_progress`; end of stream does not.
async fn copy_stream(
    mut reader: impl AsyncRead + Unpin,
    mut writer: impl AsyncWrite + Unpin,
    no_progress: Duration,
    last_progress: &mut tokio::time::Instant,
) -> Result<u64> {
    let mut buffer = vec![0; 64 * 1024];
    let mut copied = 0;
    loop {
        let read = tokio::time::timeout_at(*last_progress + no_progress, reader.read(&mut buffer))
            .await
            .map_err(|_| SessionTransferFailed::NoProgress(no_progress))?
            .map_err(|error| SessionTransferFailed::StreamBroken(error.to_string()))?;
        if read == 0 {
            return Ok(copied);
        }
        *last_progress = tokio::time::Instant::now();
        writer
            .write_all(&buffer[..read])
            .await
            .context(ExportDestinationFailed)?;
        copied += read as u64;
    }
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
        let command = ssh_command("build-box", Some("linux"));
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
        for platform in [None, Some("windows")] {
            let output = request_with_shell_fallback(
                host,
                platform,
                &request,
                command_for,
                Duration::from_secs(2),
            )
            .await
            .unwrap_or_else(|_| panic!("fallback request failed"));
            assert_eq!(output, serde_json::to_vec(&request).unwrap());
        }
        // The second request uses the remembered Windows path at once.
        assert_eq!(unix_attempts.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_missing_windows_helper_is_reported_as_missing() {
        for diagnostic in [
            r"'C:\Users\me\.local\bin\antiburn-remote.exe' is not recognized as an internal or external command,",
            r#"'"C:\Users\First Last\.local\bin\antiburn-remote.exe"' is not recognized as an internal or external command,"#,
        ] {
            let error = classify_failure(CommandFailure::Exit(diagnostic.to_owned()));
            assert_eq!(error.category(), PrerequisiteErrorCategory::HelperMissing);
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn known_windows_platform_skips_detection_without_process_state() {
        let request = Request::Hello {
            version: PROTOCOL_VERSION,
        };
        let output = request_with_shell_fallback(
            "windows-box",
            Some("windows"),
            &request,
            |host, helper| {
                assert_eq!(host, "windows-box");
                assert_eq!(helper, WINDOWS_HELPER_COMMAND);
                fake_ssh("cat")
            },
            Duration::from_secs(2),
        )
        .await
        .unwrap_or_else(|_| panic!("known Windows request failed"));
        assert_eq!(output, serde_json::to_vec(&request).unwrap());
        let command = ssh_command("windows-box", Some("windows"));
        assert_eq!(
            command.as_std().get_args().last().unwrap(),
            WINDOWS_HELPER_COMMAND
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn known_posix_platform_does_not_use_text_detection() {
        let result = request_with_shell_fallback(
            "linux-box", Some("linux"), &Request::Hello { version: PROTOCOL_VERSION },
            |_, helper| {
                assert_eq!(helper, HELPER_COMMAND);
                fake_ssh("cat >/dev/null; echo 'is not recognized as an internal or external command' >&2; exit 1")
            }, Duration::from_secs(2),
        ).await;
        assert!(matches!(result, Err(CommandFailure::Exit(_))));
    }

    #[test]
    fn windows_helper_command_quotes_the_executable_path() {
        let args: Vec<_> = ssh_command_with("win-box", WINDOWS_HELPER_COMMAND)
            .as_std()
            .get_args()
            .map(|arg| arg.to_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            args.last().map(String::as_str),
            Some(r#""%USERPROFILE%\.local\bin\antiburn-remote.exe" stdio"#)
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
                deadline,
                64,
            )
            .await;
            assert!(result.is_err(), "{script}");
            assert!(!destination.exists(), "{script}");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn only_a_slow_export_is_a_per_session_transfer_failure() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("bundle");
        let slow = export_with_command(
            &export_request(),
            &destination,
            fake_ssh("cat >/dev/null; exec sleep 2"),
            Duration::from_millis(50),
            Duration::from_secs(2),
            64,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            slow.downcast_ref::<SessionTransferFailed>(),
            Some(SessionTransferFailed::NoProgress(_))
        ));
        assert!(!destination.exists());
        for script in [
            "cat >/dev/null; printf partial; exit 255",
            "cat >/dev/null; head -c 65 /dev/zero",
        ] {
            let error = export_with_command(
                &export_request(),
                &destination,
                fake_ssh(script),
                Duration::from_secs(2),
                Duration::from_secs(2),
                64,
            )
            .await
            .unwrap_err();
            assert!(!error.is::<SessionTransferFailed>(), "{script}");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn export_failures_name_their_origin() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("bundle");
        let run = |command, destination: std::path::PathBuf| async move {
            tag_export_failure(
                export_with_command(
                    &export_request(),
                    &destination,
                    command,
                    Duration::from_secs(2),
                    Duration::from_secs(2),
                    64,
                )
                .await
                .unwrap_err(),
            )
        };
        let exit = run(fake_ssh("cat >/dev/null; exit 255"), destination.clone()).await;
        assert!(exit.is::<ExportTransportFailed>());
        assert!(!exit.is::<ExportDestinationFailed>());

        let oversize = run(
            fake_ssh("cat >/dev/null; head -c 65 /dev/zero"),
            destination.clone(),
        )
        .await;
        assert!(oversize.is::<ExportTransportFailed>());

        let missing = tokio::process::Command::new(temp.path().join("no-such-ssh"));
        let start = run(missing, destination.clone()).await;
        assert!(start.is::<ExportTransportFailed>());

        let rejected = run(
            fake_ssh(&format!(
                "cat >/dev/null; exit {}",
                crate::SESSION_REJECTED_EXIT_CODE
            )),
            destination.clone(),
        )
        .await;
        assert!(rejected.is::<SessionRejected>());
        assert!(!rejected.is::<ExportTransportFailed>());

        let missing_directory = temp.path().join("missing").join("bundle");
        let local = run(fake_ssh("cat >/dev/null"), missing_directory).await;
        assert!(local.is::<ExportDestinationFailed>());
        assert!(!local.is::<ExportTransportFailed>());
    }

    #[tokio::test]
    async fn a_destination_write_error_is_tagged_as_local() {
        let write_error = copy_stream(
            &b"bundle"[..],
            FailingWriter,
            Duration::from_secs(1),
            &mut tokio::time::Instant::now(),
        )
        .await
        .unwrap_err();
        assert!(write_error.is::<ExportDestinationFailed>());
        let read_error = copy_stream(
            FailingReader,
            tokio::io::sink(),
            Duration::from_secs(1),
            &mut tokio::time::Instant::now(),
        )
        .await
        .unwrap_err();
        assert!(!read_error.is::<ExportDestinationFailed>());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_steady_slow_stream_outlasts_the_no_progress_window() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("bundle");
        // Eight chunks 100 ms apart take longer than the 500 ms no-progress
        // window in total, but each gap stays far below it.
        export_with_command(
            &export_request(),
            &destination,
            fake_ssh("cat >/dev/null; for i in 1 2 3 4 5 6 7 8; do printf ab; sleep 0.1; done"),
            Duration::from_millis(500),
            Duration::from_secs(10),
            64,
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(&destination).unwrap().len(), 16);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_stream_that_stops_fails_after_the_no_progress_window() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("bundle");
        let started = std::time::Instant::now();
        let error = export_with_command(
            &export_request(),
            &destination,
            fake_ssh("cat >/dev/null; printf ab; exec sleep 5"),
            Duration::from_millis(200),
            Duration::from_secs(10),
            64,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<SessionTransferFailed>(),
            Some(SessionTransferFailed::NoProgress(_))
        ));
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(!destination.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_stall_after_stdout_ends_fails_after_the_no_progress_window() {
        for script in [
            // stdout ends, stderr stays open.
            "cat >/dev/null; printf ab; exec 1>&-; exec sleep 5",
            // Both output pipes close, the process stays alive.
            "cat >/dev/null; printf ab; exec 1>&- 2>&-; exec sleep 5",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let destination = temp.path().join("bundle");
            let started = std::time::Instant::now();
            let error = export_with_command(
                &export_request(),
                &destination,
                fake_ssh(script),
                Duration::from_millis(200),
                Duration::from_secs(10),
                64,
            )
            .await
            .unwrap_err();
            assert!(
                matches!(
                    error.downcast_ref::<SessionTransferFailed>(),
                    Some(SessionTransferFailed::NoProgress(_))
                ),
                "{script}"
            );
            assert!(started.elapsed() < Duration::from_secs(3), "{script}");
            assert!(!destination.exists(), "{script}");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn staged_completion_shares_one_deadline_from_the_last_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("bundle");
        let started = std::time::Instant::now();
        // Stdout ends, stderr closes 250 ms later and the process exits 250 ms
        // after that. Each stage fits the 400 ms window, but together they
        // pass it since the last bytes.
        let error = export_with_command(
            &export_request(),
            &destination,
            fake_ssh("cat >/dev/null; printf ab; exec 1>&-; sleep 0.25; exec 2>&-; sleep 0.25"),
            Duration::from_millis(400),
            Duration::from_secs(10),
            64,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<SessionTransferFailed>(),
            Some(SessionTransferFailed::NoProgress(_))
        ));
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(!destination.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_trickling_stream_fails_at_the_total_cap() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("bundle");
        let error = export_with_command(
            &export_request(),
            &destination,
            fake_ssh("cat >/dev/null; while true; do printf a; sleep 0.05; done"),
            Duration::from_secs(5),
            Duration::from_millis(400),
            1 << 20,
        )
        .await
        .unwrap_err();
        let failure = error.downcast_ref::<SessionTransferFailed>().unwrap();
        assert!(matches!(
            failure,
            SessionTransferFailed::TotalCapExceeded(_)
        ));
        assert!(failure.to_string().contains("total limit"));
        assert!(!destination.exists());
    }

    struct FailingReader;

    impl AsyncRead for FailingReader {
        fn poll_read(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
            _: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Err(std::io::Error::from(
                std::io::ErrorKind::ConnectionReset,
            )))
        }
    }

    struct FailingWriter;

    impl AsyncWrite for FailingWriter {
        fn poll_write(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
            _: &[u8],
        ) -> std::task::Poll<std::io::Result<usize>> {
            std::task::Poll::Ready(Err(std::io::Error::other("no space left on device")))
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn only_a_read_error_is_a_per_session_transfer_failure() {
        let read_error = copy_stream(
            FailingReader,
            tokio::io::sink(),
            Duration::from_secs(1),
            &mut tokio::time::Instant::now(),
        )
        .await
        .unwrap_err();
        assert!(matches!(
            read_error.downcast_ref::<SessionTransferFailed>(),
            Some(SessionTransferFailed::StreamBroken(_))
        ));
        let write_error = copy_stream(
            &b"bundle"[..],
            FailingWriter,
            Duration::from_secs(1),
            &mut tokio::time::Instant::now(),
        )
        .await
        .unwrap_err();
        assert!(!write_error.is::<SessionTransferFailed>());
        assert_eq!(
            copy_stream(
                &b"bundle"[..],
                tokio::io::sink(),
                Duration::from_secs(1),
                &mut tokio::time::Instant::now()
            )
            .await
            .unwrap(),
            6
        );
    }

    #[tokio::test]
    async fn export_never_removes_a_preexisting_destination() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("existing.bundle");
        tokio::fs::write(&destination, b"keep").await.unwrap();

        assert!(
            export_to("build-box", Some("linux"), &export_request(), &destination)
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
