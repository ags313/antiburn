use std::collections::BTreeMap;
use std::path::PathBuf;

use antiburn_checks::diagnostics::{Diagnostic, DiagnosticCode};

use super::MAX_DEFINITIONS;

const MAX_DIAGNOSTICS_PER_FILE: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SafeDiagnostic {
    code: &'static str,
    line: Option<usize>,
    column: Option<usize>,
    location: String,
    message: String,
}

impl SafeDiagnostic {
    fn new(diagnostic: &Diagnostic) -> Self {
        use DiagnosticCode::*;
        let message = match diagnostic.code {
            YamlSyntax => "Malformed YAML.",
            YamlDirective => "YAML directives are not allowed.",
            YamlDocumentCount => "Expected one YAML document.",
            YamlRootType => "The YAML root must be an object.",
            YamlNestingLimit => "YAML exceeds the nesting limit.",
            YamlAnchor | YamlAlias => "YAML anchors and aliases are not allowed.",
            YamlUnsupportedTag => "This YAML tag is not supported.",
            YamlNonStringKey => "Object keys must be strings.",
            YamlDuplicateKey => "Object keys must be unique.",
            YamlMergeKey => "YAML merge keys are not allowed.",
            YamlNonFiniteNumber => "Numbers must be finite JSON numbers.",
            YamlInvalidScalar => "The scalar does not match its YAML type.",
            ConfigMissingField => "A required field is missing.",
            ConfigUnknownField => "This field is not supported.",
            ConfigInvalidType => "The field has an incorrect type.",
            ConfigInvalidValue => "The field has an invalid value.",
            CompileUnsupportedFeature => "Burn Checks do not support this feature in pre-alpha.",
            CompileInvalidExpression => "The expression syntax is invalid.",
            CompileUnknownReference => "The expression references an unavailable field.",
            CompileInvalidWindow => "Window size must be 1 through 40, with 1 <= min <= size.",
            CompileInvalidCooldown => "Cooldown must be an integer duration ending in s, m, or h.",
            CompileInvalidDecision => "The decision does not match the Choice contract.",
            CompileInvalidAction => "The action does not match the status-action contract.",
            DiscoveryInvalidId => "The filename must match [a-z][a-z0-9-]*.yml or .yaml.",
            DiscoveryDuplicateId => "Multiple files define the same check ID in this scope.",
            DiscoveryReadFailed => &diagnostic.message,
            _ => "The check file is invalid.",
        };
        Self {
            code: diagnostic.code.as_str(),
            line: diagnostic.span.map(|span| span.line),
            column: diagnostic.span.map(|span| span.column),
            location: safe_location(diagnostic.config_path.as_deref().unwrap_or("")),
            message: message.to_owned(),
        }
    }

    pub(super) fn display(diagnostic: &Diagnostic) -> String {
        let safe = Self::new(diagnostic);
        let position = match (safe.line, safe.column) {
            (Some(line), Some(column)) => format!(" at {line}:{column}"),
            _ => String::new(),
        };
        format!(
            "{}{} [{}]: {}",
            safe.code, position, safe.location, safe.message
        )
    }
}

fn safe_location(path: &str) -> String {
    const FIELDS: &[&str] = &[
        "name",
        "description",
        "on",
        "if",
        "decisions",
        "actions",
        "tool_result",
        "tools",
        "status",
        "models",
        "fast_mode",
        "paths",
        "paths-ignore",
        "window",
        "size",
        "min",
        "cooldown",
        "model",
        "input",
        "output",
        "type",
        "question",
        "criteria",
        "result",
        "message",
    ];
    if path.is_empty() {
        return "root".into();
    }
    path.split('.')
        .take(16)
        .map(|segment| {
            let name = segment.split('[').next().unwrap_or(segment);
            if FIELDS.contains(&name)
                && segment.len() <= 32
                && segment[name.len()..]
                    .chars()
                    .all(|ch| ch.is_ascii_digit() || ch == '[' || ch == ']')
            {
                segment
            } else {
                "<name>"
            }
        })
        .collect::<Vec<_>>()
        .join(".")
}

#[derive(Default)]
pub(super) struct DiagnosticLog {
    previous: BTreeMap<PathBuf, Vec<SafeDiagnostic>>,
}

pub(super) type DiagnosticBatch = BTreeMap<PathBuf, Vec<SafeDiagnostic>>;

pub(super) fn collect(batch: &mut DiagnosticBatch, diagnostics: &[Diagnostic]) {
    for diagnostic in diagnostics {
        if matches!(
            diagnostic.code,
            DiagnosticCode::DiscoveryPaused | DiagnosticCode::DiscoveryShadowed
        ) {
            continue;
        }
        let Some(path) = &diagnostic.source_path else {
            continue;
        };
        if batch.len() >= MAX_DEFINITIONS * 2 && !batch.contains_key(path) {
            continue;
        }
        let errors = batch.entry(path.clone()).or_default();
        let safe = SafeDiagnostic::new(diagnostic);
        if errors.len() < MAX_DIAGNOSTICS_PER_FILE && !errors.contains(&safe) {
            errors.push(safe);
        }
    }
}

impl DiagnosticLog {
    pub(super) fn publish(&mut self, current: DiagnosticBatch) {
        for (path, diagnostics) in &current {
            if self.previous.get(path) == Some(diagnostics) {
                continue;
            }
            for diagnostic in diagnostics {
                ::tracing::error!(
                    event = "installed_check_invalid",
                    path = %path.display(),
                    code = diagnostic.code,
                    line = diagnostic.line,
                    column = diagnostic.column,
                    config_location = diagnostic.location,
                    message = diagnostic.message,
                );
            }
        }
        self.previous = current;
    }
}
