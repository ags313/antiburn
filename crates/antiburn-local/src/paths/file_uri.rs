//! Shared file URI path handling.

/// Remove the file scheme and normalize a local Windows drive prefix.
pub fn strip_prefix(path: &str) -> &str {
    let Some(path) = path.strip_prefix("file://") else {
        return path;
    };
    let path = path
        .strip_prefix("localhost")
        .filter(|local| local.starts_with('/'))
        .unwrap_or(path);
    match path.as_bytes() {
        [b'/', drive, b':', ..] if drive.is_ascii_alphabetic() => &path[1..],
        _ => path,
    }
}

/// Decode an absolute file URI. Other schemes and invalid UTF-8 give `None`.
pub fn to_path(uri: &str) -> Option<String> {
    uri.strip_prefix("file://")?;
    let encoded = strip_prefix(uri);
    let mut bytes = Vec::with_capacity(encoded.len());
    let mut rest = encoded.as_bytes();
    while let Some((&byte, tail)) = rest.split_first() {
        if byte == b'%'
            && let [high, low, tail @ ..] = tail
            && let (Some(high), Some(low)) =
                ((*high as char).to_digit(16), (*low as char).to_digit(16))
        {
            bytes.push((high * 16 + low) as u8);
            rest = tail;
            continue;
        }
        bytes.push(byte);
        rest = tail;
    }
    let path = String::from_utf8(bytes).ok()?;
    let path = match path.as_bytes() {
        [b'/', drive, b':', ..] if drive.is_ascii_alphabetic() => path[1..].to_owned(),
        _ => path,
    };
    (path.starts_with('/') || path.get(1..2) == Some(":")).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_absolute_file_uris() {
        for (uri, expected) in [
            ("file:///tmp/a%20b/%E2%98%83", "/tmp/a b/☃"),
            ("file:///C:/Users/A%20B", "C:/Users/A B"),
            ("file://localhost/tmp/project", "/tmp/project"),
            ("file://localhost/C:/project", "C:/project"),
            ("file:///tmp/%zz%2", "/tmp/%zz%2"),
        ] {
            assert_eq!(to_path(uri).as_deref(), Some(expected));
        }
        for uri in ["https:///tmp/a", "file://relative", "file:///tmp/%FF"] {
            assert_eq!(to_path(uri), None);
        }
    }

    #[test]
    fn strips_only_file_uri_prefixes() {
        for (input, expected) in [
            ("/tmp/a%20b", "/tmp/a%20b"),
            ("file:///tmp/a%20b", "/tmp/a%20b"),
            ("file:///C:/project", "C:/project"),
            ("file://localhost/C:/project", "C:/project"),
            ("file://localhost/tmp/project", "/tmp/project"),
        ] {
            assert_eq!(strip_prefix(input), expected);
        }
    }
}
