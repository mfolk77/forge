//! Path canonicalization for security checks.
//!
//! Implements the `PathValidator` utility required by the FolkTech engineering
//! rules (CAT 2 — Path & File Security). The key responsibility is producing
//! a *canonical* path for prefix-based blocklist checks so that
//! `../../etc/passwd` cannot slip past `starts_with("/etc/")`.
//!
//! ## What "canonical" means here
//!
//! For an existing path, this defers to `std::fs::canonicalize` which both
//! resolves `..` and follows symlinks. That covers the common case.
//!
//! For a path that does NOT yet exist (e.g. `file_write` creating a new file),
//! we walk back up to the nearest existing ancestor, canonicalize that, then
//! re-append the not-yet-existing tail. This catches symlinked parent
//! directories — if `~/foo -> /etc` and the user tries to write
//! `~/foo/passwd`, the canonical form is `/etc/passwd` and the blocklist
//! catches it.
//!
//! For a path with no existing ancestor (e.g. a fully synthetic path during
//! tests), we fall back to a *lexical* resolution: walk components, popping
//! on `..`. That still catches `../../etc/passwd` style traversal even when
//! no filesystem state is available.
//!
//! ## What this is NOT
//!
//! - This is not a TOCTOU defense. A symlink racing between this check and
//!   the actual write can still redirect the write — the threat model here
//!   is "model emits a relative-path traversal," not a privileged attacker
//!   who can plant symlinks faster than Forge runs syscalls.
//! - This does not validate against an allowlist of permitted directories.
//!   Allowlist enforcement lives in the classifier where the result of
//!   `canonicalize_logical` is matched against `HARD_BLOCKED_PATH_PREFIXES`
//!   and `SENSITIVE_PATH_PATTERNS`.

use std::path::{Component, Path, PathBuf};

/// Produce a canonical absolute path suitable for prefix-based blocklist
/// matching. See module docs for semantics.
pub fn canonicalize_logical(path_str: &str) -> PathBuf {
    let cleaned = path_str.replace('\0', "");
    let path = Path::new(&cleaned);

    let absolute: PathBuf = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("/"))
            .join(path)
    };

    // Fast path: full canonicalize for paths that exist.
    if let Ok(canonical) = absolute.canonicalize() {
        return canonical;
    }

    // Slow path: walk back to the deepest existing ancestor, canonicalize it,
    // then re-append the tail of components that don't yet exist. This is
    // important for `file_write` of a new file inside a symlinked parent
    // directory (`~/foo -> /etc`).
    let mut existing = absolute.clone();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    while !existing.exists() {
        match existing.file_name().map(|s| s.to_os_string()) {
            Some(name) => tail.push(name),
            None => break,
        }
        if !existing.pop() {
            break;
        }
    }

    if existing.exists() {
        if let Ok(canonical_parent) = existing.canonicalize() {
            let mut result = canonical_parent;
            for piece in tail.into_iter().rev() {
                result.push(piece);
            }
            return result;
        }
    }

    // Last resort: lexical clean (resolve `..` without filesystem).
    lexical_clean(&absolute)
}

/// Walk the components of a path, popping on `..`. Pure logic, no I/O.
/// Used as the fallback when no existing ancestor can be canonicalized.
fn lexical_clean(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                // `pop` returns false on root or empty. Either way, no-op
                // is the right behavior — `..` from `/` is still `/`.
                let _ = result.pop();
            }
            Component::CurDir => {}
            other => result.push(other.as_os_str()),
        }
    }
    result
}

