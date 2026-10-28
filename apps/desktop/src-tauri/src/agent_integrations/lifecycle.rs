use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{Value, json};

use super::artifact::Artifact;
use super::discovery::{self, Profile, Runtime};
use super::document;
use super::process::{PROBE_TIMEOUT, Runner};
use super::transaction::{self, Receipt, Result};
use super::{IntegrationPhase, IntegrationStatus, Vendor};

#[derive(Debug, Clone)]
pub struct CliConnection {
    pub executable: PathBuf,
    pub version: String,
    pub sha256: String,
    pub delivery_protocols: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Route {
    pub vendor: Vendor,
    pub version: String,
    pub os: String,
    pub arch: String,
    pub surface: String,
    pub route: String,
    pub native_validated: bool,
    pub reason: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Compatibility {
    schema: u32,
    routes: Vec<Route>,
}

pub fn accepted_route(vendor: Vendor, version: &str) -> Option<Route> {
    let manifest: Compatibility = serde_json::from_str(include_str!("compatibility.json")).ok()?;
    if manifest.schema != 1 {
        return None;
    }
    manifest.routes.into_iter().find(|route| {
        route.vendor == vendor
            && route.version == version
            && route.os == std::env::consts::OS
            && route.arch == std::env::consts::ARCH
            && route.surface == "cli"
            && route.native_validated
    })
}

pub struct Installation<'a, R> {
    pub state: &'a Path,
    pub resources: &'a Path,
    pub home: &'a Path,
    pub env: &'a BTreeMap<String, String>,
    pub runner: &'a R,
}

fn slug(vendor: Vendor) -> &'static str {
    match vendor {
        Vendor::Claude => "claude",
        Vendor::Codex => "codex",
        Vendor::Cursor => "cursor",
        Vendor::OpenCode => "opencode",
        Vendor::Pi => "pi",
    }
}

fn posix(value: &Path) -> Result<String> {
    let value = value.to_str().ok_or("unsupported_path_encoding")?;
    if !value.starts_with('/') || value.chars().any(char::is_control) || value.len() > 8192 {
        return Err("unsafe_path");
    }
    Ok(format!("'{}'", value.replace('\'', "'\"'\"'")))
}

pub fn hook_command(node: &Path, hook: &Path, cli: &Path) -> Result<String> {
    let node = posix(node)?;
    let cli = posix(cli)?;
    Ok(format!(
        "if [ -x {node} ] && [ -x {cli} ]; then ANTIBURN_EXECUTABLE={cli} {node} {}; fi",
        posix(hook)?
    ))
}

fn host_version(vendor: Vendor, output: &str) -> Option<String> {
    let text = output.trim();
    let version = match vendor {
        Vendor::Claude => text.strip_suffix(" (Claude Code)"),
        Vendor::Codex => text.strip_prefix("codex-cli "),
        _ => Some(text),
    }?;
    if version.is_empty()
        || version.len() > 96
        || !version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
    {
        return None;
    }
    Some(version.into())
}

