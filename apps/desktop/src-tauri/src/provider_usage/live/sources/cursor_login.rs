//! Read Cursor's IDE login without a network request or a database write.
//!
//! Only macOS has observed path evidence. Linux and Windows use conventional
//! Cursor paths. The JWT payload supplies an expiry hint, not verified identity.
//! Keep the access token in memory. Send it only to `https://api2.cursor.sh`.
//! Never log or persist it. This reader does not read or refresh refresh tokens.
//!
//! Cursor 3.8 encrypts the IDE token (`cursor-safe-storage:v1:`), and this
//! reader cannot decrypt it. When the IDE gives no usable token, the reader
//! uses the access token of the Cursor CLI. On macOS, the CLI keeps that
//! token in the login Keychain, service `cursor-access-token` and account
//! `cursor-user`. `security` reads the item, and macOS can ask the user to
//! allow the read.

use std::path::{Path, PathBuf};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::Deserialize;

/// No Debug or serialization implementation exposes the credential.
pub enum CursorLogin {
    SignedIn { access_token: String },
    SignedOut,
    Expired,
    Unreadable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CursorLoginState {
    SignedIn,
    SignedOut,
    Expired,
    Unreadable,
}

impl CursorLogin {
    pub fn state(&self) -> CursorLoginState {
        match self {
            Self::SignedIn { .. } => CursorLoginState::SignedIn,
            Self::SignedOut => CursorLoginState::SignedOut,
            Self::Expired => CursorLoginState::Expired,
            Self::Unreadable => CursorLoginState::Unreadable,
        }
    }
}

/// Read the IDE login, then the CLI login when the IDE has no usable token.
pub fn read() -> CursorLogin {
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let ide = default_path().map_or(CursorLogin::Unreadable, |path| read_at(&path, now));
    with_cli_fallback(ide, now, read_cli)
}

/// What one read of the CLI login found.
enum CliRead {
    Absent,
    Unreadable,
    Found(String),
}

#[cfg(target_os = "macos")]
fn read_cli() -> CliRead {
    use super::claude_login::macos_keychain::{KeychainRead, read_item};

    let read = read_item("cursor-access-token", "cursor-user");
    ::tracing::debug!(event = "cursor_cli_keychain_read", outcome = read.outcome());
    match read {
        KeychainRead::Absent => CliRead::Absent,
        KeychainRead::Unreadable => CliRead::Unreadable,
        KeychainRead::Found(token) => CliRead::Found(token),
    }
}

#[cfg(not(target_os = "macos"))]
fn read_cli() -> CliRead {
    CliRead::Absent
}

/// A signed-in IDE login wins, and the CLI is then not read. A signed-in CLI
/// login wins over an IDE login that is not signed in. Otherwise an expired
/// login reports `Expired`, so that the user knows which action helps.
fn with_cli_fallback(ide: CursorLogin, now: i64, cli: impl FnOnce() -> CliRead) -> CursorLogin {
    if matches!(ide, CursorLogin::SignedIn { .. }) {
        return ide;
    }
    let cli = match cli() {
        CliRead::Absent => return ide,
        CliRead::Unreadable => CursorLogin::Unreadable,
        CliRead::Found(token) => login_from_token(token.trim().to_owned(), now),
    };
    match (ide, cli) {
        (_, cli @ CursorLogin::SignedIn { .. }) => cli,
        (CursorLogin::Expired, _) | (_, CursorLogin::Expired) => CursorLogin::Expired,
        (CursorLogin::SignedOut, CursorLogin::SignedOut) => CursorLogin::SignedOut,
        _ => CursorLogin::Unreadable,
    }
}

fn default_path() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(|root| state_path(Path::new(&root), "windows"))
    } else {
        std::env::var_os("HOME").map(|root| state_path(Path::new(&root), std::env::consts::OS))
    }
}

fn state_path(root: &Path, platform: &str) -> PathBuf {
    let base = match platform {
        "macos" => root.join("Library/Application Support"),
        "windows" => root.to_path_buf(),
        _ => root.join(".config"),
    };
    base.join("Cursor/User/globalStorage/state.vscdb")
}

