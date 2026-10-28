//! Lossless JSONC insertion and exact owned-delta removal.

use std::collections::BTreeSet;
use std::ops::Range;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone)]
struct Node {
    range: Range<usize>,
    object: Vec<(String, Node)>,
    array: Vec<Node>,
    kind: u8,
}

struct Parser<'a> {
    bytes: &'a [u8],
    index: usize,
}

pub type Result<T> = std::result::Result<T, &'static str>;

impl Parser<'_> {
    fn space(&mut self) -> Result<()> {
        loop {
            while self
                .bytes
                .get(self.index)
                .is_some_and(u8::is_ascii_whitespace)
            {
                self.index += 1;
            }
            if self.bytes.get(self.index..self.index + 2) == Some(b"//") {
                self.index += 2;
                while self.bytes.get(self.index).is_some_and(|b| *b != b'\n') {
                    self.index += 1;
                }
            } else if self.bytes.get(self.index..self.index + 2) == Some(b"/*") {
                self.index += 2;
                loop {
                    if self.bytes.get(self.index..self.index + 2) == Some(b"*/") {
                        self.index += 2;
                        break;
                    }
                    if self.index >= self.bytes.len() {
                        return Err("malformed_jsonc");
                    }
                    self.index += 1;
                }
            } else {
                return Ok(());
            }
        }
    }

    fn string(&mut self) -> Result<String> {
        let start = self.index;
        if self.bytes.get(self.index) != Some(&b'"') {
            return Err("malformed_jsonc");
        }
        self.index += 1;
        while let Some(byte) = self.bytes.get(self.index) {
            self.index += 1;
            if *byte == b'\\' {
                self.index += 1;
            } else if *byte == b'"' {
                return serde_json::from_slice(&self.bytes[start..self.index])
                    .map_err(|_| "malformed_jsonc");
            }
        }
        Err("malformed_jsonc")
    }

    fn node(&mut self, depth: usize) -> Result<Node> {
        if depth > 32 {
            return Err("json_depth_limit");
        }
        self.space()?;
        let start = self.index;
        let kind = *self.bytes.get(start).ok_or("malformed_jsonc")?;
        let mut object = Vec::new();
        let mut array = Vec::new();
        if matches!(kind, b'{' | b'[') {
            self.index += 1;
            let end = if kind == b'{' { b'}' } else { b']' };
            let mut keys = BTreeSet::new();
            loop {
                self.space()?;
                if self.bytes.get(self.index) == Some(&end) {
                    self.index += 1;
                    break;
                }
                if kind == b'{' {
                    let key = self.string()?;
                    if !keys.insert(key.clone()) {
                        return Err("duplicate_json_key");
                    }
                    self.space()?;
                    if self.bytes.get(self.index) != Some(&b':') {
                        return Err("malformed_jsonc");
                    }
                    self.index += 1;
                    object.push((key, self.node(depth + 1)?));
                } else {
                    array.push(self.node(depth + 1)?);
                }
                self.space()?;
                match self.bytes.get(self.index) {
                    Some(b',') => self.index += 1,
                    Some(byte) if *byte == end => {
                        self.index += 1;
                        break;
                    }
                    _ => return Err("malformed_jsonc"),
                }
            }
        } else if kind == b'"' {
            self.string()?;
        } else {
            while self
                .bytes
                .get(self.index)
                .is_some_and(|b| !b.is_ascii_whitespace() && !b",]}/".contains(b))
            {
                self.index += 1;
            }
            if self.index == start {
                return Err("malformed_jsonc");
            }
            serde_json::from_slice::<Value>(&self.bytes[start..self.index])
                .map_err(|_| "malformed_jsonc")?;
        }
        Ok(Node {
            range: start..self.index,
            object,
            array,
            kind,
        })
    }
}

