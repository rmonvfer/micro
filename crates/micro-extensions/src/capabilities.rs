//! What an extension is allowed to ask for.
//!
//! Every extension runs out of process and reaches micro only by asking. A manifest says
//! which of those asks it intends to make, and anything outside it is refused at the point
//! it arrives rather than trusted because the file was installed. Declaring the set is what
//! lets an extension be run without vouching for the whole project it came with.
//!
//! An extension that declares nothing is not refused: its set is derived from what it
//! registered and what code written for pi expects to be able to do, and the user is asked
//! about that set once. Which is the difference between a manifest and a permission prompt
//! — the manifest is the extension saying what it needs, the prompt is what happens when
//! nobody said.

use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::Path;

/// One thing an extension may be allowed to do.
///
/// Grouped by what the ask reaches rather than by which wire message carries it: an
/// extension that may rename the session may also label an entry, because both write to the
/// same log and telling them apart would ask a reader to hold a longer list without
/// deciding anything more.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// Register tools the model may call.
    Tools,
    /// Register slash commands.
    Commands,
    /// Subscribe to what happens during a run.
    Events,
    /// Run a program.
    Exec,
    /// Run one of micro's own built-in tools.
    BuiltinTools,
    /// Stream a request through micro's provider clients.
    ProviderStream,
    /// Put a message into the conversation as though it had been typed.
    SendUserMessage,
    /// Show a message of its own beside the conversation.
    SendMessage,
    /// Write to the session log: keep an entry, label one, name the session.
    SessionWrite,
    /// Move the conversation: the model, the thinking level, compaction, forking,
    /// switching, reloading, interrupting, quitting.
    SessionControl,
    /// Change what the model is told: the system prompt, the conversation on its way to a
    /// request, the headers that request carries.
    Context,
    /// Draw on the screen and ask the reader questions.
    Ui,
    /// Declare a provider.
    Providers,
    /// Declare a command-line flag.
    Flags,
}

impl Capability {
    /// Every capability there is, in the order they are listed to a reader.
    pub const ALL: &'static [Capability] = &[
        Capability::Tools,
        Capability::Commands,
        Capability::Events,
        Capability::Exec,
        Capability::BuiltinTools,
        Capability::ProviderStream,
        Capability::SendUserMessage,
        Capability::SendMessage,
        Capability::SessionWrite,
        Capability::SessionControl,
        Capability::Context,
        Capability::Ui,
        Capability::Providers,
        Capability::Flags,
    ];

    /// What this is called in a manifest and in the ledger.
    pub fn as_str(&self) -> &'static str {
        match self {
            Capability::Tools => "tools",
            Capability::Commands => "commands",
            Capability::Events => "events",
            Capability::Exec => "exec",
            Capability::BuiltinTools => "builtin_tools",
            Capability::ProviderStream => "provider_stream",
            Capability::SendUserMessage => "send_user_message",
            Capability::SendMessage => "send_message",
            Capability::SessionWrite => "session_write",
            Capability::SessionControl => "session_control",
            Capability::Context => "context",
            Capability::Ui => "ui",
            Capability::Providers => "providers",
            Capability::Flags => "flags",
        }
    }

    /// The capability a name stands for, or nothing when it names none.
    ///
    /// Both spellings of every name are read — `builtin_tools` and `builtinTools` — because
    /// a manifest is written in a `package.json`, where the surrounding convention is
    /// camelCase, and a name refused over its punctuation would be a refusal about nothing.
    pub fn parse(name: &str) -> Option<Capability> {
        let normalized: String = name
            .trim()
            .chars()
            .flat_map(|letter| match letter.is_ascii_uppercase() {
                true => vec!['_', letter.to_ascii_lowercase()],
                false => vec![letter],
            })
            .collect();
        Capability::ALL
            .iter()
            .find(|capability| capability.as_str() == normalized)
            .copied()
    }
}

impl std::fmt::Display for Capability {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.pad(self.as_str())
    }
}

/// What one extension may do, and how that was settled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    /// The file it was loaded from, which is what identifies it on the wire.
    pub path: String,
    /// What a reader calls it: the package name where there is one, the file otherwise.
    pub name: String,
    /// Whether the set came from the extension's own manifest rather than being derived
    /// from what it registered.
    pub declared: bool,
    pub allowed: BTreeSet<Capability>,
}