impl<R: Runner> Installation<'_, R> {
    pub fn reconcile(&self, cli: Option<&CliConnection>) -> Vec<IntegrationStatus> {
        self.statuses(cli, true)
    }

    pub fn inspect(&self, cli: Option<&CliConnection>) -> Vec<IntegrationStatus> {
        self.statuses(cli, false)
    }

    fn statuses(&self, cli: Option<&CliConnection>, setup: bool) -> Vec<IntegrationStatus> {
        if cli.is_none() {
            return Vendor::ALL
                .into_iter()
                .map(|vendor| {
                    let mut result = status(
                        vendor,
                        IntegrationPhase::Unavailable,
                        "managed_cli_unavailable",
                    );
                    result.owned = transaction::load_receipt(&self.receipt_path(vendor))
                        .ok()
                        .flatten()
                        .is_some_and(|receipt| !receipt.removed);
                    result
                })
                .collect();
        }
        let _lock = match setup
            .then(|| transaction::Lock::acquire(self.state))
            .transpose()
        {
            Ok(lock) => lock,
            Err(reason) => {
                return Vendor::ALL
                    .into_iter()
                    .map(|vendor| status(vendor, IntegrationPhase::Unavailable, reason))
                    .collect();
            }
        };
        let directories = discovery::search_directories(self.home, self.env);
        let mut node = None;
        let mut node_searched = false;
        Vendor::ALL
            .into_iter()
            .map(|vendor| {
                let result = self.reconcile_vendor(
                    vendor,
                    cli,
                    &directories,
                    &mut node,
                    &mut node_searched,
                    setup,
                );
                let mut result = result
                    .unwrap_or_else(|reason| status(vendor, IntegrationPhase::Unavailable, reason));
                result.owned = transaction::load_receipt(&self.receipt_path(vendor))
                    .ok()
                    .flatten()
                    .is_some_and(|receipt| !receipt.removed);
                result
            })
            .collect()
    }

    fn reconcile_vendor(
        &self,
        vendor: Vendor,
        cli: Option<&CliConnection>,
        directories: &[PathBuf],
        node: &mut Option<Runtime>,
        node_searched: &mut bool,
        setup: bool,
    ) -> Result<IntegrationStatus> {
        let saved_path = self.state.join(format!("{}-profile.json", slug(vendor)));
        let saved = transaction::read(&saved_path)?
            .map(|bytes| {
                serde_json::from_slice::<Profile>(&bytes).map_err(|_| "invalid_saved_profile")
            })
            .transpose()?;
        let profile = discovery::profile(vendor, self.home, self.env, saved.as_ref())?;
        if !profile.root.exists() {
            return Ok(status(
                vendor,
                IntegrationPhase::NotDetected,
                "profile_not_detected",
            ));
        }
        transaction::checked_directory(&profile.root)?;
        if setup && saved.is_none() {
            let bytes = serde_json::to_vec(&profile).map_err(|_| "invalid_saved_profile")?;
            transaction::publish(&saved_path, None, &bytes)?;
        }
        let Some(host) = discovery::find_host(vendor, directories) else {
            return Ok(with_profile(
                status(
                    vendor,
                    IntegrationPhase::Unavailable,
                    "host_runtime_missing",
                ),
                &profile,
            ));
        };
        let env = self.host_environment(&profile);
        let output = self
            .runner
            .run(&host, &["--version".into()], &env, PROBE_TIMEOUT)
            .map_err(|_| "host_version_unavailable")?;
        let version = host_version(vendor, &output).ok_or("host_version_unavailable")?;
        let mut result = with_profile(
            status(
                vendor,
                IntegrationPhase::PendingNativeGate,
                "native_route_not_validated",
            ),
            &profile,
        );
        result.host_version = Some(version.clone());
        let Some(_route) = accepted_route(vendor, &version) else {
            return Ok(result);
        };
        let Some(cli) = cli else {
            result.phase = IntegrationPhase::Unavailable;
            result.reason = "managed_cli_unavailable".into();
            return Ok(result);
        };
        if !cli.executable.is_absolute()
            || !cli.delivery_protocols.iter().any(|p| p == "delivery-v1")
        {
            return Err("managed_cli_incompatible");
        }
        if vendor.needs_node() && !*node_searched {
            *node = discovery::find_node(directories, self.runner);
            *node_searched = true;
        }
        if vendor.needs_node() && node.is_none() {
            result.phase = IntegrationPhase::Unavailable;
            result.reason = "compatible_node_22_required".into();
            return Ok(result);
        }
        result.node_version = node.as_ref().map(|node| node.version.clone());
        if !setup {
            result.phase = self.inspect_profile(&profile)?;
            result.reason = match result.phase {
                IntegrationPhase::Disabled => "disabled_in_vendor_or_antiburn",
                IntegrationPhase::UserChanged => "owned_entry_changed_or_removed",
                IntegrationPhase::Unavailable => "setup_not_registered",
                _ => "registered_delivery_unverified",
            }
            .into();
            return Ok(result);
        }
        let artifact = Artifact::read(self.resources)?;
        let artifacts = self.state.join("artifacts");
        transaction::private_directory(&artifacts)?;
        let root = artifact.stage(&artifacts, &json!({"schema":1,"executable":cli.executable,"version":cli.version,"sha256":cli.sha256,"deliveryProtocols":cli.delivery_protocols}))?;
        let binding = format!(
            "{}:{}",
            root.display(),
            node.as_ref()
                .map(|node| node.executable.to_string_lossy())
                .unwrap_or_default()
        );
        let phase = if vendor == Vendor::Pi {
            self.reconcile_pi(&profile, &host, &root, &artifact.hash)?
        } else {
            self.reconcile_config(&profile, node.as_ref(), &root, &binding, &cli.executable)?
        };
        result.phase = phase;
        result.reason = match result.phase {
            IntegrationPhase::AwaitingTrust => "registered_review_native_hook_trust",
            IntegrationPhase::ReloadRequired => "registered_restart_or_reload_host",
            IntegrationPhase::UserChanged => "owned_entry_changed_or_removed",
            IntegrationPhase::Disabled => "disabled_in_vendor_or_antiburn",
            _ => "registered",
        }
        .into();
        Ok(result)
    }

    pub(super) fn inspect_profile(&self, profile: &Profile) -> Result<IntegrationPhase> {
        if profile.vendor == Vendor::Pi {
            return super::manager::inspect(profile, &self.receipt_path(profile.vendor));
        }
        let target = self.config_path(profile)?;
        let current = transaction::read(&target)?;
        if let Some(bytes) = current.as_deref()
            && profile.vendor == Vendor::Claude
            && document::boolean(bytes, &["disableAllHooks".into()])? == Some(true)
        {
            return Ok(IntegrationPhase::Disabled);
        }
        let Some(receipt) = transaction::load_receipt(&self.receipt_path(profile.vendor))? else {
            return Ok(IntegrationPhase::Unavailable);
        };
        if receipt.target != target || receipt.manager_source.is_some() {
            return Err("receipt_target_mismatch");
        }
        if receipt.removed {
            return Ok(IntegrationPhase::Disabled);
        }
        if !receipt.published || receipt.previous.is_some() {
            return Err("interrupted_installation_requires_repair");
        }
        if current.as_deref() == receipt.original.as_deref()
            || current
                .as_deref()
                .is_none_or(|bytes| receipt.rollback(bytes).is_err())
        {
            return Ok(IntegrationPhase::UserChanged);
        }
        Ok(registered_phase(profile.vendor))
    }

    fn host_environment(&self, profile: &Profile) -> BTreeMap<String, String> {
        let mut env = BTreeMap::new();
        for name in [
            "PATH",
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_CACHE_HOME",
            "XDG_STATE_HOME",
        ] {
            if let Some(value) = self.env.get(name) {
                env.insert(name.into(), value.clone());
            }
        }
        env.insert("HOME".into(), self.home.to_string_lossy().into_owned());
        env.insert("PI_OFFLINE".into(), "1".into());
        env.insert("PI_TELEMETRY".into(), "0".into());
        env.insert("NO_COLOR".into(), "1".into());
        env.insert(
            match profile.vendor {
                Vendor::Claude => "CLAUDE_CONFIG_DIR",
                Vendor::Codex => "CODEX_HOME",
                Vendor::Pi => "PI_CODING_AGENT_DIR",
                Vendor::OpenCode => "OPENCODE_CONFIG_DIR",
                Vendor::Cursor => "CURSOR_CONFIG_DIR",
            }
            .into(),
            profile.root.to_string_lossy().into_owned(),
        );
        env
    }

    fn receipt_path(&self, vendor: Vendor) -> PathBuf {
        self.state.join(format!("{}-receipt.json", slug(vendor)))
    }

    pub fn remove(&self, vendor: Vendor) -> Result<()> {
        let _lock = transaction::Lock::acquire(self.state)?;
        let saved = transaction::read(&self.state.join(format!("{}-profile.json", slug(vendor))))?
            .ok_or("profile_not_detected")?;
        let profile: Profile =
            serde_json::from_slice(&saved).map_err(|_| "invalid_saved_profile")?;
        if profile.vendor != vendor {
            return Err("invalid_saved_profile");
        }
        let path = self.receipt_path(vendor);
        let Some(mut receipt) = transaction::load_receipt(&path)? else {
            return Ok(());
        };
        if receipt.removed {
            return Ok(());
        }
        if vendor == Vendor::Pi {
            let directories = discovery::search_directories(self.home, self.env);
            let host = discovery::find_host(vendor, &directories).ok_or("host_runtime_missing")?;
            return super::manager::remove(
                &profile,
                &host,
                &mut receipt,
                &path,
                &self.host_environment(&profile),
                self.runner,
            );
        }
        if receipt.target != self.config_path(&profile)? || receipt.manager_source.is_some() {
            return Err("receipt_target_mismatch");
        }
        let current = transaction::read(&receipt.target)?;
        receipt = recover_replacement(&path, &receipt.target.clone(), receipt, current.as_deref())?;
        if current != receipt.original
            && let Some(current) = current
        {
            let next = receipt.rollback(&current)?;
            transaction::publish(&receipt.target, Some(&current), &next)?;
        }
        receipt.removed = true;
        transaction::save_receipt(&path, &receipt)
    }

    pub fn repair(&self, vendor: Vendor) -> Result<()> {
        let _lock = transaction::Lock::acquire(self.state)?;
        let path = self.receipt_path(vendor);
        let Some(receipt) = transaction::load_receipt(&path)? else {
            return Ok(());
        };
        let saved = transaction::read(&self.state.join(format!("{}-profile.json", slug(vendor))))?
            .ok_or("profile_not_detected")?;
        let profile: Profile =
            serde_json::from_slice(&saved).map_err(|_| "invalid_saved_profile")?;
        let directories = discovery::search_directories(self.home, self.env);
        let host = discovery::find_host(vendor, &directories).ok_or("host_runtime_missing")?;
        let output = self
            .runner
            .run(
                &host,
                &["--version".into()],
                &self.host_environment(&profile),
                PROBE_TIMEOUT,
            )
            .map_err(|_| "host_version_unavailable")?;
        let version = host_version(vendor, &output).ok_or("host_version_unavailable")?;
        if accepted_route(vendor, &version).is_none() {
            return Err("native_route_not_validated");
        }
        let expected = if vendor == Vendor::Pi {
            profile.root.join("settings.json")
        } else {
            self.config_path(&profile)?
        };
        if profile.vendor != vendor || receipt.target != expected {
            return Err("receipt_target_mismatch");
        }
        let current = transaction::read(&receipt.target)?;
        let receipt = if vendor == Vendor::Pi {
            receipt
        } else {
            recover_replacement(&path, &expected, receipt, current.as_deref())?
        };
        let unpublished_unchanged = !receipt.published && current == receipt.original;
        if vendor == Vendor::Pi && !receipt.removed && !unpublished_unchanged {
            if receipt.published && current.as_deref() == Some(&receipt.installed) {
                return Ok(());
            }
            return Err("native_package_repair_requires_review");
        }
        if !receipt.removed && !unpublished_unchanged {
            if current
                .as_deref()
                .is_some_and(|bytes| receipt.rollback(bytes).is_ok())
            {
                return Ok(());
            }
            self.refuse_duplicate(&profile, current.as_deref().unwrap_or(b"{}"))?;
        }
        transaction::save_receipt(&path.with_extension("previous.json"), &receipt)?;
        std::fs::remove_file(path).map_err(|_| "state_unavailable")?;
        Ok(())
    }

    fn config_path(&self, profile: &Profile) -> Result<PathBuf> {
        Ok(profile.root.join(match profile.vendor {
            Vendor::Claude => "settings.json",
            Vendor::Codex | Vendor::Cursor => "hooks.json",
            Vendor::OpenCode => {
                if self.env.contains_key("OPENCODE_CONFIG")
                    || self.env.contains_key("OPENCODE_CONFIG_CONTENT")
                {
                    return Err("custom_opencode_config_requires_review");
                }
                let json = profile.root.join("opencode.json").exists();
                let jsonc = profile.root.join("opencode.jsonc").exists();
                if json && jsonc {
                    return Err("ambiguous_opencode_config");
                }
                if jsonc {
                    "opencode.jsonc"
                } else {
                    "opencode.json"
                }
            }
            Vendor::Pi => return Err("native_package_manager_required"),
        }))
    }

    pub(super) fn reconcile_config(
        &self,
        profile: &Profile,
        node: Option<&Runtime>,
        root: &Path,
        artifact: &str,
        cli: &Path,
    ) -> Result<IntegrationPhase> {
        let target = self.config_path(profile)?;
        let receipt_path = self.receipt_path(profile.vendor);
        let current = transaction::read(&target)?;
        let mut prior = transaction::load_receipt(&receipt_path)?
            .map(|receipt| recover_replacement(&receipt_path, &target, receipt, current.as_deref()))
            .transpose()?;
        if let Some(bytes) = current.as_deref()
            && profile.vendor == Vendor::Claude
            && document::boolean(bytes, &["disableAllHooks".into()])? == Some(true)
        {
            return Ok(IntegrationPhase::Disabled);
        }
        if let Some(prior) = prior.as_mut()
            && !prior.published
            && current.as_deref() == Some(&prior.installed)
        {
            prior.published = true;
            transaction::save_receipt(&receipt_path, prior)?;
        }
        let mut baseline = current.clone().unwrap_or_else(|| b"{}".to_vec());
        if let Some(prior) = &prior {
            if prior.target != target {
                return Err("receipt_target_mismatch");
            }
            if prior.removed {
                return Ok(IntegrationPhase::Disabled);
            }
            if current.as_deref() == prior.original.as_deref() {
                // An interrupted write and a vendor-side removal can have the same bytes.
                if !prior.published {
                    return Err("interrupted_installation_requires_repair");
                }
                return Ok(IntegrationPhase::UserChanged);
            }
            let Some(current) = current.as_deref() else {
                return Ok(IntegrationPhase::UserChanged);
            };
            baseline = match prior.rollback(current) {
                Ok(bytes) => bytes,
                Err(_) => return Ok(IntegrationPhase::UserChanged),
            };
            if prior.artifact == artifact {
                return Ok(registered_phase(profile.vendor));
            }
        }
        let bytes = baseline.as_slice();
        document::validate(bytes)?;
        self.refuse_duplicate(profile, bytes)?;
        let (installed, deltas) = render_config(profile.vendor, bytes, node, root, cli)?;
        if current.as_deref() == Some(&installed)
            && let Some(mut active) = prior.take()
        {
            active.artifact = artifact.into();
            transaction::save_receipt(&receipt_path, &active)?;
            return Ok(registered_phase(profile.vendor));
        }
        let mut receipt = Receipt::prepare(
            target.clone(),
            current.clone(),
            installed,
            deltas,
            artifact.into(),
        )?;
        receipt.previous = prior.map(Box::new);
        transaction::save_receipt(&receipt_path, &receipt)?;
        transaction::publish(&target, current.as_deref(), &receipt.installed)?;
        receipt.published = true;
        receipt.previous = None;
        transaction::save_receipt(&receipt_path, &receipt)?;
        Ok(registered_phase(profile.vendor))
    }

    fn refuse_duplicate(&self, profile: &Profile, bytes: &[u8]) -> Result<()> {
        let text = std::str::from_utf8(bytes).map_err(|_| "malformed_jsonc")?;
        if text.contains("antiburn-session-checks")
            || text.contains("session-check-hook.mjs")
            || text.contains("antiburn-cli")
            || text.contains("antiburn checks")
            || (profile.vendor == Vendor::OpenCode && text.contains("/antiburn.mjs"))
        {
            return Err("existing_integration_requires_review");
        }
        if profile.vendor == Vendor::Claude {
            if document::boolean(bytes, &["disableAllHooks".into()])? == Some(true)
                || document::boolean(bytes, &["allowManagedHooksOnly".into()])? == Some(true)
            {
                return Err("hook_policy_requires_review");
            }
            let plugins = profile.root.join("plugins");
            if plugins.exists()
                && let Some(bytes) = transaction::read(&plugins.join("installed_plugins.json"))?
                && String::from_utf8_lossy(&bytes).contains("antiburn-session-checks")
            {
                return Err("existing_companion_requires_review");
            }
        }
        Ok(())
    }

    fn reconcile_pi(
        &self,
        profile: &Profile,
        host: &Path,
        root: &Path,
        artifact: &str,
    ) -> Result<IntegrationPhase> {
        super::manager::reconcile(
            profile,
            host,
            root,
            &self.receipt_path(profile.vendor),
            artifact,
            &self.host_environment(profile),
            self.runner,
        )
    }
}

