//! Native TypeSafe credentials and authorized window controls.

use antiburn_local::checks::DetectorId;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

use crate::dto::BurnCheckDetectorId;
use crate::insights_report::installed_checks::{
    InstalledCheckAvailabilityPayload, InstalledCheckDefinition, availability_rows,
};
use crate::jev::config::{
    CredentialReference, SystemOneConnection, SystemOneEndpoint, SystemOneProvider,
};
use crate::jev::worker::WorkerHandle;
use crate::store::{BurnCheckUsageSummary, Store};

const YAML_DISCOVERY_ENABLED_KEY: &str = "internal:yamlBurnChecksDiscoveryEnabledV1";
const SAVED_KEY_KEY: &str = "internal:typesafeKeySavedV1";
const AUTH_REJECTED_KEY: &str = "internal:typesafeAuthRejectedV1";
const CREDENTIAL_CHANGE_PENDING_KEY: &str = "internal:typesafeCredentialChangePendingV1";
const ACTIVE_CONNECTION_KEY: &str = "internal:smartChecksActiveConnectionV1";
const CONNECTION_PROFILES_KEY: &str = "internal:smartChecksConnectionsV1";
const PROVIDER_MIGRATION_KEY: &str = "internal:smartChecksProviderMigrationV1";
#[cfg(debug_assertions)]
const SERVICE: &str = "ai.antiburn.desktop.debug.typesafe";
#[cfg(not(debug_assertions))]
const SERVICE: &str = "ai.antiburn.desktop.typesafe";
const ACCOUNT: &str = "ignored-instructions";
const CONNECTION_PENDING_KEY: &str = "internal:smartChecksConnectionChangePendingV1";
const CONNECTION_REMOVAL_KEY: &str = "internal:smartChecksCredentialRemovalPendingV1";
pub(crate) const AVAILABILITY_EVENT: &str = "checks:availability-changed";
static CREDENTIAL_CHANGE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static CHECK_CHANGE: std::sync::Mutex<()> = std::sync::Mutex::new(());
static STARTUP_ERROR: std::sync::Mutex<Option<&'static str>> = std::sync::Mutex::new(None);

pub(crate) fn yaml_discovery_enabled(store: &Store) -> bool {
    match store
        .lock()
        .query_row(
            "SELECT value FROM setting WHERE key = ?1",
            [YAML_DISCOVERY_ENABLED_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()
    {
        Ok(None) => true,
        Ok(Some(value)) => value == "true",
        Err(_) => false,
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CheckAvailability {
    yaml_enabled: bool,
    configured: bool,
    saved_key: bool,
    error: Option<&'static str>,
    usage: BurnCheckUsageSummary,
    checks: Vec<CheckChoice>,
    installed_checks: Vec<InstalledCheckAvailabilityPayload>,
    revision: u64,
}

impl CheckAvailability {
    pub(crate) fn with_installed_checks(
        mut self,
        definitions: &[InstalledCheckDefinition],
    ) -> Self {
        self.installed_checks = availability_rows(definitions);
        self
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CheckChoice {
    id: BurnCheckDetectorId,
    enabled: bool,
}

#[derive(Clone, Serialize)]
#[serde(tag = "status", content = "snapshot", rename_all = "snake_case")]
enum CheckAvailabilityEvent {
    Updated(Box<CheckAvailability>),
    Failed,
}

fn entry() -> Result<keyring::Entry, &'static str> {
    keyring::Entry::new(SERVICE, ACCOUNT).map_err(|_| "Credential storage is unavailable.")
}

trait CredentialVault {
    type Error;

    fn read(&self, id: Option<&str>) -> Result<Option<String>, Self::Error>;
    fn write(&self, id: Option<&str>, value: &str) -> Result<(), Self::Error>;
    fn delete(&self, id: Option<&str>) -> Result<(), Self::Error>;
    fn is_missing(error: &Self::Error) -> bool;
    fn write_failure(error: &Self::Error) -> &'static str;
}

struct SystemCredentialVault;

impl CredentialVault for SystemCredentialVault {
    type Error = keyring::Error;

    fn read(&self, id: Option<&str>) -> Result<Option<String>, Self::Error> {
        let entry = match id {
            Some(id) => connection_entry(id).map_err(vault_entry_error)?,
            None => entry().map_err(vault_entry_error)?,
        };
        match entry.get_password() {
            Ok(value) if !value.trim().is_empty() => Ok(Some(value)),
            Ok(_) | Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn write(&self, id: Option<&str>, value: &str) -> Result<(), Self::Error> {
        let entry = match id {
            Some(id) => connection_entry(id).map_err(vault_entry_error)?,
            None => entry().map_err(vault_entry_error)?,
        };
        entry.set_password(value)
    }

    fn delete(&self, id: Option<&str>) -> Result<(), Self::Error> {
        let entry = match id {
            Some(id) => connection_entry(id).map_err(vault_entry_error)?,
            None => entry().map_err(vault_entry_error)?,
        };
        entry.delete_credential()
    }

    fn is_missing(error: &Self::Error) -> bool {
        matches!(error, keyring::Error::NoEntry)
    }

    fn write_failure(error: &Self::Error) -> &'static str {
        match error {
            keyring::Error::NoStorageAccess(_) => {
                "Could not access secure storage. Unlock the system credential store and retry."
            }
            keyring::Error::PlatformFailure(_) => {
                "Secure storage rejected the credential write. Check app access and retry."
            }
            _ => {
                "Could not save the API token to secure storage. Check credential storage access and retry."
            }
        }
    }
}

fn vault_entry_error(message: &'static str) -> keyring::Error {
    keyring::Error::PlatformFailure(Box::new(std::io::Error::other(message)))
}

fn write_credential<V: CredentialVault>(
    vault: &V,
    id: Option<&str>,
    value: &str,
) -> Result<(), &'static str> {
    vault
        .write(id, value)
        .map_err(|error| V::write_failure(&error))
}

fn delete_credential<V: CredentialVault>(vault: &V, id: Option<&str>) -> Result<(), &'static str> {
    vault
        .delete(id)
        .or_else(|error| {
            if V::is_missing(&error) {
                Ok(())
            } else {
                Err(error)
            }
        })
        .map_err(|_| "Could not remove the connection credential.")
}

fn read_key() -> Result<Option<String>, &'static str> {
    SystemCredentialVault
        .read(None)
        .map_err(|_| "Credential storage is unavailable.")
}

fn saved_key_marker(store: &Store) -> bool {
    store.internal_value(SAVED_KEY_KEY).as_deref() == Some("true")
        || store
            .internal_value(CREDENTIAL_CHANGE_PENDING_KEY)
            .as_deref()
            == Some("true")
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ConnectionProfiles {
    active_id: String,
    profiles: std::collections::BTreeMap<String, SystemOneConnection>,
}

#[derive(Clone, Deserialize)]
pub(crate) struct SystemOneDraft {
    pub connection_id: String,
    pub connection: SystemOneConnection,
    pub credential: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SystemOneSettings {
    active_id: String,
    profiles: std::collections::BTreeMap<String, SystemOneConnection>,
}

fn connection_entry(id: &str) -> Result<keyring::Entry, &'static str> {
    validate_connection_id(id)?;
    keyring::Entry::new(SERVICE, &format!("system-one-{id}"))
        .map_err(|_| "Credential storage is unavailable.")
}

fn validate_connection_id(id: &str) -> Result<(), &'static str> {
    if id.is_empty()
        || id.len() > 96
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err("The connection credential reference is invalid.");
    }
    Ok(())
}

fn read_connection_credential(
    connection: &SystemOneConnection,
) -> Result<Option<String>, &'static str> {
    resolve_credential(connection, None, &SystemCredentialVault)
}

fn resolve_credential(
    connection: &SystemOneConnection,
    draft: Option<String>,
    vault: &impl CredentialVault,
) -> Result<Option<String>, &'static str> {
    connection
        .validate()
        .map_err(|_| "Connection settings are invalid.")?;
    let credential = if draft.is_some() {
        draft
    } else {
        match connection.credential.as_ref() {
            Some(CredentialReference::LegacyTypeSafe) => vault.read(None),
            Some(CredentialReference::Connection(id)) => vault.read(Some(id)),
            None => return Ok(None),
        }
        .map_err(|_| "Credential storage is unavailable.")?
    };
    if credential.as_ref().is_some_and(|secret| {
        secret.trim().is_empty() || secret.len() > 4096 || secret.chars().any(char::is_control)
    }) {
        return Err("Enter a valid connection credential.");
    }
    credential
        .map(Some)
        .ok_or("The selected provider needs a credential. Add one and retry.")
}

fn ensure_no_pending_removal(store: &Store) -> Result<(), &'static str> {
    if store
        .internal_value(CONNECTION_REMOVAL_KEY)
        .is_some_and(|id| !id.is_empty())
    {
        return Err("Connection credential removal is incomplete. Retry it in Settings → Checks.");
    }
    Ok(())
}

fn ensure_no_pending_credential_change(store: &Store) -> Result<(), &'static str> {
    ensure_no_pending_removal(store)?;
    if legacy_removal_pending(store) {
        return Err("TypeSafe credential removal is incomplete. Retry it in Settings → Checks.");
    }
    if store
        .internal_value(CONNECTION_PENDING_KEY)
        .is_some_and(|pending| !pending.is_empty())
    {
        return Err(
            "Smart Burn Checks connection update is incomplete. Retry it in Settings → Checks.",
        );
    }
    Ok(())
}

fn legacy_removal_pending(store: &Store) -> bool {
    store
        .internal_value(CREDENTIAL_CHANGE_PENDING_KEY)
        .as_deref()
        == Some("true")
}

fn clear_legacy_removal<V: CredentialVault>(store: &Store, vault: &V) -> Result<(), &'static str> {
    let mut saved = profiles(store)?;
    advance_legacy_connection_revisions(&mut saved)?;
    finish_legacy_removal(store, vault, &saved)
}

fn begin_legacy_removal(
    store: &Store,
    worker: &WorkerHandle,
) -> Result<ConnectionProfiles, &'static str> {
    let mut saved = profiles(store)?;
    advance_legacy_connection_revisions(&mut saved)?;
    let active = saved
        .profiles
        .get(&saved.active_id)
        .ok_or("The active Smart Burn Checks connection is missing.")?;
    let active_uses_legacy_key =
        matches!(active.credential, Some(CredentialReference::LegacyTypeSafe));
    store
        .set_internal_value_checked(CREDENTIAL_CHANGE_PENDING_KEY, "true")
        .map_err(|_| "Could not protect the credential removal state.")?;
    if active_uses_legacy_key {
        pause_model_checks(store, worker)?;
    }
    Ok(saved)
}

fn finish_legacy_removal<V: CredentialVault>(
    store: &Store,
    vault: &V,
    saved: &ConnectionProfiles,
) -> Result<(), &'static str> {
    let encoded =
        serde_json::to_string(saved).map_err(|_| "Could not save the credential removal state.")?;
    let previous = vault
        .read(None)
        .map_err(|_| "Credential storage is unavailable.")?;
    delete_credential(vault, None)?;
    if store
        .set_internal_values_checked(&[
            (CONNECTION_PROFILES_KEY, &encoded),
            (SAVED_KEY_KEY, "false"),
            (CREDENTIAL_CHANGE_PENDING_KEY, "false"),
        ])
        .is_err()
    {
        if let Some(secret) = previous
            && write_credential(vault, None, &secret).is_err()
        {
            return Err(
                "Could not restore the API key. Retry credential removal in Settings → Checks.",
            );
        }
        return Err("Could not save the credential removal state.");
    }
    Ok(())
}

fn profiles(store: &Store) -> Result<ConnectionProfiles, &'static str> {
    migrate_provider_state(store)?;
    store
        .internal_value(CONNECTION_PROFILES_KEY)
        .and_then(|json| serde_json::from_str(&json).ok())
        .ok_or("Saved Smart Burn Checks provider settings are invalid.")
}

fn save_profiles(store: &Store, value: &ConnectionProfiles) -> Result<(), &'static str> {
    let encoded =
        serde_json::to_string(value).map_err(|_| "Could not save connection settings.")?;
    store
        .set_internal_values_checked(&[
            (CONNECTION_PROFILES_KEY, &encoded),
            (ACTIVE_CONNECTION_KEY, &value.active_id),
        ])
        .map_err(|_| "Could not save connection settings.")
}

fn suspend_for_connection_change(store: &Store, worker: &WorkerHandle) -> Result<(), &'static str> {
    worker.suspend_system_one();
    store
        .cancel_unfinished_burn_check_assessments_for_connection_change()
        .map_err(|_| {
            "Could not cancel unfinished Burn Checks. Checks are paused. Retry the connection change."
        })
}

fn pause_model_checks(store: &Store, worker: &WorkerHandle) -> Result<(), &'static str> {
    store
        .set_internal_value_checked(YAML_DISCOVERY_ENABLED_KEY, "false")
        .map_err(|_| "Could not pause YAML Burn Checks.")?;
    suspend_for_connection_change(store, worker)
}

fn enable_model_checks(
    store: &Store,
    worker: &WorkerHandle,
    connection: SystemOneConnection,
    credential: Option<String>,
) -> Result<(), &'static str> {
    if !yaml_discovery_enabled(store) {
        suspend_for_connection_change(store, worker)?;
    }
    store
        .set_internal_values_checked(&[
            (YAML_DISCOVERY_ENABLED_KEY, "true"),
            (AUTH_REJECTED_KEY, "false"),
        ])
        .map_err(|_| "Could not enable YAML Burn Checks.")?;
    if worker
        .set_system_one_connection(connection, credential)
        .is_err()
    {
        worker.suspend_system_one();
        return Err("Saved Smart Burn Checks provider settings are invalid.");
    }
    Ok(())
}

fn next_connection_revision(revision: u64) -> Result<u64, &'static str> {
    revision
        .checked_add(1)
        .ok_or("The connection revision limit was reached. Save a new connection.")
}

fn advance_legacy_connection_revisions(saved: &mut ConnectionProfiles) -> Result<(), &'static str> {
    for connection in saved.profiles.values_mut() {
        if connection.credential.as_ref() == Some(&CredentialReference::LegacyTypeSafe) {
            connection.revision = next_connection_revision(connection.revision)?;
        }
    }
    Ok(())
}

fn persist_connection_profile(
    store: &Store,
    saved: &mut ConnectionProfiles,
    id: &str,
    mut connection: SystemOneConnection,
    credential: Option<String>,
    vault: &impl CredentialVault,
    worker: &WorkerHandle,
) -> Result<(SystemOneConnection, Option<String>, bool), &'static str> {
    ensure_no_pending_removal(store)?;
    if legacy_removal_pending(store) {
        suspend_for_connection_change(store, worker)?;
        clear_legacy_removal(store, vault)?;
        *saved = profiles(store)?;
    }
    let resolved = resolve_credential(&connection, credential.clone(), vault)?;
    if connection_requires_credential(&connection) && resolved.is_none() {
        return Err("The selected provider needs a credential. Add one and retry.");
    }
    let previous_secret = if credential.is_some() {
        vault
            .read(Some(id))
            .map_err(|_| "Credential storage is unavailable.")?
    } else {
        None
    };
    let previous_connection = saved
        .profiles
        .get(&saved.active_id)
        .cloned()
        .ok_or("The active Smart Burn Checks connection is missing.")?;
    let previous_credential = match previous_connection.credential.as_ref() {
        Some(CredentialReference::LegacyTypeSafe) => vault.read(None),
        Some(CredentialReference::Connection(previous_id)) => vault.read(Some(previous_id)),
        None => Ok(None),
    }
    .map_err(|_| "Credential storage is unavailable.")?;
    let checks_enabled = yaml_discovery_enabled(store);
    let previous_enabled =
        checks_enabled && store.internal_value(AUTH_REJECTED_KEY).as_deref() != Some("true");
    let replacing = credential.is_some();
    let credential_changed = replacing && previous_secret != credential;
    if replacing {
        connection.credential = Some(CredentialReference::Connection(id.to_owned()));
    }
    let recovering = store
        .internal_value(CONNECTION_PENDING_KEY)
        .is_some_and(|pending| !pending.is_empty());
    let previous_profile = saved.profiles.get(id);
    connection.revision = previous_profile.map_or(1, |profile| profile.revision);
    let profile_changed = previous_profile != Some(&connection) || credential_changed || recovering;
    if profile_changed && previous_profile.is_some() {
        connection.revision = next_connection_revision(connection.revision)?;
    }
    let changed = saved.active_id != id || profile_changed;
    if !changed {
        return Ok((connection, resolved, false));
    }
    let mut updated = saved.clone();
    if credential_changed || recovering {
        for (profile_id, profile) in &mut updated.profiles {
            if profile_id != id
                && profile.credential.as_ref()
                    == Some(&CredentialReference::Connection(id.to_owned()))
            {
                profile.revision = next_connection_revision(profile.revision)?;
            }
        }
    }
    updated.profiles.insert(id.to_owned(), connection.clone());
    updated.active_id = id.to_owned();
    store
        .set_internal_value_checked(CONNECTION_PENDING_KEY, id)
        .map_err(|_| "Could not protect the connection update state.")?;
    suspend_for_connection_change(store, worker)?;
    if let Some(secret) = credential
        && let Err(error) = write_credential(vault, Some(id), &secret)
    {
        let current_secret = vault
                .read(Some(id))
                .map_err(|_| {
                    "Could not confirm the saved credential. Checks are paused. Retry the connection update in Settings → Checks."
                })?;
        if current_secret != previous_secret {
            let rollback = match previous_secret.as_deref() {
                Some(previous) => write_credential(vault, Some(id), previous),
                None => delete_credential(vault, Some(id)),
            };
            if rollback.is_err() {
                return Err(
                    "Could not restore the previous credential. Checks are paused. Retry the connection update in Settings → Checks.",
                );
            }
        }
        if recovering {
            return Err(error);
        }
        store
                .set_internal_value_checked(CONNECTION_PENDING_KEY, "")
                .map_err(|_| {
                    "Could not restore the previous connection state. Retry the connection update in Settings → Checks."
                })?;
        let previous_enabled =
            previous_enabled && store.internal_value(AUTH_REJECTED_KEY).as_deref() != Some("true");
        worker
                .install_system_one_connection(
                    previous_connection,
                    previous_credential.filter(|_| previous_enabled),
                    previous_enabled,
                )
                .map_err(|_| {
                    "Could not restore the previous connection. Checks are paused. Retry the connection update in Settings → Checks."
                })?;
        return Err(error);
    }
    if let Err(error) = save_profiles(store, &updated) {
        if replacing {
            let rollback = match previous_secret {
                Some(secret) => write_credential(vault, Some(id), &secret),
                None => delete_credential(vault, Some(id)),
            };
            if rollback.is_err() {
                return Err(
                    "Could not restore the connection credential. Retry the connection update in Settings → Checks.",
                );
            }
        }
        return Err(error);
    }
    *saved = updated;
    Ok((connection, resolved, changed))
}