/// Read one database at a supplied Unix time. A missing database means signed out.
/// Invalid JWTs and database errors mean unreadable. Expiry is exclusive.
pub fn read_at(path: &Path, now: i64) -> CursorLogin {
    match std::fs::metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return CursorLogin::SignedOut;
        }
        Err(_) => return CursorLogin::Unreadable,
        Ok(_) => {}
    }
    let Ok(connection) = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) else {
        return CursorLogin::Unreadable;
    };
    let result = connection
        .query_row(
            "SELECT value FROM ItemTable WHERE key = 'cursorAuth/accessToken'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional();
    match result {
        Ok(Some(token)) => login_from_token(token, now),
        Ok(None) => CursorLogin::SignedOut,
        Err(_) => CursorLogin::Unreadable,
    }
}

/// An empty token means signed out. A token without a JWT expiry is
/// unreadable. Expiry is exclusive.
fn login_from_token(token: String, now: i64) -> CursorLogin {
    if token.trim().is_empty() {
        return CursorLogin::SignedOut;
    }
    match expiry(&token) {
        Some(exp) if exp > now => CursorLogin::SignedIn {
            access_token: token,
        },
        Some(_) => CursorLogin::Expired,
        None => CursorLogin::Unreadable,
    }
}

fn expiry(token: &str) -> Option<i64> {
    #[derive(Deserialize)]
    struct Claims {
        exp: i64,
    }
    let mut parts = token.split('.');
    let header = parts.next()?;
    let payload = parts.next()?;
    let signature = parts.next()?;
    if header.is_empty() || signature.is_empty() || parts.next().is_some() {
        return None;
    }
    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    serde_json::from_slice::<Claims>(&decoded)
        .ok()
        .map(|c| c.exp)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn token(payload: &str) -> String {
        format!("e30.{}.synthetic", URL_SAFE_NO_PAD.encode(payload))
    }

    fn database(value: Option<&str>) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.vscdb");
        let db = Connection::open(&path).unwrap();
        db.execute(
            "CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value TEXT)",
            [],
        )
        .unwrap();
        if let Some(value) = value {
            db.execute(
                "INSERT INTO ItemTable VALUES ('cursorAuth/accessToken', ?1)",
                [value],
            )
            .unwrap();
        }
        (dir, path)
    }

    #[test]
    fn returns_only_unexpired_token_without_changing_database() {
        let synthetic = token(r#"{"exp":1800000001}"#);
        let (_dir, path) = database(Some(&synthetic));
        let before = std::fs::read(&path).unwrap();
        match read_at(&path, NOW) {
            CursorLogin::SignedIn { access_token } => assert!(access_token == synthetic),
            _ => panic!("expected signed in"),
        }
        assert!(std::fs::read(&path).unwrap() == before);
    }

    #[test]
    fn missing_or_empty_login_is_signed_out() {
        for value in [None, Some(""), Some("   ")] {
            let (_dir, path) = database(value);
            assert!(matches!(read_at(&path, NOW), CursorLogin::SignedOut));
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.vscdb");
        assert!(matches!(read_at(&path, NOW), CursorLogin::SignedOut));
        assert!(!path.exists());
    }

    #[test]
    fn expiry_at_or_before_now_is_expired() {
        for exp in [NOW - 1, NOW] {
            let (_dir, path) = database(Some(&token(&format!(r#"{{"exp":{exp}}}"#))));
            assert!(matches!(read_at(&path, NOW), CursorLogin::Expired));
        }
    }

    #[test]
    fn malformed_or_missing_expiry_is_unreadable() {
        for value in [
            "invalid".into(),
            "a.%%%.b".into(),
            token("not json"),
            token("{}"),
            token(r#"{"exp":"1800000001"}"#),
            token(r#"{"exp":null}"#),
            token(r#"{"exp":1800000000.5}"#),
            format!("{}.extra", token(r#"{"exp":1800000001}"#)),
        ] {
            let (_dir, path) = database(Some(&value));
            assert!(matches!(read_at(&path, NOW), CursorLogin::Unreadable));
        }
    }

    #[test]
    fn corrupt_database_wrong_schema_and_wrong_value_type_are_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.vscdb");
        std::fs::write(&path, b"synthetic invalid sqlite").unwrap();
        assert!(matches!(read_at(&path, NOW), CursorLogin::Unreadable));
        assert!(matches!(read_at(dir.path(), NOW), CursorLogin::Unreadable));
        let (_dir, path) = database(None);
        let db = Connection::open(&path).unwrap();
        db.execute("DROP TABLE ItemTable", []).unwrap();
        assert!(matches!(read_at(&path, NOW), CursorLogin::Unreadable));
        let (_dir, path) = database(None);
        let db = Connection::open(&path).unwrap();
        db.execute(
            "INSERT INTO ItemTable VALUES ('cursorAuth/accessToken', NULL)",
            [],
        )
        .unwrap();
        assert!(matches!(read_at(&path, NOW), CursorLogin::Unreadable));
    }

    #[test]
    fn conventional_paths_cover_all_desktop_platforms() {
        let root = Path::new("synthetic-root");
        for (platform, suffix) in [
            (
                "macos",
                "Library/Application Support/Cursor/User/globalStorage/state.vscdb",
            ),
            ("linux", ".config/Cursor/User/globalStorage/state.vscdb"),
            ("windows", "Cursor/User/globalStorage/state.vscdb"),
        ] {
            assert_eq!(state_path(root, platform), root.join(suffix));
        }
    }
    fn signed_in_token(login: CursorLogin) -> Option<String> {
        match login {
            CursorLogin::SignedIn { access_token } => Some(access_token),
            _ => None,
        }
    }

    #[test]
    fn a_signed_in_ide_login_does_not_read_the_cli() {
        let ide = token(r#"{"exp":1800000001}"#);
        let login = with_cli_fallback(
            CursorLogin::SignedIn {
                access_token: ide.clone(),
            },
            NOW,
            || panic!("The CLI must not be read"),
        );
        assert_eq!(signed_in_token(login), Some(ide));
    }

    #[test]
    fn an_unusable_ide_login_uses_the_cli_token() {
        let cli = token(r#"{"exp":1800000001}"#);
        for ide in [
            CursorLogin::Unreadable,
            CursorLogin::SignedOut,
            CursorLogin::Expired,
        ] {
            let found = format!("{cli}\n");
            let login = with_cli_fallback(ide, NOW, || CliRead::Found(found));
            assert_eq!(signed_in_token(login), Some(cli.clone()));
        }
    }

    #[test]
    fn without_a_usable_cli_token_the_most_useful_state_remains() {
        let expired = token(r#"{"exp":1800000000}"#);
        let state = |ide, cli| with_cli_fallback(ide, NOW, || cli).state();
        assert_eq!(
            state(CursorLogin::Unreadable, CliRead::Absent),
            CursorLoginState::Unreadable
        );
        assert_eq!(
            state(CursorLogin::SignedOut, CliRead::Absent),
            CursorLoginState::SignedOut
        );
        assert_eq!(
            state(CursorLogin::SignedOut, CliRead::Found(String::new())),
            CursorLoginState::SignedOut
        );
        assert_eq!(
            state(CursorLogin::Unreadable, CliRead::Found(expired.clone())),
            CursorLoginState::Expired
        );
        assert_eq!(
            state(CursorLogin::Expired, CliRead::Unreadable),
            CursorLoginState::Expired
        );
        assert_eq!(
            state(CursorLogin::SignedOut, CliRead::Unreadable),
            CursorLoginState::Unreadable
        );
        assert_eq!(
            state(CursorLogin::Unreadable, CliRead::Found("opaque".into())),
            CursorLoginState::Unreadable
        );
    }
}