fn parse(bytes: &[u8]) -> Result<Node> {
    if bytes.len() > 256 * 1024 {
        return Err("config_too_large");
    }
    std::str::from_utf8(bytes).map_err(|_| "malformed_jsonc")?;
    let mut parser = Parser { bytes, index: 0 };
    let node = parser.node(0)?;
    parser.space()?;
    if parser.index != bytes.len() || node.kind != b'{' {
        return Err("malformed_jsonc");
    }
    Ok(node)
}

fn member<'a>(node: &'a Node, path: &[String]) -> Option<&'a Node> {
    if path.is_empty() {
        return Some(node);
    }
    node.object
        .iter()
        .find(|(key, _)| key == &path[0])
        .and_then(|(_, child)| member(child, &path[1..]))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Delta {
    pub parent: Vec<String>,
    pub inserted: String,
    pub value: Value,
}

/// Add one array entry, or the smallest missing object that contains it.
pub fn append(bytes: &[u8], path: &[String], value: Value) -> Result<(Vec<u8>, Delta)> {
    let root = parse(bytes)?;
    if path.is_empty() || path.len() > 4 {
        return Err("invalid_config_path");
    }
    let mut parent = &root;
    let mut prefix = Vec::new();
    for (index, key) in path.iter().enumerate() {
        if parent.kind != b'{' {
            return Err("config_schema_conflict");
        }
        if let Some((_, child)) = parent.object.iter().find(|(name, _)| name == key) {
            if index + 1 == path.len() {
                if child.kind != b'[' {
                    return Err("config_schema_conflict");
                }
                let encoded =
                    serde_json::to_string(&value).map_err(|_| "config_schema_conflict")?;
                // Insert before the first item so all original comments and commas remain.
                let inserted =
                    format!("{encoded}{}", if child.array.is_empty() { "" } else { "," });
                let position = child.range.start + 1;
                let mut result = bytes.to_vec();
                result.splice(position..position, inserted.bytes());
                parse(&result)?;
                let delta = Delta {
                    parent: path.to_vec(),
                    inserted,
                    value,
                };
                return Ok((result, delta));
            }
            parent = child;
            prefix.push(key.clone());
        } else {
            let mut nested = Value::Array(vec![value.clone()]);
            for missing in path[index + 1..].iter().rev() {
                nested = serde_json::json!({ missing: nested });
            }
            let encoded_key = serde_json::to_string(key).map_err(|_| "invalid_config_path")?;
            let inserted = format!(
                "{encoded_key}:{nested}{}",
                if parent.object.is_empty() { "" } else { "," }
            );
            let mut result = bytes.to_vec();
            result.splice(
                parent.range.start + 1..parent.range.start + 1,
                inserted.bytes(),
            );
            parse(&result)?;
            return Ok((
                result,
                Delta {
                    parent: prefix,
                    inserted,
                    value,
                },
            ));
        }
    }
    Err("invalid_config_path")
}

/// Remove only the unchanged bytes at the recorded parent boundary.
pub fn remove(bytes: &[u8], delta: &Delta) -> Result<Vec<u8>> {
    let root = parse(bytes)?;
    let parent = member(&root, &delta.parent).ok_or("owned_entry_removed")?;
    let start = parent.range.start + 1;
    let inserted = delta.inserted.as_bytes();
    if bytes.get(start..start + inserted.len()) != Some(inserted) {
        return Err("owned_entry_changed");
    }
    let mut output = bytes.to_vec();
    output.drain(start..start + inserted.len());
    parse(&output)?;
    Ok(output)
}

pub fn validate(bytes: &[u8]) -> Result<()> {
    parse(bytes).map(|_| ())
}

pub fn boolean(bytes: &[u8], path: &[String]) -> Result<Option<bool>> {
    let root = parse(bytes)?;
    let Some(node) = member(&root, path) else {
        return Ok(None);
    };
    match &bytes[node.range.clone()] {
        b"true" => Ok(Some(true)),
        b"false" => Ok(Some(false)),
        _ => Err("config_schema_conflict"),
    }
}

