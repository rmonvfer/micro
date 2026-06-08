//! Filesystem tools.

use crate::required_str;
use crate::resolve_path;
use crate::truncate;
use crate::Guard;
use crate::Tool;
use async_trait::async_trait;
use micro_types::ToolDefinition;
use serde_json::json;
use serde_json::Value;
use std::path::Path;
use std::path::PathBuf;

/// How many entries a listing returns when the caller does not say.
const DEFAULT_LS_LIMIT: usize = 500;

pub struct Read {
    root: PathBuf,
    guard: Guard,
}

impl Read {
    pub fn new(root: PathBuf, guard: Guard) -> Self {
        Read { root, guard }
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
            constrained_sampling: None,
        }
    }

    async fn execute_content(
        &self,
        arguments: &Value,
        _progress: &crate::Progress,
    ) -> Result<Vec<micro_types::ContentBlock>, String> {
        let path = resolve_path(&self.root, &required_str(arguments, "path")?)?;
        self.guard.read(&path)?;

        if let Some(mime_type) = image_mime_type(&path) {
            let bytes = tokio::fs::read(&path)
                .await
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
            return Ok(vec![
                micro_types::ContentBlock::text(format!("Read image file [{mime_type}]")),
                micro_types::ContentBlock::Image {
                    data: base64::Engine::encode(
                        &base64::engine::general_purpose::STANDARD,
                        &bytes,
                    ),
                    mime_type: mime_type.to_string(),
                },
            ]);
        }

        self.execute(arguments)
            .await
            .map(|text| vec![micro_types::ContentBlock::text(text)])
    }

    async fn execute(&self, arguments: &Value) -> Result<String, String> {
        let path = resolve_path(&self.root, &required_str(arguments, "path")?)?;
        self.guard.read(&path)?;
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
    guard: Guard,
}

impl Write {
    pub fn new(root: PathBuf, guard: Guard) -> Self {
        Write { root, guard }
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
            constrained_sampling: None,
        }
    }

    async fn execute(&self, arguments: &Value) -> Result<String, String> {
        let path = resolve_path(&self.root, &required_str(arguments, "path")?)?;
        self.guard.write(&path)?;
        let content = required_str(arguments, "content")?;
        let _held = crate::mutations::hold(&path).await;

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
    guard: Guard,
}

impl Edit {
    pub fn new(root: PathBuf, guard: Guard) -> Self {
        Edit { root, guard }
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
            constrained_sampling: None,
        }
    }

    async fn execute(&self, arguments: &Value) -> Result<String, String> {
        let path = resolve_path(&self.root, &required_str(arguments, "path")?)?;
        self.guard.write(&path)?;
        let old_string = required_str(arguments, "old_string")?;
        let new_string = required_str(arguments, "new_string")?;

        if old_string == new_string {
            return Err("old_string and new_string are identical".to_string());
        }

        let _held = crate::mutations::hold(&path).await;
        let contents = tokio::fs::read_to_string(&path)
            .await
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;

        let occurrences = crate::fuzzy::count(&contents, &old_string);
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

        let found = crate::fuzzy::find(&contents, &old_string)
            .ok_or_else(|| format!("old_string not found in {}", path.display()))?;
        let fuzzy = found.fuzzy;
        let mut updated = found.haystack;
        updated.replace_range(found.start..found.start + found.length, &new_string);

        tokio::fs::write(&path, &updated)
            .await
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;

        Ok(match fuzzy {
            true => format!(
                "Edited {} (matched ignoring quote, dash and whitespace differences)",
                path.display()
            ),
            false => format!("Edited {}", path.display()),
        })
    }
}

pub struct MultiEdit {
    root: PathBuf,
    guard: Guard,
}

