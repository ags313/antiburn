use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

pub const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(unix)]
const MAX_OUTPUT: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessError {
    Unavailable,
    Timeout,
    OutputLimit,
    Failed,
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn hanging_and_unbounded_probes_are_stopped_within_limits() {
        let env = BTreeMap::new();
        let started = Instant::now();
        assert_eq!(
            NativeRunner.run(
                Path::new("/bin/sh"),
                &["-c".into(), "/bin/sleep 30".into()],
                &env,
                Duration::from_millis(50)
            ),
            Err(ProcessError::Timeout)
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(
            NativeRunner.run(Path::new("/usr/bin/yes"), &[], &env, PROBE_TIMEOUT),
            Err(ProcessError::OutputLimit)
        );
    }

    #[test]
    fn successful_probe_does_not_leave_background_helpers_running() {
        let temp = tempfile::tempdir().unwrap();
        let marker = temp.path().join("marker");
        let script = "(/bin/sleep 0.2; printf leaked > \"$1\") & printf ok";
        assert_eq!(
            NativeRunner
                .run(
                    Path::new("/bin/sh"),
                    &[
                        "-c".into(),
                        script.into(),
                        "probe".into(),
                        marker.to_string_lossy().into_owned()
                    ],
                    &BTreeMap::new(),
                    PROBE_TIMEOUT
                )
                .unwrap(),
            "ok"
        );
        std::thread::sleep(Duration::from_millis(400));
        assert!(!marker.exists());
    }
}

pub trait Runner {
    fn run(
        &self,
        executable: &Path,
        args: &[String],
        env: &BTreeMap<String, String>,
        timeout: Duration,
    ) -> Result<String, ProcessError>;
}

pub struct NativeRunner;

impl Runner for NativeRunner {
    #[cfg(unix)]
    fn run(
        &self,
        executable: &Path,
        args: &[String],
        env: &BTreeMap<String, String>,
        timeout: Duration,
    ) -> Result<String, ProcessError> {
        use std::io::Read;
        use std::os::fd::AsRawFd;
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};
        use std::time::Instant;

        if !executable.is_absolute() || timeout > Duration::from_secs(30) {
            return Err(ProcessError::Unavailable);
        }
        let mut child = Command::new(executable)
            .args(args)
            .env_clear()
            .envs(env)
            .env_remove("NODE_OPTIONS")
            .env_remove("NODE_PATH")
            .current_dir("/")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .map_err(|_| ProcessError::Unavailable)?;
        let mut stdout = child.stdout.take().ok_or(ProcessError::Unavailable)?;
        let fd = stdout.as_raw_fd();
        // The pipe is private to this child and remains open during this call.
        let nonblocking = unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL);
            flags >= 0 && libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) >= 0
        };
        let result = (|| {
            if !nonblocking {
                return Err(ProcessError::Unavailable);
            }
            let started = Instant::now();
            let mut output = Vec::new();
            loop {
                let mut chunk = [0; 4096];
                loop {
                    match stdout.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(count) => {
                            if output.len() + count > MAX_OUTPUT {
                                return Err(ProcessError::OutputLimit);
                            }
                            output.extend_from_slice(&chunk[..count]);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(_) => return Err(ProcessError::Failed),
                    }
                    if started.elapsed() >= timeout {
                        return Err(ProcessError::Timeout);
                    }
                }
                if let Some(status) = child.try_wait().map_err(|_| ProcessError::Failed)? {
                    if !status.success() {
                        return Err(ProcessError::Failed);
                    }
                    // Drain the final bounded pipe bytes after the child exits.
                    loop {
                        match stdout.read(&mut chunk) {
                            Ok(0) => break,
                            Ok(count) => {
                                if output.len() + count > MAX_OUTPUT {
                                    return Err(ProcessError::OutputLimit);
                                }
                                output.extend_from_slice(&chunk[..count]);
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                            Err(_) => return Err(ProcessError::Failed),
                        }
                    }
                    return String::from_utf8(output).map_err(|_| ProcessError::Failed);
                }
                if started.elapsed() >= timeout {
                    return Err(ProcessError::Timeout);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        })();
        // The child owns this process group. Do not leave probe helpers running.
        unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
        if result.is_err() {
            let _ = child.kill();
        }
        let _ = child.wait();
        result
    }

    #[cfg(not(unix))]
    fn run(
        &self,
        _: &Path,
        _: &[String],
        _: &BTreeMap<String, String>,
        _: Duration,
    ) -> Result<String, ProcessError> {
        Err(ProcessError::Unavailable)
    }
}
