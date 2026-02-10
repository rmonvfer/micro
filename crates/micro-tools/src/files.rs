//! Filesystem tools.

use crate::required_str;
use crate::resolve_path;
use crate::truncate;
use crate::Tool;
use async_trait::async_trait;
use micro_types::ToolDefinition;
use serde_json::json;
use serde_json::Value;
use std::path::PathBuf;

pub struct Read {
    root: PathBuf,
}

impl Read {
    pub fn new(root: PathBuf) -> Self {
        Read { root }
    }
}

#[async_trait]
impl Tool for Read {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "read".into(),
            description: "Read a file from the workspace. Returns the contents with 1-indexed \
                           line numbers. Use offset and limit for large files."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file" },
                    "offset": { "type": "integer", "description": "First line to read, 1-indexed" },
                    "limit": { "type": "integer", "description": "How many lines to read" },
                },
                "required": ["path"],
            }),
        }
    }

    async fn execute(&self, arguments: &Value) -> Result<String, String> {
        let path = resolve_path(&self.root, &required_str(arguments, "path")?)?;
        let contents = tokio::fs::read_to_string(&path)
            .await
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;

        let offset = arguments
            .get("offset")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            .max(1) as usize;
        let limit = arguments
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(u64::MAX) as usize;

        let numbered: String = contents
            .lines()
            .enumerate()
            .skip(offset - 1)
            .take(limit)
            .map(|(index, line)| format!("{:>6}\t{line}\n", index + 1))
            .collect();

        if numbered.is_empty() {
            return Ok(format!("{} is empty", path.display()));
        }
        Ok(truncate(&numbered))
    }
}

pub struct Write {
    root: PathBuf,
}

impl Write {
    pub fn new(root: PathBuf) -> Self {
        Write { root }
    }
}

#[async_trait]
impl Tool for Write {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "write".into(),
            description: "Write a file, creating parent directories and overwriting any \
                          existing contents."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file" },
                    "content": { "type": "string", "description": "Full contents to write" },
                },
                "required": ["path", "content"],
            }),
        }
    }

    async fn execute(&self, arguments: &Value) -> Result<String, String> {
        let path = resolve_path(&self.root, &required_str(arguments, "path")?)?;
        let content = required_str(arguments, "content")?;

        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
        }
        tokio::fs::write(&path, &content)
            .await
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;

        Ok(format!(
            "Wrote {} ({} bytes)",
            path.display(),
            content.len()
        ))
    }
}

pub struct Edit {
    root: PathBuf,
}

impl Edit {
    pub fn new(root: PathBuf) -> Self {
        Edit { root }
    }
}

#[async_trait]
impl Tool for Edit {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "edit".into(),
            description: "Replace an exact string in a file. The old string must appear \
                          exactly once; include surrounding context to make it unique."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file" },
                    "old_string": { "type": "string", "description": "Exact text to replace" },
                    "new_string": { "type": "string", "description": "Replacement text" },
                },
                "required": ["path", "old_string", "new_string"],
            }),
        }
    }

    async fn execute(&self, arguments: &Value) -> Result<String, String> {
        let path = resolve_path(&self.root, &required_str(arguments, "path")?)?;
        let old_string = required_str(arguments, "old_string")?;
        let new_string = required_str(arguments, "new_string")?;

        if old_string == new_string {
            return Err("old_string and new_string are identical".to_string());
        }

        let contents = tokio::fs::read_to_string(&path)
            .await
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;

        // An ambiguous match is a failure, never a guess: replacing the wrong occurrence
        // silently corrupts the file.
        let occurrences = contents.matches(&old_string).count();
        match occurrences {
            0 => return Err(format!("old_string not found in {}", path.display())),
            1 => {}
            count => {
                return Err(format!(
                    "old_string appears {count} times in {}; add surrounding context to \
                     identify a single occurrence",
                    path.display()
                ))
            }
        }

        let updated = contents.replacen(&old_string, &new_string, 1);
        tokio::fs::write(&path, &updated)
            .await
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;

        Ok(format!("Edited {}", path.display()))
    }
}

pub struct MultiEdit {
    root: PathBuf,
}

impl MultiEdit {
    pub fn new(root: PathBuf) -> Self {
        MultiEdit { root }
    }
}

