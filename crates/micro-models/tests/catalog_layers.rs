//! End-to-end checks against the public API: the three layers a catalog is assembled from, and what
//! a caller does with the result.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use micro_models::{Catalog, Resolution, TokenUsage, WireApi, COPILOT_BASE_URL};

const COPILOT_LISTING: &str = include_str!("../testdata/copilot-models.json");
const OPENROUTER_LISTING: &str = include_str!("../testdata/openrouter-models.json");

/// A scratch directory that removes itself, so tests never touch the caller's real configuration
/// directory.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> TempDir {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = format!(
            "micro-models-layers-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(unique);
        fs::create_dir_all(&path).expect("create scratch directory");
        TempDir(path)
    }

    fn write(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, contents).expect("write scratch file");
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn the_bundled_catalog_is_usable_with_no_setup() {
    let catalog = Catalog::bundled();

    assert!(!catalog.is_empty());
    let model = catalog.resolve("anthropic/claude-opus-5").model().unwrap();
    assert_eq!(model.api, WireApi::AnthropicMessages);
    assert!(model.price(TokenUsage::new(1_000, 1_000)).total() > 0.0);
}

#[test]
fn the_three_layers_stack_in_order() {
    let dir = TempDir::new();
    let user_catalog = dir.write(
        "models.json",
        r#"{"providers": {
            "openrouter": {"models": [
                {"id": "anthropic/claude-opus-5", "aliases": ["opus", "big"]}
            ]},
            "ollama": {
                "base_url": "http://localhost:11434/v1",
                "api": "openai-completions",
                "models": [{"id": "qwen3-coder:30b", "name": "Qwen3 Coder 30B", "aliases": ["local"]}]
            }
        }}"#,
    );

    let mut catalog = Catalog::load_from(&user_catalog).unwrap();
    assert_eq!(
        catalog.resolve("big").model().unwrap().qualified_id(),
        "openrouter/anthropic/claude-opus-5"
    );
    assert_eq!(
        catalog.resolve("local").model().unwrap().name,
        "Qwen3 Coder 30B"
    );

    catalog.merge_listing(micro_models::parse_openrouter(OPENROUTER_LISTING).unwrap());

    let opus = catalog
        .get("openrouter", "anthropic/claude-opus-5")
        .unwrap();
    assert_eq!(opus.cost.input, 5.0);
    assert_eq!(opus.aliases, vec!["opus".to_string(), "big".to_string()]);
    assert!(catalog.get("ollama", "qwen3-coder:30b").is_some());
}

#[test]
fn a_live_listing_adds_models_the_build_never_knew_about() {
    let mut catalog = Catalog::bundled();
    let before = catalog.len();

    catalog.merge_listing(
        micro_models::parse_openrouter(
            r#"{"data": [{
                "id": "vendor/model-from-the-future",
                "name": "Model From The Future",
                "context_length": 2000000,
                "supported_parameters": ["tools", "reasoning"],
                "pricing": {"prompt": "0.000001", "completion": "0.000004"},
                "top_provider": {"context_length": 2000000, "max_completion_tokens": 200000}
            }]}"#,
        )
        .unwrap(),
    );

    assert_eq!(catalog.len(), before + 1);
    let added = catalog
        .resolve("vendor/model-from-the-future")
        .model()
        .unwrap();
    assert_eq!(added.provider, "openrouter");
    assert_eq!(added.context_window, 2_000_000);
    assert!(added.reasoning);
}

#[test]
fn a_copilot_listing_narrows_the_catalog_to_the_entitled_models() {
    let mut catalog = Catalog::bundled();
    let listed = micro_models::parse_copilot(COPILOT_LISTING, COPILOT_BASE_URL).unwrap();
    catalog.merge_listing(listed.clone());
    catalog.retain_providers(&["github-copilot"]);

    let entitled: Vec<&str> = listed.iter().map(|m| m.id.as_str()).collect();
    let usable: Vec<&str> = catalog
        .models()
        .iter()
        .filter(|m| entitled.contains(&m.id.as_str()))
        .map(|m| m.id.as_str())
        .collect();

    assert_eq!(usable, vec!["claude-opus-5", "gpt-5.6-terra", "kimi-k3"]);
}

#[test]
fn an_ambiguous_query_reports_its_candidates() {
    let catalog = Catalog::bundled();

    let Resolution::Ambiguous(candidates) = catalog.resolve("claude-opus-5") else {
        panic!("`claude-opus-5` is served by more than one provider");
    };

    let qualified: Vec<String> = candidates.iter().map(|m| m.qualified_id()).collect();
    assert!(
        qualified.contains(&"anthropic/claude-opus-5".to_string()),
        "{qualified:?}"
    );
    assert!(
        qualified.contains(&"github-copilot/claude-opus-5".to_string()),
        "{qualified:?}"
    );
}

#[test]
fn a_resolved_model_converts_to_a_runtime_handle() {
    let catalog = Catalog::bundled();
    let model = catalog
        .resolve("anthropic/claude-sonnet-5")
        .model()
        .unwrap();

    let runtime = model.to_runtime(micro_types::ThinkingLevel::Medium);

    assert_eq!(runtime.id, "claude-sonnet-5");
    assert_eq!(runtime.provider, "anthropic");
    assert_eq!(runtime.base_url, "https://api.anthropic.com");
    assert_eq!(runtime.max_tokens, model.max_output_tokens);
    assert_eq!(runtime.thinking, micro_types::ThinkingLevel::Medium);
}

#[test]
fn a_session_total_is_the_sum_of_its_requests() {
    let catalog = Catalog::bundled();
    let model = catalog.resolve("anthropic/claude-opus-5").model().unwrap();

    let turns = [
        TokenUsage::new(2_000, 500),
        TokenUsage::new(1_000, 400).with_cache(2_000, 2_000),
        TokenUsage::new(800, 1_200).with_cache(3_000, 0),
    ];

    let total: f64 = turns.iter().map(|t| model.price(*t).total()).sum();
    let combined = turns
        .iter()
        .fold(TokenUsage::default(), |acc, turn| acc + *turn);

    assert!((model.price(combined).total() - total).abs() < 1e-9);
    assert_eq!(combined.total(), 12_900);
}