pub fn strings(bytes: &[u8], path: &[String]) -> Result<Vec<String>> {
    let root = parse(bytes)?;
    let Some(node) = member(&root, path) else {
        return Ok(Vec::new());
    };
    if node.kind != b'[' {
        return Err("config_schema_conflict");
    }
    node.array
        .iter()
        .map(|node| {
            serde_json::from_slice(&bytes[node.range.clone()]).map_err(|_| "config_schema_conflict")
        })
        .collect()
}

fn to_value(node: &Node, bytes: &[u8]) -> Result<Value> {
    match node.kind {
        b'{' => node
            .object
            .iter()
            .map(|(key, child)| Ok((key.clone(), to_value(child, bytes)?)))
            .collect::<Result<serde_json::Map<_, _>>>()
            .map(Value::Object),
        b'[' => node
            .array
            .iter()
            .map(|child| to_value(child, bytes))
            .collect::<Result<Vec<_>>>()
            .map(Value::Array),
        _ => serde_json::from_slice(&bytes[node.range.clone()]).map_err(|_| "malformed_jsonc"),
    }
}

pub fn value(bytes: &[u8], path: &[String]) -> Result<Option<Value>> {
    let root = parse(bytes)?;
    member(&root, path)
        .map(|node| to_value(node, bytes))
        .transpose()
}

pub fn insert_member(bytes: &[u8], key: &str, value: Value) -> Result<(Vec<u8>, Delta)> {
    let root = parse(bytes)?;
    if root.object.iter().any(|(name, _)| name == key) {
        return Err("config_schema_conflict");
    }
    let inserted = format!(
        "{}:{value}{}",
        serde_json::to_string(key).map_err(|_| "invalid_config_path")?,
        if root.object.is_empty() { "" } else { "," }
    );
    let mut output = bytes.to_vec();
    output.splice(root.range.start + 1..root.range.start + 1, inserted.bytes());
    parse(&output)?;
    Ok((
        output,
        Delta {
            parent: Vec::new(),
            inserted,
            value,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insertion_and_removal_preserve_all_original_bytes() {
        for original in [
            "{ // greeting\n \"other\": true, /* trailing */ }\n",
            "{\"hooks\":{\"UserPromptSubmit\":[ // old hook\n {\"hooks\":[]},],},\"other\":42}",
            "{\"hooks\":{}}",
            "{}",
        ] {
            let path = vec!["hooks".into(), "UserPromptSubmit".into()];
            let (output, delta) = append(
                original.as_bytes(),
                &path,
                serde_json::json!({"hooks": [{"command":"test"}]}),
            )
            .unwrap();
            assert_eq!(remove(&output, &delta).unwrap(), original.as_bytes());
            assert!(String::from_utf8(output).unwrap().contains("test"));
        }
    }

    #[test]
    fn removal_preserves_later_unrelated_edits_and_refuses_owned_edits() {
        let (output, delta) = append(
            br#"{"hooks":{"test":[]},"unrelated":true}"#,
            &["hooks".into(), "test".into()],
            serde_json::json!({"command":"owned"}),
        )
        .unwrap();
        let updated = String::from_utf8(output).unwrap().replace("true", "false");
        assert_eq!(
            String::from_utf8(remove(updated.as_bytes(), &delta).unwrap()).unwrap(),
            r#"{"hooks":{"test":[]},"unrelated":false}"#
        );
        assert!(remove(updated.replace("owned", "changed").as_bytes(), &delta).is_err());
    }

    #[test]
    fn malformed_duplicate_and_conflicting_documents_are_refused() {
        for bytes in [
            r#"{"hooks":{},"hooks":{}}"#,
            "{broken}",
            "{/*open",
            r#"{"hooks":false}"#,
        ] {
            assert!(
                append(
                    bytes.as_bytes(),
                    &["hooks".into(), "test".into()],
                    Value::Null
                )
                .is_err(),
                "{bytes}"
            );
        }
    }
}