#[async_trait]
impl Tool for MultiEdit {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "multi_edit".into(),
            description: "Apply several exact-string replacements to one file in order. Each \
                          old_string must appear exactly once at the point its edit runs. If \
                          any edit fails, none are applied and the file is left untouched."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file" },
                    "edits": {
                        "type": "array",
                        "description": "Edits to apply in order. A later edit can target text an \
                                        earlier one produced.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "old_string": { "type": "string", "description": "Exact text to replace" },
                                "new_string": { "type": "string", "description": "Replacement text" },
                            },
                            "required": ["old_string", "new_string"],
                        },
                    },
                },
                "required": ["path", "edits"],
            }),
        }
    }

    async fn execute(&self, arguments: &Value) -> Result<String, String> {
        let path = resolve_path(&self.root, &required_str(arguments, "path")?)?;
        let edits = arguments
            .get("edits")
            .and_then(Value::as_array)
            .ok_or_else(|| "missing required argument: edits".to_string())?;
        if edits.is_empty() {
            return Err("edits must not be empty".to_string());
        }

        let contents = tokio::fs::read_to_string(&path)
            .await
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;

        // Every edit is applied to this buffer, and the file is written only once the last
        // one succeeds. A rejected edit therefore leaves the file exactly as it was.
        let mut updated = contents;
        for (index, edit) in edits.iter().enumerate() {
            let position = index + 1;
            let old_string = required_str(edit, "old_string")
                .map_err(|error| format!("edit {position}: {error}"))?;
            let new_string = required_str(edit, "new_string")
                .map_err(|error| format!("edit {position}: {error}"))?;

            if old_string.is_empty() {
                return Err(format!("edit {position}: old_string must not be empty"));
            }
            if old_string == new_string {
                return Err(format!(
                    "edit {position}: old_string and new_string are identical"
                ));
            }

            // An ambiguous match is a failure, never a guess: replacing the wrong
            // occurrence silently corrupts the file.
            match updated.matches(&old_string).count() {
                0 => {
                    return Err(format!(
                        "edit {position}: old_string not found in {}",
                        path.display()
                    ))
                }
                1 => {}
                count => {
                    return Err(format!(
                        "edit {position}: old_string appears {count} times in {}; add \
                         surrounding context to identify a single occurrence",
                        path.display()
                    ))
                }
            }

            updated = updated.replacen(&old_string, &new_string, 1);
        }

        tokio::fs::write(&path, &updated)
            .await
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;

        Ok(format!("Edited {} ({} edits)", path.display(), edits.len()))
    }
}

pub struct Ls {
    root: PathBuf,
}

impl Ls {
    pub fn new(root: PathBuf) -> Self {
        Ls { root }
    }
}