fn connection_requires_credential(connection: &SystemOneConnection) -> bool {
    matches!(
        connection.provider,
        SystemOneProvider::Jev | SystemOneProvider::Cloudflare
    )
}

/// Copy legacy TypeSafe state only after its existing secure-storage reference is known.
/// The migration marker is written last, so a failed write can be retried safely.
fn migrate_provider_state(store: &Store) -> Result<(), &'static str> {
    if store.internal_value(PROVIDER_MIGRATION_KEY).as_deref() == Some("1") {
        return Ok(());
    }
    let mut profiles: ConnectionProfiles = store
        .internal_value(CONNECTION_PROFILES_KEY)
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default();
    profiles
        .profiles
        .entry("jev".to_owned())
        .or_insert_with(SystemOneConnection::jev_default);
    if profiles.active_id.is_empty() {
        profiles.active_id = "jev".to_owned();
    }
    let encoded = serde_json::to_string(&profiles)
        .map_err(|_| "Could not migrate Smart Burn Checks provider settings.")?;
    store
        .set_internal_value_checked(CONNECTION_PROFILES_KEY, &encoded)
        .map_err(|_| "Could not migrate Smart Burn Checks provider settings.")?;
    store
        .set_internal_value_checked(ACTIVE_CONNECTION_KEY, &profiles.active_id)
        .map_err(|_| "Could not migrate Smart Burn Checks provider settings.")?;
    store
        .set_internal_value_checked(PROVIDER_MIGRATION_KEY, "1")
        .map_err(|_| "Could not complete Smart Burn Checks provider migration.")
}

fn active_connection(store: &Store) -> Result<SystemOneConnection, &'static str> {
    let profiles = profiles(store)?;
    let connection = profiles
        .profiles
        .get(&profiles.active_id)
        .cloned()
        .ok_or("The active Smart Burn Checks connection is missing.")?;
    connection
        .validate()
        .map_err(|_| "Saved Smart Burn Checks provider settings are invalid.")?;
    Ok(connection)
}

fn restore_saved_key(
    store: &Store,
    worker: &WorkerHandle,
    read: impl FnOnce() -> Result<Option<String>, &'static str>,
) -> Result<(), &'static str> {
    restore_saved_key_with_vault(store, worker, read, &SystemCredentialVault)
}

fn restore_saved_key_with_vault(
    store: &Store,
    worker: &WorkerHandle,
    read: impl FnOnce() -> Result<Option<String>, &'static str>,
    vault: &impl CredentialVault,
) -> Result<(), &'static str> {
    worker.suspend_system_one();
    ensure_no_pending_removal(store)?;
    if store
        .internal_value(CONNECTION_PENDING_KEY)
        .is_some_and(|value| !value.is_empty())
    {
        return Err(
            "Smart Burn Checks connection update is incomplete. Retry it in Settings → Checks.",
        );
    }
    if store
        .internal_value(CREDENTIAL_CHANGE_PENDING_KEY)
        .as_deref()
        == Some("true")
    {
        return Err("TypeSafe credential removal is incomplete. Retry it in Settings → Checks.");
    }
    let connection = active_connection(store)?;
    let checks_enabled = yaml_discovery_enabled(store);
    if !checks_enabled {
        return worker
            .install_system_one_connection(connection, None, false)
            .map_err(|_| "Saved Smart Burn Checks provider settings are invalid.");
    }
    let authentication_rejected =
        store.internal_value(AUTH_REJECTED_KEY).as_deref() == Some("true");
    if authentication_rejected {
        return worker
            .install_system_one_connection(connection, None, false)
            .map_err(|_| "Saved Smart Burn Checks provider settings are invalid.");
    }
    if matches!(
        connection.credential,
        Some(CredentialReference::LegacyTypeSafe)
    ) {
        let key = if saved_key_marker(store) {
            read()?
        } else {
            None
        };
        if saved_key_marker(store) && key.is_none() {
            return Err("The saved TypeSafe API key is missing. Replace it in Settings → Checks.");
        }
        worker
            .install_system_one_connection(connection, key, true)
            .map_err(|_| "Saved Smart Burn Checks provider settings are invalid.")?;
    } else {
        let credential = resolve_credential(&connection, None, vault)?;
        if connection_requires_credential(&connection) && credential.is_none() {
            return Err(
                "The active model provider credential is missing. Update it in Settings → Checks.",
            );
        }
        worker
            .set_system_one_connection(connection, credential)
            .map_err(|_| "Saved Smart Burn Checks provider settings are invalid.")?;
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn get_system_one_settings(
    window: WebviewWindow,
    store: tauri::State<'_, Store>,
) -> Result<SystemOneSettings, String> {
    checks_settings_window(&window).map_err(str::to_owned)?;
    let saved = profiles(&store).map_err(str::to_owned)?;
    Ok(SystemOneSettings {
        active_id: saved.active_id,
        profiles: saved.profiles,
    })
}

#[tauri::command]
pub(crate) async fn test_system_one_connection(
    window: WebviewWindow,
    mut draft: SystemOneDraft,
) -> Result<(), String> {
    checks_settings_window(&window).map_err(str::to_owned)?;
    draft.connection.revision = 1;
    let provider = draft.connection.provider;
    let result = async {
        {
            let _change = CREDENTIAL_CHANGE.lock().await;
            ensure_no_pending_credential_change(&window.state::<Store>()).map_err(str::to_owned)?;
            draft.credential =
                resolve_credential(&draft.connection, draft.credential, &SystemCredentialVault)
                    .map_err(str::to_owned)?;
        }
        test_connection(draft).await
    }
    .await;
    crate::analytics::record_model_check_lifecycle(
        window.app_handle(),
        crate::analytics::event::ModelCheckLifecycle::ProviderTest {
            provider: provider.into(),
            outcome: if result.is_ok() {
                crate::analytics::event::ProviderTestOutcome::Succeeded
            } else {
                crate::analytics::event::ProviderTestOutcome::Failed
            },
        },
    );
    result
}

async fn test_connection(draft: SystemOneDraft) -> Result<(), String> {
    let request = antiburn_local::analysis::jev::JevRequest {
        model: draft.connection.model.clone(),
        state: serde_json::json!({"text":"synthetic connection validation input"}),
        questions: std::collections::BTreeMap::from([(
            "validation".to_owned(),
            antiburn_local::analysis::jev::JevQuestion::Noul {
                instructions: serde_json::json!("Is this synthetic input valid?"),
                criteria: None,
            },
        )]),
    };
    let limits = crate::jev::config::resolve_execution_capabilities(
        &draft.connection,
        draft.credential.as_deref(),
    )
    .await
    .map_err(capability_error)?;
    let response = match draft.connection.provider {
        SystemOneProvider::Jev => {
            let key = draft
                .credential
                .ok_or_else(|| "Enter a TypeSafe API key.".to_owned())?;
            let client = crate::jev::client::TypeSafeClient::new(key)
                .map_err(|_| "Enter a valid TypeSafe API key.".to_owned())?;
            let endpoint = draft
                .connection
                .inference_endpoint()
                .map_err(|_| "Connection settings are invalid.".to_owned())?;
            client
                .evaluate_at_with_capabilities_and_usage(&request, &endpoint, &limits, |_| Ok(()))
                .await
                .map_err(|_| "TypeSafe rejected the test request. Check the API key.".to_owned())
        }
        SystemOneProvider::Ollama => {
            let SystemOneEndpoint::BaseUrl(base) = &draft.connection.endpoint else {
                return Err("Connection settings are invalid.".to_owned());
            };
            let client = crate::jev_ollama::OllamaClient::new(base, draft.credential)
                .map_err(|error| error.to_string())?;
            client
                .evaluate(&request, &limits)
                .await
                .map_err(|error| match error {
                    crate::jev_ollama::OllamaError::OldVersion
                    | crate::jev_ollama::OllamaError::MissingModel
                    | crate::jev_ollama::OllamaError::UnsupportedRunner
                    | crate::jev_ollama::OllamaError::ContextRejected
                    | crate::jev_ollama::OllamaError::RequestBodyTooLarge
                    | crate::jev_ollama::OllamaError::AuthenticationRejected
                    | crate::jev_ollama::OllamaError::InvalidRequest
                    | crate::jev_ollama::OllamaError::ResponseDecode
                    | crate::jev_ollama::OllamaError::ResponseTooLarge => error.to_string(),
                    crate::jev_ollama::OllamaError::InvalidBaseUrl
                    | crate::jev_ollama::OllamaError::ProviderUnavailable
                    | crate::jev_ollama::OllamaError::RequestOutcomeUnknown => {
                        "Could not reach Ollama. Check the server address and try again.".into()
                    }
                })
        }
        SystemOneProvider::Cloudflare => {
            let SystemOneEndpoint::CloudflareAccount(account_id) = &draft.connection.endpoint
            else {
                return Err("Connection settings are invalid.".to_owned());
            };
            let token = draft
                .credential
                .ok_or_else(|| "Enter a Cloudflare API token.".to_owned())?;
            crate::jev_cloudflare::CloudflareClient::new(
                account_id.clone(),
                token,
                draft.connection.model,
            )
            .map_err(|_| "Enter a valid Cloudflare connection.".to_owned())?
            .evaluate_with_capabilities_and_usage(&request, &limits, |_| Ok(()))
            .await
            .map_err(|_| {
                "Cloudflare rejected the test request. Check the account, model, and token."
                    .to_owned()
            })
        }
        SystemOneProvider::Custom => {
            let endpoint = draft
                .connection
                .inference_endpoint()
                .map_err(|_| "Connection settings are invalid.".to_owned())?;
            crate::jev::client::evaluate_custom(
                &endpoint,
                draft.credential.as_deref(),
                draft.connection.response_mode,
                &request,
                &limits,
            )
            .await
            .map_err(|_| {
                "The custom endpoint rejected the test request. Check the URL and response mode."
                    .to_owned()
            })
        }
    };
    response.map(|_| ())
}

#[tauri::command]
pub(crate) async fn save_system_one_connection(
    app: AppHandle,
    window: WebviewWindow,
    draft: SystemOneDraft,
) -> Result<SystemOneSettings, String> {
    checks_settings_window(&window).map_err(str::to_owned)?;
    let _change = CREDENTIAL_CHANGE.lock().await;
    let provider = draft.connection.provider;
    let result = save_connection(
        &app.state::<Store>(),
        &app.state::<WorkerHandle>(),
        draft,
        &SystemCredentialVault,
    );
    if let Ok((_, true)) = &result {
        crate::analytics::record_model_check_lifecycle(
            &app,
            crate::analytics::event::ModelCheckLifecycle::ProviderSetup {
                provider: provider.into(),
                outcome: crate::analytics::event::ProviderSetupOutcome::Saved,
            },
        );
    }
    changed(&app);
    result.map(|(settings, _)| settings).map_err(str::to_owned)
}

fn save_connection(
    store: &Store,
    worker: &WorkerHandle,
    mut draft: SystemOneDraft,
    vault: &impl CredentialVault,
) -> Result<(SystemOneSettings, bool), &'static str> {
    draft.connection.revision = 1;
    let mut saved = profiles(store)?;
    if draft.credential.is_none()
        && draft.connection.credential.is_none()
        && let Some(previous) = saved.profiles.get(&draft.connection_id)
    {
        draft.connection.credential.clone_from(&previous.credential);
    }
    draft
        .connection
        .validate()
        .map_err(|_| "Connection settings are invalid.")?;
    let id = draft.connection_id.clone();
    validate_connection_id(&id)?;
    ensure_no_pending_removal(store)?;
    if store
        .internal_value(CONNECTION_PENDING_KEY)
        .is_some_and(|pending| !pending.is_empty() && pending != id)
    {
        return Err(
            "Smart Burn Checks connection update is incomplete. Retry it in Settings → Checks.",
        );
    }
    let checks_enabled = yaml_discovery_enabled(store);
    let recovering = store
        .internal_value(CONNECTION_PENDING_KEY)
        .is_some_and(|pending| !pending.is_empty());
    let (connection, credential, changed) = persist_connection_profile(
        store,
        &mut saved,
        &id,
        draft.connection,
        draft.credential.take(),
        vault,
        worker,
    )?;
    if changed || recovering {
        store
            .set_internal_values_checked(&[
                (CONNECTION_PENDING_KEY, ""),
                (AUTH_REJECTED_KEY, "false"),
            ])
            .map_err(|_| "Could not complete the connection update.")?;
    }
    let enabled =
        checks_enabled && store.internal_value(AUTH_REJECTED_KEY).as_deref() != Some("true");
    worker
        .install_system_one_connection(connection, credential.filter(|_| enabled), enabled)
        .map_err(|_| "Connection settings are invalid.")?;
    *STARTUP_ERROR
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    Ok((
        SystemOneSettings {
            active_id: saved.active_id,
            profiles: saved.profiles,
        },
        changed,
    ))
}

#[tauri::command]
pub(crate) async fn switch_system_one_connection(
    app: AppHandle,
    window: WebviewWindow,
    connection_id: String,
) -> Result<SystemOneSettings, String> {
    checks_settings_window(&window).map_err(str::to_owned)?;
    let _change = CREDENTIAL_CHANGE.lock().await;
    let result = switch_connection(
        &app.state::<Store>(),
        &app.state::<WorkerHandle>(),
        &connection_id,
        &SystemCredentialVault,
    );
    if let Ok((settings, true)) = &result {
        crate::analytics::record_model_check_lifecycle(
            &app,
            crate::analytics::event::ModelCheckLifecycle::ProviderSetup {
                provider: settings.profiles[&settings.active_id].provider.into(),
                outcome: crate::analytics::event::ProviderSetupOutcome::Switched,
            },
        );
    }
    changed(&app);
    result.map(|(settings, _)| settings).map_err(str::to_owned)
}

fn switch_connection(
    store: &Store,
    worker: &WorkerHandle,
    connection_id: &str,
    vault: &impl CredentialVault,
) -> Result<(SystemOneSettings, bool), &'static str> {
    let mut saved = profiles(store)?;
    let changed = saved.active_id != connection_id;
    ensure_no_pending_credential_change(store)?;
    let connection = saved
        .profiles
        .get(connection_id)
        .cloned()
        .ok_or("The saved connection is missing.")?;
    connection
        .validate()
        .map_err(|_| "Saved connection settings are invalid.")?;
    let checks_enabled = yaml_discovery_enabled(store);
    let credential = resolve_credential(&connection, None, vault)?;
    if connection_requires_credential(&connection) && credential.is_none() {
        return Err("The selected provider needs a credential. Add one and retry.");
    }
    if changed {
        store
            .set_internal_value_checked(CONNECTION_PENDING_KEY, connection_id)
            .map_err(|_| "Could not protect the connection update state.")?;
        suspend_for_connection_change(store, worker)?;
        saved.active_id = connection_id.to_owned();
        save_profiles(store, &saved)?;
        store
            .set_internal_values_checked(&[
                (CONNECTION_PENDING_KEY, ""),
                (AUTH_REJECTED_KEY, "false"),
            ])
            .map_err(|_| "Could not complete the connection update.")?;
    }
    let enabled =
        checks_enabled && store.internal_value(AUTH_REJECTED_KEY).as_deref() != Some("true");
    worker
        .install_system_one_connection(connection, credential.filter(|_| enabled), enabled)
        .map_err(|_| "Saved connection settings are invalid.")?;
    *STARTUP_ERROR
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    Ok((
        SystemOneSettings {
            active_id: saved.active_id,
            profiles: saved.profiles,
        },
        changed,
    ))
}

