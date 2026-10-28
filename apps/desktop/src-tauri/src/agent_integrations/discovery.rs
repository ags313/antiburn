use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::Vendor;
use super::process::{PROBE_TIMEOUT, Runner};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub vendor: Vendor,
    pub root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Runtime {
    pub executable: PathBuf,
    pub version: String,
}

fn absolute(value: &str) -> Option<PathBuf> {
    let path = PathBuf::from(value);
    (path.is_absolute()
        && value.len() <= 8192
        && !value.chars().any(char::is_control)
        && !value.starts_with("//")
        && !value.starts_with(r"\\"))
    .then_some(path)
}

pub fn profile(
    vendor: Vendor,
    home: &Path,
    env: &BTreeMap<String, String>,
    saved: Option<&Profile>,
) -> Result<Profile, &'static str> {
    if let Some(saved) = saved {
        if saved.vendor != vendor || !saved.root.is_absolute() {
            return Err("invalid_saved_profile");
        }
        return Ok(saved.clone());
    }
    let override_keys: &[&str] = match vendor {
        Vendor::Claude => &["CLAUDE_CONFIG_DIR"],
        Vendor::Codex => &["CODEX_HOME"],
        Vendor::Cursor => &[],
        Vendor::OpenCode => &["OPENCODE_CONFIG_DIR"],
        Vendor::Pi => &["PI_CODING_AGENT_DIR", "PI_AGENT_DIR"],
    };
    for key in override_keys {
        if let Some(value) = env.get(*key) {
            return Ok(Profile {
                vendor,
                root: absolute(value).ok_or("invalid_profile_override")?,
            });
        }
    }
    let root = match vendor {
        Vendor::Claude => home.join(".claude"),
        Vendor::Codex => home.join(".codex"),
        Vendor::Cursor => home.join(".cursor"),
        Vendor::Pi => home.join(".pi/agent"),
        Vendor::OpenCode => match env.get("XDG_CONFIG_HOME") {
            Some(value) => absolute(value)
                .ok_or("invalid_profile_override")?
                .join("opencode"),
            None => home.join(".config/opencode"),
        },
    };
    if !root.is_absolute() {
        return Err("invalid_home");
    }
    Ok(Profile { vendor, root })
}

pub fn search_directories(home: &Path, env: &BTreeMap<String, String>) -> Vec<PathBuf> {
    let mut directories = Vec::new();
    if let Some(path) = env.get("PATH").filter(|value| value.len() <= 32768) {
        directories.extend(
            std::env::split_paths(path)
                .filter(|p| p.is_absolute())
                .take(64),
        );
    }
    directories.extend([
        home.join(".local/bin"),
        home.join(".volta/bin"),
        home.join(".local/share/mise/shims"),
        home.join(".local/share/mise/installs/node/lts/bin"),
        home.join(".local/share/fnm/aliases/default/bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
    ]);
    for root in [
        home.join(".nvm/versions/node"),
        home.join(".local/share/mise/installs/node"),
        home.join(".asdf/installs/nodejs"),
    ] {
        if let Ok(entries) = std::fs::read_dir(root) {
            let mut versions = entries
                .take(16)
                .filter_map(std::result::Result::ok)
                .filter(|entry| {
                    entry.file_name().to_str().is_some_and(|name| {
                        name.len() < 32
                            && name
                                .bytes()
                                .all(|byte| byte.is_ascii_digit() || b"v.-".contains(&byte))
                    })
                })
                .map(|entry| entry.path().join("bin"))
                .collect::<Vec<_>>();
            versions.sort();
            versions.reverse();
            directories.extend(versions.into_iter().take(8));
        }
    }
    let mut seen = BTreeSet::new();
    directories.retain(|p| seen.insert(p.clone()));
    directories
}

fn executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return false;
        }
    }
    true
}

pub fn find_node(directories: &[PathBuf], runner: &impl Runner) -> Option<Runtime> {
    let started = Instant::now();
    for directory in directories.iter().take(96) {
        if started.elapsed() >= Duration::from_secs(4) {
            break;
        }
        let candidate = directory.join(if cfg!(windows) { "node.exe" } else { "node" });
        if !candidate.is_absolute() || !executable(&candidate) {
            continue;
        }
        let Ok(path) = candidate.canonicalize() else {
            continue;
        };
        // Resolve manager shims to the native Node binary before registration.
        let Ok(output) = runner.run(
            &path,
            &["--eval".into(), "process.stdout.write(JSON.stringify({version:process.version,executable:process.execPath}))".into()],
            &BTreeMap::new(),
            PROBE_TIMEOUT.min(Duration::from_secs(4).saturating_sub(started.elapsed())),
        ) else {
            continue;
        };
        #[derive(Deserialize)]
        struct Probe {
            version: String,
            executable: PathBuf,
        }
        let Ok(probe) = serde_json::from_str::<Probe>(&output) else {
            continue;
        };
        let Ok(path) = probe.executable.canonicalize() else {
            continue;
        };
        if compatible_node(&probe.version) && path.is_absolute() && executable(&path) {
            return Some(Runtime {
                executable: path,
                version: probe.version,
            });
        }
    }
    None
}

pub fn compatible_node(version: &str) -> bool {
    let Some(version) = version.strip_prefix('v') else {
        return false;
    };
    let parts = version.split('.').collect::<Vec<_>>();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty() && part.len() <= 4 && part.bytes().all(|b| b.is_ascii_digit())
        })
        && parts[0].parse::<u32>().is_ok_and(|major| major >= 22)
}

pub fn find_host(vendor: Vendor, directories: &[PathBuf]) -> Option<PathBuf> {
    let name = match vendor {
        Vendor::Claude => "claude",
        Vendor::Codex => "codex",
        Vendor::Cursor => "cursor-agent",
        Vendor::OpenCode => "opencode",
        Vendor::Pi => "pi",
    };
    directories
        .iter()
        .take(80)
        .map(|directory| directory.join(name))
        .find(|path| path.is_absolute() && executable(path))
    // Keep the vendor launcher so its own runtime setup remains intact.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_requires_a_stable_numeric_release_at_least_22() {
        for good in ["v22.0.0", "v25.9.0", "v24.13.1"] {
            assert!(compatible_node(good));
        }
        for bad in [
            "v21.9.0",
            "22.0.0",
            "v22.0",
            "v22.0.0-nightly",
            "v22.0.0\nextra",
            "v999999.1.1",
        ] {
            assert!(!compatible_node(bad), "{bad}");
        }
    }

    #[test]
    fn saved_profile_wins_over_changed_gui_environment() {
        let saved = Profile {
            vendor: Vendor::Codex,
            root: "/profiles/selected".into(),
        };
        let env = BTreeMap::from([("CODEX_HOME".into(), "/profiles/other".into())]);
        assert_eq!(
            profile(Vendor::Codex, Path::new("/home/test"), &env, Some(&saved)).unwrap(),
            saved
        );
        assert_eq!(
            profile(Vendor::Codex, Path::new("/home/test"), &env, None)
                .unwrap()
                .root,
            Path::new("/profiles/other")
        );
    }

    #[test]
    fn invalid_overrides_never_fall_back_to_a_different_profile() {
        let env = BTreeMap::from([("PI_CODING_AGENT_DIR".into(), "relative".into())]);
        assert!(profile(Vendor::Pi, Path::new("/home/test"), &env, None).is_err());
    }
}
