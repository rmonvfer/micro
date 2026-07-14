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

        Resolution::NotFound
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

    fn catalog() -> Catalog {
        Catalog::bundled()
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

    #[test]
    fn resolves_a_unique_prefix() {
        let catalog = catalog();
        let model = catalog.resolve("gemini-2.5-p").model().unwrap();
        assert_eq!(model.id, "gemini-2.5-pro");
    }

    #[test]
    fn resolves_a_unique_qualified_prefix() {
        let catalog = catalog();
        let model = catalog.resolve("google/gemini-2.5-f").model().unwrap();
        assert_eq!(model.qualified_id(), "google/gemini-2.5-flash");
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