#[tauri::command]
pub(crate) async fn refresh_system_one_limits(
    window: WebviewWindow,
    mut connection: SystemOneConnection,
    credential: Option<String>,
) -> Result<antiburn_local::analysis::jev::capabilities::ModelCapabilities, String> {
    checks_settings_window(&window).map_err(str::to_owned)?;
    connection.revision = 1;
    let credential = {
        let _change = CREDENTIAL_CHANGE.lock().await;
        ensure_no_pending_credential_change(&window.state::<Store>()).map_err(str::to_owned)?;
        resolve_credential(&connection, credential, &SystemCredentialVault)
            .map_err(str::to_owned)?
    };
    refresh_limits(connection, credential).await
}

async fn refresh_limits(
    connection: SystemOneConnection,
    credential: Option<String>,
) -> Result<antiburn_local::analysis::jev::capabilities::ModelCapabilities, String> {
    crate::jev::config::resolve_execution_capabilities(&connection, credential.as_deref())
        .await
        .map_err(capability_error)
}

fn capability_error(error: antiburn_local::analysis::jev::JevError) -> String {
    use antiburn_local::analysis::jev::JevError;
    match error {
        JevError::AuthenticationRejected => {
            "The selected provider rejected its credential. Update it in Settings → Checks."
        }
        JevError::UnsupportedModel => "The selected provider does not support this model or runner.",
        JevError::ContextRejected => "The selected provider did not report usable model limits.",
        JevError::InvalidRequestSchema => "Connection settings are invalid.",
        _ => "Could not read model limits from the selected provider. Check the connection and try again.",
    }
    .to_owned()
}

#[tauri::command]
pub(crate) async fn remove_system_one_credential(
    app: AppHandle,
    window: WebviewWindow,
    connection_id: String,
) -> Result<(), String> {
    checks_settings_window(&window).map_err(str::to_owned)?;
    let _change = CREDENTIAL_CHANGE.lock().await;
    let store = app.state::<Store>();
    let result = remove_connection_credential(
        &store,
        &app.state::<WorkerHandle>(),
        &connection_id,
        &SystemCredentialVault,
    );
    if result.is_ok() {
        *STARTUP_ERROR
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }
    if let Ok(Some(provider)) = &result {
        crate::analytics::record_model_check_lifecycle(
            &app,
            crate::analytics::event::ModelCheckLifecycle::ProviderSetup {
                provider: (*provider).into(),
                outcome: crate::analytics::event::ProviderSetupOutcome::CredentialRemoved,
            },
        );
    }
    changed(&app);
    result.map(|_| ()).map_err(str::to_owned)
}

fn remove_connection_credential(
    store: &Store,
    worker: &WorkerHandle,
    id: &str,
    vault: &impl CredentialVault,
) -> Result<Option<SystemOneProvider>, &'static str> {
    if legacy_removal_pending(store)
        || store
            .internal_value(CONNECTION_PENDING_KEY)
            .is_some_and(|pending| !pending.is_empty())
    {
        return Err(
            "A connection update is incomplete. Retry it in Settings → Checks before removing a credential.",
        );
    }
    if let Some(pending) = store.internal_value(CONNECTION_REMOVAL_KEY)
        && !pending.is_empty()
        && pending != id
    {
        return Err("Connection credential removal is incomplete. Retry it in Settings → Checks.");
    }
    let mut saved = profiles(store)?;
    let profile = saved
        .profiles
        .get_mut(id)
        .ok_or("The saved connection is missing.")?;
    let reference = profile.credential.clone();
    let provider = profile.provider;
    let Some(CredentialReference::Connection(key_id)) = reference else {
        if profile.credential.is_some() {
            return Err("Remove the TypeSafe API key in Settings → Checks.");
        }
        return Ok(None);
    };
    profile.credential = None;
    profile.revision = next_connection_revision(profile.revision)?;
    let shared = saved.profiles.values().any(|profile| {
        profile.credential.as_ref() == Some(&CredentialReference::Connection(key_id.clone()))
    });
    let previous = if shared {
        None
    } else {
        vault
            .read(Some(&key_id))
            .map_err(|_| "Credential storage is unavailable.")?
    };
    store
        .set_internal_value_checked(CONNECTION_REMOVAL_KEY, id)
        .map_err(|_| "Could not protect the credential removal state.")?;
    if saved.active_id == id {
        pause_model_checks(store, worker)?;
    }
    if !shared {
        delete_credential(vault, Some(&key_id))?;
    }
    let encoded =
        serde_json::to_string(&saved).map_err(|_| "Could not save connection settings.")?;
    let mut updates = vec![
        (CONNECTION_PROFILES_KEY, encoded.as_str()),
        (ACTIVE_CONNECTION_KEY, saved.active_id.as_str()),
        (CONNECTION_REMOVAL_KEY, ""),
    ];
    if saved.active_id == id {
        updates.push((AUTH_REJECTED_KEY, "false"));
    }
    if store.set_internal_values_checked(&updates).is_err() {
        if let Some(secret) = previous
            && write_credential(vault, Some(&key_id), &secret).is_err()
        {
            return Err(
                "Could not restore the connection credential. Retry credential removal in Settings → Checks.",
            );
        }
        return Err("Could not save the credential removal state.");
    }
    Ok(Some(provider))
}

/// Restore only a previously enabled key. Serialize startup with Settings changes.
pub(crate) fn restore_at_launch(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _change = CREDENTIAL_CHANGE.lock().await;
        let store = app.state::<Store>().inner().clone();
        if let Err(error) = migrate_provider_state(&store) {
            *STARTUP_ERROR
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(error);
            crate::installed_checks::credentials_restored(&app);
            changed(&app);
            return;
        }
        if let Err(error) = active_connection(&store) {
            *STARTUP_ERROR
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(error);
            crate::installed_checks::credentials_restored(&app);
            changed(&app);
            return;
        }
        let worker_app = app.clone();
        let result = tauri::async_runtime::spawn_blocking(move || {
            let worker = worker_app.state::<WorkerHandle>();
            restore_saved_key(&store, &worker, read_key)
        })
        .await;
        let error = match result {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(error),
            Err(_) => Some("Credential storage is unavailable."),
        };
        *STARTUP_ERROR
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = error;
        crate::installed_checks::credentials_restored(&app);
        changed(&app);
    });
}

fn checks_settings_window(window: &WebviewWindow) -> Result<(), &'static str> {
    checks_settings_label(window.label())
}

fn checks_settings_label(label: &str) -> Result<(), &'static str> {
    if matches!(label, crate::settings::LABEL | crate::main_window::LABEL) {
        Ok(())
    } else {
        Err("This action is available only in the main window or Settings.")
    }
}

pub(crate) fn read_check_availability(
    store: &Store,
    configured: bool,
    saved_key: bool,
    error: Option<&'static str>,
) -> Result<CheckAvailability, &'static str> {
    let (enabled_checks, revision) = store
        .check_preferences_snapshot()
        .map_err(|_| "Could not read check preferences from the local database.")?;
    Ok(CheckAvailability {
        yaml_enabled: yaml_discovery_enabled(store),
        configured,
        saved_key,
        error,
        usage: store
            .burn_check_usage_summary()
            .map_err(|_| "Could not read check usage from the local database.")?,
        checks: DetectorId::ALL
            .into_iter()
            .map(|detector| CheckChoice {
                id: detector.into(),
                enabled: enabled_checks.contains(&detector),
            })
            .collect(),
        installed_checks: Vec::new(),
        revision,
    })
}

fn availability(app: &AppHandle) -> Result<CheckAvailability, &'static str> {
    let store = app.state::<Store>();
    let removal_pending = store
        .internal_value(CREDENTIAL_CHANGE_PENDING_KEY)
        .as_deref()
        == Some("true");
    let authentication_rejected = app.state::<WorkerHandle>().authentication_rejected()
        || store.internal_value(AUTH_REJECTED_KEY).as_deref() == Some("true");
    let error = if removal_pending {
        Some("TypeSafe credential removal is incomplete. Retry it in Settings → Checks.")
    } else if store
        .internal_value(CONNECTION_REMOVAL_KEY)
        .is_some_and(|id| !id.is_empty())
    {
        Some("Connection credential removal is incomplete. Retry it in Settings → Checks.")
    } else if store
        .internal_value(CONNECTION_PENDING_KEY)
        .is_some_and(|id| !id.is_empty())
    {
        Some("Smart Burn Checks connection update is incomplete. Retry it in Settings → Checks.")
    } else if authentication_rejected {
        Some("The selected provider rejected its credential. Update it in Settings → Checks.")
    } else {
        *STARTUP_ERROR
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    };
    read_check_availability(
        &store,
        app.state::<WorkerHandle>().is_available(),
        saved_key_marker(&store),
        error,
    )
    .map(|availability| availability.with_installed_checks(&crate::installed_checks::metadata(app)))
}

pub(crate) fn changed(app: &AppHandle) {
    crate::installed_checks::wake(app);
    let event = match availability(app) {
        Ok(snapshot) => CheckAvailabilityEvent::Updated(Box::new(snapshot)),
        Err(_) => CheckAvailabilityEvent::Failed,
    };
    let _ = app.emit(AVAILABILITY_EVENT, event);
    let _ = app.emit(crate::commands::CHECKS_REPORT_CHANGED_EVENT, ());
}

pub(crate) fn progress_changed(app: &AppHandle) {
    let event = match availability(app) {
        Ok(snapshot) => CheckAvailabilityEvent::Updated(Box::new(snapshot)),
        Err(_) => CheckAvailabilityEvent::Failed,
    };
    let _ = app.emit(AVAILABILITY_EVENT, event);
}

#[tauri::command]
pub(crate) async fn set_smart_burn_checks_enabled(
    app: AppHandle,
    window: WebviewWindow,
    enabled: bool,
) -> Result<CheckAvailability, String> {
    checks_settings_window(&window).map_err(str::to_owned)?;
    let _change = CREDENTIAL_CHANGE.lock().await;
    let store = app.state::<Store>().inner().clone();
    let was_enabled = yaml_discovery_enabled(&store);
    if !enabled {
        if let Err(error) = pause_model_checks(&store, &app.state::<WorkerHandle>()) {
            changed(&app);
            return Err(error.to_owned());
        }
        if let Err(error) = crate::installed_checks::settle_disable(&app).await {
            changed(&app);
            return Err(error.to_string());
        }
    } else {
        let connection = active_connection(&store).map_err(str::to_owned)?;
        ensure_no_pending_removal(&store).map_err(str::to_owned)?;
        if store
            .internal_value(CONNECTION_PENDING_KEY)
            .is_some_and(|id| !id.is_empty())
        {
            return Err(
                "Smart Burn Checks connection update is incomplete. Retry it in Settings → Checks."
                    .to_owned(),
            );
        }
        if store
            .internal_value(CREDENTIAL_CHANGE_PENDING_KEY)
            .as_deref()
            == Some("true")
        {
            return Err(
                "TypeSafe credential removal is incomplete. Retry it in Settings → Checks."
                    .to_owned(),
            );
        }
        if store.internal_value(AUTH_REJECTED_KEY).as_deref() == Some("true") {
            return Err(
                "The selected provider rejected its credential. Update it in Settings → Checks."
                    .to_owned(),
            );
        }
        let connection_for_worker = connection.clone();
        let credential = tauri::async_runtime::spawn_blocking(move || {
            read_connection_credential(&connection_for_worker)
        })
        .await
        .map_err(|_| "Credential storage is unavailable.".to_owned())?
        .map_err(str::to_owned)?;
        if connection_requires_credential(&connection) && credential.is_none() {
            return Err("The selected provider needs a valid credential.".to_owned());
        }
        enable_model_checks(&store, &app.state::<WorkerHandle>(), connection, credential)
            .map_err(str::to_owned)?;
    }
    if was_enabled != enabled {
        crate::analytics::record_model_check_lifecycle(
            &app,
            crate::analytics::event::ModelCheckLifecycle::Enablement { enabled },
        );
    }
    changed(&app);
    availability(&app).map_err(str::to_owned)
}

#[tauri::command]
pub(crate) fn get_check_availability(app: AppHandle) -> Result<CheckAvailability, String> {
    availability(&app).map_err(str::to_owned)
}

#[tauri::command]
pub(crate) fn set_check_enabled(
    app: AppHandle,
    window: WebviewWindow,
    detector: BurnCheckDetectorId,
    enabled: bool,
) -> Result<CheckAvailability, String> {
    checks_settings_window(&window).map_err(str::to_owned)?;
    let _change = CHECK_CHANGE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let detector = DetectorId::from(detector);
    let store = app.state::<Store>();
    if store
        .check_enabled(detector)
        .map_err(|_| "Could not read the check preference.".to_owned())?
        == enabled
    {
        return availability(&app).map_err(str::to_owned);
    }
    let changed_preference = store
        .set_check_enabled(detector, enabled)
        .map_err(|_| "Could not save the check preference.".to_owned())?;
    if !changed_preference {
        return availability(&app).map_err(str::to_owned);
    }
    app.state::<crate::insights_ipc::InsightsController>()
        .cancel();
    crate::session_lifecycle::report(
        &app,
        crate::session_lifecycle::SyncObservation::IndexChanged {
            reason: crate::session_lifecycle::IndexChangeReason::Invalidated,
        },
    );
    changed(&app);
    crate::analytics::record_check_enablement_saved(&app, detector, enabled);
    availability(&app).map_err(str::to_owned)
}

fn resume_after_key_save(was_enabled: bool, replacing: bool, was_configured: bool) -> bool {
    was_enabled || (replacing && !was_configured)
}

fn save_legacy_key(
    store: &Store,
    worker: &WorkerHandle,
    key: Option<String>,
    vault: &impl CredentialVault,
) -> Result<(), &'static str> {
    let replacing = key.is_some();
    let resume = resume_after_key_save(
        yaml_discovery_enabled(store),
        replacing,
        saved_key_marker(store),
    );
    let previous = vault
        .read(None)
        .map_err(|_| "Credential storage is unavailable.")?;
    let key = key
        .or_else(|| previous.clone())
        .ok_or("The saved TypeSafe API key is missing. Enter a new key.")?;
    let mut saved = profiles(store)?;
    let credential_changed = previous.as_ref() != Some(&key);
    if credential_changed || legacy_removal_pending(store) {
        advance_legacy_connection_revisions(&mut saved)?;
    }
    let connection = saved
        .profiles
        .get(&saved.active_id)
        .cloned()
        .ok_or("The active Smart Burn Checks connection is missing.")?;
    connection
        .validate()
        .map_err(|_| "Saved Smart Burn Checks provider settings are invalid.")?;
    let credential = if connection.credential.as_ref() == Some(&CredentialReference::LegacyTypeSafe)
    {
        Some(key.clone())
    } else {
        resolve_credential(&connection, None, vault)?
    };
    let encoded =
        serde_json::to_string(&saved).map_err(|_| "Could not save the credential update state.")?;
    store
        .set_internal_value_checked(CREDENTIAL_CHANGE_PENDING_KEY, "true")
        .map_err(|_| "Could not protect the credential update state.")?;
    suspend_for_connection_change(store, worker)?;
    if credential_changed && let Err(error) = write_credential(vault, None, &key) {
        restore_legacy_key(vault, previous.as_deref())?;
        return Err(error);
    }
    if store
        .set_internal_values_checked(&[
            (CONNECTION_PROFILES_KEY, &encoded),
            (SAVED_KEY_KEY, "true"),
            (
                YAML_DISCOVERY_ENABLED_KEY,
                if resume { "true" } else { "false" },
            ),
            (CREDENTIAL_CHANGE_PENDING_KEY, "false"),
            (AUTH_REJECTED_KEY, "false"),
        ])
        .is_err()
    {
        if credential_changed {
            restore_legacy_key(vault, previous.as_deref())?;
        }
        return Err("Could not save the credential update state.");
    }
    worker
        .install_system_one_connection(connection, credential.filter(|_| resume), resume)
        .map_err(|_| "Saved Smart Burn Checks provider settings are invalid.")
}

fn restore_legacy_key(
    vault: &impl CredentialVault,
    previous: Option<&str>,
) -> Result<(), &'static str> {
    match previous {
        Some(secret) => write_credential(vault, None, secret),
        None => delete_credential(vault, None),
    }
    .map_err(|_| "Could not restore the API key. Checks are paused. Retry the key update.")
}

