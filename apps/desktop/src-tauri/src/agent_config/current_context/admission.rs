use std::{
    collections::HashSet,
    fs,
    path::{Component, Path, PathBuf},
};

use antiburn_local::paths::protected;

#[derive(Debug, Clone, Copy)]
pub(super) struct PathAdmission<'a> {
    home: &'a Path,
    granted_dirs: &'a HashSet<String>,
}

impl<'a> PathAdmission<'a> {
    pub fn new(home: &'a Path, granted_dirs: &'a HashSet<String>) -> Self {
        Self { home, granted_dirs }
    }

    pub fn check(self, path: &Path) -> Result<(), &'static str> {
        if !self.home.is_absolute()
            || self
                .home
                .components()
                .any(|part| part == Component::ParentDir)
        {
            return Err("home_unavailable");
        }
        if !path.is_absolute() || path.components().any(|part| part == Component::ParentDir) {
            return Err("path_outside_allowed_root");
        }
        #[cfg(target_os = "macos")]
        let blocked = protected::protected_dir_names().iter().any(|name| {
            !self.granted_dirs.contains(*name)
                && may_alias_protected_prefix(path, &self.home.join(name))
        });
        #[cfg(not(target_os = "macos"))]
        let blocked = protected::cwd_resolution_blocked_in(self.home, path, self.granted_dirs);
        if blocked {
            return Err("protected_path_not_granted");
        }
        Ok(())
    }

    pub fn canonical_directory(self, path: &Path) -> Result<PathBuf, &'static str> {
        self.check(path)?;
        let mut current = PathBuf::new();
        for component in path.components() {
            current.push(component);
            if !current.is_absolute() {
                continue;
            }
            self.check(&current)?;
            let metadata = fs::symlink_metadata(&current).map_err(|_| "directory_unavailable")?;
            if metadata.file_type().is_symlink() {
                return Err("symlink_not_read");
            }
            if !metadata.is_dir() {
                return Err("directory_unavailable");
            }
        }
        let canonical = path.canonicalize().map_err(|_| "directory_unavailable")?;
        self.check(&canonical)?;
        Ok(canonical)
    }
}

#[cfg(target_os = "macos")]
fn may_alias_protected_prefix(path: &Path, prefix: &Path) -> bool {
    // Do not probe the volume for its case or Unicode rules. Match ASCII case
    // on all macOS volumes and treat non-ASCII components as possible aliases.
    let mut actual = path.components();
    prefix.components().all(|expected| {
        let Some(actual) = actual.next() else {
            return false;
        };
        match (actual, expected) {
            (Component::Normal(actual), Component::Normal(expected)) => {
                let actual = actual.as_encoded_bytes();
                let expected = expected.as_encoded_bytes();
                !actual.is_ascii() || !expected.is_ascii() || actual.eq_ignore_ascii_case(expected)
            }
            _ => actual == expected,
        }
    })
}
