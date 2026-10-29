use super::*;

const TERMINAL_IDENTITY: &str = "ai.antiburn.desktop.cli.terminal";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalState {
    pub enabled: bool,
    pub installed: bool,
    pub executable: Option<PathBuf>,
    pub diagnostic: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TerminalReceipt {
    schema: u32,
    identity: String,
    enabled: bool,
    destination: PathBuf,
    active: Option<Entry>,
    pending: Option<Entry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    sha256: String,
    file_id: (u64, u64),
    stage: String,
}

fn command_name() -> &'static str {
    if cfg!(windows) {
        "antiburn.exe"
    } else {
        "antiburn"
    }
}

fn validate_destination(path: &Path) -> Result<()> {
    ensure!(path.is_absolute(), "Terminal directory must be absolute");
    ensure!(
        path.components().all(|part| matches!(
            part,
            std::path::Component::RootDir
                | std::path::Component::Prefix(_)
                | std::path::Component::Normal(_)
        )),
        "Terminal directory cannot contain parent or current directory components"
    );
    Ok(())
}

fn file_identity(path: &Path) -> Result<(u64, u64)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::symlink_metadata(path)?;
        ensure!(
            metadata.uid() == unsafe { libc::geteuid() },
            "Terminal command belongs to another user"
        );
        Ok((metadata.dev(), metadata.ino()))
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
        };
        check_owner(path, &fs::symlink_metadata(path)?)?;
        let file = File::open(path)?;
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        ensure!(
            unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } != 0,
            "Cannot inspect terminal command identity"
        );
        Ok((
            u64::from(info.dwVolumeSerialNumber),
            (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        ))
    }
}

impl Installation {
    fn terminal_receipt_path(&self) -> PathBuf {
        self.root.join("terminal-receipt.json")
    }

    fn terminal_receipt(&self) -> Result<Option<TerminalReceipt>> {
        let path = self.terminal_receipt_path();
        if !present(&path)? {
            return Ok(None);
        }
        check_owner(&path, &fs::symlink_metadata(&path)?)?;
        let receipt: TerminalReceipt = serde_json::from_slice(&read_file(&path, MAX_JSON)?)
            .context("Terminal ownership receipt is invalid; preserve it for review")?;
        ensure!(
            receipt.schema == 1 && receipt.identity == TERMINAL_IDENTITY,
            "Terminal ownership receipt is not recognized"
        );
        validate_destination(&receipt.destination)?;
        ensure!(
            receipt
                .destination
                .file_name()
                .is_some_and(|name| name == command_name()),
            "Terminal receipt has an invalid command name"
        );
        ensure!(
            !receipt.destination.starts_with(&self.root),
            "Terminal command must be outside the private runtime"
        );
        for entry in [&receipt.active, &receipt.pending].into_iter().flatten() {
            ensure!(
                entry.sha256.len() == 64 && entry.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
                "Terminal receipt digest is invalid"
            );
            ensure!(
                entry
                    .stage
                    .strip_prefix(".antiburn-terminal-")
                    .is_some_and(|suffix| suffix.len() == 32
                        && suffix.bytes().all(|b| b.is_ascii_hexdigit())),
                "Terminal receipt stage is invalid"
            );
        }
        Ok(Some(receipt))
    }

    fn save_terminal(&self, receipt: &TerminalReceipt) -> Result<()> {
        let path = self.terminal_receipt_path();
        atomic_bytes(
            &path,
            &serde_json::to_vec(receipt)?,
            false,
            None,
            present(&path)?,
        )
    }

    fn terminal_matches(&self, path: &Path, entry: &Entry) -> Result<bool> {
        if !present(path)? {
            return Ok(false);
        }
        #[cfg(windows)]
        if !fs::symlink_metadata(path)?.is_file() {
            return Ok(false);
        }
        if file_identity(path)? != entry.file_id {
            return Ok(false);
        }
        #[cfg(unix)]
        {
            Ok(fs::read_link(path).is_ok_and(|target| target == self.stable()))
        }
        #[cfg(windows)]
        {
            Ok(hash(path)? == entry.sha256)
        }
    }

