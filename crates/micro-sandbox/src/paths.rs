//! Turning a path the model asked for into the path the kernel would open.

use std::ffi::OsString;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

/// Resolve `path` against `base` and follow every symlink the filesystem can follow.
pub(crate) fn resolve(base: &Path, path: &Path) -> PathBuf {
    let mut resolved = if path.is_absolute() {
        PathBuf::new()
    } else {
        base.to_path_buf()
    };

    for component in path.components() {
        match component {
            Component::Prefix(prefix) => resolved.push(prefix.as_os_str()),
            Component::RootDir => resolved.push(Component::RootDir.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if let Ok(canonical) = resolved.canonicalize() {
                    resolved = canonical;
                }
                resolved.pop();
            }
            Component::Normal(name) => resolved.push(name),
        }
    }

    canonicalize_deepest_existing(&resolved)
}

/// Canonicalize as much of `path` as exists, keeping the rest as written.
pub(crate) fn canonicalize_deepest_existing(path: &Path) -> PathBuf {
    let mut existing = path.to_path_buf();
    let mut missing: Vec<OsString> = Vec::new();

    loop {
        if let Ok(canonical) = existing.canonicalize() {
            let mut resolved = canonical;
            resolved.extend(missing.iter().rev());
            return resolved;
        }
        match existing.file_name() {
            Some(name) => missing.push(name.to_os_string()),
            None => return path.to_path_buf(),
        }
        if !existing.pop() {
            return path.to_path_buf();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("micro-sandbox-paths-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    #[test]
    fn a_relative_path_resolves_against_the_base() {
        let dir = scratch("relative");
        std::fs::write(dir.join("file.txt"), "hi").unwrap();
        assert_eq!(resolve(&dir, Path::new("file.txt")), dir.join("file.txt"));
    }

    #[test]
    fn a_path_that_does_not_exist_yet_keeps_the_names_below_what_does() {
        let dir = scratch("missing");
        let resolved = resolve(&dir, Path::new("a/b/c.txt"));
        assert_eq!(resolved, dir.join("a/b/c.txt"));
    }

    #[test]
    fn a_symlink_resolves_to_what_it_points_at() {
        let dir = scratch("symlink");
        let target = dir.join("target");
        std::fs::create_dir(&target).unwrap();
        std::os::unix::fs::symlink(&target, dir.join("link")).unwrap();
        assert_eq!(
            resolve(&dir, Path::new("link/inner.txt")),
            target.join("inner.txt")
        );
    }

    #[test]
    fn stepping_out_of_a_symlink_lands_where_the_link_points() {
        let dir = scratch("symlink-parent");
        let outside = dir.join("outside");
        let inside = dir.join("inside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::create_dir(&inside).unwrap();
        std::os::unix::fs::symlink(&outside, inside.join("link")).unwrap();
        assert_eq!(
            resolve(&inside, Path::new("link/../sibling.txt")),
            dir.join("sibling.txt")
        );
    }
}
