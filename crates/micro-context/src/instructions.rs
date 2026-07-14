//! Project instruction discovery.
//!
//! Instruction files are collected from the workspace, from every directory above it, and
//! from micro's own home directory, then concatenated into the text that goes into a
//! system prompt.

use crate::ContextError;
use crate::Result;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;

/// Environment variable naming micro's home directory.
pub const MICRO_DIR_ENV: &str = "MICRO_DIR";

/// The instruction file names recognised in a directory, in ascending order of precedence:
/// `AGENTS.md` is micro's own file, so it has the last word where both are present.
pub const INSTRUCTION_FILE_NAMES: &[&str] = &["CLAUDE.md", "AGENTS.md"];

/// How many levels of `@import` are followed before a directive is left as written.
pub const DEFAULT_MAX_IMPORT_DEPTH: usize = 5;

/// Marks where each file's contribution begins. An HTML comment keeps the boundary
/// visible to a reader without adding a heading that would compete with the file's own.
const SOURCE_MARKER: &str = "<!-- source:";

/// The assembled project instructions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Instructions {
    /// Every source concatenated, lowest precedence first, so the most specific
    /// instructions are the last thing the model reads.
    pub text: String,
    /// Every file that contributed, in the order it appears in `text`. Imported files are
    /// listed at the position their contents were inlined.
    pub sources: Vec<PathBuf>,
}

impl Instructions {
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

/// Finds and assembles instruction files.
#[derive(Debug, Clone)]
pub struct InstructionLoader {
    global_dir: PathBuf,
    max_import_depth: usize,
}

impl InstructionLoader {
    /// A loader whose global instructions live in `global_dir`.
    pub fn new(global_dir: impl Into<PathBuf>) -> Self {
        InstructionLoader {
            global_dir: global_dir.into(),
            max_import_depth: DEFAULT_MAX_IMPORT_DEPTH,
        }
    }

    /// A loader rooted at `$MICRO_DIR`, falling back to `~/.micro`.
    pub fn from_env() -> Result<Self> {
        Ok(InstructionLoader::new(micro_home()?))
    }

    pub fn with_max_import_depth(mut self, depth: usize) -> Self {
        self.max_import_depth = depth;
        self
    }

    /// Collects the instructions that apply to `workspace`.
    ///
    /// Global files come first, then each ancestor directory from the filesystem root down
    /// to the workspace itself, so a nearer file is read after — and therefore overrides —
    /// a farther one.
    pub async fn load(&self, workspace: impl AsRef<Path>) -> Result<Instructions> {
        let workspace = absolute(workspace.as_ref());

        let mut candidates = self.global_candidates();
        candidates.extend(project_candidates(&workspace));

        let mut assembled = Assembly::new(self.max_import_depth);
        for candidate in candidates {
            assembled.add_file(&candidate).await?;
        }
        Ok(assembled.finish())
    }

    fn global_candidates(&self) -> Vec<PathBuf> {
        INSTRUCTION_FILE_NAMES
            .iter()
            .map(|name| self.global_dir.join(name))
            .collect()
    }
}

/// Every instruction file from the filesystem root down to `workspace`.
fn project_candidates(workspace: &Path) -> Vec<PathBuf> {
    let mut per_directory: Vec<Vec<PathBuf>> = Vec::new();
    let mut cursor = Some(workspace);

    while let Some(directory) = cursor {
        per_directory.push(
            INSTRUCTION_FILE_NAMES
                .iter()
                .map(|name| directory.join(name))
                .collect(),
        );
        cursor = directory.parent();
    }

    // Reversed so the outermost ancestor is read first and the workspace's own file last.
    per_directory.into_iter().rev().flatten().collect()
}

/// Accumulates instruction text, inlining imports and refusing to visit a file twice.
struct Assembly {
    text: String,
    sources: Vec<PathBuf>,
    visited: HashSet<PathBuf>,
    max_import_depth: usize,
}

impl Assembly {
    fn new(max_import_depth: usize) -> Self {
        Assembly {
            text: String::new(),
            sources: Vec::new(),
            visited: HashSet::new(),
            max_import_depth,
        }
    }

