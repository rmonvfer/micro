use crate::catalog::{Catalog, ModelDef};

/// The outcome of resolving a user-typed model string.
#[derive(Debug, Clone, PartialEq)]
pub enum Resolution<'a> {
    /// Exactly one model matched.
    Match(&'a ModelDef),
    /// Several models matched equally well. The caller decides — by asking, or
    /// by reporting the candidates — rather than the catalog guessing.
    Ambiguous(Vec<&'a ModelDef>),
    NotFound,
}

impl<'a> Resolution<'a> {
    /// The single match, or `None` when the query was ambiguous or unknown.
    pub fn model(&self) -> Option<&'a ModelDef> {
        match self {
            Resolution::Match(model) => Some(model),
            _ => None,
        }
    }

    /// Every model the query matched: one for a match, several when ambiguous,
    /// none when unknown.
    pub fn candidates(&self) -> Vec<&'a ModelDef> {
        match self {
            Resolution::Match(model) => vec![*model],
            Resolution::Ambiguous(models) => models.clone(),
            Resolution::NotFound => Vec::new(),
        }
    }
}

impl Catalog {
    /// Resolve a user-typed string to a model.
    ///
    /// Tiers are tried in order and the first tier that matches anything
    /// decides the outcome — one hit resolves, several are reported as
    /// ambiguous rather than picked between:
    ///
    /// 1. provider-qualified id — `anthropic/claude-opus-5`, or
    ///    `openrouter/anthropic/claude-opus-5` for a nested id
    /// 2. exact model id on any provider — `claude-opus-5`
    /// 3. alias — `opus`
    /// 4. prefix of a provider-qualified id
    /// 5. prefix of a model id
    /// 6. substring of a model id or display name — `sonnet`
    ///
    /// Matching ignores case throughout.
    pub fn resolve<'a>(&'a self, query: &str) -> Resolution<'a> {
        let query = query.trim();
        if query.is_empty() {
            return Resolution::NotFound;
        }

        // The qualified form is the one spelling that is unique by
        // construction, so it is tried first and on its own: without that, a
        // model whose id contains a slash would make every provider-qualified
        // query ambiguous with the nested id it looks like.
        if let Some(model) = self.match_qualified(query) {
            return Resolution::Match(model);
        }

        let tiers = [
            Self::match_exact_id,
            Self::match_alias,
            Self::match_qualified_prefix,
            Self::match_id_prefix,
            Self::match_substring,
        ];

        for tier in tiers {
            match tier(self, query).as_slice() {
                [] => continue,
                [single] => return Resolution::Match(single),
                several => return Resolution::Ambiguous(several.to_vec()),
            }
        }

        // Nothing matched as written. A name typed from memory is usually close rather
        // than exact — letters in the right order with the rest left out — so the last
        // reading is the forgiving one, ranked best first.
        match self.match_fuzzy(query).as_slice() {
            [] => Resolution::NotFound,
            [single] => Resolution::Match(single),
            several => Resolution::Ambiguous(several.to_vec()),
        }
    }

    /// Models whose id or name contains the query's characters in order, best first.
    fn match_fuzzy(&self, query: &str) -> Vec<&ModelDef> {
        let mut scored: Vec<(f64, &ModelDef)> = self
            .models()
            .iter()
            .filter_map(|model| {
                // Judged on whichever spelling reads better, since a user may type either
                // the id or the name.
                let qualified = model.qualified_id();
                [
                    crate::fuzzy::match_score(query, &model.id),
                    crate::fuzzy::match_score(query, &model.name),
                    crate::fuzzy::match_score(query, &qualified),
                ]
                .into_iter()
                .flatten()
                .map(|found| found.score)
                .reduce(f64::min)
                .map(|score| (score, model))
            })
            .collect();

        // Lower is better, and a tie is broken by the order the catalog presents.
        scored.sort_by(|left, right| {
            left.0
                .partial_cmp(&right.0)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.into_iter().map(|(_, model)| model).collect()
    }

    fn match_qualified(&self, query: &str) -> Option<&ModelDef> {
        let (provider, id) = query.split_once('/')?;
        self.models()
            .iter()
            .find(|m| eq(&m.provider, provider) && eq(&m.id, id))
    }

    fn match_exact_id(&self, query: &str) -> Vec<&ModelDef> {
        self.models().iter().filter(|m| eq(&m.id, query)).collect()
    }

    fn match_alias(&self, query: &str) -> Vec<&ModelDef> {
        self.models()
            .iter()
            .filter(|m| m.aliases.iter().any(|alias| eq(alias, query)))
            .collect()
    }

    fn match_qualified_prefix(&self, query: &str) -> Vec<&ModelDef> {
        self.models()
            .iter()
            .filter(|m| starts_with(&m.qualified_id(), query))
            .collect()
    }

    fn match_id_prefix(&self, query: &str) -> Vec<&ModelDef> {
        self.models()
            .iter()
            .filter(|m| starts_with(&m.id, query))
            .collect()
    }

    fn match_substring(&self, query: &str) -> Vec<&ModelDef> {
        self.models()
            .iter()
            .filter(|m| contains(&m.id, query) || contains(&m.name, query))
            .collect()
    }
}

fn eq(haystack: &str, needle: &str) -> bool {
    haystack.eq_ignore_ascii_case(needle)
}

fn starts_with(haystack: &str, prefix: &str) -> bool {
    haystack.is_char_boundary(prefix.len()) && eq(&haystack[..prefix.len()], prefix)
}

fn contains(haystack: &str, needle: &str) -> bool {
    haystack
        .to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed catalog, so these tests describe how a query is matched rather than which
    /// models a service happened to offer when the bundled catalog was last built.
    fn catalog() -> Catalog {
        Catalog::from_json(include_str!("../testdata/resolve-catalog.json")).unwrap()
    }

    #[test]
    fn resolves_a_provider_qualified_id() {
        let catalog = catalog();
        let model = catalog.resolve("anthropic/claude-opus-5").model().unwrap();
        assert_eq!(model.provider, "anthropic");
        assert_eq!(model.id, "claude-opus-5");
    }

    #[test]
    fn resolves_a_nested_openrouter_id_when_provider_qualified() {
        let catalog = catalog();
        let model = catalog
            .resolve("openrouter/anthropic/claude-sonnet-5")
            .model()
            .unwrap();
        assert_eq!(model.provider, "openrouter");
        assert_eq!(model.id, "anthropic/claude-sonnet-5");
    }

    #[test]
    fn a_qualified_id_wins_over_a_nested_id_that_looks_the_same() {
        // `anthropic/claude-sonnet-5` is both a qualified id on Anthropic and a
        // bare model id on OpenRouter. The qualified reading wins, which keeps
        // every model reachable by some unambiguous spelling.
        let catalog = catalog();
        let model = catalog
            .resolve("anthropic/claude-sonnet-5")
            .model()
            .unwrap();
        assert_eq!(model.provider, "anthropic");
    }

    #[test]
    fn resolves_an_exact_id_served_by_one_provider() {
        let catalog = catalog();
        let model = catalog
            .resolve("claude-haiku-4-5-20251001")
            .model()
            .unwrap();
        assert_eq!(model.provider, "anthropic");
    }

    #[test]
    fn resolves_an_alias() {
        let catalog = catalog();
        let model = catalog.resolve("opus").model().unwrap();
        assert_eq!(model.qualified_id(), "openrouter/anthropic/claude-opus-5");
    }

    /// The bundled catalog is assembled from what services publish, so it is checked for
    /// shape rather than for any particular model.
    #[test]
    fn the_bundled_catalog_resolves_what_it_lists() {
        let bundled = Catalog::bundled();
        let first = bundled.models().first().expect("the catalog lists models").clone();

        let resolved = bundled.resolve(&first.qualified_id()).model().cloned();
        assert_eq!(resolved.map(|model| model.qualified_id()), Some(first.qualified_id()));
    }

    #[test]
    fn resolves_a_unique_prefix() {
        let catalog = catalog();
        let model = catalog.resolve("gemini-2.5-p").model().unwrap();
        assert_eq!(model.id, "gemini-2.5-pro");
    }

    #[test]
    fn resolves_a_unique_qualified_prefix() {
        let catalog = catalog();
        let model = catalog.resolve("google/gemini-3-p").model().unwrap();
        assert_eq!(model.qualified_id(), "google/gemini-3-pro");
    }

    #[test]
    fn matching_ignores_case() {
        let catalog = catalog();
        assert_eq!(
            catalog
                .resolve("ANTHROPIC/Claude-Opus-5")
                .model()
                .unwrap()
                .id,
            "claude-opus-5"
        );
        assert_eq!(
            catalog.resolve("OPUS").model().unwrap().qualified_id(),
            "openrouter/anthropic/claude-opus-5"
        );
    }

    #[test]
    fn surrounding_whitespace_is_ignored() {
        let catalog = catalog();
        assert!(catalog.resolve("  opus  ").model().is_some());
    }

    #[test]
    fn an_id_on_several_providers_is_ambiguous() {
        let catalog = catalog();
        let Resolution::Ambiguous(candidates) = catalog.resolve("claude-opus-5") else {
            panic!("expected `claude-opus-5` to be ambiguous across providers");
        };

        let providers: Vec<&str> = candidates.iter().map(|m| m.provider.as_str()).collect();
        assert!(providers.contains(&"anthropic"));
        assert!(providers.contains(&"github-copilot"));
        assert!(candidates.len() >= 2);
    }

    #[test]
    fn an_ambiguous_prefix_returns_every_candidate() {
        let catalog = catalog();
        let Resolution::Ambiguous(candidates) = catalog.resolve("gemini-3") else {
            panic!("expected `gemini-3` to match several models");
        };
        assert!(candidates.len() > 1);
        assert!(candidates.iter().all(|m| m.id.starts_with("gemini-3")));
    }

    #[test]
    fn a_substring_of_an_id_falls_back_to_matching() {
        let catalog = catalog();
        let Resolution::Ambiguous(candidates) = catalog.resolve("flash-lite") else {
            panic!("expected `flash-lite` to match several models");
        };
        assert!(candidates.len() > 1);
        assert!(candidates.iter().all(|m| m.id.contains("flash-lite")));
    }

    #[test]
    fn a_substring_of_a_display_name_falls_back_to_matching() {
        let catalog = catalog();
        // Matches "DeepSeek V4 Pro" by name; the id spells it with a hyphen.
        let model = catalog.resolve("deepseek v4").model().unwrap();
        assert_eq!(model.id, "deepseek/deepseek-v4-pro");
    }

    #[test]
    fn an_unknown_query_is_not_found() {
        let catalog = catalog();
        assert_eq!(catalog.resolve("llama-9000"), Resolution::NotFound);
        assert_eq!(catalog.resolve(""), Resolution::NotFound);
        assert_eq!(catalog.resolve("   "), Resolution::NotFound);
    }

    #[test]
    fn a_qualified_query_for_an_unknown_provider_is_not_found() {
        let catalog = catalog();
        assert_eq!(catalog.resolve("nope/claude-opus-5"), Resolution::NotFound);
    }

    #[test]
    fn candidates_are_exposed_for_every_outcome() {
        let catalog = catalog();
        assert_eq!(
            catalog
                .resolve("anthropic/claude-opus-5")
                .candidates()
                .len(),
            1
        );
        assert!(catalog.resolve("claude-opus-5").candidates().len() >= 2);
        assert!(catalog.resolve("llama-9000").candidates().is_empty());
    }

    #[test]
    fn narrowing_to_one_provider_makes_a_shared_id_unambiguous() {
        let mut catalog = catalog();
        catalog.retain_providers(&["anthropic"]);
        let model = catalog.resolve("claude-opus-5").model().unwrap();
        assert_eq!(model.provider, "anthropic");
    }

    #[test]
    fn a_prefix_that_is_also_an_exact_id_prefers_the_exact_match() {
        let mut catalog = Catalog::from_json(
            r#"{"providers": {"local": {
                "base_url": "http://localhost:1234/v1",
                "api": "openai-completions",
                "models": [{"id": "coder"}, {"id": "coder-plus"}]
            }}}"#,
        )
        .unwrap();
        catalog.retain_providers(&["local"]);

        let model = catalog.resolve("coder").model().unwrap();
        assert_eq!(model.id, "coder");
    }
}

