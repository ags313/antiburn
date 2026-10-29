use std::{fs, path::Path, process::Command};

use antiburn_cli_runtime::{Installation, TARGET, file_name};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

struct Fixture {
    _temp: TempDir,
    root: std::path::PathBuf,
    commands: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let base = fs::canonicalize(temp.path()).unwrap();
        let payload = base.join("payload");
        fs::create_dir(&payload).unwrap();
        let bytes = fs::read(env!("CARGO_BIN_EXE_antiburn")).unwrap();
        let digest: String = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        fs::write(payload.join(file_name()), bytes).unwrap();
        fs::write(
            payload.join("manifest.json"),
            serde_json::to_vec(&json!({
                "schema": 1,
                "target": TARGET,
                "executable": file_name(),
                "version": env!("CARGO_PKG_VERSION"),
                "sha256": digest,
                "delivery_protocols": ["delivery-v1"]
            }))
            .unwrap(),
        )
        .unwrap();
        let root = base.join("cli");
        Installation {
            root: root.clone(),
            terminal: base.join("legacy").join(file_name()),
            payload,
            target: TARGET.to_owned(),
        }
        .reconcile()
        .unwrap();
        Self {
            _temp: temp,
            root,
            commands: base.join("commands"),
        }
    }

    fn call(&self, args: &[&str]) -> Value {
        let output = Command::new(env!("CARGO_BIN_EXE_antiburn"))
            .arg("terminal")
            .arg("--runtime-root")
            .arg(&self.root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn install(&self) -> Value {
        self.call(&["install", "--directory", self.commands.to_str().unwrap()])
    }
}

#[test]
fn explicit_setup_runs_the_cli_and_uninstall_keeps_the_private_runtime() {
    let f = Fixture::new();
    assert_eq!(f.call(&["status"])["enabled"], false);
    assert!(!f.commands.exists());
    let installed = f.install();
    assert_eq!(installed["installed"], true);
    assert_eq!(f.install()["installed"], true);
    assert_eq!(f.call(&["status"])["installed"], true);
    let exported = Path::new(installed["executable"].as_str().unwrap());
    let output = Command::new(exported)
        .args(["capabilities", "--json"])
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let capabilities: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(capabilities["delivery_protocols"], json!(["delivery-v1"]));
    assert_eq!(f.call(&["uninstall"])["enabled"], false);
    assert!(!exported.exists());
    assert!(f.root.join("bin").join(file_name()).exists());
    assert_eq!(f.call(&["status"])["installed"], false);
    assert!(!f.root.join("bin/calls.log").exists());
    assert!(!f.commands.join("calls.log").exists());
}

#[test]
fn terminal_status_does_not_create_a_missing_runtime_root() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap().join("missing");
    let output = Command::new(env!("CARGO_BIN_EXE_antiburn"))
        .args(["terminal", "status", "--runtime-root"])
        .arg(&root)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let state: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(state["enabled"], false);
    assert!(!root.exists());
}

#[test]
fn setup_preserves_an_existing_desktop_or_user_command() {
    let f = Fixture::new();
    fs::create_dir(&f.commands).unwrap();
    let path = f.commands.join(if cfg!(windows) {
        "antiburn.exe"
    } else {
        "antiburn"
    });
    fs::write(&path, b"existing desktop launcher").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_antiburn"))
        .args(["terminal", "install", "--runtime-root"])
        .arg(&f.root)
        .arg("--directory")
        .arg(&f.commands)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(fs::read(path).unwrap(), b"existing desktop launcher");
    assert!(!String::from_utf8_lossy(&output.stderr).contains("file tracing unavailable"));
}

#[cfg(unix)]
#[test]
fn a_fresh_shell_resolves_the_export_and_keeps_a_shadowing_command() {
    let f = Fixture::new();
    f.install();
    let output = Command::new("/bin/sh")
        .args(["-c", "command -v antiburn; antiburn capabilities --json"])
        .env("PATH", &f.commands)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let (resolved, json) = stdout.split_once('\n').unwrap();
    assert_eq!(Path::new(resolved), f.commands.join("antiburn"));
    let capabilities: Value = serde_json::from_str(json).unwrap();
    assert_eq!(capabilities["delivery_protocols"], json!(["delivery-v1"]));

    let shadow = f.commands.parent().unwrap().join("earlier");
    fs::create_dir(&shadow).unwrap();
    fs::write(
        shadow.join("antiburn"),
        b"#!/bin/sh\nprintf desktop-launcher",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(shadow.join("antiburn"), fs::Permissions::from_mode(0o700)).unwrap();
    let output = Command::new("/bin/sh")
        .args(["-c", "antiburn"])
        .env(
            "PATH",
            std::env::join_paths([&shadow, &f.commands]).unwrap(),
        )
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"desktop-launcher");
    assert_eq!(f.call(&["status"])["installed"], true);
}