#[async_trait]
impl Tool for Ls {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "ls".into(),
            description: "List the entries of a directory. Directories are suffixed with a slash."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory to list, defaults to the workspace root" },
                },
            }),
        }
    }

    async fn execute(&self, arguments: &Value) -> Result<String, String> {
        let requested = arguments.get("path").and_then(Value::as_str).unwrap_or(".");
        let path = resolve_path(&self.root, requested)?;

        let mut entries = tokio::fs::read_dir(&path)
            .await
            .map_err(|error| format!("cannot list {}: {error}", path.display()))?;

        let mut names = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|error| format!("cannot list {}: {error}", path.display()))?
        {
            let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
            let name = entry.file_name().to_string_lossy().to_string();
            names.push(if is_dir { format!("{name}/") } else { name });
        }
        names.sort();

        if names.is_empty() {
            return Ok(format!("{} is empty", path.display()));
        }
        Ok(truncate(&names.join("\n")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("micro-tools-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn read_numbers_lines_from_an_offset() {
        let root = scratch("read");
        std::fs::write(root.join("a.txt"), "one\ntwo\nthree\n").unwrap();

        let tool = Read::new(root);
        let output = tool
            .execute(&json!({ "path": "a.txt", "offset": 2 }))
            .await
            .unwrap();

        assert!(output.contains("     2\ttwo"));
        assert!(output.contains("     3\tthree"));
        assert!(!output.contains("one"));
    }

    #[tokio::test]
    async fn write_creates_missing_parent_directories() {
        let root = scratch("write");
        let tool = Write::new(root.clone());

        tool.execute(&json!({ "path": "nested/deep/a.txt", "content": "hi" }))
            .await
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(root.join("nested/deep/a.txt")).unwrap(),
            "hi"
        );
    }

    #[tokio::test]
    async fn edit_replaces_a_unique_match() {
        let root = scratch("edit-unique");
        std::fs::write(root.join("a.txt"), "keep\nreplace me\nkeep\n").unwrap();

        Edit::new(root.clone())
            .execute(&json!({
                "path": "a.txt",
                "old_string": "replace me",
                "new_string": "replaced",
            }))
            .await
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "keep\nreplaced\nkeep\n"
        );
    }

    #[tokio::test]
    async fn edit_refuses_an_ambiguous_match_and_leaves_the_file_alone() {
        let root = scratch("edit-ambiguous");
        std::fs::write(root.join("a.txt"), "dup\ndup\n").unwrap();

        let error = Edit::new(root.clone())
            .execute(&json!({ "path": "a.txt", "old_string": "dup", "new_string": "x" }))
            .await
            .unwrap_err();

        assert!(error.contains("appears 2 times"));
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "dup\ndup\n"
        );
    }

    #[tokio::test]
    async fn multi_edit_applies_every_edit_in_order() {
        let root = scratch("multi-edit-order");
        std::fs::write(root.join("a.rs"), "let alpha = 1;\nlet beta = 2;\n").unwrap();

        let output = MultiEdit::new(root.clone())
            .execute(&json!({
                "path": "a.rs",
                "edits": [
                    { "old_string": "alpha", "new_string": "first" },
                    { "old_string": "beta", "new_string": "second" },
                ],
            }))
            .await
            .unwrap();

        assert!(output.contains("2 edits"));
        assert_eq!(
            std::fs::read_to_string(root.join("a.rs")).unwrap(),
            "let first = 1;\nlet second = 2;\n"
        );
    }

    #[tokio::test]
    async fn multi_edit_lets_a_later_edit_target_earlier_output() {
        let root = scratch("multi-edit-chain");
        std::fs::write(root.join("a.rs"), "one\n").unwrap();

        MultiEdit::new(root.clone())
            .execute(&json!({
                "path": "a.rs",
                "edits": [
                    { "old_string": "one", "new_string": "two" },
                    { "old_string": "two", "new_string": "three" },
                ],
            }))
            .await
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(root.join("a.rs")).unwrap(),
            "three\n"
        );
    }

    #[tokio::test]
    async fn multi_edit_applies_nothing_when_a_later_edit_fails() {
        let root = scratch("multi-edit-atomic");
        let original = "let alpha = 1;\nlet beta = 2;\n";
        std::fs::write(root.join("a.rs"), original).unwrap();

        let error = MultiEdit::new(root.clone())
            .execute(&json!({
                "path": "a.rs",
                "edits": [
                    { "old_string": "alpha", "new_string": "first" },
                    { "old_string": "gamma", "new_string": "third" },
                ],
            }))
            .await
            .unwrap_err();

        assert!(error.contains("edit 2"));
        assert!(error.contains("not found"));
        assert_eq!(
            std::fs::read_to_string(root.join("a.rs")).unwrap(),
            original
        );
    }

    #[tokio::test]
    async fn multi_edit_refuses_an_ambiguous_match_and_leaves_the_file_alone() {
        let root = scratch("multi-edit-ambiguous");
        let original = "dup\ndup\nunique\n";
        std::fs::write(root.join("a.rs"), original).unwrap();

        let error = MultiEdit::new(root.clone())
            .execute(&json!({
                "path": "a.rs",
                "edits": [
                    { "old_string": "unique", "new_string": "one" },
                    { "old_string": "dup", "new_string": "x" },
                ],
            }))
            .await
            .unwrap_err();

        assert!(error.contains("edit 2"));
        assert!(error.contains("appears 2 times"));
        assert_eq!(
            std::fs::read_to_string(root.join("a.rs")).unwrap(),
            original
        );
    }

    #[tokio::test]
    async fn multi_edit_rejects_a_degenerate_edit_list() {
        let root = scratch("multi-edit-degenerate");
        std::fs::write(root.join("a.rs"), "keep\n").unwrap();
        let tool = MultiEdit::new(root.clone());

        let empty = tool
            .execute(&json!({ "path": "a.rs", "edits": [] }))
            .await
            .unwrap_err();
        assert!(empty.contains("must not be empty"));

        let identical = tool
            .execute(&json!({
                "path": "a.rs",
                "edits": [{ "old_string": "keep", "new_string": "keep" }],
            }))
            .await
            .unwrap_err();
        assert!(identical.contains("identical"));

        let blank = tool
            .execute(&json!({
                "path": "a.rs",
                "edits": [{ "old_string": "", "new_string": "x" }],
            }))
            .await
            .unwrap_err();
        assert!(blank.contains("old_string must not be empty"));

        assert_eq!(
            std::fs::read_to_string(root.join("a.rs")).unwrap(),
            "keep\n"
        );
    }

    #[tokio::test]
    async fn ls_marks_directories_and_sorts() {
        let root = scratch("ls");
        std::fs::create_dir(root.join("zdir")).unwrap();
        std::fs::write(root.join("afile"), "").unwrap();

        let output = Ls::new(root).execute(&json!({})).await.unwrap();
        assert_eq!(output, "afile\nzdir/");
    }

    #[tokio::test]
    async fn tools_reject_paths_outside_the_workspace() {
        let root = scratch("escape");
        let error = Read::new(root)
            .execute(&json!({ "path": "../../etc/passwd" }))
            .await
            .unwrap_err();
        assert!(error.contains("escapes the workspace"));
    }
}