    fn terminal_owned(&self, receipt: &TerminalReceipt) -> Result<bool> {
        for entry in [&receipt.active, &receipt.pending].into_iter().flatten() {
            if self.terminal_matches(&receipt.destination, entry)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn cleanup_terminal_stage(&self, receipt: &TerminalReceipt, entry: &Entry) -> Result<()> {
        let stage = parent(&receipt.destination)?.join(&entry.stage);
        if self.terminal_matches(&stage, entry)? {
            fs::remove_file(&stage)?;
            sync_dir(parent(&stage)?)?;
        }
        Ok(())
    }

    fn terminal_state_locked(&self, receipt: Option<&TerminalReceipt>) -> Result<TerminalState> {
        let Some(receipt) = receipt else {
            return Ok(TerminalState {
                enabled: false,
                installed: false,
                executable: None,
                diagnostic: None,
            });
        };
        if !receipt.enabled && receipt.active.is_none() && receipt.pending.is_none() {
            return self.terminal_state_locked(None);
        }
        let exists = present(&receipt.destination)?;
        if exists {
            safe_dir(parent(&receipt.destination)?, false)?;
        }
        let owned = exists && self.terminal_owned(receipt)?;
        let runtime = self.runtime(&self.receipt()?);
        #[cfg(unix)]
        let current = runtime.is_ok();
        #[cfg(windows)]
        let current = runtime.as_ref().is_ok_and(|runtime| {
            [&receipt.active, &receipt.pending]
                .into_iter()
                .flatten()
                .any(|entry| {
                    entry.sha256 == runtime.sha256
                        && self
                            .terminal_matches(&receipt.destination, entry)
                            .unwrap_or(false)
                })
        });
        let diagnostic = if exists && !owned {
            Some("Terminal command was replaced or modified; preserve it and choose another directory".into())
        } else if receipt.enabled && !exists {
            Some("Terminal command is missing; run terminal install to enable it again".into())
        } else if owned && !current {
            Some(runtime.as_ref().err().map(ToString::to_string).unwrap_or_else(||
                "Terminal command update is pending; close running commands and repair the CLI".into()))
        } else {
            None
        };
        Ok(TerminalState {
            enabled: receipt.enabled,
            installed: owned && current,
            executable: Some(receipt.destination.clone()),
            diagnostic,
        })
    }

    pub fn terminal_status(&self) -> Result<TerminalState> {
        if !present(&self.root)? || !present(&self.terminal_receipt_path())? {
            return self.terminal_state_locked(None);
        }
        // Status opens the existing lock without creating installation files.
        safe_dir(&self.root, false)?;
        let path = self.root.join("install.lock");
        check_owner(&path, &fs::symlink_metadata(&path)?)?;
        ensure!(
            fs::symlink_metadata(&path)?.is_file()
                && !fs::symlink_metadata(&path)?.file_type().is_symlink(),
            "CLI lock is redirected"
        );
        let mut options = OpenOptions::new();
        options.read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let lock = options.open(path)?;
        lock.try_lock()
            .context("CLI installation is busy; retry after the other operation finishes")?;
        self.terminal_state_locked(self.terminal_receipt()?.as_ref())
    }

    pub fn terminal_install(&self, destination_dir: &Path) -> Result<TerminalState> {
        validate_destination(destination_dir)?;
        ensure!(
            !destination_dir.starts_with(&self.root),
            "Terminal command must be outside the private runtime"
        );
        let _lock = self.lock()?;
        let runtime = self.runtime(&self.receipt()?)?;
        safe_dir(destination_dir, true)?;
        let destination = destination_dir.join(command_name());
        let mut receipt = match self.terminal_receipt()? {
            Some(receipt) => {
                ensure!(
                    receipt.destination == destination
                        || (!receipt.enabled
                            && receipt.active.is_none()
                            && receipt.pending.is_none()),
                    "Uninstall the current terminal command before choosing another directory"
                );
                if receipt.destination == destination {
                    receipt
                } else {
                    new_receipt(destination)
                }
            }
            None => new_receipt(destination),
        };
        ensure!(
            !present(&receipt.destination)? || self.terminal_owned(&receipt)?,
            "Terminal destination collides with an unrelated or modified command; choose another directory"
        );
        receipt.enabled = true;
        self.save_terminal(&receipt)?;
        self.publish_terminal(&mut receipt, &runtime)?;
        self.terminal_state_locked(Some(&receipt))
    }

    pub fn terminal_uninstall(&self) -> Result<TerminalState> {
        if !present(&self.root)? || !present(&self.terminal_receipt_path())? {
            return self.terminal_state_locked(None);
        }
        let _lock = self.lock()?;
        let Some(mut receipt) = self.terminal_receipt()? else {
            return self.terminal_state_locked(None);
        };
        // Save opt-out before removal so interrupted cleanup cannot enable the command.
        receipt.enabled = false;
        self.save_terminal(&receipt)?;
        self.remove_terminal(&mut receipt)?;
        self.terminal_state_locked(Some(&receipt))
    }

    fn remove_terminal(&self, receipt: &mut TerminalReceipt) -> Result<()> {
        if (receipt.active.is_some() || receipt.pending.is_some()) && present(&receipt.destination)?
        {
            safe_dir(parent(&receipt.destination)?, false)?;
            if self.terminal_owned(receipt)? {
                fs::remove_file(&receipt.destination).context(
                    "Terminal command is locked; opt-out was saved and cleanup will retry",
                )?;
                sync_dir(parent(&receipt.destination)?)?;
            }
        }
        for entry in [&receipt.active, &receipt.pending].into_iter().flatten() {
            self.cleanup_terminal_stage(receipt, entry)?;
        }
        receipt.active = None;
        receipt.pending = None;
        self.save_terminal(receipt)
    }

    pub(super) fn reconcile_terminal(&self) -> Result<()> {
        let Some(mut receipt) = self.terminal_receipt()? else {
            return Ok(());
        };
        if !receipt.enabled {
            return self.remove_terminal(&mut receipt);
        }
        if !present(&receipt.destination)? && receipt.pending.is_none() {
            // A removed command remains absent until the user installs it again.
            receipt.enabled = false;
            receipt.active = None;
            return self.save_terminal(&receipt);
        }
        let runtime = self.runtime(&self.receipt()?)?;
        self.publish_terminal(&mut receipt, &runtime)
    }

    fn publish_terminal(&self, receipt: &mut TerminalReceipt, runtime: &CliRuntime) -> Result<()> {
        safe_dir(parent(&receipt.destination)?, false)?;
        ensure!(
            !present(&receipt.destination)? || self.terminal_owned(receipt)?,
            "Terminal command was replaced or modified; preserve it and choose another directory"
        );
        if let Some(pending) = receipt.pending.clone() {
            if !self.terminal_matches(&receipt.destination, &pending)? {
                let stage = parent(&receipt.destination)?.join(&pending.stage);
                ensure!(
                    self.terminal_matches(&stage, &pending)?,
                    "Interrupted terminal publication lost its staged command; preserve the receipt for review"
                );
                self.publish_terminal_stage(receipt, &stage)?;
            }
            self.cleanup_terminal_stage(receipt, &pending)?;
            receipt.active = Some(pending);
            receipt.pending = None;
            self.save_terminal(receipt)?;
        }
        if let Some(active) = &receipt.active
            && self.terminal_matches(&receipt.destination, active)?
            && (cfg!(unix) || active.sha256 == runtime.sha256)
        {
            return Ok(());
        }
        let mut random = [0u8; 16];
        getrandom::fill(&mut random)
            .map_err(|e| anyhow::anyhow!("Terminal temporary name: {e}"))?;
        let name = format!(".antiburn-terminal-{:032x}", u128::from_le_bytes(random));
        let stage = parent(&receipt.destination)?.join(&name);
        #[cfg(unix)]
        std::os::unix::fs::symlink(self.stable(), &stage)?;
        #[cfg(windows)]
        {
            let source = self
                .root
                .join("versions")
                .join(&runtime.sha256)
                .join(file_name());
            atomic_bytes(
                &stage,
                &read_file(&source, MAX_BINARY)?,
                true,
                Some(&source),
                false,
            )?;
        }
        sync_dir(parent(&stage)?)?;
        let pending = Entry {
            sha256: runtime.sha256.clone(),
            file_id: file_identity(&stage)?,
            stage: name,
        };
        receipt.pending = Some(pending.clone());
        self.save_terminal(receipt)?;
        self.publish_terminal_stage(receipt, &stage)?;
        self.cleanup_terminal_stage(receipt, &pending)?;
        receipt.active = Some(pending);
        receipt.pending = None;
        self.save_terminal(receipt)
    }

    fn publish_terminal_stage(&self, receipt: &TerminalReceipt, stage: &Path) -> Result<()> {
        if present(&receipt.destination)? {
            ensure!(
                self.terminal_owned(receipt)?,
                "Terminal command changed during publication; it was preserved"
            );
            #[cfg(unix)]
            fs::rename(stage, &receipt.destination)?;
            #[cfg(windows)]
            crate::windows::replace_preserving_policy(&receipt.destination, stage)?;
        } else {
            fs::hard_link(stage, &receipt.destination).context(
                "Terminal destination appeared during publication or does not support native links",
            )?;
        }
        sync_dir(parent(&receipt.destination)?)
    }
}

fn new_receipt(destination: PathBuf) -> TerminalReceipt {
    TerminalReceipt {
        schema: 1,
        identity: TERMINAL_IDENTITY.into(),
        enabled: true,
        destination,
        active: None,
        pending: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        _temp: tempfile::TempDir,
        installation: Installation,
        destination: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let base = temp.path().canonicalize().unwrap();
            let installation = Installation {
                root: base.join("private/cli"),
                terminal: base.join("bin").join(file_name()),
                payload: base.join("bundle"),
                target: "synthetic-target".into(),
            };
            fs::create_dir(&installation.payload).unwrap();
            let fixture = Self {
                _temp: temp,
                installation,
                destination: base.join("bin"),
            };
            fixture.payload(b"synthetic CLI first version");
            fixture
        }

        fn payload(&self, bytes: &[u8]) {
            let manifest = Manifest {
                schema: 1,
                target: self.installation.target.clone(),
                executable: file_name().into(),
                version: "0.1.0".into(),
                sha256: digest(bytes),
                delivery_protocols: vec!["delivery-v1".into()],
            };
            fs::write(self.installation.payload.join(file_name()), bytes).unwrap();
            fs::write(
                self.installation.payload.join("manifest.json"),
                serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
        }

        fn install(&self) -> PathBuf {
            self.installation.reconcile().unwrap();
            self.installation
                .terminal_install(&self.destination)
                .unwrap()
                .executable
                .unwrap()
        }
    }

    #[test]
    fn status_does_not_create_an_installation_and_setup_requires_private_runtime() {
        let f = Fixture::new();
        assert!(!f.installation.terminal_status().unwrap().installed);
        assert!(!f.installation.root.exists());
        assert!(f.installation.terminal_install(&f.destination).is_err());
        assert!(!f.destination.exists());
        f.installation.reconcile().unwrap();
        assert!(!f.installation.terminal_status().unwrap().enabled);
        assert!(!f.destination.exists());
    }

    #[test]
    fn explicit_install_is_idempotent_and_private_updates_refresh_the_command() {
        let f = Fixture::new();
        let command = f.install();
        let original = file_identity(&command).unwrap();
        assert!(
            f.installation
                .terminal_install(&f.destination)
                .unwrap()
                .installed
        );
        assert_eq!(file_identity(&command).unwrap(), original);
        f.payload(b"synthetic CLI second version");
        f.installation.reconcile().unwrap();
        assert_eq!(fs::read(&command).unwrap(), b"synthetic CLI second version");
        assert!(f.installation.terminal_status().unwrap().installed);
        #[cfg(unix)]
        assert_eq!(fs::read_link(&command).unwrap(), f.installation.stable());
    }

    #[test]
    fn unrelated_desktop_command_and_directory_are_preserved() {
        for directory in [false, true] {
            let f = Fixture::new();
            f.installation.reconcile().unwrap();
            safe_dir(&f.destination, true).unwrap();
            let command = f.destination.join(command_name());
            if directory {
                fs::create_dir(&command).unwrap();
            } else {
                fs::write(&command, b"desktop launcher").unwrap();
            }
            assert!(f.installation.terminal_install(&f.destination).is_err());
            assert_eq!(command.is_dir(), directory);
            if !directory {
                assert_eq!(fs::read(command).unwrap(), b"desktop launcher");
            }
            assert!(!f.installation.terminal_receipt_path().exists());
        }
    }

    #[test]
    fn replacement_survives_uninstall_and_does_not_block_repair_or_relocation() {
        for directory in [false, true] {
            let f = Fixture::new();
            let command = f.install();
            let mut receipt = f.installation.terminal_receipt().unwrap().unwrap();
            let pending = receipt.active.as_ref().unwrap().clone();
            let stage = f.destination.join(&pending.stage);
            fs::hard_link(&command, &stage).unwrap();
            receipt.pending = Some(pending);
            f.installation.save_terminal(&receipt).unwrap();
            fs::remove_file(&command).unwrap();
            if directory {
                fs::create_dir(&command).unwrap();
            } else {
                fs::write(&command, b"user replacement").unwrap();
            }
            assert!(f.installation.reconcile().is_err());
            assert!(f.installation.resolve().is_ok());
            let state = f.installation.terminal_uninstall().unwrap();
            assert!(!state.enabled);
            assert!(!state.installed);
            assert!(state.diagnostic.is_none());
            let receipt = f.installation.terminal_receipt().unwrap().unwrap();
            assert!(!receipt.enabled);
            assert!(receipt.active.is_none());
            assert!(receipt.pending.is_none());
            assert!(!present(&stage).unwrap());
            f.installation.reconcile().unwrap();
            assert!(f.installation.terminal_uninstall().is_ok());
            let alternate = f.destination.with_file_name("alternate-bin");
            assert!(
                f.installation
                    .terminal_install(&alternate)
                    .unwrap()
                    .installed
            );
            f.installation.reconcile().unwrap();
            assert_eq!(command.is_dir(), directory);
            if !directory {
                assert_eq!(fs::read(&command).unwrap(), b"user replacement");
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn replacement_symlink_with_the_same_target_is_not_adopted() {
        let f = Fixture::new();
        let command = f.install();
        let replacement = f.destination.join("replacement");
        std::os::unix::fs::symlink(f.installation.stable(), &replacement).unwrap();
        fs::rename(replacement, &command).unwrap();
        assert!(!f.installation.terminal_uninstall().unwrap().enabled);
        f.installation.reconcile().unwrap();
        assert!(command.exists());
    }

    #[test]
    fn removal_and_explicit_opt_out_survive_updates() {
        for explicit in [false, true] {
            let f = Fixture::new();
            let command = f.install();
            if explicit {
                assert!(!f.installation.terminal_uninstall().unwrap().enabled);
            } else {
                fs::remove_file(&command).unwrap();
            }
            f.payload(b"updated CLI after command removal");
            f.installation.reconcile().unwrap();
            assert!(!present(&command).unwrap());
            assert!(!f.installation.terminal_status().unwrap().enabled);
            assert!(
                f.installation
                    .terminal_install(&f.destination)
                    .unwrap()
                    .installed
            );
        }
    }

    #[test]
    fn legacy_cleanup_does_not_remove_the_opted_in_command() {
        let f = Fixture::new();
        let command = f.install();
        let mut legacy = f.installation.receipt().unwrap();
        legacy.terminal_seen = true;
        legacy.terminal_sha256 = Some(f.installation.resolve().unwrap().sha256);
        #[cfg(unix)]
        std::os::unix::fs::symlink(f.installation.stable(), &f.installation.terminal).unwrap();
        #[cfg(windows)]
        fs::copy(f.installation.stable(), &f.installation.terminal).unwrap();
        f.installation.save(&legacy).unwrap();
        f.installation.reconcile().unwrap();
        assert!(command.exists());
        assert!(!present(&f.installation.terminal).unwrap());
    }

    #[test]
    fn terminal_operations_share_the_private_runtime_lock() {
        let f = Fixture::new();
        let command = f.install();
        let lock = f.installation.lock().unwrap();
        assert!(f.installation.terminal_status().is_err());
        assert!(f.installation.terminal_install(&f.destination).is_err());
        assert!(f.installation.terminal_uninstall().is_err());
        assert!(command.exists());
        drop(lock);
        assert!(f.installation.terminal_status().unwrap().installed);
    }

    #[test]
    fn pending_publication_recovers_before_and_after_link_creation() {
        for published in [false, true] {
            let f = Fixture::new();
            let command = f.install();
            let mut receipt = f.installation.terminal_receipt().unwrap().unwrap();
            let entry = receipt.active.take().unwrap();
            let stage = f.destination.join(&entry.stage);
            fs::hard_link(&command, &stage).unwrap();
            receipt.pending = Some(entry);
            f.installation.save_terminal(&receipt).unwrap();
            if !published {
                fs::remove_file(&command).unwrap();
            }
            f.installation.reconcile().unwrap();
            assert!(f.installation.terminal_status().unwrap().installed);
            assert!(!present(&stage).unwrap());
            assert!(
                f.installation
                    .terminal_receipt()
                    .unwrap()
                    .unwrap()
                    .pending
                    .is_none()
            );
        }
    }

    #[test]
    fn pending_publication_does_not_overwrite_a_new_collision() {
        let f = Fixture::new();
        let command = f.install();
        let mut receipt = f.installation.terminal_receipt().unwrap().unwrap();
        let entry = receipt.active.take().unwrap();
        fs::hard_link(&command, f.destination.join(&entry.stage)).unwrap();
        receipt.pending = Some(entry);
        f.installation.save_terminal(&receipt).unwrap();
        fs::remove_file(&command).unwrap();
        fs::write(&command, b"new desktop launcher").unwrap();
        assert!(f.installation.reconcile().is_err());
        assert_eq!(fs::read(&command).unwrap(), b"new desktop launcher");
    }

    #[test]
    fn relative_and_private_destinations_are_rejected() {
        let f = Fixture::new();
        f.installation.reconcile().unwrap();
        for path in [
            PathBuf::from("bin"),
            f.installation.root.join("bin"),
            f.destination.join("../bin"),
        ] {
            assert!(f.installation.terminal_install(&path).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn redirected_or_shared_destination_is_preserved() {
        use std::os::unix::fs::PermissionsExt;
        let f = Fixture::new();
        f.installation.reconcile().unwrap();
        safe_dir(&f.destination, true).unwrap();
        fs::set_permissions(&f.destination, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(f.installation.terminal_install(&f.destination).is_err());
        assert_eq!(
            fs::metadata(&f.destination).unwrap().permissions().mode() & 0o777,
            0o777
        );
        fs::set_permissions(&f.destination, fs::Permissions::from_mode(0o700)).unwrap();
        let link = f.destination.with_file_name("redirect");
        std::os::unix::fs::symlink(&f.destination, &link).unwrap();
        assert!(f.installation.terminal_install(&link).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn locked_windows_uninstall_keeps_ownership_until_cleanup_retries() {
        use std::os::windows::fs::OpenOptionsExt;
        let f = Fixture::new();
        let command = f.install();
        let handle = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&command)
            .unwrap();
        assert!(f.installation.terminal_uninstall().is_err());
        let receipt = f.installation.terminal_receipt().unwrap().unwrap();
        assert!(!receipt.enabled);
        assert!(receipt.active.is_some());
        assert!(command.exists());
        let alternate = f.destination.with_file_name("alternate-bin");
        assert!(f.installation.terminal_install(&alternate).is_err());
        drop(handle);
        f.installation.reconcile().unwrap();
        assert!(!command.exists());
        assert!(
            f.installation
                .terminal_receipt()
                .unwrap()
                .unwrap()
                .active
                .is_none()
        );
        assert!(
            f.installation
                .terminal_install(&alternate)
                .unwrap()
                .installed
        );
    }

    #[cfg(windows)]
    #[test]
    fn locked_windows_export_keeps_pending_update_until_retry() {
        use std::os::windows::fs::OpenOptionsExt;
        let f = Fixture::new();
        let command = f.install();
        let handle = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&command)
            .unwrap();
        f.payload(b"updated CLI with locked public command");
        assert!(f.installation.reconcile().is_err());
        assert!(
            f.installation
                .terminal_receipt()
                .unwrap()
                .unwrap()
                .pending
                .is_some()
        );
        drop(handle);
        f.installation.reconcile().unwrap();
        assert_eq!(
            fs::read(command).unwrap(),
            b"updated CLI with locked public command"
        );
    }
}
