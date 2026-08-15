//! Finding extensions on disk.

use serde::Deserialize;
use std::path::Path;
use std::path::PathBuf;

/// The directory a project keeps its own extensions in.
pub const PROJECT_DIR: &str = ".micro/extensions";

/// What a `package.json` says about the extensions it carries.
#[derive(Debug, Clone, Default, Deserialize)]
struct Manifest {
    #[serde(default)]
    micro: Option<ManifestSection>,
    #[serde(default)]
    pi: Option<ManifestSection>,
}

impl Manifest {
    /// The entries it declares, under whichever name it declared them.
    fn extensions(self) -> Vec<String> {
        for section in [self.micro, self.pi].into_iter().flatten() {
            if !section.extensions.is_empty() {
                return section.extensions;
            }
        }
        Vec::new()
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct ManifestSection {
    #[serde(default)]
    extensions: Vec<String>,
}

/// Every extension to load, in the order they should be loaded.
pub fn discover(
    workspace: &Path,
    home: &Path,
    configured: &[String],
    trusted: bool,
) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut seen = std::collections::BTreeSet::new();

    let mut add = |paths: Vec<PathBuf>, found: &mut Vec<PathBuf>| {
        for path in paths {
            let resolved = path.canonicalize().unwrap_or(path);
            if seen.insert(resolved.clone()) {
                found.push(resolved);
            }
        }
    };

    if trusted {
        add(in_directory(&workspace.join(PROJECT_DIR)), &mut found);
    }
    add(in_directory(&home.join("extensions")), &mut found);

    for path in configured {
        let path = expand(path, workspace);
        if path.is_dir() {
            match entries_of(&path) {
                Some(entries) => add(entries, &mut found),
                None => add(in_directory(&path), &mut found),
            }
            continue;
        }
        add(vec![path], &mut found);
    }

    found
}

/// The extensions in one directory, by the rules this module describes.
pub fn in_directory(directory: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };

    
    let mut names: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .collect();
    names.sort();

    let mut found = Vec::new();
    for path in names {
        if path.is_file() && is_extension_file(&path) {
            found.push(path);
            continue;
        }
        if path.is_dir() {
            if let Some(entries) = entries_of(&path) {
                found.extend(entries);
            }
        }
    }
    found
}

/// Whether a file is one an extension can be written in.
fn is_extension_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("ts") | Some("js")
    )
}

/// What a directory offers as its entry points, or nothing when it offers none.
pub fn package_name(directory: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(directory.join("package.json")).ok()?;
    let manifest: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let name = manifest.get("name")?.as_str()?;
    match name.is_empty() {
        true => None,
        false => Some(name.to_string()),
    }
}

pub fn entries_of(directory: &Path) -> Option<Vec<PathBuf>> {
    
    let manifest_path = directory.join("package.json");
    if let Ok(raw) = std::fs::read_to_string(&manifest_path) {
        if let Ok(manifest) = serde_json::from_str::<Manifest>(&raw) {
            let declared: Vec<PathBuf> = manifest
                .extensions()
                .iter()
                .map(|entry| directory.join(entry))
                .filter(|path| path.exists())
                .collect();
            if !declared.is_empty() {
                return Some(declared);
            }
        }
    }

    for name in ["index.ts", "index.js"] {
        let candidate = directory.join(name);
        if candidate.exists() {
            return Some(vec![candidate]);
        }
    }
    None
}