fn recover_replacement(
    path: &Path,
    target: &Path,
    mut receipt: Receipt,
    current: Option<&[u8]>,
) -> Result<Receipt> {
    if receipt.target != target || receipt.manager_source.is_some() {
        return Err("receipt_target_mismatch");
    }
    if receipt.published || receipt.previous.is_none() {
        return Ok(receipt);
    }
    let previous = receipt.previous.take().ok_or("state_invalid")?;
    if previous.target != target
        || !previous.published
        || previous.removed
        || previous.manager_source.is_some()
        || previous.previous.is_some()
    {
        return Err("state_invalid");
    }
    if current == Some(receipt.installed.as_slice())
        || current.is_some_and(|bytes| receipt.rollback(bytes).is_ok())
    {
        receipt.published = true;
        transaction::save_receipt(path, &receipt)?;
        return Ok(receipt);
    }
    if current == receipt.original.as_deref()
        || current == previous.original.as_deref()
        || current.is_some_and(|bytes| previous.rollback(bytes).is_ok())
    {
        transaction::save_receipt(path, &previous)?;
        return Ok(*previous);
    }
    Err("interrupted_upgrade_requires_review")
}

pub(super) fn status(vendor: Vendor, phase: IntegrationPhase, reason: &str) -> IntegrationStatus {
    IntegrationStatus {
        vendor,
        phase,
        reason: reason.into(),
        host_version: None,
        profile: None,
        node_version: None,
        owned: false,
    }
}

