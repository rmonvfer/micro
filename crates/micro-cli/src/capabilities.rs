//! Settling what each loaded extension may do.

use micro_extensions::Capability;
use micro_extensions::Grant;
use micro_extensions::Grants;
use std::path::Path;
use std::path::PathBuf;

/// What was settled, and what a reader should be told about how it was settled.
pub struct Resolved {
    pub grants: Grants,
    pub notices: Vec<String>,
}

/// Work out what every loaded extension may do.
pub async fn resolve(
    loaded: &micro_extensions::Loaded,
    roots: &[(PathBuf, String)],
    trusted: bool,
    has_ui: bool,
) -> Resolved {
    let mut store = micro_config::CapabilityStore::load()
        .await
        .unwrap_or_default();
    let mut grants = Vec::new();
    let mut notices = Vec::new();
    let mut decided = false;

    for extension in &loaded.extensions {
        let name = crate::runtime::extension_name(&extension.path, roots);
        let path = Path::new(&extension.path);

        let spoken = extension
            .capabilities
            .clone()
            .or_else(|| micro_extensions::declared(path));

        if let Some(spoken) = spoken {
            let (allowed, unknown) = micro_extensions::parse_all(&spoken);
            if !unknown.is_empty() {
                notices.push(format!(
                    "{name} asks for a capability micro does not have: {}",
                    unknown.join(", ")
                ));
            }
            grants.push(Grant {
                path: extension.path.clone(),
                name,
                declared: true,
                allowed,
            });
            continue;
        }

        let would_need = micro_extensions::derived(extension);
        let allowed = if trusted {
            would_need
        } else if let Some(decision) = store.decision(path) {
            let (allowed, _) = micro_extensions::parse_all(&decision.capabilities);
            allowed
        } else if has_ui {
            let granted = ask_about_capabilities(&name, &extension.path, &would_need);
            store.decide(
                path,
                match granted {
                    true => would_need
                        .iter()
                        .map(|capability| capability.as_str().to_string())
                        .collect(),
                    false => Vec::new(),
                },
            );
            decided = true;
            match granted {
                true => would_need,
                false => Default::default(),
            }
        } else {
            notices.push(format!(
                "{name} declares no capabilities and nobody is here to be asked, so it can \
                 only read: add \"micro\": {{ \"capabilities\": [...] }} to its package.json, \
                 or trust this project."
            ));
            Default::default()
        };

        grants.push(Grant {
            path: extension.path.clone(),
            name,
            declared: false,
            allowed,
        });
    }

    if decided {
        if let Err(error) = store.save().await {
            notices.push(format!("the decision was not saved: {error}"));
        }
    }

    Resolved {
        grants: Grants::new(grants),
        notices,
    }
}