impl MultiEdit {
    pub fn new(root: PathBuf, guard: Guard) -> Self {
        MultiEdit { root, guard }
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
            constrained_sampling: None,
        }
    }

    async fn execute(&self, arguments: &Value) -> Result<String, String> {
        let path = resolve_path(&self.root, &required_str(arguments, "path")?)?;
        self.guard.write(&path)?;
        let edits = arguments
            .get("edits")
            .and_then(Value::as_array)
            .ok_or_else(|| "missing required argument: edits".to_string())?;
        if edits.is_empty() {
            return Err("edits must not be empty".to_string());
        }

        let _held = crate::mutations::hold(&path).await;
        let contents = tokio::fs::read_to_string(&path)
            .await
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;

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

            match crate::fuzzy::count(&updated, &old_string) {
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

            let found = crate::fuzzy::find(&updated, &old_string).ok_or_else(|| {
                format!(
                    "edit {position}: old_string not found in {}",
                    path.display()
                )
            })?;
            updated = found.haystack;
            updated.replace_range(found.start..found.start + found.length, &new_string);
        }

        tokio::fs::write(&path, &updated)
            .await
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;

        Ok(format!("Edited {} ({} edits)", path.display(), edits.len()))
    }
}

pub struct Ls {
    root: PathBuf,
    guard: Guard,
}

impl Ls {
    pub fn new(root: PathBuf, guard: Guard) -> Self {
        Ls { root, guard }
    }
}

#[async_trait]
impl Tool for Ls {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "ls".into(),
            description: "List directory contents. Returns entries sorted alphabetically, \
                          with '/' suffix for directories. Includes dotfiles."
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory to list (default: current directory)" },
                    "limit": {
                        "type": "number",
                        "description": "Maximum number of entries to return (default: 500)",
                    },
                },
            }),
            constrained_sampling: None,
        }
    }

    async fn execute(&self, arguments: &Value) -> Result<String, String> {
        let requested = arguments.get("path").and_then(Value::as_str).unwrap_or(".");
        let path = resolve_path(&self.root, requested)?;
        self.guard.read(&path)?;
        let limit = arguments
            .get("limit")
            .and_then(Value::as_u64)
            .map(|limit| limit as usize)
            .unwrap_or(DEFAULT_LS_LIMIT)
            .max(1);

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

        let total = names.len();
        names.truncate(limit);
        let mut listing = names.join("\n");
        if total > limit {
            listing.push_str(&format!(
                "\n\n{limit} entry limit reached, {total} entries total"
            ));
        }
        Ok(truncate(&listing))
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

        let tool = Read::new(root.clone(), Guard::for_workspace(&root));
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
        let tool = Write::new(root.clone(), Guard::for_workspace(&root));

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

        Edit::new(root.clone(), Guard::for_workspace(&root))
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

        let error = Edit::new(root.clone(), Guard::for_workspace(&root))
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

        let output = MultiEdit::new(root.clone(), Guard::for_workspace(&root))
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

        MultiEdit::new(root.clone(), Guard::for_workspace(&root))
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

        let error = MultiEdit::new(root.clone(), Guard::for_workspace(&root))
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

        let error = MultiEdit::new(root.clone(), Guard::for_workspace(&root))
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
        let tool = MultiEdit::new(root.clone(), Guard::for_workspace(&root));

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

        let output = Ls::new(root.clone(), Guard::for_workspace(&root))
            .execute(&json!({}))
            .await
            .unwrap();
        assert_eq!(output, "afile\nzdir/");
    }

    #[tokio::test]
    async fn tools_reject_paths_outside_the_workspace() {
        let root = scratch("escape");
        let error = Read::new(root.clone(), Guard::for_workspace(&root))
            .execute(&json!({ "path": "../../etc/passwd" }))
            .await
            .unwrap_err();
        assert!(error.contains("escapes the workspace"));
    }
}

/// What kind of image a path holds, going by its extension.
fn image_mime_type(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "bmp" => Some("image/bmp"),
        _ => None,
    }
}

#[cfg(test)]
mod images {
    use super::*;
    use crate::Progress;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("micro-image-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The smallest valid PNG, so the test does not depend on a fixture file.
    const TINY_PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89,
    ];

