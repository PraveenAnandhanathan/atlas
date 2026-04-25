//! Path normalization and splitting.
//!
//! ATLAS paths are always absolute unix-style strings: start with `/`,
//! no backslashes, no `..`, no `.`, no trailing slash (except the root
//! which is exactly `"/"`).

use atlas_core::{Error, Result};

/// Normalize a user-provided path into canonical ATLAS form.
///
/// - Replaces backslashes with forward slashes.
/// - Collapses duplicate slashes.
/// - Rejects `..` and `.` segments.
/// - Strips trailing slashes (root is preserved as `"/"`).
pub fn normalize_path(input: &str) -> Result<String> {
    if input.is_empty() {
        return Err(Error::BadPath("empty path".into()));
    }
    let normalized = input.replace('\\', "/");
    if !normalized.starts_with('/') {
        return Err(Error::BadPath(format!("not absolute: {input}")));
    }
    let mut parts: Vec<&str> = Vec::new();
    for part in normalized.split('/') {
        if part.is_empty() {
            continue;
        }
        if part == "." || part == ".." {
            return Err(Error::BadPath(format!("illegal segment in path: {input}")));
        }
        parts.push(part);
    }
    if parts.is_empty() {
        return Ok("/".to_string());
    }
    Ok(format!("/{}", parts.join("/")))
}

/// Split an absolute path into its name segments.
///
/// Root `"/"` splits into an empty vector.
pub fn split_path(path: &str) -> Result<Vec<String>> {
    let n = normalize_path(path)?;
    if n == "/" {
        return Ok(Vec::new());
    }
    Ok(n.trim_start_matches('/')
        .split('/')
        .map(String::from)
        .collect())
}

/// Split a path into `(parent_dir, basename)`.
///
/// Fails on the root path — root has no parent.
pub fn parent_and_name(path: &str) -> Result<(String, String)> {
    let parts = split_path(path)?;
    if parts.is_empty() {
        return Err(Error::BadPath("root has no parent".into()));
    }
    let name = parts.last().unwrap().clone();
    let parent = if parts.len() == 1 {
        "/".to_string()
    } else {
        format!("/{}", parts[..parts.len() - 1].join("/"))
    };
    Ok((parent, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_cases() {
        assert_eq!(normalize_path("/").unwrap(), "/");
        assert_eq!(normalize_path("/a").unwrap(), "/a");
        assert_eq!(normalize_path("/a/b/c").unwrap(), "/a/b/c");
        assert_eq!(normalize_path("/a//b/").unwrap(), "/a/b");
        assert_eq!(normalize_path("\\a\\b").unwrap(), "/a/b");
    }

    #[test]
    fn rejects_relative_and_dots() {
        assert!(normalize_path("").is_err());
        assert!(normalize_path("a/b").is_err());
        assert!(normalize_path("/a/../b").is_err());
        assert!(normalize_path("/a/./b").is_err());
    }

    #[test]
    fn split_and_parent() {
        assert_eq!(split_path("/").unwrap(), Vec::<String>::new());
        assert_eq!(split_path("/a/b").unwrap(), vec!["a", "b"]);
        assert_eq!(
            parent_and_name("/a/b/c").unwrap(),
            ("/a/b".to_string(), "c".to_string())
        );
        assert_eq!(
            parent_and_name("/a").unwrap(),
            ("/".to_string(), "a".to_string())
        );
        assert!(parent_and_name("/").is_err());
    }
}