/// Put the question to whoever is at the terminal, before the interface takes it over.
fn ask_about_capabilities(
    name: &str,
    path: &str,
    would_need: &std::collections::BTreeSet<Capability>,
) -> bool {
    use std::io::BufRead as _;
    use std::io::Write as _;

    let listed: Vec<&str> = Capability::ALL
        .iter()
        .filter(|capability| would_need.contains(capability))
        .map(Capability::as_str)
        .collect();

    println!("Allow the extension {name}?");
    println!("{path}");
    println!();
    println!("It declares no capabilities, so micro worked out what it would need:");
    println!("  {}", listed.join(", "));
    print!("Allow it? [y/N] ");
    let _ = std::io::stdout().flush();

    let mut answer = String::new();
    if std::io::stdin().lock().read_line(&mut answer).is_err() {
        return false;
    }
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// One extension's capabilities as `micro list` prints them.
pub fn describe(grant: &Grant) -> String {
    let listed = grant.listed();
    let listed = match listed.is_empty() {
        true => "read-only".to_string(),
        false => listed.join(", "),
    };
    match grant.declared {
        true => listed,
        false => format!("legacy: {listed}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extension(path: &str, capabilities: Option<Vec<String>>) -> micro_extensions::Registered {
        micro_extensions::Registered {
            path: path.to_string(),
            capabilities,
            tools: vec![micro_extensions::RegisteredTool {
                name: "thing".into(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    /// A declared manifest is the whole answer: what it named is what it may do, and nothing it did
    /// not name comes along with it.
    #[tokio::test]
    async fn a_declared_manifest_grants_exactly_what_it_names() {
        let loaded = micro_extensions::Loaded {
            extensions: vec![extension(
                "/x/declared.ts",
                Some(vec!["tools".into(), "exec".into()]),
            )],
            errors: Vec::new(),
        };

        let resolved = resolve(&loaded, &[], false, false).await;
        let grant = resolved
            .grants
            .grant(Some("/x/declared.ts"))
            .expect("a grant");
        assert!(grant.declared);
        assert!(grant.allows(Capability::Tools));
        assert!(grant.allows(Capability::Exec));
        assert!(!grant.allows(Capability::SessionControl));
    }

    #[tokio::test]
    async fn a_capability_nobody_has_heard_of_is_reported() {
        let loaded = micro_extensions::Loaded {
            extensions: vec![extension(
                "/x/odd.ts",
                Some(vec!["exec".into(), "telepathy".into()]),
            )],
            errors: Vec::new(),
        };

        let resolved = resolve(&loaded, &[], false, false).await;
        assert!(
            resolved
                .notices
                .iter()
                .any(|notice| notice.contains("telepathy")),
            "{:?}",
            resolved.notices
        );
        assert!(resolved
            .grants
            .grant(Some("/x/odd.ts"))
            .expect("a grant")
            .allows(Capability::Exec));
    }

    /// A trusted project runs what it ships exactly as it did before capabilities existed, with
    /// nobody asked anything.
    #[tokio::test]
    async fn a_legacy_extension_in_a_trusted_project_keeps_everything() {
        let loaded = micro_extensions::Loaded {
            extensions: vec![extension("/x/legacy.ts", None)],
            errors: Vec::new(),
        };

        let resolved = resolve(&loaded, &[], true, false).await;
        let grant = resolved
            .grants
            .grant(Some("/x/legacy.ts"))
            .expect("a grant");
        assert!(!grant.declared);
        assert!(grant.allows(Capability::Tools));
        assert!(grant.allows(Capability::Exec));
        assert!(grant.allows(Capability::SessionControl));
        assert!(resolved.notices.is_empty(), "{:?}", resolved.notices);
    }

    /// Headless and untrusted there is nobody to ask.
    #[tokio::test]
    async fn a_legacy_extension_with_nobody_to_ask_can_only_read() {
        let loaded = micro_extensions::Loaded {
            extensions: vec![extension("/x/legacy.ts", None)],
            errors: Vec::new(),
        };

        let resolved = resolve(&loaded, &[], false, false).await;
        let grant = resolved
            .grants
            .grant(Some("/x/legacy.ts"))
            .expect("a grant");
        assert!(grant.allowed.is_empty());
        assert!(
            resolved
                .notices
                .iter()
                .any(|notice| notice.contains("capabilities")),
            "{:?}",
            resolved.notices
        );
    }

    #[test]
    fn what_a_listing_says_tells_a_manifest_apart_from_a_derived_set() {
        let declared = Grant {
            path: "/x/a.ts".into(),
            name: "a".into(),
            declared: true,
            allowed: [Capability::Tools].into_iter().collect(),
        };
        assert_eq!(describe(&declared), "tools");

        let legacy = Grant {
            path: "/x/b.ts".into(),
            name: "b".into(),
            declared: false,
            allowed: [Capability::Ui, Capability::Exec].into_iter().collect(),
        };

        assert_eq!(describe(&legacy), "legacy: exec, ui");

        let refused = Grant {
            path: "/x/c.ts".into(),
            name: "c".into(),
            declared: false,
            allowed: Default::default(),
        };
        assert_eq!(describe(&refused), "legacy: read-only");
    }
}