    /// An image comes back as something the model looks at, not as text that fails to decode.
    #[tokio::test]
    async fn reading_an_image_hands_back_an_image() {
        let root = scratch("png");
        std::fs::write(root.join("shot.png"), TINY_PNG).unwrap();

        let content = Read::new(root.clone(), Guard::for_workspace(&root))
            .execute_content(&json!({ "path": "shot.png" }), &Progress::default())
            .await
            .expect("an image reads");

        assert!(matches!(
            content.first(),
            Some(micro_types::ContentBlock::Text { .. })
        ));
        match content.get(1) {
            Some(micro_types::ContentBlock::Image { mime_type, data }) => {
                assert_eq!(mime_type, "image/png");
                assert!(!data.is_empty(), "the bytes travel base64 encoded");
            }
            other => panic!("expected an image block, got {other:?}"),
        }
    }

    /// Text still reads as text, numbered as before.
    #[tokio::test]
    async fn reading_text_is_unchanged() {
        let root = scratch("text");
        std::fs::write(root.join("a.txt"), "hello\n").unwrap();

        let content = Read::new(root.clone(), Guard::for_workspace(&root))
            .execute_content(&json!({ "path": "a.txt" }), &Progress::default())
            .await
            .unwrap();

        assert_eq!(content.len(), 1);
        let text: String = content
            .iter()
            .map(micro_types::ContentBlock::as_text)
            .collect();
        assert!(text.contains("hello"));
        assert!(text.contains("1\t"), "line numbers are still there: {text}");
    }

    #[test]
    fn only_formats_a_model_can_look_at_count_as_images() {
        for name in ["a.png", "b.JPG", "c.jpeg", "d.gif", "e.webp", "f.bmp"] {
            assert!(image_mime_type(Path::new(name)).is_some(), "{name}");
        }
        for name in ["a.txt", "b.rs", "c.svg", "d.pdf", "noextension"] {
            assert!(image_mime_type(Path::new(name)).is_none(), "{name}");
        }
    }
}

#[cfg(test)]
mod forgiving_edits {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("micro-forgiving-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A model that read the file through a renderer writes back curly quotes.
    #[tokio::test]
    async fn an_edit_written_with_smart_quotes_still_lands() {
        let root = scratch("quotes");
        std::fs::write(root.join("a.rs"), "let name = \"micro\";\n").unwrap();

        let said = Edit::new(root.clone(), Guard::for_workspace(&root))
            .execute(&json!({
                "path": "a.rs",
                "old_string": "let name = \u{201C}micro\u{201D};",
                "new_string": "let name = \"pi\";",
            }))
            .await
            .expect("the quotes do not count");

        assert!(said.contains("matched ignoring"), "it says so: {said}");
        let after = std::fs::read_to_string(root.join("a.rs")).unwrap();
        assert!(after.contains("let name = \"pi\";"), "{after}");
    }

    #[tokio::test]
    async fn an_edit_written_with_a_dash_still_lands() {
        let root = scratch("dash");
        std::fs::write(root.join("run.sh"), "cargo test --workspace\n").unwrap();

        Edit::new(root.clone(), Guard::for_workspace(&root))
            .execute(&json!({
                "path": "run.sh",
                "old_string": "cargo test \u{2013}-workspace",
                "new_string": "cargo test --all",
            }))
            .await
            .expect("a dash is a dash");

        let after = std::fs::read_to_string(root.join("run.sh")).unwrap();
        assert!(after.contains("cargo test --all"), "{after}");
    }

    #[tokio::test]
    async fn an_edit_for_absent_text_still_fails() {
        let root = scratch("absent");
        std::fs::write(root.join("a.rs"), "let a = 1;\n").unwrap();

        let error = Edit::new(root.clone(), Guard::for_workspace(&root))
            .execute(&json!({
                "path": "a.rs",
                "old_string": "let c = 3;",
                "new_string": "let d = 4;",
            }))
            .await
            .unwrap_err();
        assert!(error.contains("not found"), "{error}");
    }
}

/// What the policy does to a file tool.
#[cfg(test)]
mod policy {
    use super::*;
    use micro_sandbox::Sandbox;
    use micro_sandbox::SandboxPolicy;
    use micro_types::LedgerEvent;

