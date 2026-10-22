//! Current-file discovery with rule sections for the retained check.

use std::path::Path;

use crate::analysis::SourceFormat;
pub use crate::instructions::{
    InstructionAdapter, MAX_INSTRUCTION_FILES, MAX_INSTRUCTION_TOTAL_BYTES,
};

use super::instructions::{InstructionProvenance, InstructionSnapshot, segment_markdown};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstructionDiscovery {
    pub snapshots: Vec<InstructionSnapshot>,
    pub limitations: Vec<String>,
    pub scan_complete: bool,
}

pub async fn discover_current_instructions(
    agent: &str,
    format: SourceFormat,
    worktree_root: &Path,
    session_cwd: &Path,
    home: &Path,
) -> InstructionDiscovery {
    let current = crate::instructions::discover_current_instructions(
        agent,
        format,
        worktree_root,
        session_cwd,
        home,
    )
    .await;
    let mut discovery = InstructionDiscovery {
        snapshots: Vec::with_capacity(current.snapshots.len()),
        limitations: current.limitations,
        scan_complete: current.scan_complete,
    };
    for snapshot in current.snapshots {
        match segment_markdown(&snapshot.source, &snapshot.text) {
            Ok(sections) => discovery.snapshots.push(InstructionSnapshot {
                id: snapshot.id,
                digest: snapshot.digest,
                source: snapshot.source,
                provenance: InstructionProvenance::CurrentFileComparison,
                scope: snapshot.scope,
                text: snapshot.text,
                sections,
                imports: snapshot.imports,
                limitations: snapshot.limitations,
            }),
            Err(_) => {
                discovery
                    .limitations
                    .push("instruction_markdown_segmentation_limit".to_owned());
                discovery.scan_complete = false;
            }
        }
    }
    discovery
}

#[cfg(test)]
mod tests {
    use super::super::instructions::snapshot_from_text;
    use super::*;

    #[tokio::test]
    async fn current_file_adapter_preserves_legacy_snapshots_and_rule_sections() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("worktree");
        let home = temp.path().join("home");
        std::fs::create_dir_all(root.join(".cursor/rules")).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(root.join("AGENTS.md"), "# Rules\nUse bounded reads.\n").unwrap();
        std::fs::write(
            root.join(".cursor/rules/api.mdc"),
            "---\nglobs: src/**\n---\n# API\n- Validate input.\n- Keep errors explicit.\n",
        )
        .unwrap();

        let current = crate::instructions::discover_current_instructions(
            "cursor",
            SourceFormat::CursorCliAgentJsonl,
            &root,
            &root,
            &home,
        )
        .await;
        let legacy = discover_current_instructions(
            "cursor",
            SourceFormat::CursorCliAgentJsonl,
            &root,
            &root,
            &home,
        )
        .await;

        assert_eq!(legacy.scan_complete, current.scan_complete);
        assert_eq!(legacy.limitations, current.limitations);
        assert_eq!(legacy.snapshots.len(), 2);
        assert_eq!(legacy.snapshots.len(), current.snapshots.len());
        for (actual, raw) in legacy.snapshots.iter().zip(current.snapshots) {
            let mut expected = snapshot_from_text(
                raw.source,
                raw.text,
                InstructionProvenance::CurrentFileComparison,
                raw.scope,
            )
            .unwrap();
            expected.imports = raw.imports;
            expected.limitations = raw.limitations;
            assert_eq!(actual, &expected);
            assert_eq!(actual.id, raw.id);
            assert_eq!(actual.digest, raw.digest);
        }
    }
}
