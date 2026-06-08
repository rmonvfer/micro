//! Skills: instructions a user writes once and the model reaches for when they apply.

mod frontmatter;

pub use frontmatter::parse_frontmatter;

use std::path::Path;
use std::path::PathBuf;

/// A name may be this long, per the skills format.
const MAX_NAME: usize = 64;
/// And a description this long.
const MAX_DESCRIPTION: usize = 1024;

/// One skill, as loaded from disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,

    pub path: PathBuf,
    /// The directory it lives in, which anything it refers to is relative to.
    pub base_dir: PathBuf,
    /// Where it came from, for reporting which shelf a skill is on.
    pub source: String,
    /// Whether the model may invoke it, as opposed to the user invoking it by name.
    pub model_invocable: bool,
}

impl Skill {
    /// The line the system prompt carries: enough to know the skill exists and when it applies,
    /// without its body.
    pub fn summary(&self) -> String {
        format!("- {}: {}", self.name, self.description)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Loaded {
    pub skills: Vec<Skill>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Read the skills at `path`, which may be a directory of them or one file.
pub async fn load_from_path(path: impl AsRef<Path>, source: &str) -> Loaded {
    let path = path.as_ref();
    if path.is_dir() {
        return load_from_dir(path, source).await;
    }

    let mut loaded = Loaded::default();
    let base = path.parent().unwrap_or(path);
    read_skill(path, base, source, &mut loaded).await;
    loaded
}

/// Read every skill under `dir`.
pub async fn load_from_dir(dir: impl AsRef<Path>, source: &str) -> Loaded {
    let mut loaded = Loaded::default();
    walk(dir.as_ref(), source, true, &mut loaded).await;
    loaded.skills.sort_by(|a, b| a.name.cmp(&b.name));
    loaded
}

/// Where a user's shared skills live, if a home directory is known.
pub fn user_agents_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".agents/skills"))
}

/// Read the skills a workspace and a user have between them.
pub async fn discover(
    workspace: impl AsRef<Path>,
    home: impl AsRef<Path>,
    agents: Option<PathBuf>,
    trusted: bool,
) -> Loaded {
    let mut loaded = Loaded::default();
    let mut roots: Vec<(PathBuf, &str)> = Vec::new();

    if trusted {
        roots.push((workspace.as_ref().join(".micro/skills"), "project"));
        roots.push((workspace.as_ref().join(".agents/skills"), "project"));
    }

    roots.push((home.as_ref().join("skills"), "user"));
    roots.extend(agents.map(|dir| (dir, "user")));

    for (dir, source) in roots {
        let found = load_from_dir(&dir, source).await;
        for skill in found.skills {
            if !loaded.skills.iter().any(|kept| kept.name == skill.name) {
                loaded.skills.push(skill);
            }
        }
        loaded.diagnostics.extend(found.diagnostics);
    }
    loaded.skills.sort_by(|a, b| a.name.cmp(&b.name));
    loaded
}

async fn walk(dir: &Path, source: &str, include_root_files: bool, loaded: &mut Loaded) {
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return;
    };

    let mut files: Vec<PathBuf> = Vec::new();
    let mut directories: Vec<PathBuf> = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        match tokio::fs::metadata(&path).await {
            Ok(meta) if meta.is_dir() => directories.push(path),
            Ok(_) => files.push(path),
            Err(_) => {}
        }
    }

    if let Some(declared) = files.iter().find(|path| is_skill_file(path)) {
        read_skill(declared, dir, source, loaded).await;
        return;
    }

    if include_root_files {
        files.sort();
        for path in files.iter().filter(|path| is_markdown(path)) {
            read_skill(path, dir, source, loaded).await;
        }
    }

    directories.sort();
    for path in directories {
        Box::pin(walk(&path, source, false, loaded)).await;
    }
}

fn is_skill_file(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name == "SKILL.md")
}

fn is_markdown(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "md")
}

async fn read_skill(path: &Path, base_dir: &Path, source: &str, loaded: &mut Loaded) {
    let Ok(text) = tokio::fs::read_to_string(path).await else {
        loaded.diagnostics.push(Diagnostic {
            path: path.to_path_buf(),
            message: "cannot be read".to_string(),
        });
        return;
    };

    let parsed = parse_frontmatter(&text);
    let name = parsed
        .field("name")
        .map(str::to_string)
        .or_else(|| directory_name(path, base_dir))
        .unwrap_or_default();

    let description = parsed.field("description").unwrap_or_default().to_string();

    let mut problems = validate_name(&name);
    problems.extend(validate_description(&description));
    if !problems.is_empty() {
        loaded.diagnostics.push(Diagnostic {
            path: path.to_path_buf(),
            message: problems.join("; "),
        });
        return;
    }

    loaded.skills.push(Skill {
        name,
        description,
        path: path.to_path_buf(),
        base_dir: base_dir.to_path_buf(),
        source: source.to_string(),

        model_invocable: parsed.field("disable-model-invocation") != Some("true"),
    });
}

