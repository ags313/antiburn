//! Read Cursor's IDE login without a network request or a database write.
//!
//! Only macOS has observed path evidence. Linux and Windows use conventional
//! Cursor paths. The JWT payload supplies an expiry hint, not verified identity.
//! Keep the access token in memory. Send it only to `https://api2.cursor.sh`.
//! Never log or persist it. This reader does not read or refresh refresh tokens.

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

/// Read the current user's conventional IDE state path.
pub fn read() -> CursorLogin {
    let Some(path) = default_path() else {
        return CursorLogin::Unreadable;
    };
    read_at(&path, time::OffsetDateTime::now_utc().unix_timestamp())
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
    let token = match result {
        Ok(Some(token)) if !token.trim().is_empty() => token,
        Ok(_) => return CursorLogin::SignedOut,
        Err(_) => return CursorLogin::Unreadable,
    };
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
}