#[tauri::command]
pub(crate) async fn set_typesafe_api_key(
    app: AppHandle,
    window: WebviewWindow,
    key: Option<String>,
) -> Result<CheckAvailability, String> {
    checks_settings_window(&window).map_err(str::to_owned)?;
    let _change = CREDENTIAL_CHANGE.lock().await;
    ensure_no_pending_removal(&app.state::<Store>()).map_err(str::to_owned)?;
    if app
        .state::<Store>()
        .internal_value(CONNECTION_PENDING_KEY)
        .is_some_and(|id| !id.is_empty())
    {
        return Err(
            "Smart Burn Checks connection update is incomplete. Retry it in Settings → Checks."
                .to_owned(),
        );
    }
    if key.is_none()
        && app
            .state::<Store>()
            .internal_value(CREDENTIAL_CHANGE_PENDING_KEY)
            .as_deref()
            == Some("true")
    {
        return Err(
            "TypeSafe credential removal is incomplete. Retry it in Settings → Checks.".to_owned(),
        );
    }
    if key.is_none() && !saved_key_marker(&app.state::<Store>()) {
        return Err("Enter a TypeSafe API key first.".to_owned());
    }
    if key.is_none()
        && app
            .state::<Store>()
            .internal_value(AUTH_REJECTED_KEY)
            .as_deref()
            == Some("true")
    {
        return Err("TypeSafe rejected this API key. Enter a replacement key.".to_owned());
    }
    let key = key.map(|key| key.trim().to_owned());
    #[cfg(feature = "analytics")]
    let first_enablement = key.is_some() && !saved_key_marker(&app.state::<Store>());
    if key
        .as_ref()
        .is_some_and(|key| key.is_empty() || key.len() > 4096 || key.chars().any(char::is_control))
    {
        return Err("Enter a valid TypeSafe API key.".to_owned());
    }
    let store = app.state::<Store>().inner().clone();
    let worker_app = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        save_legacy_key(
            &store,
            &worker_app.state::<WorkerHandle>(),
            key,
            &SystemCredentialVault,
        )
    })
    .await
    .map_err(|_| "Credential storage is unavailable.".to_owned())?
    .map_err(str::to_owned);
    if let Err(error) = result {
        changed(&app);
        return Err(error);
    }
    #[cfg(feature = "analytics")]
    if first_enablement {
        crate::analytics::record_model_check_lifecycle(
            &app,
            crate::analytics::event::ModelCheckLifecycle::Enablement { enabled: true },
        );
    }
    *STARTUP_ERROR
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    changed(&app);
    availability(&app).map_err(str::to_owned)
}

