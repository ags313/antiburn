use std::{path::Path, time::Duration};

use rusqlite::{Connection, OpenFlags, config::DbConfig};

/// Open a provider database with defensive settings and a fixed execution budget.
pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
    connection.pragma_update(None, "trusted_schema", false)?;
    connection.busy_timeout(Duration::from_millis(100))?;
    let mut remaining = 100_000_u32;
    connection.progress_handler(
        1_000,
        Some(move || {
            remaining = remaining.saturating_sub(1);
            remaining == 0
        }),
    );
    Ok(connection)
}

pub(crate) fn is_table(connection: &Connection, name: &str) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [name],
        |row| row.get(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn defensive_reader_interrupts_unbounded_queries() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let connection = open(file.path()).unwrap();
        assert!(
            connection
                .db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE)
                .unwrap()
        );
        assert!(
            !connection
                .query_row("PRAGMA trusted_schema", [], |row| row.get::<_, bool>(0))
                .unwrap()
        );
        let start = Instant::now();
        let error = connection.query_row(
            "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n) SELECT sum(x) FROM n",
            [], |row| row.get::<_, i64>(0),
        ).unwrap_err();
        assert_eq!(
            error.sqlite_error_code(),
            Some(rusqlite::ErrorCode::OperationInterrupted)
        );
        assert!(start.elapsed() < Duration::from_secs(10));
    }
}