impl Grant {
    pub fn allows(&self, capability: Capability) -> bool {
        self.allowed.contains(&capability)
    }

    /// The set as a reader sees it, in a fixed order so two runs describe it the same way.
    pub fn listed(&self) -> Vec<&'static str> {
        Capability::ALL
            .iter()
            .filter(|capability| self.allowed.contains(capability))
            .map(Capability::as_str)
            .collect()
    }
}

/// What every loaded extension may do, by the path it was loaded from.
///
/// Keyed by path because that is the one name that travels on the wire: an ask arrives
/// tagged with the file that made it, and the package name it resolves to is for saying so
/// afterwards.
#[derive(Debug, Clone, Default)]
pub struct Grants {
    grants: Vec<Grant>,
}

impl Grants {
    pub fn new(grants: Vec<Grant>) -> Grants {
        Grants { grants }
    }

    /// Whether this ask is one the extension that made it may make.
    ///
    /// An ask from nobody in particular — a path this run never loaded, or a message that
    /// arrived without one — is allowed: refusing it would refuse micro's own asks along
    /// with anyone else's, and there is nothing to attribute a refusal to.
    pub fn allows(&self, path: Option<&str>, capability: Capability) -> bool {
        match self.grant(path) {
            Some(grant) => grant.allows(capability),
            None => true,
        }
    }

    pub fn grant(&self, path: Option<&str>) -> Option<&Grant> {
        let path = path?;
        self.grants.iter().find(|grant| grant.path == path)
    }

    /// What to call whoever made this ask.
    pub fn name_of(&self, path: Option<&str>) -> String {
        match self.grant(path) {
            Some(grant) => grant.name.clone(),
            None => path.unwrap_or("an extension").to_string(),
        }
    }

    pub fn all(&self) -> &[Grant] {
        &self.grants
    }

    pub fn is_empty(&self) -> bool {
        self.grants.is_empty()
    }
}

/// The capability set an extension would need for what it registered.
///
/// The registrations are the part that can be observed: a tool it added, a command, a flag,
/// a provider, an event it listens for. The rest — running a program, moving the
/// conversation, drawing — cannot be seen from a load report at all, because an extension
/// only reaches for those while it is running. An extension written before manifests
/// existed expects all of them, so the derived set says so plainly rather than pretending
/// the registrations were the whole story.
pub fn derived(registered: &crate::Registered) -> BTreeSet<Capability> {
    let mut allowed = BTreeSet::new();
    if !registered.tools.is_empty() {
        allowed.insert(Capability::Tools);
    }
    if !registered.commands.is_empty() {
        allowed.insert(Capability::Commands);
    }
    if !registered.events.is_empty() {
        allowed.insert(Capability::Events);
    }
    if !registered.flags.is_empty() {
        allowed.insert(Capability::Flags);
    }
    if !registered.providers.is_empty() {
        allowed.insert(Capability::Providers);
    }
    allowed.extend([
        Capability::Exec,
        Capability::BuiltinTools,
        Capability::ProviderStream,
        Capability::SendUserMessage,
        Capability::SendMessage,
        Capability::SessionWrite,
        Capability::SessionControl,
        Capability::Context,
        Capability::Ui,
    ]);
    allowed
}

/// The set named in a list of capability names, and the names that stand for nothing.
pub fn parse_all(names: &[String]) -> (BTreeSet<Capability>, Vec<String>) {
    let mut allowed = BTreeSet::new();
    let mut unknown = Vec::new();
    for name in names {
        match Capability::parse(name) {
            Some(capability) => {
                allowed.insert(capability);
            }
            None => unknown.push(name.clone()),
        }
    }
    (allowed, unknown)
}

