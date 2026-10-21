//! The remote store of each agent that the helper can export.
//!
//! [`REMOTE_STORES`] is the only list of remote agents. The protocol agent
//! lists, the bundle layout, and the reader dispatch come from it.

use antiburn_local::model::AgentKind;

use crate::export::BundleCompanion;

/// The kind of the first file of a bundle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreFormat {
    /// A database that holds the session rows.
    Db,
    /// A transcript file.
    File,
}

#[derive(Clone, Copy, Debug)]
pub struct RemoteStore {
    pub agent: AgentKind,
    /// A base agent is in every helper and desktop, and the collector reads
    /// its transcripts. The engine's discovery reads the other agents, and a
    /// desktop must request them by name.
    pub base: bool,
    pub format: StoreFormat,
    /// The path of the first bundle file for a session ID, relative to the
    /// cache directory.
    pub primary: fn(&str) -> String,
    /// The fixed companions of a store bundle, in bundle order after the
    /// first file.
    pub companions: &'static [BundleCompanion],
}

pub const REMOTE_STORES: &[RemoteStore] = &[
    RemoteStore {
        agent: AgentKind::Claude,
        base: true,
        format: StoreFormat::File,
        primary: |session_id| format!("{session_id}.jsonl"),
        companions: &[],
    },
    RemoteStore {
        agent: AgentKind::Codex,
        base: true,
        format: StoreFormat::File,
        primary: |session_id| format!("{session_id}.jsonl"),
        companions: &[],
    },
    RemoteStore {
        agent: AgentKind::Cursor,
        base: false,
        format: StoreFormat::File,
        primary: |session_id| format!("{session_id}.jsonl"),
        companions: &[],
    },
    RemoteStore {
        agent: AgentKind::OpenCode,
        base: false,
        format: StoreFormat::Db,
        primary: |_| "opencode.db".to_owned(),
        companions: &[],
    },
    RemoteStore {
        agent: AgentKind::Antigravity,
        base: false,
        format: StoreFormat::Db,
        primary: |session_id| format!("conversations/{session_id}.db"),
        companions: &[BundleCompanion::AntigravityBrain],
    },
    RemoteStore {
        agent: AgentKind::Junie,
        base: false,
        format: StoreFormat::File,
        // The Junie parser takes the session ID from the folder name.
        primary: |session_id| format!("{session_id}/events.jsonl"),
        companions: &[],
    },
];

impl RemoteStore {
    pub fn for_slug(agent: &str) -> Option<&'static Self> {
        REMOTE_STORES
            .iter()
            .find(|store| store.agent.slug() == agent)
    }

    /// The agent of a store that the engine's discovery reads.
    pub(crate) fn discovered_kind(agent: &str) -> Option<AgentKind> {
        Self::for_slug(agent)
            .filter(|store| !store.base)
            .map(|store| store.agent)
    }
}

/// The number of stores that `base` selects. `None` selects every store.
const fn count(base: Option<bool>) -> usize {
    let mut total = 0;
    let mut index = 0;
    while index < REMOTE_STORES.len() {
        if selects(base, REMOTE_STORES[index].base) {
            total += 1;
        }
        index += 1;
    }
    total
}

/// The slugs of the stores that `base` selects, in table order.
const fn slugs<const N: usize>(base: Option<bool>) -> [&'static str; N] {
    let mut slugs = [""; N];
    let mut next = 0;
    let mut index = 0;
    while index < REMOTE_STORES.len() {
        if selects(base, REMOTE_STORES[index].base) {
            slugs[next] = REMOTE_STORES[index].agent.slug();
            next += 1;
        }
        index += 1;
    }
    slugs
}

const fn selects(base: Option<bool>, store_base: bool) -> bool {
    match base {
        Some(base) => base == store_base,
        None => true,
    }
}

pub(crate) const BASE_SLUGS: [&str; count(Some(true))] = slugs(Some(true));
pub(crate) const OPTIONAL_SLUGS: [&str; count(Some(false))] = slugs(Some(false));
pub(crate) const SUPPORTED_SLUGS: [&str; count(None)] = slugs(None);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_agent_has_one_store() {
        for store in REMOTE_STORES {
            assert_eq!(
                RemoteStore::for_slug(store.agent.slug()).map(|found| found.agent),
                Some(store.agent)
            );
        }
        assert!(RemoteStore::for_slug("unknown").is_none());
    }

    #[test]
    fn agent_lists_follow_the_table() {
        assert_eq!(BASE_SLUGS, ["claude-code", "codex"]);
        assert_eq!(
            OPTIONAL_SLUGS,
            ["cursor", "opencode", "antigravity", "junie"]
        );
        assert_eq!(
            SUPPORTED_SLUGS.to_vec(),
            [BASE_SLUGS.as_slice(), OPTIONAL_SLUGS.as_slice()].concat()
        );
    }

    #[test]
    fn only_discovered_stores_have_a_discovered_kind() {
        assert_eq!(RemoteStore::discovered_kind("claude-code"), None);
        assert_eq!(RemoteStore::discovered_kind("codex"), None);
        assert_eq!(
            RemoteStore::discovered_kind("antigravity"),
            Some(AgentKind::Antigravity)
        );
        assert_eq!(RemoteStore::discovered_kind("unknown"), None);
    }

    #[test]
    fn base_stores_have_no_fixed_companions() {
        for store in REMOTE_STORES.iter().filter(|store| store.base) {
            assert!(store.companions.is_empty());
            assert_eq!(store.format, StoreFormat::File);
        }
    }

    #[test]
    fn database_stores_name_a_database() {
        for store in REMOTE_STORES {
            let primary = (store.primary)("session-1");
            assert_eq!(
                primary.ends_with(".db"),
                store.format == StoreFormat::Db,
                "{primary}"
            );
        }
    }
}