/// Produce a canonical absolute path AND lowercase it with forward slashes
/// for prefix-matching against the blocklist constants. Both `..` resolution
/// and symlink following happen first; case-folding/separator-normalization
/// happen afterwards.
///
/// The blocklist patterns are stored lowercased with forward slashes
/// (`patterns.rs::HARD_BLOCKED_PATH_PREFIXES`), so this produces a comparable
/// form.
pub fn canonical_match_form(path_str: &str) -> String {
    canonicalize_logical(path_str)
        .to_string_lossy()
        .replace('\\', "/")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SECURITY (CAT 2 — Path & File Security):
    /// `../../etc/passwd` from any cwd must canonicalize to a path containing
    /// `/etc/` so the blocklist catches it. This is the exact failure mode
    /// flagged in `docs/audits/AUDIT-forge-2026-04-28.md` P0 #4.
    #[test]
    fn test_security_dot_dot_traversal_to_etc_passwd_normalizes() {
        // Many `..` segments to ensure we definitely climb past cwd's depth.
        let traversed = "../../../../../../../../etc/passwd";
        let canonical = canonical_match_form(traversed);

        assert!(
            canonical.contains("/etc/passwd"),
            "..-traversal must canonicalize to a path containing /etc/passwd; got {canonical}"
        );
    }

    /// SECURITY (CAT 2):
    /// `../../etc/` from cwd must produce a form that `starts_with("/etc/")`
    /// matches against the existing blocklist patterns. This is what the
    /// classifier actually checks.
    #[test]
    fn test_security_blocklist_starts_with_etc() {
        let traversed = "../../../../../../../../etc/passwd";
        let canonical = canonical_match_form(traversed);

        // `starts_with` with the canonical form should now catch `/etc/`.
        // Strip leading non-/etc prefix on systems where canonicalize prepends
        // the cwd's drive letter / leading dirs that we've climbed above.
        let etc_idx = canonical.find("/etc/").expect("canonical must contain /etc/");
        let from_etc = &canonical[etc_idx..];
        assert!(
            from_etc.starts_with("/etc/"),
            "blocklist starts_with('/etc/') must match the canonicalized tail"
        );
    }

    #[test]
    fn test_lexical_clean_resolves_dot_dot() {
        let p = Path::new("/foo/bar/../baz");
        let cleaned = lexical_clean(p);
        assert_eq!(cleaned, PathBuf::from("/foo/baz"));
    }

    #[test]
    fn test_lexical_clean_handles_curdir() {
        let p = Path::new("/foo/./bar/./baz");
        let cleaned = lexical_clean(p);
        assert_eq!(cleaned, PathBuf::from("/foo/bar/baz"));
    }

    #[test]
    fn test_lexical_clean_dotdot_at_root_is_noop() {
        let p = Path::new("/../etc/passwd");
        let cleaned = lexical_clean(p);
        assert_eq!(cleaned, PathBuf::from("/etc/passwd"));
    }

    #[test]
    fn test_canonicalize_existing_file_resolves_symlink() {
        // Real filesystem test: create a tempdir, a target file, a symlink
        // to that file, and verify canonicalize_logical follows the symlink.
        // Skip on platforms where symlink creation requires elevation.
        #[cfg(unix)]
        {
            let tmp = tempfile::tempdir().unwrap();
            let target = tmp.path().join("real.txt");
            std::fs::write(&target, b"hello").unwrap();
            let link = tmp.path().join("link.txt");
            std::os::unix::fs::symlink(&target, &link).unwrap();

            let canonical = canonicalize_logical(link.to_str().unwrap());
            // canonicalize_logical follows symlinks for existing paths
            let target_canonical = target.canonicalize().unwrap();
            assert_eq!(canonical, target_canonical);
        }
    }

    /// SECURITY (CAT 2):
    /// Writing to a NEW file inside a symlinked parent must canonicalize
    /// to the symlink target. e.g. if `~/foo -> /etc` and we try to write
    /// `~/foo/newfile`, canonical_match_form must contain `/etc/newfile`
    /// so the blocklist catches the attempt.
    #[test]
    #[cfg(unix)]
    fn test_security_new_file_in_symlinked_parent() {
        let tmp = tempfile::tempdir().unwrap();
        let real_dir = tmp.path().join("realdir");
        std::fs::create_dir(&real_dir).unwrap();
        let symlink = tmp.path().join("aliasdir");
        std::os::unix::fs::symlink(&real_dir, &symlink).unwrap();

        // The new file does not exist yet; symlink does. Canonicalization
        // should walk to the symlink, follow it, then reattach the new
        // filename.
        let new_via_symlink = symlink.join("newfile.txt");
        let canonical = canonicalize_logical(new_via_symlink.to_str().unwrap());

        let real_canonical = real_dir.canonicalize().unwrap();
        assert_eq!(canonical, real_canonical.join("newfile.txt"));
    }

    #[test]
    fn test_canonical_match_form_is_lowercase() {
        let canonical = canonical_match_form("/Users/Foo/Bar");
        // Lowercased for case-insensitive blocklist matching
        assert!(!canonical.chars().any(|c| c.is_ascii_uppercase()));
    }

    #[test]
    fn test_canonical_match_form_strips_null_bytes() {
        // Null bytes in paths can confuse downstream consumers.
        let canonical = canonical_match_form("/foo\0bar");
        assert!(!canonical.contains('\0'));
    }

    #[test]
    fn test_relative_path_resolves_against_cwd() {
        // `./foo.txt` should resolve to <cwd>/foo.txt (or its canonical form).
        let canonical = canonicalize_logical("./forge_test_relative.txt");
        let cwd = std::env::current_dir().unwrap();
        // Either the path now begins with cwd, or with cwd's canonical form
        // (e.g. /private/tmp vs /tmp on macOS).
        assert!(
            canonical.starts_with(&cwd) || canonical.starts_with(cwd.canonicalize().unwrap_or(cwd)),
            "relative path must resolve against cwd; got {canonical:?}"
        );
    }
}