fn directory_name(path: &Path, base_dir: &Path) -> Option<String> {
    match is_skill_file(path) {
        true => base_dir.file_name()?.to_str().map(str::to_string),
        false => path.file_stem()?.to_str().map(str::to_string),
    }
}

/// A name is lowercase letters, digits and single hyphens, and no longer than the format allows.
fn validate_name(name: &str) -> Vec<String> {
    let mut problems = Vec::new();
    if name.is_empty() {
        problems.push("name is required".to_string());
        return problems;
    }
    if name.chars().count() > MAX_NAME {
        problems.push(format!(
            "name exceeds {MAX_NAME} characters ({})",
            name.chars().count()
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        problems.push(
            "name contains invalid characters (must be lowercase a-z, 0-9, hyphens only)"
                .to_string(),
        );
    }
    if name.starts_with('-') || name.ends_with('-') {
        problems.push("name must not start or end with a hyphen".to_string());
    }
    if name.contains("--") {
        problems.push("name must not contain consecutive hyphens".to_string());
    }
    problems
}

fn validate_description(description: &str) -> Vec<String> {
    if description.trim().is_empty() {
        return vec!["description is required".to_string()];
    }
    if description.chars().count() > MAX_DESCRIPTION {
        return vec![format!(
            "description exceeds {MAX_DESCRIPTION} characters ({})",
            description.chars().count()
        )];
    }
    Vec::new()
}

/// The part of the system prompt that tells the model which skills it has.
pub fn system_prompt_section(skills: &[Skill]) -> Option<String> {
    let usable: Vec<&Skill> = skills
        .iter()
        .filter(|skill| skill.model_invocable)
        .collect();
    if usable.is_empty() {
        return None;
    }

    let mut out = String::from(
        "You have skills available. Each is a set of instructions for a particular kind of \
         task. When one applies, read its file with the read tool before starting.\n\n",
    );
    for skill in usable {
        out.push_str(&format!("{} ({})\n", skill.summary(), skill.path.display()));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("micro-skills-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, body: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    const GOOD: &str =
        "---\nname: review-code\ndescription: Review a diff for defects.\n---\n\nDo the thing.\n";

    #[tokio::test]
    async fn a_skill_directory_is_read_as_one_skill() {
        let root = scratch("dir");
        write(&root.join("reviewer/SKILL.md"), GOOD);
        write(&root.join("reviewer/reference.md"), "not a skill");

        let loaded = load_from_dir(&root, "project").await;
        assert_eq!(loaded.skills.len(), 1, "{:?}", loaded.skills);
        assert_eq!(loaded.skills[0].name, "review-code");
    }

    /// A directory with no `SKILL.md` offers its own markdown files, which is how a flat shelf of
    /// skills works.
    #[tokio::test]
    async fn loose_markdown_files_are_each_a_skill() {
        let root = scratch("loose");
        write(&root.join("one.md"), GOOD);
        write(
            &root.join("two.md"),
            "---\nname: other\ndescription: Something else.\n---\n",
        );

        let loaded = load_from_dir(&root, "user").await;
        assert_eq!(loaded.skills.len(), 2);
    }

    #[tokio::test]
    async fn a_skill_without_a_name_takes_its_directorys() {
        let root = scratch("implicit");
        write(
            &root.join("summarize/SKILL.md"),
            "---\ndescription: Summarize a document.\n---\n",
        );

        let loaded = load_from_dir(&root, "project").await;
        assert_eq!(loaded.skills[0].name, "summarize");
    }

    #[tokio::test]
    async fn a_bad_name_is_reported_rather_than_dropped() {
        let root = scratch("badname");
        write(
            &root.join("SKILL.md"),
            "---\nname: Not Valid\ndescription: x\n---\n",
        );

        let loaded = load_from_dir(&root, "project").await;
        assert!(loaded.skills.is_empty());
        assert_eq!(loaded.diagnostics.len(), 1);
        assert!(loaded.diagnostics[0].message.contains("invalid characters"));
    }

    #[tokio::test]
    async fn a_missing_description_is_reported() {
        let root = scratch("nodesc");
        write(&root.join("SKILL.md"), "---\nname: fine\n---\n");

        let loaded = load_from_dir(&root, "project").await;
        assert!(loaded.skills.is_empty());
        assert!(loaded.diagnostics[0]
            .message
            .contains("description is required"));
    }

    #[test]
    fn names_follow_the_format() {
        assert!(validate_name("review-code").is_empty());
        assert!(validate_name("a1").is_empty());
        assert!(!validate_name("Review").is_empty());
        assert!(!validate_name("-lead").is_empty());
        assert!(!validate_name("trail-").is_empty());
        assert!(!validate_name("double--hyphen").is_empty());
        assert!(!validate_name(&"x".repeat(MAX_NAME + 1)).is_empty());
    }

    #[tokio::test]
    async fn a_project_skill_wins_over_a_users_with_the_same_name() {
        let root = scratch("precedence");
        let workspace = root.join("work");
        let home = root.join("home");
        write(
            &workspace.join(".micro/skills/thing/SKILL.md"),
            "---\nname: thing\ndescription: The project's.\n---\n",
        );
        write(
            &home.join("skills/thing/SKILL.md"),
            "---\nname: thing\ndescription: The user's.\n---\n",
        );

        let loaded = discover(&workspace, &home, None, true).await;
        assert_eq!(loaded.skills.len(), 1);
        assert_eq!(loaded.skills[0].description, "The project's.");
    }

    #[tokio::test]
    async fn skills_are_read_from_the_shared_directory_too() {
        let root = scratch("shared");
        let workspace = root.join("work");
        let home = root.join("home");
        let agents = root.join("agents");
        write(
            &workspace.join(".agents/skills/project-one/SKILL.md"),
            "---\nname: project-one\ndescription: Shared, in the project.\n---\n",
        );
        write(
            &agents.join("user-one/SKILL.md"),
            "---\nname: user-one\ndescription: Shared, for the user.\n---\n",
        );

        let loaded = discover(&workspace, &home, Some(agents), true).await;

        let names: Vec<&str> = loaded.skills.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["project-one", "user-one"]);
    }

    /// micro's own directory is read first, so a name in both is micro's.
    #[tokio::test]
    async fn micros_own_directory_wins_over_the_shared_one() {
        let root = scratch("shared-precedence");
        let workspace = root.join("work");
        let home = root.join("home");
        write(
            &workspace.join(".micro/skills/thing/SKILL.md"),
            "---\nname: thing\ndescription: micro's own.\n---\n",
        );
        write(
            &workspace.join(".agents/skills/thing/SKILL.md"),
            "---\nname: thing\ndescription: The shared one.\n---\n",
        );

        let loaded = discover(&workspace, &home, None, true).await;
        assert_eq!(loaded.skills.len(), 1);
        assert_eq!(loaded.skills[0].description, "micro's own.");
    }

    #[tokio::test]
    async fn the_projects_shared_skills_wait_on_trust() {
        let root = scratch("shared-trust");
        let workspace = root.join("work");
        let home = root.join("home");
        write(
            &workspace.join(".agents/skills/thing/SKILL.md"),
            "---\nname: thing\ndescription: The project's.\n---\n",
        );

        assert!(discover(&workspace, &home, None, false)
            .await
            .skills
            .is_empty());
        assert_eq!(
            discover(&workspace, &home, None, true).await.skills.len(),
            1
        );
    }

    #[tokio::test]
    async fn a_skill_can_be_kept_from_the_model() {
        let root = scratch("manual");
        write(
            &root.join("SKILL.md"),
            "---\nname: manual\ndescription: Only when asked.\ndisable-model-invocation: true\n---\n",
        );

        let loaded = load_from_dir(&root, "project").await;
        assert!(!loaded.skills[0].model_invocable);
        assert!(system_prompt_section(&loaded.skills).is_none());
    }

    #[tokio::test]
    async fn the_prompt_section_names_every_skill_and_where_to_read_it() {
        let root = scratch("prompt");
        write(&root.join("reviewer/SKILL.md"), GOOD);

        let loaded = load_from_dir(&root, "project").await;
        let section = system_prompt_section(&loaded.skills).expect("a section");
        assert!(section.contains("review-code: Review a diff for defects."));
        assert!(section.contains("SKILL.md"), "it says where to read it");
    }

    #[tokio::test]
    async fn a_missing_directory_is_not_an_error() {
        let loaded = load_from_dir("/nowhere/at/all", "project").await;
        assert!(loaded.skills.is_empty());
        assert!(loaded.diagnostics.is_empty());
    }
}
