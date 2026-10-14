//! Read-only SQLite copies for agent stores that keep sessions in a database.
//!
//! The helper never sends a live database file. It reads one consistent
//! snapshot inside a read transaction and sends that snapshot as bytes.

use std::path::Path;

use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, DatabaseName, OpenFlags};

use crate::evidence::AdmittedFile;
use crate::export::MAX_BUNDLE_BYTES;

const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// The OpenCode tables that the engine's `OpenCodeSqliteV2` reader reads.
const OPENCODE_TABLES: [&str; 3] = ["session", "message", "part"];

/// Copy one Antigravity conversation database.
pub(crate) fn snapshot(path: &Path, admitted: &AdmittedFile) -> Result<Vec<u8>> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .context("Cannot open the session database")?;
    ensure_same_file(path, admitted)?;
    connection.busy_timeout(BUSY_TIMEOUT)?;
    connection.execute_batch("BEGIN")?;
    // The first read starts the read transaction, so every later page comes
    // from the same snapshot.
    connection.query_row("SELECT COUNT(*) FROM sqlite_master", [], |row| {
        row.get::<_, i64>(0)
    })?;
    ensure_size(&connection, DatabaseName::Main)?;
    let bytes = serialize(&connection, DatabaseName::Main)?;
    connection.execute_batch("COMMIT")?;
    Ok(bytes)
}

/// Copy one OpenCode session, its child sessions, and their messages and
/// parts into a new database with the source table definitions.
pub(crate) fn opencode_extract(
    path: &Path,
    admitted: &AdmittedFile,
    session_id: &str,
) -> Result<Vec<u8>> {
    let connection = Connection::open_with_flags(
        ":memory:",
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(BUSY_TIMEOUT)?;
    // The copied table definitions reference tables that the copy omits, such
    // as `project`. Bundled SQLite enforces foreign keys by default.
    connection.execute_batch("PRAGMA foreign_keys = OFF")?;
    connection
        .execute(
            "ATTACH DATABASE ?1 AS source",
            [read_only_uri(path).context("Unsupported session database path")?],
        )
        .context("Cannot open the session database")?;
    ensure_same_file(path, admitted)?;
    connection.execute_batch("BEGIN")?;
    for table in OPENCODE_TABLES {
        let definition: String = connection
            .query_row(
                "SELECT sql FROM source.sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |row| row.get(0),
            )
            .with_context(|| format!("The session database has no {table} table"))?;
        connection.execute_batch(&definition)?;
    }
    let has_parent = connection
        .prepare("SELECT 1 FROM pragma_table_info('session', 'source') WHERE name = 'parent_id'")?
        .exists([])?;
    let cluster = if has_parent {
        "WITH RECURSIVE cluster(id) AS (
             SELECT id FROM source.session WHERE id = ?1
             UNION
             SELECT child.id FROM source.session AS child
               JOIN cluster ON child.parent_id = cluster.id
         )"
    } else {
        "WITH cluster(id) AS (SELECT id FROM source.session WHERE id = ?1)"
    };
    let sessions = connection.execute(
        &format!(
            "INSERT INTO main.session
             {cluster}
             SELECT * FROM source.session WHERE id IN (SELECT id FROM cluster)
              ORDER BY rowid"
        ),
        [session_id],
    )?;
    ensure!(sessions > 0, "Session is no longer in the database");
    connection.execute(
        "INSERT INTO main.message
         SELECT * FROM source.message
          WHERE session_id IN (SELECT id FROM main.session)
          ORDER BY rowid",
        [],
    )?;
    connection.execute(
        "INSERT INTO main.part
         SELECT * FROM source.part
          WHERE message_id IN (SELECT id FROM main.message)
          ORDER BY rowid",
        [],
    )?;
    let indexes: Vec<String> = connection
        .prepare(
            "SELECT sql FROM source.sqlite_master
              WHERE type = 'index' AND sql IS NOT NULL
                AND tbl_name IN ('session', 'message', 'part')
              ORDER BY name",
        )?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    connection.execute_batch("COMMIT")?;
    connection.execute_batch("DETACH DATABASE source")?;
    for index in indexes {
        connection.execute_batch(&index)?;
    }
    ensure_size(&connection, DatabaseName::Main)?;
    serialize(&connection, DatabaseName::Main)
}

/// Make sure that the path still names the admitted file. SQLite opens the
/// path itself, because it finds the write-ahead log from the path.
fn ensure_same_file(path: &Path, admitted: &AdmittedFile) -> Result<()> {
    let opened = std::fs::symlink_metadata(path)?;
    ensure!(
        opened.file_type().is_file(),
        "Session database is not a regular file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let pinned = admitted.file.metadata()?;
        ensure!(
            opened.dev() == pinned.dev() && opened.ino() == pinned.ino(),
            "Session database changed during collection; retry"
        );
    }
    #[cfg(not(unix))]
    let _ = admitted;
    Ok(())
}

fn ensure_size(connection: &Connection, schema: DatabaseName<'_>) -> Result<()> {
    let name = match schema {
        DatabaseName::Main => "main",
        DatabaseName::Temp => "temp",
        DatabaseName::Attached(name) => name,
    };
    let pages: u64 =
        connection.query_row(&format!("PRAGMA {name}.page_count"), [], |row| row.get(0))?;
    let page_size: u64 =
        connection.query_row(&format!("PRAGMA {name}.page_size"), [], |row| row.get(0))?;
    ensure!(
        pages.saturating_mul(page_size) <= MAX_BUNDLE_BYTES,
        "Session database exceeds 1 GiB"
    );
    Ok(())
}

fn serialize(connection: &Connection, schema: DatabaseName<'_>) -> Result<Vec<u8>> {
    let mut bytes = connection.serialize(schema)?.to_vec();
    ensure!(bytes.len() >= 100, "Session database is empty");
    // The copy has no write-ahead log. Header bytes 18 and 19 select the
    // rollback journal, so a reader does not look for a log that is absent.
    bytes[18] = 1;
    bytes[19] = 1;
    Ok(bytes)
}

/// A read-only SQLite URI for an absolute path.
fn read_only_uri(path: &Path) -> Option<String> {
    let path = path.to_str()?;
    if !path.starts_with('/') {
        return None;
    }
    let mut encoded = String::with_capacity(path.len() + 16);
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'-' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    Some(format!("file:{encoded}?mode=ro"))
}
