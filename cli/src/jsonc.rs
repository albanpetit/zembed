//! Writes JSON with comments, the way Zed's own files look: two-space
//! indentation, a `// comment` before an entry, short arrays on one line.

use serde_json::Value;

/// A top-level array preceded by `header` lines, each entry preceded by its
/// comment.
pub fn array(header: &[&str], entries: &[(&str, Value)]) -> String {
    let mut out = String::new();
    for line in header {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str("[\n");
    for (i, (comment, value)) in entries.iter().enumerate() {
        if !comment.is_empty() {
            out.push_str(&format!("  // {comment}\n"));
        }
        out.push_str("  ");
        write(value, 1, &mut out);
        out.push_str(if i + 1 < entries.len() { ",\n" } else { "\n" });
    }
    out.push_str("]\n");
    out
}

fn write(value: &Value, depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth + 1);
    let close = "  ".repeat(depth);
    match value {
        Value::Object(map) if !map.is_empty() => {
            out.push_str("{\n");
            for (i, (key, value)) in map.iter().enumerate() {
                out.push_str(&pad);
                out.push_str(&Value::String(key.clone()).to_string());
                out.push_str(": ");
                write(value, depth + 1, out);
                out.push_str(if i + 1 < map.len() { ",\n" } else { "\n" });
            }
            out.push_str(&close);
            out.push('}');
        }
        // Arrays of plain values stay on one line: ["run", "-t", "upload"].
        Value::Array(items) if items.iter().all(|v| !v.is_object() && !v.is_array()) => {
            let items: Vec<String> = items.iter().map(Value::to_string).collect();
            out.push('[');
            out.push_str(&items.join(", "));
            out.push(']');
        }
        Value::Array(items) => {
            out.push_str("[\n");
            for (i, item) in items.iter().enumerate() {
                out.push_str(&pad);
                write(item, depth + 1, out);
                out.push_str(if i + 1 < items.len() { ",\n" } else { "\n" });
            }
            out.push_str(&close);
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// Parses JSON with `//` line comments, as Zed reads its files.
#[cfg(test)]
pub fn parse(text: &str) -> Value {
    let stripped: String = text
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    serde_json::from_str(&stripped).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn writes_commented_arrays() {
        let text = array(
            &["// header"],
            &[
                (
                    "first",
                    json!({ "label": "a", "build": { "args": ["run", "-t"] } }),
                ),
                ("", json!({ "label": "b", "empty": {} })),
            ],
        );
        assert_eq!(
            text,
            "// header\n[\n  // first\n  {\n    \"label\": \"a\",\n    \"build\": {\n      \"args\": [\"run\", \"-t\"]\n    }\n  },\n  {\n    \"label\": \"b\",\n    \"empty\": {}\n  }\n]\n"
        );
        assert_eq!(parse(&text)[0]["build"]["args"][1], "-t");
    }
}