    /// Reads one file if it exists and has not been seen, expanding its imports.
    async fn add_file(&mut self, path: &Path) -> Result<()> {
        let Some((canonical, contents)) = self.take(path).await? else {
            return Ok(());
        };

        // Expansion appends whatever this file imports, so the slot the importer belongs in
        // is the one recorded before expanding.
        let position = self.sources.len();
        let expanded = self.expand(&canonical, &contents, 0).await?;
        let trimmed = expanded.trim();
        if trimmed.is_empty() {
            return Ok(());
        }

        self.sources.insert(position, canonical.clone());
        if !self.text.is_empty() {
            self.text.push_str("\n\n");
        }
        self.text
            .push_str(&format!("{SOURCE_MARKER} {} -->\n", canonical.display()));
        self.text.push_str(trimmed);
        Ok(())
    }

    /// Claims a file: returns its canonical path and contents, or nothing when it is
    /// missing, unreadable, or already part of the assembly.
    async fn take(&mut self, path: &Path) -> Result<Option<(PathBuf, String)>> {
        let canonical = match tokio::fs::canonicalize(path).await {
            Ok(canonical) => canonical,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(ContextError::io(path, source)),
        };
        if !self.visited.insert(canonical.clone()) {
            return Ok(None);
        }

        match tokio::fs::read_to_string(&canonical).await {
            Ok(contents) => Ok(Some((canonical, contents))),
            // A directory named AGENTS.md, or a file that is not text, is not instructions.
            Err(_) => Ok(None),
        }
    }

    /// Replaces every `@path` directive with the contents of the file it names.
    ///
    /// Paths resolve against the importing file's directory. A directive is left exactly as
    /// written when the depth limit is reached, when the file is missing, or when it is
    /// already in the assembly, which is what stops a cycle.
    async fn expand(&mut self, importer: &Path, contents: &str, depth: usize) -> Result<String> {
        let directory = importer.parent().unwrap_or(importer).to_path_buf();
        let mut expanded = String::with_capacity(contents.len());

        for line in contents.lines() {
            let Some(target) = import_directive(line) else {
                expanded.push_str(line);
                expanded.push('\n');
                continue;
            };

            if depth >= self.max_import_depth {
                expanded.push_str(line);
                expanded.push('\n');
                continue;
            }

            let resolved = resolve_import(&directory, target);
            let Some((canonical, imported)) = self.take(&resolved).await? else {
                expanded.push_str(line);
                expanded.push('\n');
                continue;
            };

            let position = self.sources.len();
            let nested = Box::pin(self.expand(&canonical, &imported, depth + 1)).await?;
            self.sources.insert(position, canonical.clone());

            expanded.push_str(&format!("{SOURCE_MARKER} {} -->\n", canonical.display()));
            expanded.push_str(nested.trim_end());
            expanded.push('\n');
        }

        Ok(expanded)
    }

    fn finish(self) -> Instructions {
        Instructions {
            text: self.text,
            sources: self.sources,
        }
    }
}

/// The path an `@import` line names, or nothing when the line is ordinary text.
fn import_directive(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let target = trimmed.strip_prefix('@')?.trim();
    // Only a line that is nothing but a directive counts, so an address or a handle in
    // prose is never mistaken for an import.
    if target.is_empty() || target.contains(char::is_whitespace) {
        return None;
    }
    Some(target)
}

fn resolve_import(directory: &Path, target: &str) -> PathBuf {
    if let Some(rest) = target.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return home.join(rest);
        }
    }
    let path = Path::new(target);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        directory.join(path)
    }
}

/// `$MICRO_DIR`, or `~/.micro` when it is unset.
pub fn micro_home() -> Result<PathBuf> {
    home_from(std::env::var(MICRO_DIR_ENV).ok().as_deref(), home_dir())
}

