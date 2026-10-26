use super::*;

use crate::analysis::SourceFormat;

#[cfg(test)]
mod tests;

type Candidate = (String, i64);

struct Family {
    table: &'static str,
    format: SourceFormat,
    parent: bool,
    exclusion: &'static str,
    headers: Vec<Candidate>,
    roots: BTreeMap<String, i64>,
    root_cache: HashMap<String, String>,
}

enum ActivityScan {
    Complete(Vec<BTreeMap<String, i64>>),
    Limited,
}

pub(super) fn query(
    connection: &Connection,
    path: &Path,
    cutoff: i64,
    mixed: bool,
    budget: &QueryBudget<'_>,
) -> Result<Vec<SessionLog>, &'static str> {
    budget.check()?;
    let mut families = vec![Family::read_headers(connection, "session_v2", cutoff)?];
    if mixed {
        families.push(Family::read_headers(connection, "session", cutoff)?);
    }
    for family in &mut families {
        family.roots = family
            .resolve(connection, family.headers.clone(), budget)
            .map_err(|_| "hierarchy")?;
    }
    budget.check()?;
    let mut stage = "activity";
    let activity = budget
        .optional(|| {
            let activity = families
                .iter()
                .map(|family| family.read_activity(connection, cutoff))
                .collect::<rusqlite::Result<Vec<_>>>()?;
            stage = "hierarchy";
            families
                .iter_mut()
                .zip(activity)
                .map(|(family, activity)| {
                    let mut candidates = family.headers.clone();
                    candidates.extend(activity);
                    family.resolve(connection, candidates, budget)
                })
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(|_| budget.failure(stage))?;
    let activity = match activity {
        Some(activity) => ActivityScan::Complete(activity),
        None => ActivityScan::Limited,
    };
    match activity {
        ActivityScan::Complete(activity) => {
            for (family, roots) in families.iter_mut().zip(activity) {
                family.roots = roots;
            }
        }
        ActivityScan::Limited => {
            tracing::warn!(
                event = "opencode_sqlite_activity_scan_limited",
                retained = "recent_session_headers"
            );
        }
    }
    budget.check()?;
    let mut logs: Vec<_> = families
        .into_iter()
        .flat_map(|family| {
            family
                .roots
                .into_iter()
                .filter(|(_, latest)| *latest > 0)
                .map(move |(session_id, latest)| SessionLog {
                    environment: Default::default(),
                    agent_type: AgentKind::OpenCode,
                    source: SessionSource::ProviderDb {
                        agent: AgentKind::OpenCode,
                        db_path: path.to_path_buf(),
                        session_id,
                        source_format: Some(family.format),
                    },
                    updated_at: Some(latest),
                })
        })
        .collect();
    logs.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then(a.source_label().cmp(&b.source_label()))
    });
    Ok(logs)
}