#[tauri::command]
pub(crate) async fn remove_typesafe_api_key(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<CheckAvailability, String> {
    checks_settings_window(&window).map_err(str::to_owned)?;
    let _change = CREDENTIAL_CHANGE.lock().await;
    let had_saved_key = saved_key_marker(&app.state::<Store>());
    if !had_saved_key && !app.state::<WorkerHandle>().is_available() {
        return availability(&app).map_err(str::to_owned);
    }
    let store = app.state::<Store>();
    let active = active_connection(&store).map_err(str::to_owned)?;
    #[cfg(feature = "analytics")]
    let was_enabled = yaml_discovery_enabled(&store);
    let active_uses_legacy_key =
        matches!(active.credential, Some(CredentialReference::LegacyTypeSafe));
    let retrying_legacy_removal = legacy_removal_pending(&store);
    let saved =
        begin_legacy_removal(&store, &app.state::<WorkerHandle>()).map_err(str::to_owned)?;
    changed(&app);
    let removal = tauri::async_runtime::spawn_blocking({
        let store = store.inner().clone();
        move || finish_legacy_removal(&store, &SystemCredentialVault, &saved)
    })
    .await
    .map_err(|_| "Credential storage is unavailable.".to_owned())?
    .map_err(str::to_owned);
    if let Err(error) = removal {
        if active_uses_legacy_key || retrying_legacy_removal {
            *STARTUP_ERROR
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                Some("Could not remove the API key from credential storage.");
        }
        changed(&app);
        return Err(error);
    }
    if active_uses_legacy_key || retrying_legacy_removal {
        *STARTUP_ERROR
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }
    #[cfg(feature = "analytics")]
    if active_uses_legacy_key && was_enabled {
        crate::analytics::record_model_check_lifecycle(
            &app,
            crate::analytics::event::ModelCheckLifecycle::Enablement { enabled: false },
        );
    }
    if had_saved_key {
        crate::analytics::record_model_check_lifecycle(
            &app,
            crate::analytics::event::ModelCheckLifecycle::ProviderSetup {
                provider: crate::analytics::event::DecisionProvider::Jev,
                outcome: crate::analytics::event::ProviderSetupOutcome::CredentialRemoved,
            },
        );
    }
    changed(&app);
    availability(&app).map_err(str::to_owned)
}

#[cfg(test)]
pub(crate) mod headless {
    use super::*;

    struct OfflineKeylessVault;

    impl CredentialVault for OfflineKeylessVault {
        type Error = &'static str;

        fn read(&self, _: Option<&str>) -> Result<Option<String>, Self::Error> {
            Ok(None)
        }

        fn write(&self, _: Option<&str>, _: &str) -> Result<(), Self::Error> {
            Err("The keyless test vault does not accept writes.")
        }

        fn delete(&self, _: Option<&str>) -> Result<(), Self::Error> {
            Err("The keyless test vault does not accept removals.")
        }

        fn is_missing(_: &Self::Error) -> bool {
            false
        }

        fn write_failure(_: &Self::Error) -> &'static str {
            "The keyless test vault does not accept writes."
        }
    }

    pub(crate) fn setup_keyless_profiles(
        store: &Store,
        profiles: std::collections::BTreeMap<String, SystemOneConnection>,
        active_id: &str,
        enabled: bool,
    ) -> Result<(), &'static str> {
        if !profiles.contains_key(active_id) {
            return Err("The test active profile is missing.");
        }
        for connection in profiles.values() {
            if connection.provider != SystemOneProvider::Custom || connection.credential.is_some() {
                return Err("The test profiles must use keyless Custom connections.");
            }
            connection
                .validate()
                .map_err(|_| "The test connection is invalid.")?;
        }
        migrate_provider_state(store)?;
        save_profiles(
            store,
            &ConnectionProfiles {
                active_id: active_id.to_owned(),
                profiles,
            },
        )?;
        store
            .set_internal_value_checked(
                YAML_DISCOVERY_ENABLED_KEY,
                if enabled { "true" } else { "false" },
            )
            .map_err(|_| "Could not save test enablement.")
    }

    pub(crate) fn switch_keyless_profile(
        store: &Store,
        worker: &WorkerHandle,
        connection_id: &str,
    ) -> Result<bool, &'static str> {
        switch_connection(store, worker, connection_id, &OfflineKeylessVault)
            .map(|(_, changed)| changed)
    }

    pub(crate) fn restore_keyless_profile(
        store: &Store,
        worker: &WorkerHandle,
    ) -> Result<(), &'static str> {
        restore_saved_key_with_vault(store, worker, || Ok(None), &OfflineKeylessVault)
    }

    pub(crate) fn pause_model_checks(
        store: &Store,
        worker: &WorkerHandle,
    ) -> Result<(), &'static str> {
        super::pause_model_checks(store, worker)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::path::Path;

    use super::{
        AUTH_REJECTED_KEY, CONNECTION_PENDING_KEY, CONNECTION_PROFILES_KEY,
        CREDENTIAL_CHANGE_PENDING_KEY, CheckAvailabilityEvent, CredentialVault,
        PROVIDER_MIGRATION_KEY, SAVED_KEY_KEY, YAML_DISCOVERY_ENABLED_KEY, active_connection,
        delete_credential, migrate_provider_state, persist_connection_profile, restore_saved_key,
        saved_key_marker, write_credential,
    };
    use crate::jev::worker::WorkerHandle;
    use crate::store::Store;

    #[test]
    fn yaml_discovery_defaults_enabled_and_preserves_saved_choices() {
        let store =
            Store::open_in_memory(Path::new("/tmp/antiburn-yaml-discovery-setting")).unwrap();
        assert!(super::yaml_discovery_enabled(&store));
        assert!(store.internal_value(YAML_DISCOVERY_ENABLED_KEY).is_none());
        store.set_internal_value("internal:burnChecksEnabledAtEpochV1", "123");
        assert!(super::yaml_discovery_enabled(&store));
        assert!(!saved_key_marker(&store));
        store
            .set_internal_value_checked(YAML_DISCOVERY_ENABLED_KEY, "false")
            .unwrap();
        assert!(!super::yaml_discovery_enabled(&store));
        assert!(
            !super::read_check_availability(&store, false, false, None)
                .unwrap()
                .yaml_enabled
        );
        assert_eq!(
            store.internal_value(YAML_DISCOVERY_ENABLED_KEY).as_deref(),
            Some("false")
        );
        store
            .set_internal_value_checked(YAML_DISCOVERY_ENABLED_KEY, "true")
            .unwrap();
        assert!(super::yaml_discovery_enabled(&store));
    }

    #[test]
    fn yaml_discovery_fails_closed_for_malformed_values() {
        let store = Store::open_in_memory(Path::new("synthetic-yaml-preference")).unwrap();
        for value in ["", "TRUE", "1", "null", " true", "true ", "\"true\""] {
            store
                .set_internal_value_checked(YAML_DISCOVERY_ENABLED_KEY, value)
                .unwrap();
            assert!(!super::yaml_discovery_enabled(&store), "{value:?}");
            assert_eq!(
                store.internal_value(YAML_DISCOVERY_ENABLED_KEY).as_deref(),
                Some(value)
            );
        }
    }

    #[test]
    fn yaml_discovery_fails_closed_for_unreadable_storage() {
        let store = Store::open_in_memory(Path::new("synthetic-yaml-preference")).unwrap();
        store
            .set_internal_value_checked(YAML_DISCOVERY_ENABLED_KEY, "true")
            .unwrap();
        store.lock().execute_batch("DROP TABLE setting").unwrap();
        assert!(!super::yaml_discovery_enabled(&store));
    }

    #[test]
    fn fresh_availability_enables_discovery_without_a_provider_or_discovered_checks() {
        let store = Store::open_in_memory(Path::new("synthetic-fresh-availability")).unwrap();
        let snapshot = super::read_check_availability(&store, false, false, None).unwrap();
        assert!(snapshot.yaml_enabled);
        assert!(!snapshot.configured);
        assert!(!snapshot.saved_key);
        assert!(snapshot.installed_checks.is_empty());
        assert!(store.internal_value(YAML_DISCOVERY_ENABLED_KEY).is_none());
    }

    #[derive(Default)]
    struct OfflineVault {
        fail_read: Cell<bool>,
        fail_write: Cell<bool>,
        write_calls: Cell<usize>,
        fail_write_at: Cell<Option<usize>>,
        fail_delete: Cell<bool>,
        value: std::cell::RefCell<Option<String>>,
        deleted_ids: std::cell::RefCell<Vec<Option<String>>>,
    }

    impl CredentialVault for OfflineVault {
        type Error = &'static str;

        fn read(&self, _id: Option<&str>) -> Result<Option<String>, Self::Error> {
            if self.fail_read.replace(false) {
                return Err("read");
            }
            Ok(self.value.borrow().clone())
        }

        fn write(&self, _id: Option<&str>, value: &str) -> Result<(), Self::Error> {
            let call = self.write_calls.get() + 1;
            self.write_calls.set(call);
            if self.fail_write.replace(false) || self.fail_write_at.get() == Some(call) {
                return Err("write");
            }
            *self.value.borrow_mut() = Some(value.to_owned());
            Ok(())
        }

        fn delete(&self, id: Option<&str>) -> Result<(), Self::Error> {
            if self.fail_delete.replace(false) {
                return Err("delete");
            }
            self.value.borrow_mut().take();
            self.deleted_ids.borrow_mut().push(id.map(str::to_owned));
            Ok(())
        }

        fn is_missing(error: &Self::Error) -> bool {
            *error == "missing"
        }

        fn write_failure(_error: &Self::Error) -> &'static str {
            "Could not save the API token to secure storage. Check credential storage access and retry."
        }
    }

    fn custom_connection() -> crate::jev::config::SystemOneConnection {
        crate::jev::config::SystemOneConnection {
            provider: crate::jev::config::SystemOneProvider::Custom,
            endpoint: crate::jev::config::SystemOneEndpoint::ExactUrl(
                "https://proxy.example/systemone".into(),
            ),
            model: "proxy-model".into(),
            credential: None,
            context_override: Some(crate::jev::config::ContextLimitOverride {
                total_input_tokens: Some(8192),
                ..crate::jev::config::ContextLimitOverride::default()
            }),
            ..crate::jev::config::SystemOneConnection::default()
        }
    }

    fn saved_custom(active: bool) -> (Store, WorkerHandle, OfflineVault) {
        let store = Store::open_in_memory(Path::new("/tmp/antiburn-credential-lifecycle")).unwrap();
        migrate_provider_state(&store).unwrap();
        let worker = WorkerHandle::default();
        let vault = OfflineVault::default();
        store.set_internal_value(
            YAML_DISCOVERY_ENABLED_KEY,
            if active { "true" } else { "false" },
        );
        super::save_connection(
            &store,
            &worker,
            super::SystemOneDraft {
                connection_id: "proxy".into(),
                connection: custom_connection(),
                credential: Some("synthetic-secret".into()),
            },
            &vault,
        )
        .unwrap();
        (store, worker, vault)
    }

    #[test]
    fn custom_without_input_bound_cannot_save_or_activate() {
        let store = Store::open_in_memory(Path::new("/tmp/antiburn-custom-limits")).unwrap();
        migrate_provider_state(&store).unwrap();
        let worker = WorkerHandle::default();
        let vault = OfflineVault::default();
        let original = super::profiles(&store).unwrap();
        for limits in [
            None,
            Some(crate::jev::config::ContextLimitOverride::default()),
            Some(crate::jev::config::ContextLimitOverride {
                state_and_longest_question_tokens: Some(8192),
                ..crate::jev::config::ContextLimitOverride::default()
            }),
        ] {
            let mut connection = custom_connection();
            connection.context_override = limits;
            assert!(
                super::save_connection(
                    &store,
                    &worker,
                    super::SystemOneDraft {
                        connection_id: "proxy".into(),
                        connection,
                        credential: Some("synthetic-secret".into()),
                    },
                    &vault,
                )
                .is_err()
            );
            let saved = super::profiles(&store).unwrap();
            assert_eq!(saved.active_id, original.active_id);
            assert_eq!(saved.profiles, original.profiles);
            assert!(vault.value.borrow().is_none());
        }
    }

    #[test]
    fn provider_operation_outcomes_require_real_saved_transitions() {
        let (store, worker, vault) = saved_custom(true);
        let draft = super::SystemOneDraft {
            connection_id: "proxy".into(),
            connection: custom_connection(),
            credential: Some("synthetic-secret".into()),
        };
        assert!(
            !super::save_connection(&store, &worker, draft.clone(), &vault)
                .unwrap()
                .1
        );
        let mut replacement = draft;
        replacement.credential = Some("replacement-secret".into());
        assert!(
            super::save_connection(&store, &worker, replacement, &vault)
                .unwrap()
                .1
        );
        assert!(
            !super::switch_connection(&store, &worker, "proxy", &vault)
                .unwrap()
                .1
        );
        let saved = super::profiles(&store).unwrap();
        super::save_connection(
            &store,
            &worker,
            super::SystemOneDraft {
                connection_id: "alias".into(),
                connection: saved.profiles["proxy"].clone(),
                credential: None,
            },
            &vault,
        )
        .unwrap();
        assert!(
            super::switch_connection(&store, &worker, "proxy", &vault)
                .unwrap()
                .1
        );
        assert_eq!(
            super::remove_connection_credential(&store, &worker, "proxy", &vault).unwrap(),
            Some(super::SystemOneProvider::Custom)
        );
        assert_eq!(
            super::remove_connection_credential(&store, &worker, "proxy", &vault).unwrap(),
            None
        );
    }

    #[test]
    fn saved_revisions_ignore_client_values_and_advance_only_for_profile_changes() {
        let (store, worker, vault) = saved_custom(true);
        let initial_generation = worker.execution_snapshot().unwrap().generation;
        for revision in [0, 1, u64::MAX] {
            let mut connection = active_connection(&store).unwrap();
            connection.revision = revision;
            let (settings, changed) = super::save_connection(
                &store,
                &worker,
                super::SystemOneDraft {
                    connection_id: "proxy".into(),
                    connection,
                    credential: None,
                },
                &vault,
            )
            .unwrap();
            assert!(!changed);
            assert_eq!(settings.profiles["proxy"].revision, 1);
            assert_eq!(
                worker.execution_snapshot().unwrap().generation,
                initial_generation
            );
        }
        for index in 0..6 {
            let mut connection = active_connection(&store).unwrap();
            connection.revision = 0;
            let mut credential = None;
            match index {
                0 => {
                    connection.endpoint = super::SystemOneEndpoint::ExactUrl(
                        "https://other-proxy.example/systemone".into(),
                    )
                }
                1 => connection.model = "updated-model".into(),
                2 => connection.model_revision = Some("updated-model-revision".into()),
                3 => {
                    connection.response_mode =
                        crate::jev::config::SystemOneResponseMode::CloudflareEnvelope
                }
                4 => {
                    connection
                        .context_override
                        .as_mut()
                        .unwrap()
                        .total_input_tokens = Some(16384)
                }
                _ => credential = Some("updated-credential".into()),
            }
            let (settings, changed) = super::save_connection(
                &store,
                &worker,
                super::SystemOneDraft {
                    connection_id: "proxy".into(),
                    connection,
                    credential,
                },
                &vault,
            )
            .unwrap();
            assert!(changed);
            assert_eq!(settings.profiles["proxy"].revision, index + 2);
        }
    }

    #[test]
    fn revision_overflow_fails_before_credential_or_runtime_changes() {
        let (store, worker, vault) = saved_custom(true);
        let mut saved = super::profiles(&store).unwrap();
        saved.profiles.get_mut("proxy").unwrap().revision = u64::MAX;
        super::save_profiles(&store, &saved).unwrap();
        let generation = worker.execution_snapshot().unwrap().generation;
        assert!(
            super::save_connection(
                &store,
                &worker,
                super::SystemOneDraft {
                    connection_id: "proxy".into(),
                    connection: custom_connection(),
                    credential: Some("replacement".into()),
                },
                &vault,
            )
            .is_err()
        );
        assert_eq!(vault.value.borrow().as_deref(), Some("synthetic-secret"));
        assert_eq!(worker.execution_snapshot().unwrap().generation, generation);
        assert_eq!(
            super::profiles(&store).unwrap().profiles["proxy"].revision,
            u64::MAX
        );
    }

    #[test]
    fn legacy_removal_preflights_every_revision_before_marking_or_pausing() {
        for exhausted_id in ["jev", "legacy-alias"] {
            let store =
                Store::open_in_memory(Path::new("synthetic-legacy-removal-overflow")).unwrap();
            let worker = WorkerHandle::default();
            let vault = OfflineVault::default();
            super::save_legacy_key(&store, &worker, Some("synthetic-secret".into()), &vault)
                .unwrap();
            let mut saved = super::profiles(&store).unwrap();
            saved
                .profiles
                .insert("legacy-alias".into(), saved.profiles["jev"].clone());
            saved.profiles.get_mut(exhausted_id).unwrap().revision = u64::MAX;
            super::save_profiles(&store, &saved).unwrap();
            let before = store.internal_value(CONNECTION_PROFILES_KEY);
            let before_runtime = worker.execution_snapshot().unwrap();
            let pending_before = [
                store.internal_value(CREDENTIAL_CHANGE_PENDING_KEY),
                store.internal_value(CONNECTION_PENDING_KEY),
                store.internal_value(super::CONNECTION_REMOVAL_KEY),
            ];
            let writes_before = vault.write_calls.get();
            assert!(
                super::begin_legacy_removal(&store, &worker)
                    .unwrap_err()
                    .contains("revision limit")
            );
            let after_runtime = worker.execution_snapshot().unwrap();
            assert_eq!(after_runtime.generation, before_runtime.generation);
            assert_eq!(after_runtime.connection, before_runtime.connection);
            assert_eq!(after_runtime.credential, before_runtime.credential);
            assert!(super::yaml_discovery_enabled(&store));
            assert_eq!(store.internal_value(CONNECTION_PROFILES_KEY), before);
            assert_eq!(
                [
                    store.internal_value(CREDENTIAL_CHANGE_PENDING_KEY),
                    store.internal_value(CONNECTION_PENDING_KEY),
                    store.internal_value(super::CONNECTION_REMOVAL_KEY),
                ],
                pending_before
            );
            assert_eq!(vault.value.borrow().as_deref(), Some("synthetic-secret"));
            assert_eq!(vault.write_calls.get(), writes_before);
            assert!(vault.deleted_ids.borrow().is_empty());
            let (settings, changed) = super::save_connection(
                &store,
                &worker,
                super::SystemOneDraft {
                    connection_id: "other-profile".into(),
                    connection: custom_connection(),
                    credential: None,
                },
                &vault,
            )
            .unwrap();
            assert!(changed);
            assert_eq!(settings.active_id, "other-profile");
            assert!(worker.is_available());
        }
    }

    #[test]
    fn legacy_removal_commits_its_preflight_revisions_once() {
        let store = Store::open_in_memory(Path::new("synthetic-legacy-removal-preflight")).unwrap();
        let worker = WorkerHandle::default();
        let vault = OfflineVault::default();
        super::save_legacy_key(&store, &worker, Some("synthetic-secret".into()), &vault).unwrap();
        let mut saved = super::profiles(&store).unwrap();
        let mut alias = saved.profiles["jev"].clone();
        alias.revision = 7;
        saved.profiles.insert("legacy-alias".into(), alias);
        super::save_profiles(&store, &saved).unwrap();
        let prepared = super::begin_legacy_removal(&store, &worker).unwrap();
        assert!(!worker.is_available());
        assert_eq!(vault.value.borrow().as_deref(), Some("synthetic-secret"));
        assert_eq!(
            store
                .internal_value(CREDENTIAL_CHANGE_PENDING_KEY)
                .as_deref(),
            Some("true")
        );
        super::finish_legacy_removal(&store, &vault, &prepared).unwrap();
        let saved = super::profiles(&store).unwrap();
        assert_eq!(saved.profiles["jev"].revision, 3);
        assert_eq!(saved.profiles["legacy-alias"].revision, 8);
        assert_eq!(
            store
                .internal_value(CREDENTIAL_CHANGE_PENDING_KEY)
                .as_deref(),
            Some("false")
        );
        assert!(vault.value.borrow().is_none());
    }

    #[test]
    fn shared_credential_updates_advance_every_affected_profile_revision() {
        let (store, worker, vault) = saved_custom(true);
        let mut saved = super::profiles(&store).unwrap();
        saved
            .profiles
            .insert("alias".into(), saved.profiles["proxy"].clone());
        super::save_profiles(&store, &saved).unwrap();
        super::save_connection(
            &store,
            &worker,
            super::SystemOneDraft {
                connection_id: "proxy".into(),
                connection: saved.profiles["proxy"].clone(),
                credential: Some("replacement".into()),
            },
            &vault,
        )
        .unwrap();
        let saved = super::profiles(&store).unwrap();
        assert_eq!(saved.profiles["proxy"].revision, 2);
        assert_eq!(saved.profiles["alias"].revision, 2);
    }

    #[test]
    fn save_switch_restore_and_remove_cover_each_provider_and_custom_response_mode() {
        for (provider, response_mode, optional) in [
            (
                super::SystemOneProvider::Jev,
                crate::jev::config::SystemOneResponseMode::Direct,
                false,
            ),
            (
                super::SystemOneProvider::Cloudflare,
                crate::jev::config::SystemOneResponseMode::Direct,
                false,
            ),
            (
                super::SystemOneProvider::Ollama,
                crate::jev::config::SystemOneResponseMode::Direct,
                true,
            ),
            (
                super::SystemOneProvider::Custom,
                crate::jev::config::SystemOneResponseMode::Direct,
                true,
            ),
            (
                super::SystemOneProvider::Custom,
                crate::jev::config::SystemOneResponseMode::CloudflareEnvelope,
                true,
            ),
        ] {
            for with_credential in [false, true] {
                let store =
                    Store::open_in_memory(Path::new("synthetic-provider-settings")).unwrap();
                store.set_internal_value(YAML_DISCOVERY_ENABLED_KEY, "true");
                let worker = WorkerHandle::default();
                let vault = OfflineVault::default();
                let mut connection = match provider {
                    super::SystemOneProvider::Jev => {
                        crate::jev::config::SystemOneConnection::jev_default()
                    }
                    super::SystemOneProvider::Ollama => crate::jev::config::SystemOneConnection {
                        provider,
                        endpoint: super::SystemOneEndpoint::BaseUrl(
                            "http://127.0.0.1:11434".into(),
                        ),
                        model: "clef-flash".into(),
                        ..custom_connection()
                    },
                    super::SystemOneProvider::Cloudflare => {
                        crate::jev::config::SystemOneConnection {
                            provider,
                            endpoint: super::SystemOneEndpoint::CloudflareAccount(
                                "synthetic-account".into(),
                            ),
                            model: "clef-flash".into(),
                            ..custom_connection()
                        }
                    }
                    super::SystemOneProvider::Custom => custom_connection(),
                };
                connection.response_mode = response_mode;
                connection.credential = None;
                connection.revision = u64::MAX;
                let result = super::save_connection(
                    &store,
                    &worker,
                    super::SystemOneDraft {
                        connection_id: "selected".into(),
                        connection,
                        credential: with_credential.then(|| "synthetic-token".into()),
                    },
                    &vault,
                );
                if !optional && !with_credential {
                    assert!(result.is_err());
                    assert!(!worker.is_available());
                    continue;
                }
                let (settings, changed) = result.unwrap();
                assert!(changed);
                assert_eq!(settings.active_id, "selected");
                assert_eq!(settings.profiles["selected"].revision, 1);
                let snapshot = worker.execution_snapshot().unwrap();
                assert_eq!(snapshot.connection.provider, provider);
                assert_eq!(snapshot.connection.response_mode, response_mode);
                assert_eq!(snapshot.credential.is_some(), with_credential);
                super::switch_connection(&store, &worker, "selected", &vault).unwrap();
                assert_eq!(
                    worker.execution_snapshot().unwrap().generation,
                    snapshot.generation
                );
                let mut saved = super::profiles(&store).unwrap();
                saved.profiles.insert("other".into(), custom_connection());
                super::save_profiles(&store, &saved).unwrap();
                assert!(
                    super::switch_connection(&store, &worker, "other", &vault)
                        .unwrap()
                        .1
                );
                assert!(
                    super::switch_connection(&store, &worker, "selected", &vault)
                        .unwrap()
                        .1
                );
                assert_eq!(
                    worker.system_one_connection(),
                    settings.profiles["selected"]
                );
                let restored = WorkerHandle::default();
                super::restore_saved_key_with_vault(
                    &store,
                    &restored,
                    || panic!("a saved connection slot must not read the legacy key"),
                    &vault,
                )
                .unwrap();
                let restored_snapshot = restored.execution_snapshot().unwrap();
                assert_eq!(restored_snapshot.connection, snapshot.connection);
                assert_eq!(restored_snapshot.credential, snapshot.credential);
                assert!(
                    !store
                        .internal_value(CONNECTION_PROFILES_KEY)
                        .unwrap()
                        .contains("synthetic-token")
                );
                assert_eq!(
                    super::remove_connection_credential(&store, &worker, "selected", &vault)
                        .unwrap(),
                    with_credential.then_some(provider),
                );
                assert_eq!(worker.is_available(), !with_credential);
                assert!(
                    super::profiles(&store).unwrap().profiles["selected"]
                        .credential
                        .is_none()
                );
            }
        }
    }

    #[test]
    fn startup_uses_saved_active_id_instead_of_the_legacy_mirror() {
        let (store, worker, vault) = saved_custom(true);
        super::save_connection(
            &store,
            &worker,
            super::SystemOneDraft {
                connection_id: "keyless".into(),
                connection: custom_connection(),
                credential: None,
            },
            &vault,
        )
        .unwrap();
        store.set_internal_value(super::ACTIVE_CONNECTION_KEY, "jev");
        restore_saved_key(&store, &worker, || {
            panic!("saved activeId must control startup")
        })
        .unwrap();
        assert_eq!(
            worker.system_one_connection().provider,
            super::SystemOneProvider::Custom
        );
        assert!(worker.execution_snapshot().unwrap().credential.is_none());
        assert_eq!(super::profiles(&store).unwrap().active_id, "keyless");
    }

    #[test]
    fn saving_without_a_new_secret_keeps_the_saved_reference_until_explicit_removal() {
        let (store, worker, vault) = saved_custom(true);
        let (_, changed) = super::save_connection(
            &store,
            &worker,
            super::SystemOneDraft {
                connection_id: "proxy".into(),
                connection: custom_connection(),
                credential: None,
            },
            &vault,
        )
        .unwrap();
        assert!(!changed);
        assert_eq!(
            worker.execution_snapshot().unwrap().credential.as_deref(),
            Some("synthetic-secret")
        );
        super::remove_connection_credential(&store, &worker, "proxy", &vault).unwrap();
        assert_eq!(
            super::profiles(&store).unwrap().profiles["proxy"].revision,
            2
        );
    }

    #[test]
    fn legacy_key_save_advances_revision_and_failed_metadata_rolls_back_the_secret() {
        let store = Store::open_in_memory(Path::new("synthetic-legacy-key-update")).unwrap();
        let worker = WorkerHandle::default();
        let vault = OfflineVault::default();
        super::save_legacy_key(&store, &worker, Some("first-key".into()), &vault).unwrap();
        assert_eq!(active_connection(&store).unwrap().revision, 2);
        assert!(worker.is_available());
        fail_profile_writes(&store);
        assert!(
            super::save_legacy_key(&store, &worker, Some("replacement".into()), &vault).is_err()
        );
        assert!(!worker.is_available());
        assert_eq!(vault.value.borrow().as_deref(), Some("first-key"));
        assert_eq!(active_connection(&store).unwrap().revision, 2);
        assert_eq!(
            store
                .internal_value(CREDENTIAL_CHANGE_PENDING_KEY)
                .as_deref(),
            Some("true")
        );
        store
            .lock()
            .execute_batch("DROP TRIGGER fail_profiles")
            .unwrap();
        super::save_legacy_key(&store, &worker, Some("replacement".into()), &vault).unwrap();
        assert_eq!(active_connection(&store).unwrap().revision, 3);
        assert!(worker.is_available());
    }

    fn insert_unfinished_assessment(store: &Store, state: &str) {
        store
            .lock()
            .execute(
                "INSERT INTO burn_check_candidate (
                candidate_id, check_id, activation_scope_key, generation, branch_key,
                session_key_json, trigger_position, selected_publication_head,
                dependencies_json, frozen_input_json, input_fingerprint, state,
                created_at_epoch, updated_at_epoch)
             VALUES ('synthetic-candidate', 'synthetic-check', 'synthetic-scope', 1,
                'synthetic-branch', '{}', 1, 1, '[]', '{}', 'synthetic-input', ?1, 1, 1)",
                [state],
            )
            .unwrap();
    }

    fn unfinished_assessment_state(store: &Store) -> String {
        store
            .lock()
            .query_row(
                "SELECT state FROM burn_check_candidate WHERE candidate_id = 'synthetic-candidate'",
                [],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn insert_accepted_status_and_source(store: &Store) {
        store
            .lock()
            .execute_batch(
                "INSERT INTO burn_check_activation (
                check_id, activation_scope_key, definition_revision, enabled,
                generation, bootstrap_sequence, applied_sequence, updated_at_epoch,
                checkpoint_json, lifecycle_json)
             VALUES ('synthetic-check', 'synthetic-scope', 'synthetic-revision', 1,
                1, 0, 1, 1, '{\"sequence\":1}', '{}');
             INSERT INTO burn_check_observation (
                check_id, activation_scope_key, observation_id, session_key_json,
                publication_sequence, source_format, native_identity, native_revision,
                evidence_version, removed, payload_json)
             VALUES ('synthetic-check', 'synthetic-scope', 'synthetic-observation', '{}',
                1, 'synthetic-format', 'synthetic-native-id', 1, 'synthetic-evidence',
                0, '{\"text\":\"synthetic source\"}');
             INSERT INTO burn_check_candidate (
                candidate_id, check_id, activation_scope_key, generation, branch_key,
                session_key_json, trigger_position, selected_publication_head,
                dependencies_json, frozen_input_json, input_fingerprint, state,
                created_at_epoch, updated_at_epoch)
             SELECT 'synthetic-accepted', check_id, activation_scope_key, generation,
                branch_key, session_key_json, 0, selected_publication_head,
                dependencies_json, frozen_input_json, 'synthetic-accepted-input',
                'accepted', 1, 1
             FROM burn_check_candidate WHERE candidate_id = 'synthetic-candidate';
             INSERT INTO burn_check_assessment_history (
                assessment_id, candidate_id, check_id, activation_scope_key, generation,
                branch_key, trigger_position, outcome, actions_json, evidence_valid,
                accepted_at_epoch)
             VALUES ('synthetic-assessment', 'synthetic-accepted', 'synthetic-check',
                'synthetic-scope', 1, 'synthetic-branch', 0, 'assessed', '{}', 1, 1);
             INSERT INTO burn_check_current_status (
                check_id, activation_scope_key, branch_key, generation, status,
                assessment_id, position, evidence_valid, updated_at_epoch)
             VALUES ('synthetic-check', 'synthetic-scope', 'synthetic-branch', 1,
                'success', 'synthetic-assessment', 0, 1, 1);",
            )
            .unwrap();
    }

    fn source_and_status_snapshot(store: &Store) -> Vec<String> {
        let connection = store.lock();
        [
            "burn_check_activation",
            "burn_check_observation",
            "burn_check_current_status",
        ]
        .into_iter()
        .map(|table| {
            let mut statement = connection
                .prepare(&format!("SELECT * FROM {table}"))
                .unwrap();
            let columns = statement.column_count();
            let rows = statement
                .query_map([], |row| {
                    (0..columns)
                        .map(|column| row.get::<_, rusqlite::types::Value>(column))
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            format!("{table}: {rows:?}")
        })
        .collect()
    }

    fn assert_paused_selection_noops(store: &Store, worker: &WorkerHandle, vault: &OfflineVault) {
        let draft = super::SystemOneDraft {
            connection_id: "proxy".into(),
            connection: active_connection(store).unwrap(),
            credential: None,
        };
        assert!(
            !super::save_connection(store, worker, draft, vault)
                .unwrap()
                .1
        );
        assert!(!worker.is_available());
        assert!(
            !super::switch_connection(store, worker, "proxy", vault)
                .unwrap()
                .1
        );
        assert!(!worker.is_available());
        assert_eq!(unfinished_assessment_state(store), "pending");
    }

    fn run_connection_transition(
        operation: &str,
        store: &Store,
        worker: &WorkerHandle,
        vault: &OfflineVault,
    ) -> Result<(), &'static str> {
        match operation {
            "save" => {
                let mut connection = custom_connection();
                connection.model = "replacement-model".into();
                super::save_connection(
                    store,
                    worker,
                    super::SystemOneDraft {
                        connection_id: "proxy".into(),
                        connection,
                        credential: Some("replacement-secret".into()),
                    },
                    vault,
                )
                .map(|_| ())
            }
            "switch" => super::switch_connection(store, worker, "keyless", vault).map(|_| ()),
            "remove" => {
                super::remove_connection_credential(store, worker, "proxy", vault).map(|_| ())
            }
            "pause" => super::pause_model_checks(store, worker),
            _ => unreachable!(),
        }
    }

    #[test]
    fn meaningful_transitions_cancel_unfinished_assessments_before_installation() {
        for operation in ["save", "switch", "remove", "pause"] {
            for state in ["pending", "claimed", "dispatching", "answered"] {
                let (store, worker, vault) = saved_custom(true);
                let mut saved = super::profiles(&store).unwrap();
                saved.profiles.insert("keyless".into(), custom_connection());
                super::save_profiles(&store, &saved).unwrap();
                insert_unfinished_assessment(&store, state);
                let generation = worker.execution_snapshot().unwrap().generation;
                run_connection_transition(operation, &store, &worker, &vault).unwrap();
                assert_eq!(unfinished_assessment_state(&store), "cancelled");
                assert!(!worker.key_is_current(generation));
                if operation == "switch" {
                    super::switch_connection(&store, &worker, "proxy", &vault).unwrap();
                    assert_eq!(unfinished_assessment_state(&store), "cancelled");
                }
            }
        }
    }

    #[test]
    fn cancellation_failures_keep_each_transition_paused_without_replacing_the_secret() {
        for operation in ["save", "switch", "remove", "pause"] {
            let (store, worker, vault) = saved_custom(true);
            let mut saved = super::profiles(&store).unwrap();
            saved.profiles.insert("keyless".into(), custom_connection());
            super::save_profiles(&store, &saved).unwrap();
            let before = store.internal_value(CONNECTION_PROFILES_KEY);
            insert_unfinished_assessment(&store, "pending");
            store.lock().execute_batch(
                "CREATE TEMP TRIGGER reject_settings_cancel BEFORE UPDATE ON burn_check_candidate
                 WHEN NEW.state = 'cancelled'
                 BEGIN SELECT RAISE(ABORT, 'injected cancellation failure'); END;",
            ).unwrap();
            let error = run_connection_transition(operation, &store, &worker, &vault).unwrap_err();
            assert!(error.contains("cancel unfinished"));
            assert!(!worker.is_available());
            assert!(worker.execution_snapshot().is_none());
            assert_eq!(unfinished_assessment_state(&store), "pending");
            assert_eq!(store.internal_value(CONNECTION_PROFILES_KEY), before);
            assert_eq!(vault.value.borrow().as_deref(), Some("synthetic-secret"));
            assert!(vault.deleted_ids.borrow().is_empty());
        }
    }

    #[test]
    fn failed_master_pause_stays_paused_through_noops_restart_and_failed_enable_until_retry() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path()).unwrap();
        let worker = WorkerHandle::default();
        let vault = OfflineVault::default();
        store.set_internal_value(YAML_DISCOVERY_ENABLED_KEY, "true");
        super::save_connection(
            &store,
            &worker,
            super::SystemOneDraft {
                connection_id: "proxy".into(),
                connection: custom_connection(),
                credential: Some("synthetic-secret".into()),
            },
            &vault,
        )
        .unwrap();
        insert_unfinished_assessment(&store, "pending");
        insert_accepted_status_and_source(&store);
        let source_and_status = source_and_status_snapshot(&store);
        let profiles_before = store.internal_value(CONNECTION_PROFILES_KEY);
        let pending_before = store.internal_value(CONNECTION_PENDING_KEY);
        store
            .lock()
            .execute_batch(
                "CREATE TRIGGER reject_master_pause_cancel BEFORE UPDATE ON burn_check_candidate
             WHEN NEW.state = 'cancelled'
             BEGIN SELECT RAISE(ABORT, 'injected cancellation failure'); END;",
            )
            .unwrap();
        assert!(super::pause_model_checks(&store, &worker).is_err());
        assert!(!super::yaml_discovery_enabled(&store));
        assert_paused_selection_noops(&store, &worker, &vault);
        assert!(
            super::enable_model_checks(
                &store,
                &worker,
                active_connection(&store).unwrap(),
                Some("synthetic-secret".into()),
            )
            .is_err()
        );
        assert!(!worker.is_available());
        assert!(!super::yaml_discovery_enabled(&store));
        assert_eq!(source_and_status_snapshot(&store), source_and_status);
        assert_eq!(store.internal_value(CONNECTION_PENDING_KEY), pending_before);
        drop(worker);
        drop(store);

        let store = Store::open(directory.path()).unwrap();
        let worker = WorkerHandle::default();
        vault.fail_read.set(true);
        super::restore_saved_key_with_vault(
            &store,
            &worker,
            || panic!("paused restart must not read the legacy key"),
            &vault,
        )
        .unwrap();
        assert!(vault.fail_read.get());
        vault.fail_read.set(false);
        assert_paused_selection_noops(&store, &worker, &vault);
        assert!(
            super::enable_model_checks(
                &store,
                &worker,
                active_connection(&store).unwrap(),
                Some("synthetic-secret".into()),
            )
            .is_err()
        );
        assert!(!worker.is_available());
        assert!(!super::yaml_discovery_enabled(&store));
        assert_eq!(unfinished_assessment_state(&store), "pending");
        assert_eq!(source_and_status_snapshot(&store), source_and_status);

        store
            .lock()
            .execute_batch("DROP TRIGGER reject_master_pause_cancel")
            .unwrap();
        super::pause_model_checks(&store, &worker).unwrap();
        assert!(!worker.is_available());
        assert!(!super::yaml_discovery_enabled(&store));
        assert_eq!(unfinished_assessment_state(&store), "cancelled");
        assert_eq!(source_and_status_snapshot(&store), source_and_status);
        super::enable_model_checks(
            &store,
            &worker,
            active_connection(&store).unwrap(),
            Some("synthetic-secret".into()),
        )
        .unwrap();
        assert!(worker.is_available());
        assert_eq!(unfinished_assessment_state(&store), "cancelled");
        assert_eq!(source_and_status_snapshot(&store), source_and_status);
        assert_eq!(
            store.internal_value(CONNECTION_PROFILES_KEY),
            profiles_before
        );
        assert_eq!(store.internal_value(CONNECTION_PENDING_KEY), pending_before);
        let cancellations: u64 = store
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM burn_check_assessment_history
             WHERE candidate_id = 'synthetic-candidate' AND outcome = 'cancelled'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(cancellations, 1);
        assert!(
            !super::switch_connection(&store, &worker, "proxy", &vault)
                .unwrap()
                .1
        );
        assert_eq!(unfinished_assessment_state(&store), "cancelled");
    }

    #[test]
    fn master_pause_failure_keeps_existing_profile_pending_markers() {
        let (store, worker, _) = saved_custom(true);
        store.set_internal_value(CONNECTION_PENDING_KEY, "other-profile");
        store.set_internal_value(super::CONNECTION_REMOVAL_KEY, "other-removal");
        insert_unfinished_assessment(&store, "pending");
        store.lock().execute_batch(
            "CREATE TEMP TRIGGER reject_master_pause_cancel BEFORE UPDATE ON burn_check_candidate
             WHEN NEW.state = 'cancelled'
             BEGIN SELECT RAISE(ABORT, 'injected cancellation failure'); END;",
        ).unwrap();
        assert!(super::pause_model_checks(&store, &worker).is_err());
        assert!(!worker.is_available());
        assert!(!super::yaml_discovery_enabled(&store));
        assert_eq!(
            store.internal_value(CONNECTION_PENDING_KEY).as_deref(),
            Some("other-profile")
        );
        assert_eq!(
            store
                .internal_value(super::CONNECTION_REMOVAL_KEY)
                .as_deref(),
            Some("other-removal")
        );
    }

    #[test]
    fn headless_settings_switch_and_restore_use_keyless_profiles_in_a_file_store() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path()).unwrap();
        let worker = WorkerHandle::default();
        let first = custom_connection();
        let mut second = first.clone();
        second.model = "other-model".into();
        super::headless::setup_keyless_profiles(
            &store,
            std::collections::BTreeMap::from([
                ("first".into(), first.clone()),
                ("second".into(), second.clone()),
            ]),
            "first",
            true,
        )
        .unwrap();
        super::headless::restore_keyless_profile(&store, &worker).unwrap();
        let snapshot = worker.execution_snapshot().unwrap();
        assert_eq!(snapshot.connection, first);
        assert!(snapshot.credential.is_none());
        assert!(super::headless::switch_keyless_profile(&store, &worker, "second").unwrap());
        assert_eq!(worker.execution_snapshot().unwrap().connection, second);
        super::headless::pause_model_checks(&store, &worker).unwrap();
        assert!(!worker.is_available());
        drop(worker);
        drop(store);
        let store = Store::open(directory.path()).unwrap();
        let worker = WorkerHandle::default();
        super::headless::restore_keyless_profile(&store, &worker).unwrap();
        assert_eq!(worker.system_one_connection(), second);
        assert!(!worker.is_available());
        assert!(!super::yaml_discovery_enabled(&store));
    }

    #[test]
    fn repeated_saved_selection_does_not_cancel_current_work_or_clear_auth_rejection() {
        let (store, worker, vault) = saved_custom(true);
        insert_unfinished_assessment(&store, "pending");
        let generation = worker.execution_snapshot().unwrap().generation;
        let draft = super::SystemOneDraft {
            connection_id: "proxy".into(),
            connection: active_connection(&store).unwrap(),
            credential: None,
        };
        assert!(
            !super::save_connection(&store, &worker, draft.clone(), &vault)
                .unwrap()
                .1
        );
        assert!(
            !super::switch_connection(&store, &worker, "proxy", &vault)
                .unwrap()
                .1
        );
        assert_eq!(unfinished_assessment_state(&store), "pending");
        assert!(worker.key_is_current(generation));
        worker.reject_authentication(&store, generation).unwrap();
        assert!(
            !super::save_connection(&store, &worker, draft, &vault)
                .unwrap()
                .1
        );
        assert!(
            !super::switch_connection(&store, &worker, "proxy", &vault)
                .unwrap()
                .1
        );
        assert!(!worker.is_available());
        assert_eq!(
            store.internal_value(AUTH_REJECTED_KEY).as_deref(),
            Some("true")
        );
    }

    #[test]
    fn pending_secret_update_requires_the_same_profile_and_a_new_backend_revision() {
        let (store, worker, vault) = saved_custom(true);
        fail_profile_writes(&store);
        vault.fail_write_at.set(Some(vault.write_calls.get() + 2));
        let draft = super::SystemOneDraft {
            connection_id: "proxy".into(),
            connection: custom_connection(),
            credential: Some("replacement".into()),
        };
        assert!(super::save_connection(&store, &worker, draft, &vault).is_err());
        assert_eq!(vault.value.borrow().as_deref(), Some("replacement"));
        assert!(!worker.is_available());
        assert!(super::switch_connection(&store, &worker, "proxy", &vault).is_err());
        assert!(super::remove_connection_credential(&store, &worker, "proxy", &vault).is_err());
        assert!(
            super::save_connection(
                &store,
                &worker,
                super::SystemOneDraft {
                    connection_id: "other".into(),
                    connection: custom_connection(),
                    credential: None,
                },
                &vault,
            )
            .is_err()
        );
        store
            .lock()
            .execute_batch("DROP TRIGGER fail_profiles")
            .unwrap();
        let (settings, changed) = super::save_connection(
            &store,
            &worker,
            super::SystemOneDraft {
                connection_id: "proxy".into(),
                connection: active_connection(&store).unwrap(),
                credential: None,
            },
            &vault,
        )
        .unwrap();
        assert!(changed);
        assert_eq!(settings.profiles["proxy"].revision, 2);
        assert_eq!(
            worker.execution_snapshot().unwrap().credential.as_deref(),
            Some("replacement")
        );
    }

    #[test]
    fn failed_credential_write_cannot_resume_a_rejected_connection() {
        let (store, worker, vault) = saved_custom(true);
        let generation = worker.execution_snapshot().unwrap().generation;
        worker.reject_authentication(&store, generation).unwrap();
        vault.fail_write.set(true);
        assert!(
            super::save_connection(
                &store,
                &worker,
                super::SystemOneDraft {
                    connection_id: "proxy".into(),
                    connection: custom_connection(),
                    credential: Some("replacement".into()),
                },
                &vault,
            )
            .is_err()
        );
        assert!(!worker.is_available());
        assert_eq!(
            store.internal_value(AUTH_REJECTED_KEY).as_deref(),
            Some("true")
        );
        assert_eq!(vault.value.borrow().as_deref(), Some("synthetic-secret"));
    }

    #[test]
    fn rejected_startup_restores_the_selected_profile_without_reading_its_secret() {
        let (store, worker, vault) = saved_custom(true);
        let connection = active_connection(&store).unwrap();
        store.set_internal_value(AUTH_REJECTED_KEY, "true");
        vault.fail_read.set(true);
        let restored = WorkerHandle::default();
        super::restore_saved_key_with_vault(
            &store,
            &restored,
            || panic!("a rejected connection must not read the legacy key"),
            &vault,
        )
        .unwrap();
        assert_eq!(restored.system_one_connection(), connection);
        assert!(restored.execution_snapshot().is_none());
        assert!(vault.fail_read.get());
        assert!(worker.is_available());
    }

    fn fail_profile_writes(store: &Store) {
        store
            .lock()
            .execute_batch(
                "CREATE TEMP TRIGGER fail_profiles BEFORE UPDATE ON setting
             WHEN NEW.key = 'internal:smartChecksConnectionsV1'
             BEGIN SELECT RAISE(ABORT, 'injected profile failure'); END;",
            )
            .unwrap();
    }

    #[test]
    fn active_credential_removal_pauses_checks_detaches_metadata_and_is_idempotent() {
        let (store, worker, vault) = saved_custom(true);
        assert!(worker.is_available());
        store.set_internal_value(AUTH_REJECTED_KEY, "true");
        super::remove_connection_credential(&store, &worker, "proxy", &vault).unwrap();
        assert!(!worker.is_available());
        assert!(!super::yaml_discovery_enabled(&store));
        assert!(active_connection(&store).unwrap().credential.is_none());
        assert!(vault.value.borrow().is_none());
        assert_eq!(
            store.internal_value(AUTH_REJECTED_KEY).as_deref(),
            Some("false")
        );
        super::remove_connection_credential(&store, &worker, "proxy", &vault).unwrap();
        assert_eq!(*vault.deleted_ids.borrow(), vec![Some("proxy".into())]);
        restore_saved_key(&store, &worker, || {
            panic!("removed secret must not be read")
        })
        .unwrap();
        assert!(!worker.is_available());
    }

    #[test]
    fn removing_an_inactive_shared_reference_preserves_the_active_credential() {
        let (store, worker, vault) = saved_custom(true);
        let mut saved = super::profiles(&store).unwrap();
        saved
            .profiles
            .insert("alias".into(), saved.profiles["proxy"].clone());
        super::save_profiles(&store, &saved).unwrap();
        super::remove_connection_credential(&store, &worker, "alias", &vault).unwrap();
        assert!(worker.is_available());
        assert!(active_connection(&store).unwrap().credential.is_some());
        assert!(
            super::profiles(&store).unwrap().profiles["alias"]
                .credential
                .is_none()
        );
        assert!(vault.deleted_ids.borrow().is_empty());
        assert_eq!(vault.value.borrow().as_deref(), Some("synthetic-secret"));
    }

    #[test]
    fn inactive_removal_deletes_the_referenced_key_slot_without_changing_active_runtime() {
        let (store, worker, vault) = saved_custom(true);
        super::save_connection(
            &store,
            &worker,
            super::SystemOneDraft {
                connection_id: "keyless".into(),
                connection: custom_connection(),
                credential: None,
            },
            &vault,
        )
        .unwrap();
        let mut saved = super::profiles(&store).unwrap();
        saved.profiles.get_mut("proxy").unwrap().credential = Some(
            crate::jev::config::CredentialReference::Connection("key-slot".into()),
        );
        super::save_profiles(&store, &saved).unwrap();
        let active = worker.system_one_connection();
        super::remove_connection_credential(&store, &worker, "proxy", &vault).unwrap();
        assert!(worker.is_available());
        assert_eq!(worker.system_one_connection(), active);
        assert_eq!(super::profiles(&store).unwrap().active_id, "keyless");
        assert!(
            super::profiles(&store).unwrap().profiles["proxy"]
                .credential
                .is_none()
        );
        assert_eq!(*vault.deleted_ids.borrow(), vec![Some("key-slot".into())]);
    }

    #[test]
    fn removal_preflight_and_pending_marker_failures_do_not_remove_keys_or_suspend_runtime() {
        let (store, worker, vault) = saved_custom(true);
        let before = store.internal_value(CONNECTION_PROFILES_KEY);
        vault.fail_read.set(true);
        assert!(super::remove_connection_credential(&store, &worker, "proxy", &vault).is_err());
        assert!(worker.is_available());
        store
            .lock()
            .execute_batch(
                "CREATE TEMP TRIGGER fail_removal_marker BEFORE INSERT ON setting
             WHEN NEW.key = 'internal:smartChecksCredentialRemovalPendingV1'
             BEGIN SELECT RAISE(ABORT, 'injected marker failure'); END;",
            )
            .unwrap();
        assert!(super::remove_connection_credential(&store, &worker, "proxy", &vault).is_err());
        assert!(worker.is_available());
        assert!(super::yaml_discovery_enabled(&store));
        assert_eq!(store.internal_value(CONNECTION_PROFILES_KEY), before);
        assert_eq!(vault.value.borrow().as_deref(), Some("synthetic-secret"));
        assert!(vault.deleted_ids.borrow().is_empty());
    }

    #[test]
    fn removal_finalization_failure_rolls_back_all_metadata_and_restores_the_key() {
        let (store, worker, vault) = saved_custom(true);
        let before = store.internal_value(CONNECTION_PROFILES_KEY);
        store
            .lock()
            .execute_batch(
                "CREATE TEMP TRIGGER fail_removal_finalize BEFORE UPDATE ON setting
             WHEN NEW.key = 'internal:smartChecksCredentialRemovalPendingV1' AND NEW.value = ''
             BEGIN SELECT RAISE(ABORT, 'injected finalization failure'); END;",
            )
            .unwrap();
        assert!(super::remove_connection_credential(&store, &worker, "proxy", &vault).is_err());
        assert_eq!(store.internal_value(CONNECTION_PROFILES_KEY), before);
        assert_eq!(
            store
                .internal_value(super::CONNECTION_REMOVAL_KEY)
                .as_deref(),
            Some("proxy")
        );
        assert_eq!(vault.value.borrow().as_deref(), Some("synthetic-secret"));
        assert!(!worker.is_available());
        store
            .lock()
            .execute_batch("DROP TRIGGER fail_removal_finalize")
            .unwrap();
        super::remove_connection_credential(&store, &worker, "proxy", &vault).unwrap();
        assert!(vault.value.borrow().is_none());
        assert!(active_connection(&store).unwrap().credential.is_none());
    }

    #[test]
    fn pending_removal_blocks_replacement_and_restore_until_the_same_removal_retries() {
        let (store, worker, vault) = saved_custom(true);
        vault.fail_delete.set(true);
        assert!(super::remove_connection_credential(&store, &worker, "proxy", &vault).is_err());
        assert!(!worker.is_available());
        assert!(active_connection(&store).unwrap().credential.is_some());
        assert!(
            restore_saved_key(&store, &worker, || panic!("pending removal read a key")).is_err()
        );
        assert!(
            super::save_connection(
                &store,
                &worker,
                super::SystemOneDraft {
                    connection_id: "proxy".into(),
                    connection: custom_connection(),
                    credential: Some("replacement".into()),
                },
                &vault
            )
            .is_err()
        );
        assert!(super::switch_connection(&store, &worker, "proxy", &vault).is_err());
        assert!(super::remove_connection_credential(&store, &worker, "other", &vault).is_err());
        assert_eq!(vault.value.borrow().as_deref(), Some("synthetic-secret"));
        super::remove_connection_credential(&store, &worker, "proxy", &vault).unwrap();
        super::save_connection(
            &store,
            &worker,
            super::SystemOneDraft {
                connection_id: "proxy".into(),
                connection: custom_connection(),
                credential: Some("replacement".into()),
            },
            &vault,
        )
        .unwrap();
        super::remove_connection_credential(&store, &worker, "alias", &vault).unwrap_err();
        assert_eq!(vault.value.borrow().as_deref(), Some("replacement"));
    }

    #[test]
    fn removal_metadata_failure_restores_the_secret_and_rollback_failure_stays_retryable() {
        for rollback_fails in [false, true] {
            let (store, worker, vault) = saved_custom(true);
            fail_profile_writes(&store);
            vault.fail_write.set(rollback_fails);
            let error =
                super::remove_connection_credential(&store, &worker, "proxy", &vault).unwrap_err();
            assert_eq!(error.contains("restore"), rollback_fails);
            assert!(active_connection(&store).unwrap().credential.is_some());
            assert!(!worker.is_available());
            assert_eq!(vault.value.borrow().is_none(), rollback_fails);
            assert!(
                restore_saved_key(&store, &worker, || panic!(
                    "incomplete removal reads no keys"
                ))
                .is_err()
            );
            store
                .lock()
                .execute_batch("DROP TRIGGER fail_profiles")
                .unwrap();
            super::remove_connection_credential(&store, &worker, "proxy", &vault).unwrap();
            assert!(active_connection(&store).unwrap().credential.is_none());
            assert!(vault.value.borrow().is_none());
        }
    }

    #[test]
    fn missing_or_unreadable_saved_credentials_do_not_mutate_save_or_switch_state() {
        for missing in [true, false] {
            let (store, worker, vault) = saved_custom(true);
            let mut saved = super::profiles(&store).unwrap();
            saved
                .profiles
                .insert("target".into(), saved.profiles["proxy"].clone());
            super::save_profiles(&store, &saved).unwrap();
            let before = store.internal_value(CONNECTION_PROFILES_KEY);
            let connection = active_connection(&store).unwrap();
            if missing {
                vault.value.borrow_mut().take();
            } else {
                vault.fail_read.set(true);
            }
            assert!(
                super::save_connection(
                    &store,
                    &worker,
                    super::SystemOneDraft {
                        connection_id: "new".into(),
                        connection: connection.clone(),
                        credential: None,
                    },
                    &vault
                )
                .is_err()
            );
            assert_eq!(store.internal_value(CONNECTION_PROFILES_KEY), before);
            assert!(worker.is_available());
            if !missing {
                vault.fail_read.set(true);
            }
            assert!(super::switch_connection(&store, &worker, "target", &vault).is_err());
            assert_eq!(store.internal_value(CONNECTION_PROFILES_KEY), before);
            assert_eq!(
                store
                    .internal_value(super::CONNECTION_PENDING_KEY)
                    .as_deref(),
                Some("")
            );
            assert_eq!(worker.system_one_connection(), connection);
            assert!(worker.is_available());
        }
    }

    #[test]
    fn failed_save_metadata_rolls_back_the_key_and_does_not_publish_draft_profiles() {
        let (store, worker, vault) = saved_custom(true);
        let before = store.internal_value(CONNECTION_PROFILES_KEY);
        fail_profile_writes(&store);
        let draft = super::SystemOneDraft {
            connection_id: "proxy".into(),
            connection: custom_connection(),
            credential: Some("replacement".into()),
        };
        assert!(super::save_connection(&store, &worker, draft.clone(), &vault).is_err());
        assert_eq!(store.internal_value(CONNECTION_PROFILES_KEY), before);
        assert_eq!(vault.value.borrow().as_deref(), Some("synthetic-secret"));
        assert!(!worker.is_available());
        assert!(
            restore_saved_key(&store, &worker, || panic!("pending update reads no keys")).is_err()
        );
        store
            .lock()
            .execute_batch("DROP TRIGGER fail_profiles")
            .unwrap();
        super::save_connection(&store, &worker, draft, &vault).unwrap();
        assert!(worker.is_available());
        assert_eq!(vault.value.borrow().as_deref(), Some("replacement"));
        assert!(
            !store
                .internal_value(CONNECTION_PROFILES_KEY)
                .unwrap()
                .contains("replacement")
        );
    }

    #[test]
    fn null_draft_uses_only_its_explicit_reference_and_keyless_drafts_skip_the_vault() {
        let vault = OfflineVault::default();
        let keyless = custom_connection();
        vault.fail_read.set(true);
        assert_eq!(
            super::resolve_credential(&keyless, None, &vault).unwrap(),
            None
        );
        let mut referenced = keyless.clone();
        referenced.credential = Some(crate::jev::config::CredentialReference::Connection(
            "proxy".into(),
        ));
        assert!(super::resolve_credential(&referenced, None, &vault).is_err());
        assert!(super::resolve_credential(&referenced, None, &vault).is_err());
        *vault.value.borrow_mut() = Some("saved-secret".into());
        assert_eq!(
            super::resolve_credential(&referenced, None, &vault)
                .unwrap()
                .as_deref(),
            Some("saved-secret")
        );
        assert_eq!(
            super::resolve_credential(&referenced, Some("draft-secret".into()), &vault)
                .unwrap()
                .as_deref(),
            Some("draft-secret")
        );
        assert!(super::resolve_credential(&keyless, Some("".into()), &vault).is_err());
        assert!(
            !serde_json::to_string(&referenced)
                .unwrap()
                .contains("saved-secret")
        );
    }

    #[test]
    fn failed_save_rollback_reports_failure_and_keeps_launch_blocked_until_retry() {
        let (store, worker, vault) = saved_custom(true);
        fail_profile_writes(&store);
        vault.fail_write_at.set(Some(vault.write_calls.get() + 2));
        let draft = super::SystemOneDraft {
            connection_id: "proxy".into(),
            connection: custom_connection(),
            credential: Some("replacement".into()),
        };
        let error = super::save_connection(&store, &worker, draft.clone(), &vault).unwrap_err();
        assert!(error.contains("restore"));
        assert_eq!(vault.value.borrow().as_deref(), Some("replacement"));
        assert!(!worker.is_available());
        assert!(
            restore_saved_key(&store, &worker, || panic!("failed rollback reads no keys")).is_err()
        );
        store
            .lock()
            .execute_batch("DROP TRIGGER fail_profiles")
            .unwrap();
        super::save_connection(&store, &worker, draft, &vault).unwrap();
        assert!(worker.is_available());
    }

    #[test]
    fn finalization_failure_keeps_the_staged_profile_disabled_and_can_be_retried() {
        let (store, worker, vault) = saved_custom(true);
        store
            .lock()
            .execute_batch(
                "CREATE TEMP TRIGGER fail_finalize BEFORE UPDATE ON setting
             WHEN NEW.key = 'internal:smartChecksConnectionChangePendingV1' AND NEW.value = ''
             BEGIN SELECT RAISE(ABORT, 'injected finalization failure'); END;",
            )
            .unwrap();
        let draft = super::SystemOneDraft {
            connection_id: "keyless".into(),
            connection: custom_connection(),
            credential: None,
        };
        assert!(super::save_connection(&store, &worker, draft.clone(), &vault).is_err());
        assert_eq!(super::profiles(&store).unwrap().active_id, "keyless");
        assert!(!worker.is_available());
        assert!(
            restore_saved_key(&store, &worker, || panic!("staged update reads no keys")).is_err()
        );
        store
            .lock()
            .execute_batch("DROP TRIGGER fail_finalize")
            .unwrap();
        super::save_connection(&store, &worker, draft, &vault).unwrap();
        assert!(worker.is_available());
    }

    #[test]
    fn paused_save_switch_and_launch_keep_keyless_checks_disabled_until_reenabled() {
        let (store, worker, vault) = saved_custom(false);
        assert!(!worker.is_available());
        let connection = custom_connection();
        super::save_connection(
            &store,
            &worker,
            super::SystemOneDraft {
                connection_id: "keyless".into(),
                connection: connection.clone(),
                credential: None,
            },
            &vault,
        )
        .unwrap();
        assert!(!worker.is_available());
        super::switch_connection(&store, &worker, "keyless", &vault).unwrap();
        assert!(!worker.is_available());
        restore_saved_key(&store, &worker, || panic!("paused launch reads no keys")).unwrap();
        assert!(!worker.is_available());
        store.set_internal_value(YAML_DISCOVERY_ENABLED_KEY, "true");
        super::switch_connection(&store, &worker, "keyless", &vault).unwrap();
        assert!(worker.is_available());
        worker.suspend_system_one();
        assert!(!worker.is_available());
        restore_saved_key(&store, &worker, || panic!("keyless launch reads no keys")).unwrap();
        assert!(worker.is_available());
    }

    #[tokio::test]
    async fn test_and_refresh_send_saved_authentication_but_keep_keyless_requests_keyless() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        for saved_secret in [None, Some("synthetic-local-token")] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let connection = crate::jev::config::SystemOneConnection {
                provider: crate::jev::config::SystemOneProvider::Ollama,
                endpoint: crate::jev::config::SystemOneEndpoint::BaseUrl(format!(
                    "http://{}",
                    listener.local_addr().unwrap()
                )),
                model: "clef-flash".into(),
                credential: saved_secret
                    .map(|_| crate::jev::config::CredentialReference::Connection("local".into())),
                ..custom_connection()
            };
            let server = tokio::spawn(async move {
                let discovery = [
                    ("/api/version", serde_json::json!({"version":"0.35.0"})),
                    (
                        "/api/tags",
                        serde_json::json!({"models":[{"name":"clef-flash:latest", "digest":"synthetic-digest"}]}),
                    ),
                    (
                        "/api/show",
                        serde_json::json!({"capabilities":["decision"], "model_info":{"general.architecture":"clef","clef.context_length":16384}}),
                    ),
                    (
                        "/api/ps",
                        serde_json::json!({"models":[{"name":"clef-flash:latest","digest":"synthetic-digest","context_length":8192}]}),
                    ),
                ];
                for (route, body) in discovery.clone().into_iter().chain([(
                    "/v1/systemone",
                    serde_json::json!({"model":"clef-flash", "answers":{"validation":{"type":"noul","noul":0.9}}, "usage":{"input_tokens":4,"output_tokens":1}}),
                )]).chain(discovery) {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let mut request = Vec::new();
                    loop {
                        let mut buffer = [0; 4096];
                        let count = socket.read(&mut buffer).await.unwrap();
                        assert!(count > 0);
                        request.extend_from_slice(&buffer[..count]);
                        assert!(request.len() < 65536);
                        if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n")
                        {
                            let headers =
                                String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                            let body_bytes = headers
                                .lines()
                                .find_map(|line| line.strip_prefix("content-length: "))
                                .map(|length| length.parse::<usize>().unwrap())
                                .unwrap_or(0);
                            if request.len() >= end + 4 + body_bytes {
                                assert!(headers.contains(route));
                                match saved_secret {
                                    Some(secret) => assert!(
                                        headers
                                            .contains(&format!("authorization: bearer {secret}"))
                                    ),
                                    None => assert!(!headers.contains("authorization:")),
                                }
                                break;
                            }
                        }
                    }
                    let body = body.to_string();
                    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
                }
            });
            let vault = OfflineVault::default();
            *vault.value.borrow_mut() = saved_secret.map(str::to_owned);
            let credential = super::resolve_credential(&connection, None, &vault).unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                super::test_connection(super::SystemOneDraft {
                    connection_id: "local".into(),
                    connection: connection.clone(),
                    credential: credential.clone(),
                })
                .await
                .unwrap();
                let capabilities = super::refresh_limits(connection.clone(), credential)
                    .await
                    .unwrap();
                assert_eq!(
                    capabilities.model_revision.as_deref(),
                    Some("synthetic-digest")
                );
                assert_eq!(capabilities.runtime_context_tokens.value, Some(8192));
                server.await.unwrap();
            })
            .await
            .unwrap();
            assert!(
                !serde_json::to_string(&connection)
                    .unwrap()
                    .contains("synthetic-local-token")
            );
        }
    }

    #[tokio::test]
    async fn discovery_failures_cannot_use_manual_limits_or_change_the_selected_profile() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        for testing in [false, true] {
            for authentication_rejected in [false, true] {
                let (store, worker, vault) = saved_custom(true);
                let before = store.internal_value(CONNECTION_PROFILES_KEY);
                let generation = worker.execution_snapshot().unwrap().generation;
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let connection = crate::jev::config::SystemOneConnection {
                    provider: super::SystemOneProvider::Ollama,
                    endpoint: super::SystemOneEndpoint::BaseUrl(format!(
                        "http://{}",
                        listener.local_addr().unwrap()
                    )),
                    model: "clef-flash".into(),
                    context_override: Some(crate::jev::config::ContextLimitOverride {
                        total_input_tokens: Some(8192),
                        state_and_longest_question_tokens: Some(8192),
                        runtime_context_tokens: Some(8192),
                    }),
                    ..custom_connection()
                };
                let server = tokio::spawn(async move {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let mut request = Vec::new();
                    while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                        let mut buffer = [0; 4096];
                        let count = socket.read(&mut buffer).await.unwrap();
                        assert!(count > 0);
                        request.extend_from_slice(&buffer[..count]);
                        assert!(request.len() < 65536);
                    }
                    assert!(String::from_utf8_lossy(&request).contains("/api/version"));
                    let (status, body) = if authentication_rejected {
                        ("401 Unauthorized", r#"{"error":"authentication required"}"#)
                    } else {
                        ("200 OK", r#"{"version":"0.1.0"}"#)
                    };
                    socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
                });
                let result = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                    if testing {
                        super::test_connection(super::SystemOneDraft {
                            connection_id: "draft-only".into(),
                            connection,
                            credential: Some("draft-secret".into()),
                        })
                        .await
                        .map(|_| ())
                    } else {
                        super::refresh_limits(connection, Some("draft-secret".into()))
                            .await
                            .map(|_| ())
                    }
                })
                .await
                .unwrap();
                let error = result.unwrap_err();
                if authentication_rejected {
                    assert!(error.contains("rejected its credential"));
                } else {
                    assert!(error.contains("does not support this model or runner"));
                }
                server.await.unwrap();
                assert_eq!(store.internal_value(CONNECTION_PROFILES_KEY), before);
                assert_eq!(worker.execution_snapshot().unwrap().generation, generation);
                assert_eq!(vault.value.borrow().as_deref(), Some("synthetic-secret"));
            }
        }
    }

    #[test]
    fn vault_read_write_and_delete_failures_are_injectable_offline() {
        let vault = OfflineVault::default();
        vault.fail_read.set(true);
        assert!(vault.read(None).is_err());
        vault.fail_write.set(true);
        assert!(write_credential(&vault, Some("jev"), "synthetic").is_err());
        assert!(vault.value.borrow().is_none());
        write_credential(&vault, Some("jev"), "synthetic").unwrap();
        vault.fail_delete.set(true);
        assert!(delete_credential(&vault, Some("jev")).is_err());
        assert_eq!(vault.value.borrow().as_deref(), Some("synthetic"));
        delete_credential(&vault, Some("jev")).unwrap();
        assert!(vault.value.borrow().is_none());
    }

    #[test]
    fn failed_connection_secret_write_restores_the_previous_provider_and_allows_retry() {
        let store =
            Store::open_in_memory(Path::new("/tmp/antiburn-provider-save-retry")).expect("store");
        migrate_provider_state(&store).expect("migrate");
        let mut saved = super::profiles(&store).expect("read profiles");
        let connection = crate::jev::config::SystemOneConnection {
            provider: crate::jev::config::SystemOneProvider::Custom,
            endpoint: crate::jev::config::SystemOneEndpoint::ExactUrl(
                "https://proxy.example/v1/systemone".to_owned(),
            ),
            model: "proxy-model".to_owned(),
            model_revision: None,
            response_mode: crate::jev::config::SystemOneResponseMode::Direct,
            credential: None,
            revision: 1,
            context_override: Some(crate::jev::config::ContextLimitOverride {
                total_input_tokens: Some(8192),
                ..crate::jev::config::ContextLimitOverride::default()
            }),
        };
        let vault = OfflineVault::default();
        vault.fail_write.set(true);
        let worker = WorkerHandle::default();

        assert_eq!(
            persist_connection_profile(
                &store,
                &mut saved,
                "proxy",
                connection.clone(),
                Some("synthetic-secret".to_owned()),
                &vault,
                &worker,
            )
            .unwrap_err(),
            "Could not save the API token to secure storage. Check credential storage access and retry."
        );
        assert_eq!(
            store
                .internal_value(super::CONNECTION_PENDING_KEY)
                .as_deref(),
            Some("")
        );
        assert_eq!(
            active_connection(&store).unwrap().provider,
            crate::jev::config::SystemOneProvider::Jev
        );
        assert!(!worker.is_available());

        persist_connection_profile(
            &store,
            &mut saved,
            "proxy",
            connection.clone(),
            Some("synthetic-secret".to_owned()),
            &vault,
            &worker,
        )
        .expect("retry save");
        let active = active_connection(&store).unwrap();
        assert_eq!(active.provider, connection.provider);
        assert_eq!(active.endpoint, connection.endpoint);
        assert_eq!(
            active.credential,
            Some(crate::jev::config::CredentialReference::Connection(
                "proxy".to_owned()
            ))
        );
        assert_eq!(vault.value.borrow().as_deref(), Some("synthetic-secret"));
    }

    #[test]
    fn migration_retries_after_profile_state_was_written_without_its_marker() {
        let store = Store::open_in_memory(Path::new("/tmp/antiburn-provider-migrate-retry"))
            .expect("store");
        let mut saved = super::ConnectionProfiles::default();
        saved.profiles.insert(
            "local".to_owned(),
            crate::jev::config::SystemOneConnection {
                provider: crate::jev::config::SystemOneProvider::Ollama,
                endpoint: crate::jev::config::SystemOneEndpoint::BaseUrl(
                    "http://127.0.0.1:11434".to_owned(),
                ),
                model: "clef-flash".to_owned(),
                model_revision: None,
                response_mode: crate::jev::config::SystemOneResponseMode::Direct,
                credential: None,
                revision: 1,
                context_override: None,
            },
        );
        saved.active_id = "local".to_owned();
        super::save_profiles(&store, &saved).expect("persist before migration marker");
        store.set_internal_value(YAML_DISCOVERY_ENABLED_KEY, "true");

        migrate_provider_state(&store).expect("retry interrupted migration");

        assert_eq!(active_connection(&store).unwrap(), saved.profiles["local"]);
        assert_eq!(
            store.internal_value(PROVIDER_MIGRATION_KEY).as_deref(),
            Some("1")
        );
        assert!(super::yaml_discovery_enabled(&store));
    }

    #[test]
    fn availability_failure_event_has_a_typed_failure_status() {
        assert_eq!(
            serde_json::to_value(CheckAvailabilityEvent::Failed).unwrap(),
            serde_json::json!({"status":"failed"})
        );
    }

    #[test]
    fn check_mutations_accept_only_main_and_settings_windows() {
        assert!(super::checks_settings_label(crate::main_window::LABEL).is_ok());
        assert!(super::checks_settings_label(crate::settings::LABEL).is_ok());
        assert!(super::checks_settings_label("popover").is_err());
    }

    #[test]
    fn replacing_a_saved_key_preserves_a_paused_master_switch() {
        assert!(!super::resume_after_key_save(false, true, true));
        assert!(super::resume_after_key_save(true, true, true));
        assert!(super::resume_after_key_save(false, true, false));
    }

    #[test]
    fn availability_lists_deterministic_preferences_without_retired_history_fields() {
        let store = Store::open_in_memory(Path::new("/tmp/antiburn-check-availability")).unwrap();
        let snapshot = super::read_check_availability(&store, false, false, None).unwrap();
        assert_eq!(
            snapshot.checks.len(),
            antiburn_local::checks::DetectorId::COUNT
        );
        for choice in &snapshot.checks {
            let detector = antiburn_local::checks::DetectorId::from(choice.id);
            assert_eq!(choice.enabled, store.check_enabled(detector).unwrap());
        }
        store
            .set_check_enabled(antiburn_local::checks::DetectorId::ALL[0], false)
            .unwrap();
        let encoded = serde_json::to_value(&snapshot).unwrap();
        assert!(encoded.get("historyDays").is_none());
        assert!(encoded.get("backfill").is_none());
        assert_eq!(
            super::read_check_availability(&store, false, false, None)
                .unwrap()
                .revision,
            1
        );
    }

    #[test]
    fn startup_skips_credential_storage_without_a_saved_key() {
        let store =
            Store::open_in_memory(Path::new("/tmp/antiburn-typesafe-unconfigured")).expect("store");
        let worker = WorkerHandle::default();
        restore_saved_key(&store, &worker, || {
            panic!("unconfigured startup read the keychain")
        })
        .expect("no key needed");
        assert!(!worker.is_available());
    }

    #[test]
    fn pausing_keeps_the_saved_key_marker_separate_from_enablement() {
        let store =
            Store::open_in_memory(Path::new("/tmp/antiburn-typesafe-paused")).expect("store");
        store.set_internal_value(YAML_DISCOVERY_ENABLED_KEY, "true");
        store.set_internal_value(SAVED_KEY_KEY, "true");
        store
            .set_internal_value_checked(YAML_DISCOVERY_ENABLED_KEY, "false")
            .expect("pause checks");

        assert!(saved_key_marker(&store));
        assert!(!super::yaml_discovery_enabled(&store));
    }

    #[test]
    fn startup_restores_only_a_saved_valid_key() {
        let store =
            Store::open_in_memory(Path::new("/tmp/antiburn-typesafe-restored")).expect("store");
        let worker = WorkerHandle::default();
        store.set_internal_value(YAML_DISCOVERY_ENABLED_KEY, "true");
        store.set_internal_value(SAVED_KEY_KEY, "true");
        restore_saved_key(&store, &worker, || Ok(Some("synthetic-key".to_owned())))
            .expect("key restored");
        assert!(worker.is_available());

        worker.suspend_system_one();
        store.set_internal_value(AUTH_REJECTED_KEY, "true");
        restore_saved_key(&store, &worker, || Ok(Some("rejected-key".to_owned())))
            .expect("rejected key read");
        assert!(!worker.is_available());
    }

    #[test]
    fn startup_restores_jev_for_yaml_checks_without_legacy_enablement() {
        let store =
            Store::open_in_memory(Path::new("/tmp/antiburn-yaml-jev-restored")).expect("store");
        let worker = WorkerHandle::default();
        store.set_internal_value(YAML_DISCOVERY_ENABLED_KEY, "true");
        store.set_internal_value(SAVED_KEY_KEY, "true");

        restore_saved_key(&store, &worker, || Ok(Some("synthetic-key".to_owned())))
            .expect("yaml setting restores the Jev connection");

        assert!(worker.is_available());
        assert!(super::yaml_discovery_enabled(&store));
    }

    #[test]
    fn removal_before_startup_restore_prevents_a_keychain_read() {
        let store =
            Store::open_in_memory(Path::new("/tmp/antiburn-typesafe-removed")).expect("store");
        let worker = WorkerHandle::default();
        store.set_internal_value(YAML_DISCOVERY_ENABLED_KEY, "true");
        store
            .set_internal_value_checked(YAML_DISCOVERY_ENABLED_KEY, "false")
            .expect("pause checks");
        restore_saved_key(&store, &worker, || panic!("removed key read from keychain"))
            .expect("restore skipped");
        assert!(!worker.is_available());
    }

    #[test]
    fn missing_saved_key_does_not_start_checks() {
        let store =
            Store::open_in_memory(Path::new("/tmp/antiburn-typesafe-missing")).expect("store");
        let worker = WorkerHandle::default();
        store.set_internal_value(YAML_DISCOVERY_ENABLED_KEY, "true");
        store.set_internal_value(SAVED_KEY_KEY, "true");
        assert!(restore_saved_key(&store, &worker, || Ok(None)).is_err());
        assert!(!worker.is_available());
    }

    #[test]
    fn pending_credential_removal_blocks_startup_restore() {
        let store = Store::open_in_memory(Path::new("/tmp/antiburn-typesafe-pending-remove"))
            .expect("store");
        let worker = WorkerHandle::default();
        store.set_internal_value(YAML_DISCOVERY_ENABLED_KEY, "true");
        store.set_internal_value(CREDENTIAL_CHANGE_PENDING_KEY, "true");
        assert!(
            restore_saved_key(&store, &worker, || {
                panic!("pending removal read the old credential")
            })
            .is_err()
        );
        assert!(!worker.is_available());
    }

    #[test]
    fn failed_legacy_removal_can_retry_after_replacement_and_reenable_checks() {
        let store =
            Store::open_in_memory(Path::new("/tmp/antiburn-legacy-removal-retry")).expect("store");
        migrate_provider_state(&store).expect("migrate");
        store.set_internal_value(YAML_DISCOVERY_ENABLED_KEY, "true");
        store.set_internal_value(CREDENTIAL_CHANGE_PENDING_KEY, "true");
        let vault = OfflineVault::default();
        let mut profiles = super::profiles(&store).expect("read profiles");
        let replacement = crate::jev::config::SystemOneConnection {
            provider: crate::jev::config::SystemOneProvider::Ollama,
            endpoint: crate::jev::config::SystemOneEndpoint::BaseUrl(
                "http://127.0.0.1:11434".into(),
            ),
            model: "clef-flash".into(),
            credential: None,
            ..Default::default()
        };
        let worker = WorkerHandle::default();
        vault.fail_delete.set(true);
        assert!(
            persist_connection_profile(
                &store,
                &mut profiles,
                "local",
                replacement.clone(),
                None,
                &vault,
                &worker,
            )
            .is_err()
        );
        assert_eq!(
            active_connection(&store).unwrap().provider,
            crate::jev::config::SystemOneProvider::Jev
        );
        vault.fail_delete.set(false);
        persist_connection_profile(
            &store,
            &mut profiles,
            "local",
            replacement,
            None,
            &vault,
            &worker,
        )
        .expect("retry cleanup and provider replacement");
        assert_eq!(
            store
                .internal_value(CREDENTIAL_CHANGE_PENDING_KEY)
                .as_deref(),
            Some("false")
        );
        store.set_internal_value(CONNECTION_PENDING_KEY, "");
        restore_saved_key(&store, &worker, || {
            panic!("replacement must not read legacy key")
        })
        .expect("restore replacement");
        assert!(worker.is_available());
    }

    #[test]
    fn provider_migration_is_idempotent_and_preserves_yaml_enablement_and_credentials() {
        let store =
            Store::open_in_memory(Path::new("/tmp/antiburn-provider-migration")).expect("store");
        store.set_internal_value(YAML_DISCOVERY_ENABLED_KEY, "true");
        store.set_internal_value(super::SAVED_KEY_KEY, "true");

        migrate_provider_state(&store).expect("migrate");
        let first = store.internal_value(CONNECTION_PROFILES_KEY).unwrap();
        migrate_provider_state(&store).expect("repeat migration");

        assert_eq!(store.internal_value(CONNECTION_PROFILES_KEY), Some(first));
        assert_eq!(
            store.internal_value(PROVIDER_MIGRATION_KEY).as_deref(),
            Some("1")
        );
        assert!(super::yaml_discovery_enabled(&store));
        assert!(saved_key_marker(&store));
        assert!(active_connection(&store).is_ok());
    }

    #[test]
    fn keyless_provider_migration_does_not_read_or_create_a_credential() {
        let store =
            Store::open_in_memory(Path::new("/tmp/antiburn-keyless-migration")).expect("store");
        migrate_provider_state(&store).expect("migrate keyless settings");
        assert!(!saved_key_marker(&store));
        assert!(active_connection(&store).is_ok());
    }

    #[test]
    fn keyless_ollama_connection_restores_without_touching_the_keyring() {
        let store =
            Store::open_in_memory(Path::new("/tmp/antiburn-keyless-ollama")).expect("store");
        migrate_provider_state(&store).expect("migrate default settings");
        let mut profiles: super::ConnectionProfiles =
            serde_json::from_str(&store.internal_value(CONNECTION_PROFILES_KEY).unwrap()).unwrap();
        let connection = crate::jev::config::SystemOneConnection {
            provider: crate::jev::config::SystemOneProvider::Ollama,
            endpoint: crate::jev::config::SystemOneEndpoint::BaseUrl(
                "http://127.0.0.1:11434".to_owned(),
            ),
            model: "clef-flash".to_owned(),
            model_revision: None,
            response_mode: crate::jev::config::SystemOneResponseMode::Direct,
            credential: None,
            revision: 1,
            context_override: None,
        };
        profiles
            .profiles
            .insert("local".to_owned(), connection.clone());
        profiles.active_id = "local".to_owned();
        super::save_profiles(&store, &profiles).unwrap();
        store.set_internal_value(YAML_DISCOVERY_ENABLED_KEY, "true");

        let worker = WorkerHandle::default();
        super::restore_saved_key(&store, &worker, || {
            panic!("Ollama must not read TypeSafe keyring")
        })
        .unwrap();

        assert_eq!(worker.system_one_connection(), connection);
        assert!(worker.is_available());
        store
            .set_internal_value_checked(YAML_DISCOVERY_ENABLED_KEY, "false")
            .unwrap();
        super::restore_saved_key(&store, &worker, || panic!("paused startup reads no keys"))
            .unwrap();
        assert_eq!(worker.system_one_connection(), connection);
        assert!(!worker.is_available());
    }

    #[test]
    fn saved_connection_profiles_switch_and_restore_to_jev_without_serializing_secrets() {
        let store =
            Store::open_in_memory(Path::new("/tmp/antiburn-provider-profiles")).expect("store");
        migrate_provider_state(&store).expect("migrate");
        let mut saved = super::profiles(&store).expect("read default profile");
        let ollama = crate::jev::config::SystemOneConnection {
            provider: crate::jev::config::SystemOneProvider::Ollama,
            endpoint: crate::jev::config::SystemOneEndpoint::BaseUrl(
                "http://127.0.0.1:11434".to_owned(),
            ),
            model: "clef-flash".to_owned(),
            model_revision: None,
            response_mode: crate::jev::config::SystemOneResponseMode::Direct,
            credential: None,
            revision: 1,
            context_override: None,
        };
        saved.profiles.insert("local".to_owned(), ollama.clone());
        saved.active_id = "local".to_owned();
        super::save_profiles(&store, &saved).unwrap();
        assert_eq!(active_connection(&store).unwrap(), ollama);

        saved.active_id = "jev".to_owned();
        super::save_profiles(&store, &saved).unwrap();
        assert_eq!(
            active_connection(&store).unwrap(),
            crate::jev::config::SystemOneConnection::jev_default()
        );
        let encoded = store.internal_value(CONNECTION_PROFILES_KEY).unwrap();
        assert!(!encoded.contains("secret"));
        assert!(!encoded.contains("api_key"));
    }
}
