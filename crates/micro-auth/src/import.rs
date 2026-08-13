//! Bringing credentials across from agent47.
//!
//! agent47 writes the same credential shapes micro does, so an import is a copy with the
//! provider names folded onto micro's own. The source file is only ever read.

use crate::canonical_provider;
use crate::save;
use crate::AuthError;
use crate::AuthStore;
use crate::Credential;
use crate::Result;
use crate::providers;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

/// Environment variable naming agent47's home directory.
pub const AGENT47_DIR_ENV: &str = "AGENT47_DIR";
const AGENT47_FILE: &str = "auth.json";

/// What became of one entry in the source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportOutcome {
    Imported,
    /// Imported over a credential micro already held.
    Replaced,
    /// Micro already has one, and the caller did not ask to overwrite.
    AlreadyPresent,
    /// Micro has no provider by that name.
    Unsupported,
    /// Not a credential micro can read. Nothing from the entry is repeated, here or
    /// anywhere else: an entry that failed to parse may still hold a secret.
    Unreadable,
}

impl ImportOutcome {
    pub fn is_imported(&self) -> bool {
        matches!(self, ImportOutcome::Imported | ImportOutcome::Replaced)
    }

    /// Why an entry was left alone, for a caller that reports one line per provider.
    pub fn reason(&self) -> &'static str {
        match self {
            ImportOutcome::Imported => "imported",
            ImportOutcome::Replaced => "imported, replacing the one micro held",
            ImportOutcome::AlreadyPresent => "skipped: micro already has a credential for it",
            ImportOutcome::Unsupported => "skipped: micro has no provider by that name",
            ImportOutcome::Unreadable => "skipped: not a credential micro can read",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportEntry {
    /// The provider under micro's own name, not agent47's.
    pub provider: String,
    pub outcome: ImportOutcome,
}

/// What an import did, per provider. Holds no credential material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReport {
    pub source: PathBuf,
    /// One entry per credential in the source file, sorted by provider.
    pub entries: Vec<ImportEntry>,
}

impl ImportReport {
    pub fn imported(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.outcome.is_imported())
            .count()
    }

    pub fn skipped(&self) -> usize {
        self.entries.len() - self.imported()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// One line per provider, ready to print.
impl fmt::Display for ImportReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.entries.is_empty() {
            return write!(formatter, "{} holds no credentials", self.source.display());
        }

        let width = self
            .entries
            .iter()
            .map(|entry| entry.provider.chars().count())
            .max()
            .unwrap_or(0);

        for (position, entry) in self.entries.iter().enumerate() {
            if position > 0 {
                writeln!(formatter)?;
            }
            write!(
                formatter,
                "{:width$}  {}",
                entry.provider,
                entry.outcome.reason(),
                width = width
            )?;
        }
        Ok(())
    }
}

/// `$AGENT47_DIR/auth.json`, falling back to `~/.agent47/auth.json`.
pub fn agent47_path() -> Option<PathBuf> {
    let home = std::env::var("HOME")
        .ok()
        .or_else(|| std::env::var("USERPROFILE").ok());
    path_from(
        std::env::var(AGENT47_DIR_ENV).ok().as_deref(),
        home.as_deref(),
    )
}

/// Whether there is anything to import, so a caller can offer only when there is.
pub fn agent47_available() -> bool {
    agent47_path().is_some_and(|path| path.is_file())
}

fn path_from(agent47_dir: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    if let Some(dir) = agent47_dir.map(str::trim).filter(|dir| !dir.is_empty()) {
        return Some(PathBuf::from(dir).join(AGENT47_FILE));
    }
    home.map(str::trim)
        .filter(|home| !home.is_empty())
        .map(|home| PathBuf::from(home).join(".agent47").join(AGENT47_FILE))
}

impl AuthStore {
    /// Copy agent47's credentials in from its usual place.
    pub fn import_agent47(&self, overwrite: bool) -> Result<ImportReport> {
        let path = agent47_path().ok_or_else(|| AuthError::Import {
            path: "~/.agent47".into(),
            message: "no home directory; set AGENT47_DIR".into(),
        })?;
        self.import_from(path, overwrite)
    }