/// A configured path, with `~` and a relative path resolved the way a shell would.
fn expand(path: &str, workspace: &Path) -> PathBuf {
    let trimmed = path.trim();
    if let Some(rest) = trimmed.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    let path = PathBuf::from(trimmed);
    match path.is_absolute() {
        true => path,
        false => workspace.join(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(label: &str) -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "micro-extensions-{label}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn write(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn a_loose_file_is_an_extension() {
        let root = scratch("loose");
        write(&root.join("hello.ts"), "export default () => {}");
        write(&root.join("also.js"), "export default () => {}");
        write(&root.join("notes.md"), "not an extension");

        let found = in_directory(&root);
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found.iter().all(|path| is_extension_file(path)));
    }

    #[test]
    fn a_directory_with_an_index_is_one_extension() {
        let root = scratch("index");
        write(&root.join("thing/index.ts"), "export default () => {}");
        write(&root.join("thing/helper.ts"), "export const x = 1");

        let found = in_directory(&root);
        assert_eq!(found.len(), 1, "the helper is not loaded: {found:?}");
        assert!(found[0].ends_with("index.ts"));
    }

    /// A package declaring its entries under `pi` still loads.
    #[test]
    fn a_package_written_for_pi_still_loads() {
        let root = scratch("pi-manifest");
        write(
            &root.join("adapter/package.json"),
            r#"{ "name": "pi-mcp-adapter", "pi": { "extensions": ["index.ts"] } }"#,
        );
        write(&root.join("adapter/index.ts"), "export default () => {}");
        assert_eq!(in_directory(&root).len(), 1);
    }

    /// A manifest is the only way to load something other than an index.
    #[test]
    fn a_manifest_says_what_a_package_loads() {
        let root = scratch("manifest");
        write(
            &root.join("thing/package.json"),
            r#"{ "name": "thing", "micro": { "extensions": ["main.ts", "second.ts", "missing.ts"] } }"#,
        );
        write(&root.join("thing/main.ts"), "export default () => {}");
        write(&root.join("thing/second.ts"), "export default () => {}");
        write(&root.join("thing/index.ts"), "export default () => {}");

        let found = in_directory(&root);
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found[0].ends_with("main.ts"));
        assert!(found[1].ends_with("second.ts"));
    }

    #[test]
    fn a_manifest_naming_nothing_falls_back_to_the_index() {
        let root = scratch("empty-manifest");
        write(&root.join("thing/package.json"), r#"{ "name": "thing" }"#);
        write(&root.join("thing/index.ts"), "export default () => {}");

        let found = in_directory(&root);
        assert_eq!(found.len(), 1);
        assert!(found[0].ends_with("index.ts"));
    }

    #[test]
    fn nothing_deeper_than_one_level_is_taken() {
        let root = scratch("deep");
        write(&root.join("outer/inner/deep.ts"), "export default () => {}");
        assert!(in_directory(&root).is_empty());
    }

    /// The project's own extensions come before the user's, and a path found twice is loaded once.
    #[test]
    fn the_project_comes_before_the_user_and_nothing_loads_twice() {
        let root = scratch("order");
        let workspace = root.join("workspace");
        let home = root.join("home");
        write(
            &workspace.join(PROJECT_DIR).join("local.ts"),
            "export default () => {}",
        );
        write(
            &home.join("extensions/global.ts"),
            "export default () => {}",
        );

        let found = discover(&workspace, &home, &[], true);
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found[0].ends_with("local.ts"));
        assert!(found[1].ends_with("global.ts"));

        
        let configured = vec![found[0].display().to_string()];
        assert_eq!(discover(&workspace, &home, &configured, true).len(), 2);
    }

    #[test]
    fn a_configured_file_is_taken_as_it_is_written() {
        let root = scratch("configured");
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        write(&root.join("elsewhere/thing.ts"), "export default () => {}");

        let found = discover(
            &workspace,
            &root.join("home"),
            &[root.join("elsewhere/thing.ts").display().to_string()],
            true,
        );
        assert_eq!(found.len(), 1);
        assert!(found[0].ends_with("thing.ts"));
    }

    #[test]
    fn a_configured_directory_is_searched_the_same_way() {
        let root = scratch("configured-dir");
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        write(&root.join("bundle/one.ts"), "export default () => {}");
        write(&root.join("bundle/two.ts"), "export default () => {}");

        let found = discover(
            &workspace,
            &root.join("home"),
            &[root.join("bundle").display().to_string()],
            true,
        );
        assert_eq!(found.len(), 2, "{found:?}");
    }

    #[test]
    fn a_directory_that_is_not_there_finds_nothing() {
        assert!(in_directory(Path::new("/nowhere-at-all")).is_empty());
        assert!(discover(
            Path::new("/nowhere-at-all"),
            Path::new("/nor-here"),
            &[],
            true
        )
        .is_empty());
    }
}