fn home_from(micro_dir: Option<&str>, home: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(dir) = micro_dir.map(str::trim).filter(|dir| !dir.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    home.map(|home| home.join(".micro"))
        .ok_or(ContextError::NoHome { env: MICRO_DIR_ENV })
}

fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .or_else(|| std::env::var("USERPROFILE").ok())
        .map(|home| home.trim().to_string())
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

/// Normalizes a path lexically. The workspace may not exist yet, so this never touches
/// the filesystem.
fn absolute(path: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };

    let mut normalized = PathBuf::new();
    for component in joined.components() {
        match component {
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("micro-instructions-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::canonicalize(&dir).unwrap()
    }

    fn write(root: &Path, relative: &str, contents: &str) -> PathBuf {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
        path
    }

    /// A loader whose global directory is inside the scratch tree, so no test reads the
    /// real `~/.micro`.
    fn loader(root: &Path) -> InstructionLoader {
        InstructionLoader::new(root.join("global"))
    }

    #[tokio::test]
    async fn an_absent_workspace_file_yields_nothing() {
        let root = scratch("empty");
        let loaded = loader(&root).load(root.join("project")).await.unwrap();
        assert!(loaded.is_empty());
        assert!(loaded.sources.is_empty());
    }

    #[tokio::test]
    async fn the_workspace_file_is_read() {
        let root = scratch("single");
        write(&root, "project/AGENTS.md", "use tabs");

        let loaded = loader(&root).load(root.join("project")).await.unwrap();
        assert!(loaded.text.contains("use tabs"));
        assert_eq!(loaded.sources, vec![root.join("project/AGENTS.md")]);
    }

    #[tokio::test]
    async fn a_nearer_file_is_read_after_a_farther_one() {
        let root = scratch("precedence-depth");
        write(&root, "AGENTS.md", "outer rule");
        write(&root, "project/AGENTS.md", "inner rule");
        write(&root, "project/nested/AGENTS.md", "nested rule");

        let loaded = loader(&root)
            .load(root.join("project/nested"))
            .await
            .unwrap();

        let outer = loaded.text.find("outer rule").unwrap();
        let inner = loaded.text.find("inner rule").unwrap();
        let nested = loaded.text.find("nested rule").unwrap();
        assert!(outer < inner, "the farther file must come first");
        assert!(inner < nested, "the nearer file must come last");
        assert_eq!(
            loaded.sources,
            vec![
                root.join("AGENTS.md"),
                root.join("project/AGENTS.md"),
                root.join("project/nested/AGENTS.md"),
            ]
        );
    }

    #[tokio::test]
    async fn global_instructions_come_before_project_instructions() {
        let root = scratch("precedence-global");
        write(&root, "global/AGENTS.md", "global rule");
        write(&root, "project/AGENTS.md", "project rule");

        let loaded = loader(&root).load(root.join("project")).await.unwrap();
        let global = loaded.text.find("global rule").unwrap();
        let project = loaded.text.find("project rule").unwrap();
        assert!(
            global < project,
            "project instructions must have the last word"
        );
    }

    #[tokio::test]
    async fn agents_md_outranks_claude_md_in_the_same_directory() {
        let root = scratch("precedence-names");
        write(&root, "project/CLAUDE.md", "claude rule");
        write(&root, "project/AGENTS.md", "agents rule");

        let loaded = loader(&root).load(root.join("project")).await.unwrap();
        let claude = loaded.text.find("claude rule").unwrap();
        let agents = loaded.text.find("agents rule").unwrap();
        assert!(claude < agents);
        assert_eq!(loaded.sources.len(), 2);
    }

    #[tokio::test]
    async fn an_import_is_inlined_relative_to_the_importing_file() {
        let root = scratch("import");
        write(&root, "project/AGENTS.md", "before\n@docs/style.md\nafter");
        write(&root, "project/docs/style.md", "imported style");

        let loaded = loader(&root).load(root.join("project")).await.unwrap();
        assert!(loaded.text.contains("before"));
        assert!(loaded.text.contains("imported style"));
        assert!(loaded.text.contains("after"));
        assert_eq!(
            loaded.sources,
            vec![
                root.join("project/AGENTS.md"),
                root.join("project/docs/style.md"),
            ]
        );
    }

    #[tokio::test]
    async fn a_nested_import_is_followed() {
        let root = scratch("import-nested");
        write(&root, "project/AGENTS.md", "@one.md");
        write(&root, "project/one.md", "first\n@two.md");
        write(&root, "project/two.md", "second");

        let loaded = loader(&root).load(root.join("project")).await.unwrap();
        assert!(loaded.text.contains("first"));
        assert!(loaded.text.contains("second"));
        assert_eq!(loaded.sources.len(), 3);
    }

    #[tokio::test]
    async fn a_file_that_imports_itself_terminates() {
        let root = scratch("import-self");
        write(&root, "project/AGENTS.md", "rule\n@AGENTS.md");

        let loaded = loader(&root).load(root.join("project")).await.unwrap();
        assert!(loaded.text.contains("rule"));
        // The unresolvable directive stays visible rather than silently vanishing.
        assert!(loaded.text.contains("@AGENTS.md"));
        assert_eq!(loaded.sources, vec![root.join("project/AGENTS.md")]);
    }

    #[tokio::test]
    async fn a_cycle_between_two_files_terminates() {
        let root = scratch("import-cycle");
        write(&root, "project/AGENTS.md", "@a.md");
        write(&root, "project/a.md", "alpha\n@b.md");
        write(&root, "project/b.md", "beta\n@a.md");

        let loaded = loader(&root).load(root.join("project")).await.unwrap();
        assert!(loaded.text.contains("alpha"));
        assert!(loaded.text.contains("beta"));
        assert_eq!(loaded.text.matches("alpha").count(), 1);
        assert_eq!(loaded.sources.len(), 3);
    }

    #[tokio::test]
    async fn imports_stop_at_the_depth_limit() {
        let root = scratch("import-depth");
        write(&root, "project/AGENTS.md", "@one.md");
        write(&root, "project/one.md", "first\n@two.md");
        write(&root, "project/two.md", "second\n@three.md");
        write(&root, "project/three.md", "third");

        let loaded = InstructionLoader::new(root.join("global"))
            .with_max_import_depth(1)
            .load(root.join("project"))
            .await
            .unwrap();

        assert!(loaded.text.contains("first"));
        assert!(!loaded.text.contains("second"));
        assert!(loaded.text.contains("@two.md"));
    }

    #[tokio::test]
    async fn a_missing_import_leaves_the_directive_in_place() {
        let root = scratch("import-missing");
        write(&root, "project/AGENTS.md", "rule\n@nowhere.md");

        let loaded = loader(&root).load(root.join("project")).await.unwrap();
        assert!(loaded.text.contains("rule"));
        assert!(loaded.text.contains("@nowhere.md"));
    }

    #[tokio::test]
    async fn prose_containing_an_at_sign_is_not_an_import() {
        let root = scratch("import-prose");
        write(
            &root,
            "project/AGENTS.md",
            "email @ramon about docs/style.md\n@ someone",
        );

        let loaded = loader(&root).load(root.join("project")).await.unwrap();
        assert!(loaded.text.contains("email @ramon about docs/style.md"));
        assert_eq!(loaded.sources.len(), 1);
    }

    #[tokio::test]
    async fn an_empty_file_contributes_nothing() {
        let root = scratch("import-blank");
        write(&root, "project/AGENTS.md", "   \n\n");
        write(&root, "project/CLAUDE.md", "real rule");

        let loaded = loader(&root).load(root.join("project")).await.unwrap();
        assert!(loaded.text.contains("real rule"));
        assert_eq!(loaded.sources, vec![root.join("project/CLAUDE.md")]);
    }

    #[test]
    fn the_home_directory_follows_the_environment() {
        assert_eq!(
            home_from(Some("/opt/micro"), Some(PathBuf::from("/home/ramon"))).unwrap(),
            PathBuf::from("/opt/micro")
        );
        assert_eq!(
            home_from(None, Some(PathBuf::from("/home/ramon"))).unwrap(),
            PathBuf::from("/home/ramon/.micro")
        );
        assert_eq!(
            home_from(Some("  "), Some(PathBuf::from("/home/ramon"))).unwrap(),
            PathBuf::from("/home/ramon/.micro")
        );
        assert!(matches!(
            home_from(None, None).unwrap_err(),
            ContextError::NoHome { .. }
        ));
    }
}