/// What a package's own `package.json` declares its extensions may do.
///
/// Walked up from the entry file rather than read beside it: a package names its entry
/// under `dist/` as often as at its own root, so the manifest is wherever the first one
/// above the file is. The walk stops at [`MANIFEST_SEARCH_DEPTH`] so a file dropped loose
/// into a directory cannot pick up the capabilities of whatever package happens to be
/// further up the tree.
pub fn declared(entry: &Path) -> Option<Vec<String>> {
    let mut directory = entry.parent()?;
    for _ in 0..MANIFEST_SEARCH_DEPTH {
        if let Some(declared) = in_manifest(&directory.join("package.json")) {
            return Some(declared);
        }
        directory = directory.parent()?;
    }
    None
}

/// How far above an entry file a package manifest is looked for.
const MANIFEST_SEARCH_DEPTH: usize = 3;

/// The capabilities one `package.json` declares under `micro` or `pi`, the same names
/// [`crate::entries_of`] reads a package's entry points
/// from, so a package published for pi declares both in one place.
fn in_manifest(path: &Path) -> Option<Vec<String>> {
    let raw = std::fs::read_to_string(path).ok()?;
    let manifest: serde_json::Value = serde_json::from_str(&raw).ok()?;
    for section in ["micro", "pi"] {
        let Some(declared) = manifest.get(section).and_then(|section| section.get("capabilities"))
        else {
            continue;
        };
        let Some(listed) = declared.as_array() else {
            continue;
        };
        return Some(
            listed
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string)
                .collect(),
        );
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_capability_reads_back_from_the_name_it_is_written_as() {
        for capability in Capability::ALL {
            assert_eq!(Capability::parse(capability.as_str()), Some(*capability));
        }
        assert_eq!(Capability::parse("telepathy"), None);
    }

    /// A manifest lives in a `package.json`, where the convention around it is camelCase,
    /// so both spellings name the same thing rather than one of them naming nothing.
    #[test]
    fn a_camel_cased_name_means_the_same_capability() {
        assert_eq!(
            Capability::parse("builtinTools"),
            Some(Capability::BuiltinTools)
        );
        assert_eq!(
            Capability::parse("sendUserMessage"),
            Some(Capability::SendUserMessage)
        );
        assert_eq!(Capability::parse("  exec "), Some(Capability::Exec));
    }

    #[test]
    fn a_name_that_stands_for_nothing_is_reported_rather_than_dropped() {
        let (allowed, unknown) = parse_all(&["exec".into(), "telepathy".into()]);
        assert!(allowed.contains(&Capability::Exec));
        assert_eq!(unknown, vec!["telepathy".to_string()]);
    }

    /// An ask from a path this run never loaded is not attributable to anyone, so there is
    /// nobody to refuse and nothing to record it against.
    #[test]
    fn an_ask_from_nobody_is_allowed() {
        let grants = Grants::default();
        assert!(grants.allows(None, Capability::Exec));
        assert!(grants.allows(Some("/nowhere.ts"), Capability::Exec));
        assert_eq!(grants.name_of(Some("/nowhere.ts")), "/nowhere.ts");
    }

    #[test]
    fn a_declared_manifest_is_read_from_above_the_entry_file() {
        let root = std::env::temp_dir().join(format!(
            "micro-capabilities-manifest-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|since| since.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(root.join("dist")).unwrap();
        std::fs::write(
            root.join("package.json"),
            r#"{ "name": "thing", "micro": { "capabilities": ["tools", "exec"] } }"#,
        )
        .unwrap();
        std::fs::write(root.join("dist/index.js"), "export default () => {}").unwrap();

        let declared = declared(&root.join("dist/index.js")).expect("the manifest is found");
        assert_eq!(declared, vec!["tools".to_string(), "exec".to_string()]);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// What an extension registered is what can be observed about it; the rest of the
    /// legacy set is what code written before manifests existed expects to be able to do.
    #[test]
    fn a_legacy_set_covers_what_was_registered_and_what_cannot_be_seen() {
        let registered = crate::Registered {
            path: "/x/thing.ts".into(),
            tools: vec![crate::RegisteredTool {
                name: "thing".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let allowed = derived(&registered);
        assert!(allowed.contains(&Capability::Tools));
        assert!(!allowed.contains(&Capability::Commands), "nothing registered a command");
        assert!(allowed.contains(&Capability::Exec));
        assert!(allowed.contains(&Capability::SessionControl));
    }
}