fn with_profile(mut status: IntegrationStatus, profile: &Profile) -> IntegrationStatus {
    status.profile = Some(profile.root.clone());
    status
}

fn registered_phase(vendor: Vendor) -> IntegrationPhase {
    if matches!(vendor, Vendor::Claude | Vendor::Codex) {
        IntegrationPhase::AwaitingTrust
    } else {
        IntegrationPhase::ReloadRequired
    }
}

pub fn render_config(
    vendor: Vendor,
    original: &[u8],
    node: Option<&Runtime>,
    root: &Path,
    cli: &Path,
) -> Result<(Vec<u8>, Vec<document::Delta>)> {
    let mut bytes = original.to_vec();
    let mut deltas = Vec::new();
    if vendor == Vendor::Cursor {
        match document::value(&bytes, &["version".into()])? {
            None => {
                let (next, delta) = document::insert_member(&bytes, "version", json!(1))?;
                bytes = next;
                deltas.push(delta);
            }
            Some(version) if version == json!(1) => (),
            _ => return Err("cursor_schema_version_requires_review"),
        }
    }
    let entries: Vec<(&[&str], Value)> = match vendor {
        Vendor::Claude | Vendor::Codex => {
            let node = node.ok_or("compatible_node_22_required")?;
            let command = hook_command(
                &node.executable,
                &root.join(slug(vendor)).join("hooks/session-check-hook.mjs"),
                cli,
            )?;
            let hook = json!({"hooks":[{"type":"command","command":command,"timeout":5}]});
            if vendor == Vendor::Claude {
                vec![
                    (&["hooks", "UserPromptSubmit"], hook.clone()),
                    (&["hooks", "PostToolUse"], hook),
                ]
            } else {
                vec![(&["hooks", "UserPromptSubmit"], hook)]
            }
        }
        Vendor::Cursor => {
            let node = node.ok_or("compatible_node_22_required")?;
            let command = hook_command(
                &node.executable,
                &root.join("cursor/hooks/session-check-hook.mjs"),
                cli,
            )?;
            // Cursor's version is part of its native schema, never inferred from a project hook.
            vec![(&["hooks", "postToolUse"], json!({"command":command}))]
        }
        Vendor::OpenCode => {
            let url = url::Url::from_file_path(root.join("opencode/runtime/antiburn.mjs"))
                .map_err(|_| "unsafe_path")?;
            vec![(&["plugin"], Value::String(url.to_string()))]
        }
        Vendor::Pi => return Err("native_package_manager_required"),
    };
    for (path, value) in entries {
        let path = path.iter().map(|key| (*key).to_owned()).collect::<Vec<_>>();
        let (next, delta) = document::append(&bytes, &path, value)?;
        bytes = next;
        deltas.push(delta);
    }
    Ok((bytes, deltas))
}