#[cfg(test)]
mod forgiving {
    use super::*;

    fn catalog() -> Catalog {
        Catalog::bundled()
    }

    /// A name typed from memory finds the model, with the letters in order and the rest
    /// left out.
    #[test]
    fn letters_in_order_are_enough() {
        let held = catalog();
        let found = held.resolve("opus5");
        assert!(
            found.model().is_some() || matches!(found, Resolution::Ambiguous(_)),
            "opus5 finds something",
        );

        let held = catalog();
        let best = match held.resolve("clopus5") {
            Resolution::Match(model) => model.id.clone(),
            Resolution::Ambiguous(candidates) => candidates[0].id.clone(),
            Resolution::NotFound => panic!("clopus5 should find claude-opus-5"),
        };
        assert!(best.contains("opus"), "got {best}");
    }

    /// The exact spellings still win: a forgiving reading is the last resort, never the
    /// first, so an id typed in full is never beaten by something that merely resembles it.
    #[test]
    fn an_exact_name_still_wins() {
        let catalog = catalog();
        let model = catalog
            .resolve("anthropic/claude-opus-5")
            .model()
            .expect("the qualified form resolves");
        assert_eq!(model.id, "claude-opus-5");
        assert_eq!(model.provider, "anthropic");
    }

    /// Nonsense still finds nothing rather than the whole catalog.
    #[test]
    fn nonsense_finds_nothing() {
        let held = catalog();
        assert!(matches!(held.resolve("zzqqxxjjvv"), Resolution::NotFound));
    }
}