impl Family {
    fn selected_id(&self) -> &'static str {
        if self.table == "session_v2" {
            "CASE WHEN length(CAST(s.id AS BLOB)) <= 256 THEN s.id END"
        } else {
            "s.id"
        }
    }

    fn result_limit(&self) -> i64 {
        if self.table == "session_v2" {
            MAX_V2_RECENT_SESSIONS as i64
        } else {
            -1
        }
    }

    fn read_headers(
        connection: &Connection,
        table: &'static str,
        cutoff: i64,
    ) -> Result<Self, &'static str> {
        let columns = columns(connection, table).map_err(|_| "schema")?;
        let (activity, filter) = activity_sql(&columns, "s.");
        let mut family = Self {
            table,
            format: if table == "session_v2" {
                SourceFormat::OpenCodeSqliteV2
            } else {
                SourceFormat::OpenCodeSqliteV1
            },
            parent: columns.contains("parent_id"),
            exclusion: if table == "session" {
                "AND NOT EXISTS(SELECT 1 FROM session_v2 WHERE session_v2.id = s.id)"
            } else {
                ""
            },
            headers: Vec::new(),
            roots: BTreeMap::new(),
            root_cache: HashMap::new(),
        };
        let mut statement = connection
            .prepare(&format!(
                "SELECT {},
                        {activity} AS activity, {}
                   FROM {table} s WHERE ({filter}) {}
                  ORDER BY activity DESC, s.id LIMIT ?2",
                family.selected_id(),
                if family.parent {
                    "s.parent_id IS NULL"
                } else {
                    "1"
                },
                family.exclusion,
            ))
            .map_err(|_| "query")?;
        let headers = statement
            .query_map(
                params![cutoff.saturating_mul(1000), family.result_limit()],
                |row| Ok((candidate(row)?, row.get::<_, bool>(2)?)),
            )
            .map_err(|_| "query")?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|_| "row")?;
        for ((id, activity), root) in headers {
            if root {
                family.root_cache.insert(id.clone(), id.clone());
            }
            family.headers.push((id, activity));
        }
        Ok(family)
    }

    fn read_activity(
        &self,
        connection: &Connection,
        cutoff: i64,
    ) -> rusqlite::Result<Vec<Candidate>> {
        let tables: &[&str] = if self.table == "session_v2" {
            &["session_message"]
        } else {
            &["message", "part"]
        };
        let mut candidates = Vec::new();
        for table in tables {
            let columns = columns(connection, table)?;
            if !columns.contains("session_id") {
                continue;
            }
            let (activity, filter) = activity_sql(&columns, "");
            // The cutoff excludes old rows before aggregation. Update time has no producer index.
            let mut statement = connection.prepare(&format!(
                "WITH recent_activity AS (
                     SELECT session_id, MAX({activity}) AS activity FROM {table}
                      WHERE {filter} GROUP BY session_id
                 )
                  SELECT {},
                         a.activity
                   FROM recent_activity a JOIN {} s ON s.id = a.session_id
                  WHERE 1 {} ORDER BY a.activity DESC, s.id LIMIT ?2",
                self.selected_id(),
                self.table,
                self.exclusion,
            ))?;
            candidates.extend(
                statement
                    .query_map(
                        params![cutoff.saturating_mul(1000), self.result_limit()],
                        candidate,
                    )?
                    .collect::<rusqlite::Result<Vec<_>>>()?,
            );
        }
        Ok(candidates)
    }

    fn resolve(
        &mut self,
        connection: &Connection,
        candidates: Vec<Candidate>,
        budget: &QueryBudget<'_>,
    ) -> rusqlite::Result<BTreeMap<String, i64>> {
        let mut latest = HashMap::<String, i64>::new();
        for (id, activity) in candidates {
            latest
                .entry(id)
                .and_modify(|current| *current = (*current).max(activity))
                .or_insert(activity);
        }
        let mut candidates: Vec<_> = latest.into_iter().collect();
        candidates.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        if self.table == "session_v2" {
            candidates.truncate(MAX_V2_RECENT_SESSIONS);
        }
        let mut root_query = connection.prepare(&format!(
            "SELECT {}, {}
               FROM {} s WHERE id = ?1 {}",
            if self.parent && self.table == "session_v2" {
                "CASE WHEN length(CAST(parent_id AS BLOB)) <= 256 THEN parent_id END"
            } else if self.parent {
                "parent_id"
            } else {
                "NULL"
            },
            if self.parent && self.table == "session_v2" {
                "parent_id IS NOT NULL AND length(CAST(parent_id AS BLOB)) > 256"
            } else {
                "0"
            },
            self.table,
            self.exclusion,
        ))?;
        let mut roots = BTreeMap::<String, i64>::new();
        for (id, activity) in candidates {
            budget.checkpoint()?;
            if self.table == "session_v2" && !valid_opencode_v2_session_id(&id) {
                continue;
            }
            let root = if let Some(root) = self.root_cache.get(&id) {
                root.clone()
            } else {
                let root = self.root(&mut root_query, &id, budget)?;
                self.root_cache.insert(id, root.clone());
                root
            };
            let activity = normalize_db_epoch(Some(activity)).unwrap_or_default();
            roots
                .entry(root)
                .and_modify(|current| *current = (*current).max(activity))
                .or_insert(activity);
        }
        Ok(roots)
    }

    fn root(
        &self,
        query: &mut rusqlite::Statement<'_>,
        id: &str,
        budget: &QueryBudget<'_>,
    ) -> rusqlite::Result<String> {
        if !self.parent {
            return Ok(id.to_owned());
        }
        let mut current = id.to_owned();
        let mut previous = current.clone();
        let mut seen = HashSet::from([current.clone()]);
        let mut depth = 0;
        loop {
            budget.checkpoint()?;
            let parent = query
                .query_row([&current], |row| {
                    if row.get::<_, bool>(1)? {
                        return Err(rusqlite::Error::InvalidQuery);
                    }
                    row.get::<_, Option<String>>(0)
                })
                .optional()?;
            let Some(parent) = parent else {
                return if self.table == "session" && depth > 0 {
                    Ok(previous)
                } else {
                    Err(rusqlite::Error::InvalidQuery)
                };
            };
            let Some(next) = parent else {
                return Ok(current);
            };
            if (self.table == "session_v2"
                && (depth == MAX_V2_PARENT_DEPTH || !valid_opencode_v2_session_id(&next)))
                || !seen.insert(next.clone())
            {
                return Err(rusqlite::Error::InvalidQuery);
            }
            previous = current;
            current = next;
            depth += 1;
        }
    }
}

fn columns(connection: &Connection, table: &str) -> rusqlite::Result<HashSet<String>> {
    connection
        .prepare("SELECT name FROM pragma_table_info(?1)")?
        .query_map([table], |row| row.get(0))?
        .collect()
}

fn activity_sql(columns: &HashSet<String>, prefix: &str) -> (String, String) {
    let clocks: Vec<_> = ["time_created", "time_updated"]
        .into_iter()
        .filter(|name| columns.contains(*name))
        .map(|name| format!("{prefix}{name}"))
        .collect();
    let activity = if clocks.is_empty() {
        "0".to_owned()
    } else {
        format!(
            "MAX(0, {})",
            clocks
                .iter()
                .map(|clock| format!("COALESCE({clock}, 0)"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let filter = if clocks.is_empty() {
        "0".to_owned()
    } else {
        clocks
            .iter()
            .map(|clock| format!("{clock} >= ?1"))
            .collect::<Vec<_>>()
            .join(" OR ")
    };
    (activity, filter)
}

fn candidate(row: &rusqlite::Row<'_>) -> rusqlite::Result<Candidate> {
    Ok((row.get(0)?, row.get(1)?))
}
