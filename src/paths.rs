//! Small path helpers shared by the adapters.
//!
//! Paths are handled as strings: the extension runs in WASM and the paths
//! belong to the user's machine (possibly a remote one), not to the sandbox.

pub fn is_absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    path.starts_with('/')
        || path.starts_with('\\')
        || (bytes.len() > 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':')
}

/// Resolves `path` against `base` unless it is already absolute.
pub fn resolve_path(path: &str, base: &str) -> String {
    if is_absolute(path) {
        return path.to_string();
    }
    let path = path.strip_prefix("./").unwrap_or(path);
    format!("{}/{}", base.trim_end_matches(['/', '\\']), path)
}

/// Expands a leading `~` to the user's home directory, when it is known.
pub fn expand_home(path: &str, home: Option<&str>) -> String {
    match (path.strip_prefix('~'), home) {
        (Some(""), Some(home)) => home.to_string(),
        (Some(rest), Some(home)) if rest.starts_with('/') => {
            format!("{}{}", home.trim_end_matches('/'), rest)
        }
        _ => path.to_string(),
    }
}

/// Quotes a string for a POSIX shell.
pub fn sh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_paths() {
        assert_eq!(resolve_path("a/b.elf", "/work/"), "/work/a/b.elf");
        assert_eq!(resolve_path("./a.elf", "/work"), "/work/a.elf");
        assert_eq!(resolve_path("/abs/a.elf", "/work"), "/abs/a.elf");
        assert_eq!(resolve_path("C:\\fw\\a.elf", "/work"), "C:\\fw\\a.elf");
    }

    #[test]
    fn expands_home() {
        assert_eq!(
            expand_home("~/bin/gdb", Some("/home/me")),
            "/home/me/bin/gdb"
        );
        assert_eq!(expand_home("~", Some("/home/me/")), "/home/me/");
        assert_eq!(expand_home("~user/gdb", Some("/home/me")), "~user/gdb");
        assert_eq!(expand_home("~/bin/gdb", None), "~/bin/gdb");
        assert_eq!(
            expand_home("/usr/bin/gdb", Some("/home/me")),
            "/usr/bin/gdb"
        );
    }

    #[test]
    fn quotes_for_shell() {
        assert_eq!(sh_quote("a b"), "'a b'");
        assert_eq!(sh_quote("it's"), r"'it'\''s'");
    }
}