    /// Copy credentials in from an agent47 credential file, which is only read.
    ///
    /// One entry that cannot be read does not stop the others: every credential the file
    /// holds is reported on, and the store is written once at the end.
    pub fn import_from(&self, path: impl AsRef<Path>, overwrite: bool) -> Result<ImportReport> {
        let path = path.as_ref();
        let contents = fs::read_to_string(path).map_err(|error| AuthError::Import {
            path: path.display().to_string(),
            message: match error.kind() {
                std::io::ErrorKind::NotFound => "no credential file there".to_string(),
                _ => error.to_string(),
            },
        })?;

        let source: BTreeMap<String, Value> =
            serde_json::from_str(&contents).map_err(|error| AuthError::Import {
                path: path.display().to_string(),
                message: format!("is not a credential file: {error}"),
            })?;

        let mut entries = Vec::new();
        let mut cache = self.lock();
        // Importing writes the file like any other change, so it takes the same lock and
        // works from what is on disk now rather than what was there at startup.
        let _held = crate::lockfile::FileLock::acquire(&self.path)
            .map_err(|error| crate::storage_error(&self.path, error))?;
        let mut credentials = crate::load(&self.path)?;
        let mut changed = false;

        for (name, value) in source {
            let provider = canonical_provider(&name).to_string();
            let outcome = if !providers().contains(&provider.as_str()) {
                ImportOutcome::Unsupported
            } else {
                match serde_json::from_value::<Credential>(value) {
                    Err(_) => ImportOutcome::Unreadable,
                    Ok(credential) => {
                        let held = credentials.contains_key(&provider);
                        if held && !overwrite {
                            ImportOutcome::AlreadyPresent
                        } else {
                            credentials.insert(provider.clone(), credential);
                            changed = true;
                            if held {
                                ImportOutcome::Replaced
                            } else {
                                ImportOutcome::Imported
                            }
                        }
                    }
                }
            };

            entries.push(ImportEntry { provider, outcome });
        }

        if changed {
            save(&self.path, &credentials)?;
            cache.revision = crate::revision_of(&self.path);
        }
        cache.credentials = credentials;
        drop(cache);

        entries.sort_by(|left, right| left.provider.cmp(&right.provider));
        Ok(ImportReport {
            source: path.to_path_buf(),
            entries,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OAuthCredential;
    use std::sync::atomic::AtomicU32;
    use std::sync::atomic::Ordering;

    /// A directory of this process's own. No test reads the real `~/.agent47`.
    fn scratch(label: &str) -> PathBuf {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let directory = std::env::temp_dir().join(format!(
            "micro-import-{label}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    /// A stand-in for agent47's file, with invented tokens.
    const FIXTURE: &str = r#"{
        "github-copilot": {
            "type": "oauth",
            "accessToken": "fixture-access",
            "refreshToken": "fixture-refresh",
            "expires": 0
        },
        "openrouter": { "type": "api_key", "key": "fixture-openrouter" }
    }"#;

    struct Fixture {
        source: PathBuf,
        store: AuthStore,
    }

    fn fixture(label: &str, contents: &str) -> Fixture {
        let directory = scratch(label);
        let source = directory.join("agent47-auth.json");
        fs::write(&source, contents).unwrap();

        Fixture {
            source,
            store: AuthStore::open_at(directory.join("auth.json")).unwrap(),
        }
    }

    fn outcome(report: &ImportReport, provider: &str) -> ImportOutcome {
        report
            .entries
            .iter()
            .find(|entry| entry.provider == provider)
            .unwrap_or_else(|| panic!("no entry for {provider} in {report:?}"))
            .outcome
    }

    #[test]
    fn both_credential_shapes_come_across() {
        let fixture = fixture("both", FIXTURE);
        let report = fixture.store.import_from(&fixture.source, false).unwrap();

        assert_eq!(report.imported(), 2);
        assert_eq!(report.skipped(), 0);
        assert_eq!(outcome(&report, "github-copilot"), ImportOutcome::Imported);
        assert_eq!(outcome(&report, "openrouter"), ImportOutcome::Imported);

        assert_eq!(
            fixture.store.get("openrouter"),
            Some(Credential::api_key("fixture-openrouter"))
        );
        assert_eq!(
            fixture.store.get("github-copilot"),
            Some(Credential::OAuth(OAuthCredential {
                access_token: "fixture-access".into(),
                refresh_token: "fixture-refresh".into(),
                expires: 0,
            }))
        );
    }

    #[test]
    fn imported_credentials_survive_a_reopen() {
        let fixture = fixture("persist", FIXTURE);
        fixture.store.import_from(&fixture.source, false).unwrap();

        let reopened = AuthStore::open_at(fixture.store.path()).unwrap();
        assert_eq!(reopened.providers(), vec!["github-copilot", "openrouter"]);
    }

    #[test]
    fn the_source_file_is_left_exactly_as_it_was() {
        let fixture = fixture("read-only", FIXTURE);
        let before = fs::read(&fixture.source).unwrap();

        fixture.store.import_from(&fixture.source, true).unwrap();

        assert_eq!(fs::read(&fixture.source).unwrap(), before);
    }

    #[test]
    fn a_provider_name_is_folded_onto_micros_own() {
        let fixture = fixture(
            "aliases",
            r#"{ "google": { "type": "api_key", "key": "fixture-gemini" } }"#,
        );
        let report = fixture.store.import_from(&fixture.source, false).unwrap();

        assert_eq!(outcome(&report, "google"), ImportOutcome::Imported);
        assert_eq!(fixture.store.providers(), vec!["google"]);
    }

    #[test]
    fn an_existing_credential_is_kept_unless_replacing_is_asked_for() {
        let fixture = fixture("existing", FIXTURE);
        fixture
            .store
            .store_api_key("openrouter", "already-mine")
            .unwrap();

        let report = fixture.store.import_from(&fixture.source, false).unwrap();
        assert_eq!(
            outcome(&report, "openrouter"),
            ImportOutcome::AlreadyPresent
        );
        assert_eq!(
            fixture.store.get("openrouter").unwrap().token(),
            "already-mine"
        );

        let report = fixture.store.import_from(&fixture.source, true).unwrap();
        assert_eq!(outcome(&report, "openrouter"), ImportOutcome::Replaced);
        assert_eq!(
            fixture.store.get("openrouter").unwrap().token(),
            "fixture-openrouter"
        );
    }

    #[test]
    fn a_provider_micro_cannot_talk_to_is_skipped() {
        let fixture = fixture(
            "unsupported",
            r#"{
                "not-a-service": { "type": "api_key", "key": "fixture" },
                "openrouter": { "type": "api_key", "key": "fixture-openrouter" }
            }"#,
        );
        let report = fixture.store.import_from(&fixture.source, false).unwrap();

        assert_eq!(outcome(&report, "not-a-service"), ImportOutcome::Unsupported);
        assert_eq!(report.imported(), 1);
        assert_eq!(fixture.store.providers(), vec!["openrouter"]);
    }

    #[test]
    fn one_unreadable_entry_does_not_stop_the_others() {
        let fixture = fixture(
            "unreadable",
            r#"{
                "anthropic": { "type": "sorcery", "key": "fixture-anthropic" },
                "gemini": { "type": "api_key" },
                "openrouter": { "type": "api_key", "key": "fixture-openrouter" }
            }"#,
        );
        let report = fixture.store.import_from(&fixture.source, false).unwrap();

        assert_eq!(outcome(&report, "anthropic"), ImportOutcome::Unreadable);
        assert_eq!(outcome(&report, "google"), ImportOutcome::Unreadable);
        assert_eq!(outcome(&report, "openrouter"), ImportOutcome::Imported);
        assert_eq!(fixture.store.providers(), vec!["openrouter"]);
    }

    #[test]
    fn an_absent_file_is_reported_rather_than_panicking() {
        let directory = scratch("absent");
        let store = AuthStore::open_at(directory.join("auth.json")).unwrap();

        let error = store
            .import_from(directory.join("nothing-here.json"), false)
            .unwrap_err();
        assert!(matches!(error, AuthError::Import { .. }), "{error}");
        assert!(error.to_string().contains("no credential file"), "{error}");
    }

    #[test]
    fn a_malformed_file_is_reported_rather_than_panicking() {
        let fixture = fixture("malformed", "{ not json");
        let error = fixture
            .store
            .import_from(&fixture.source, false)
            .unwrap_err();

        assert!(matches!(error, AuthError::Import { .. }), "{error}");
        assert!(
            error.to_string().contains("not a credential file"),
            "{error}"
        );
    }

    #[test]
    fn an_empty_file_imports_nothing_and_says_so() {
        let fixture = fixture("empty", "{}");
        let report = fixture.store.import_from(&fixture.source, false).unwrap();

        assert!(report.is_empty());
        assert_eq!(report.imported(), 0);
        assert!(report.to_string().contains("holds no credentials"));
    }

    #[test]
    fn nothing_is_written_when_nothing_is_imported() {
        let fixture = fixture(
            "no-write",
            r#"{ "not-a-service": { "type": "api_key", "key": "x" } }"#,
        );
        fixture.store.import_from(&fixture.source, false).unwrap();

        assert!(
            !fixture.store.path().exists(),
            "the store was written for an import that changed nothing"
        );
    }

    #[test]
    fn the_report_names_providers_and_outcomes_and_no_secrets() {
        let fixture = fixture("report", FIXTURE);
        let report = fixture.store.import_from(&fixture.source, false).unwrap();
        let printed = report.to_string();

        assert!(printed.contains("github-copilot  imported"), "{printed}");
        assert!(printed.contains("openrouter"), "{printed}");
        for secret in ["fixture-access", "fixture-refresh", "fixture-openrouter"] {
            assert!(!printed.contains(secret), "the report leaked a token");
            assert!(
                !format!("{report:?}").contains(secret),
                "the debug output leaked a token"
            );
        }
    }

    #[test]
    fn agent47_dir_overrides_the_home_directory() {
        assert_eq!(
            path_from(Some("/tmp/agent47"), Some("/home/x")),
            Some(PathBuf::from("/tmp/agent47/auth.json"))
        );
        assert_eq!(
            path_from(None, Some("/home/x")),
            Some(PathBuf::from("/home/x/.agent47/auth.json"))
        );
        assert_eq!(path_from(None, None), None);
    }
}