    fn workspace(name: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("micro-tools-policy-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        let workspace = dir.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        (
            dir.canonicalize().unwrap(),
            workspace.canonicalize().unwrap(),
        )
    }

    /// A workspace with a symlink out of it.
    #[tokio::test]
    async fn a_write_that_leaves_the_workspace_by_symlink_is_refused_by_name() {
        let (dir, root) = workspace("symlink");
        let outside = dir.join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();

        let (sender, mut events) = tokio::sync::mpsc::unbounded_channel();
        let guard =
            Guard::new(Sandbox::new(SandboxPolicy::workspace_write(), &root)).recording(sender);

        let error = Write::new(root, guard)
            .execute(&json!({ "path": "escape/loot.txt", "content": "taken" }))
            .await
            .expect_err("the policy does not allow this");

        assert!(error.contains("workspace-write"), "{error}");
        assert!(!outside.join("loot.txt").exists(), "nothing was written");
        assert!(
            matches!(
                events.try_recv(),
                Ok(LedgerEvent::SandboxDecision { allowed: false, .. })
            ),
            "the refusal was recorded"
        );
    }

    /// Under `read-only` the workspace itself is off limits, and reading it is not.
    #[tokio::test]
    async fn read_only_stops_every_write_and_no_read() {
        let (_dir, root) = workspace("read-only");
        std::fs::write(root.join("notes.txt"), "kept\n").unwrap();
        let guard = Guard::new(Sandbox::new(SandboxPolicy::ReadOnly, &root));

        let refused = Write::new(root.clone(), guard.clone())
            .execute(&json!({ "path": "notes.txt", "content": "changed" }))
            .await
            .expect_err("read-only writes nothing");
        assert!(refused.contains("read-only"), "{refused}");

        let edit = Edit::new(root.clone(), guard.clone())
            .execute(&json!({ "path": "notes.txt", "old_string": "kept", "new_string": "gone" }))
            .await
            .expect_err("nor by editing");
        assert!(edit.contains("read-only"), "{edit}");

        Read::new(root.clone(), guard)
            .execute(&json!({ "path": "notes.txt" }))
            .await
            .expect("reading is allowed under every policy");
        assert_eq!(
            std::fs::read_to_string(root.join("notes.txt")).unwrap(),
            "kept\n"
        );
    }

    /// The policy does not narrow what the workspace was already good for: a tool inside it works
    /// exactly as it did.
    #[tokio::test]
    async fn the_workspace_itself_is_untouched_by_the_default_policy() {
        let (_dir, root) = workspace("inside");
        let guard = Guard::for_workspace(&root);

        Write::new(root.clone(), guard.clone())
            .execute(&json!({ "path": "nested/a.txt", "content": "hi" }))
            .await
            .expect("the workspace is writable");
        assert_eq!(
            std::fs::read_to_string(root.join("nested/a.txt")).unwrap(),
            "hi"
        );

        let listing = Ls::new(root, guard).execute(&json!({})).await.unwrap();
        assert!(listing.contains("nested/"), "{listing}");
    }

    /// `.git` stays read-only inside a writable workspace: what runs on the next commit is not
    /// something a session gets to rewrite.
    #[tokio::test]
    async fn the_git_directory_stays_read_only_inside_a_writable_workspace() {
        let (_dir, root) = workspace("git");
        std::fs::create_dir_all(root.join(".git/hooks")).unwrap();

        let error = Write::new(root.clone(), Guard::for_workspace(&root))
            .execute(&json!({ "path": ".git/hooks/pre-commit", "content": "#!/bin/sh\n" }))
            .await
            .expect_err("hooks are not writable");
        assert!(error.contains("read-only"), "{error}");
    }
}
