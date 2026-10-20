//! Validate the bounded OpenCode V2 SQLite contract.

use std::collections::HashSet;

use rusqlite::Connection;

pub(crate) fn is_supported(connection: &Connection) -> rusqlite::Result<bool> {
    if !has_columns(
        connection,
        "session_v2",
        &[
            "id",
            "parent_id",
            "directory",
            "title",
            "time_created",
            "time_updated",
        ],
    )? || !has_columns(
        connection,
        "session_message",
        &[
            "id",
            "session_id",
            "type",
            "seq",
            "time_created",
            "time_updated",
            "data",
        ],
    )? {
        return Ok(false);
    }

    Ok(has_unique_index(connection, "session_v2", &["id"])?
        && has_unique_index(connection, "session_message", &["id"])?
        && has_unique_index(connection, "session_message", &["session_id", "seq"])?)
}

pub(crate) fn has_event_sequence_contract(connection: &Connection) -> rusqlite::Result<bool> {
    Ok(
        has_columns(connection, "event_sequence", &["aggregate_id", "seq"])?
            && has_non_null_columns(connection, "event_sequence", &["aggregate_id", "seq"])?
            && has_unique_index(connection, "event_sequence", &["aggregate_id"])?,
    )
}

fn has_columns(connection: &Connection, table: &str, required: &[&str]) -> rusqlite::Result<bool> {
    let mut statement = connection.prepare("SELECT name FROM pragma_table_info(?1)")?;
    let columns: HashSet<String> = statement
        .query_map([table], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(required.iter().all(|column| columns.contains(*column)))
}

fn has_non_null_columns(
    connection: &Connection,
    table: &str,
    required: &[&str],
) -> rusqlite::Result<bool> {
    let mut statement = connection
        .prepare("SELECT name FROM pragma_table_info(?1) WHERE \"notnull\" = 1 OR pk > 0")?;
    let columns: HashSet<String> = statement
        .query_map([table], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(required.iter().all(|column| columns.contains(*column)))
}

fn has_unique_index(
    connection: &Connection,
    table: &str,
    required: &[&str],
) -> rusqlite::Result<bool> {
    let mut statement = connection.prepare(
        "SELECT name FROM pragma_index_list(?1)
          WHERE \"unique\" = 1 AND partial = 0",
    )?;
    let indexes = statement
        .query_map([table], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for index in indexes {
        let mut statement =
            connection.prepare("SELECT name FROM pragma_index_info(?1) ORDER BY seqno")?;
        let columns = statement
            .query_map([index], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if columns
            .iter()
            .map(String::as_str)
            .eq(required.iter().copied())
        {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE session_v2 (
                     id TEXT PRIMARY KEY, parent_id TEXT, directory TEXT,
                     title TEXT, time_created INTEGER, time_updated INTEGER
                 );
                 CREATE TABLE session_message (
                     id TEXT PRIMARY KEY, session_id TEXT, type TEXT, seq INTEGER,
                     time_created INTEGER, time_updated INTEGER, data TEXT
                 );",
            )
            .unwrap();
        connection
    }

    #[test]
    fn accepts_the_pinned_session_sequence_index() {
        let connection = fixture();
        assert!(!is_supported(&connection).unwrap());

        connection
            .execute_batch(
                "CREATE UNIQUE INDEX session_message_session_seq_idx
                 ON session_message(session_id, seq);",
            )
            .unwrap();

        assert!(is_supported(&connection).unwrap());
    }

    #[test]
    fn rejects_a_non_unique_session_sequence_index() {
        let connection = fixture();
        connection
            .execute_batch(
                "CREATE INDEX session_message_session_seq_idx
                 ON session_message(session_id, seq);",
            )
            .unwrap();

        assert!(!is_supported(&connection).unwrap());
    }

    #[test]
    fn rejects_a_partial_unique_session_sequence_index() {
        let connection = fixture();
        connection
            .execute_batch(
                "CREATE UNIQUE INDEX session_message_session_seq_idx
                 ON session_message(session_id, seq) WHERE seq > 0;
                 INSERT INTO session_v2 VALUES ('ses_1', NULL, '/repo', NULL, 1, 1);
                 INSERT INTO session_message VALUES
                     ('msg_1', 'ses_1', 'user', 0, 1, 1, '{}'),
                     ('msg_2', 'ses_1', 'user', 0, 1, 1, '{}');",
            )
            .unwrap();

        assert!(!is_supported(&connection).unwrap());
    }

    #[test]
    fn event_sequence_requires_a_non_null_sequence_and_unique_aggregate_key() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE event_sequence (
                     aggregate_id TEXT PRIMARY KEY NOT NULL, seq INTEGER
                 );",
            )
            .unwrap();

        assert!(!has_event_sequence_contract(&connection).unwrap());

        connection
            .execute_batch(
                "DROP TABLE event_sequence;
                            CREATE TABLE event_sequence (
                                aggregate_id TEXT PRIMARY KEY NOT NULL,
                                seq INTEGER NOT NULL
                            );",
            )
            .unwrap();
        assert!(has_event_sequence_contract(&connection).unwrap());
    }
}
