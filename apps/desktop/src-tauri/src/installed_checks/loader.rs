use std::collections::HashSet;
use std::io::{self, Read};
use std::path::Path;

use antiburn_checks::diagnostics::{Diagnostic, DiagnosticCode};
use antiburn_checks::discovery::CheckFile;
use antiburn_local::paths::protected;

use super::{MAX_DEFINITIONS, MAX_FILE_BYTES};
use crate::store::Store;

pub(super) struct DirectoryChecks {
    pub files: Vec<CheckFile>,
    pub error: Option<Diagnostic>,
}

pub(super) fn load_directory(
    store: &Store,
    directory: &Path,
    granted: &HashSet<String>,
) -> DirectoryChecks {
    if protected::cwd_resolution_blocked(directory, granted) {
        return unavailable(directory, "Folder access is not granted.");
    }
    match read_check_files(directory) {
        Ok(files) => DirectoryChecks { files, error: None },
        Err(error) => {
            if error.kind() == io::ErrorKind::PermissionDenied
                && let Some(name) = protected::protected_dir_name(directory)
                && let Err(error) = store.revoke_dir_grant(&name)
            {
                ::tracing::error!(event = "installed_check_grant_revoke_failed", error = %error);
            }
            unavailable(directory, &format!("Cannot read check directory: {error}."))
        }
    }
}

pub(super) fn unavailable(directory: &Path, message: &str) -> DirectoryChecks {
    DirectoryChecks {
        files: Vec::new(),
        error: Some(read_failure(directory, message)),
    }
}

fn read_check_files(directory: &Path) -> io::Result<Vec<CheckFile>> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if !matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("yml" | "yaml")
        ) {
            continue;
        }
        let content = match entry.file_type() {
            Ok(kind) if !kind.is_file() => continue,
            Ok(_) => read_content(&path)
                .map_err(|error| read_failure(&path, &format!("Cannot read check file: {error}."))),
            Err(error) => Err(read_failure(
                &path,
                &format!("Cannot inspect check file: {error}."),
            )),
        };
        if files.len() == MAX_DEFINITIONS {
            return Err(io::Error::other("More than 128 check files"));
        }
        files.push(CheckFile {
            path,
            content,
            enabled: true,
        });
    }
    Ok(files)
}

fn read_content(path: &Path) -> io::Result<String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(io::Error::other("Check file exceeds 256 KiB"));
    }
    String::from_utf8(bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Check file is not valid UTF-8"))
}

fn read_failure(path: &Path, message: &str) -> Diagnostic {
    Diagnostic::new(DiagnosticCode::DiscoveryReadFailed, message).with_source_path(path)
}
