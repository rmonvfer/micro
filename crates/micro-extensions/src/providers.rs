//! A provider an extension declared, turned into something the catalog understands.
//!
//! ohm's `registerProvider` describes a provider the way a person would write it: camelCase
//! keys, a credential that may be a literal, an environment variable, or a command to run.
//! micro's catalog reads a different spelling, so the two are translated here rather than
//! at the point of use.
//!
//! Declaring a provider that already exists changes it: a base URL alone points an existing
//! provider somewhere else, which is how a proxy is put in front of one.

use serde_json::json;
use serde_json::Map;
use serde_json::Value;

/// The catalog fragment for one declared provider, and the credential it needs.
#[derive(Debug, Clone, PartialEq)]
pub struct Declared {
    pub name: String,
    /// What [`micro_models::Catalog::apply_overrides`] takes.
    pub catalog: Value,
    /// The key to use, once whatever names it has been resolved.
    pub api_key: Option<String>,
}

/// Read a declaration into the shapes micro uses.
pub fn declare(name: &str, config: &Value) -> Result<Declared, String> {
    let Some(config) = config.as_object() else {
        return Err(format!(
            "{name} was declared as something other than an object"
        ));
    };

    let mut provider = Map::new();
    if let Some(label) = config.get("name").and_then(Value::as_str) {
        provider.insert("name".into(), json!(label));
    }
    if let Some(base_url) = config.get("baseUrl").and_then(Value::as_str) {
        provider.insert("base_url".into(), json!(base_url));
    }
    if let Some(api) = config.get("api").and_then(Value::as_str) {
        provider.insert("api".into(), json!(api));
    }
    if let Some(headers) = config.get("headers").and_then(Value::as_object) {
        provider.insert("headers".into(), Value::Object(headers.clone()));
    }

    if let Some(models) = config.get("models").and_then(Value::as_array) {
        let described: Result<Vec<Value>, String> = models.iter().map(model).collect();
        provider.insert("models".into(), Value::Array(described?));
    }

    // A provider with nothing to say about itself changes nothing, and saying so is more
    // useful than merging an empty object into the catalog.
    if provider.is_empty() {
        return Err(format!("{name} was declared without anything to apply"));
    }

    let api_key = match config.get("apiKey").and_then(Value::as_str) {
        Some(written) => Some(resolve_key(written)?),
        None => None,
    };

    Ok(Declared {
        name: name.to_string(),
        catalog: json!({ "providers": { name: Value::Object(provider) } }),
        api_key,
    })
}

/// One model, in the spelling the catalog reads.
fn model(described: &Value) -> Result<Value, String> {
    let Some(described) = described.as_object() else {
        return Err("a model was declared as something other than an object".to_string());
    };
    let Some(id) = described.get("id").and_then(Value::as_str) else {
        return Err("a model was declared without an id".to_string());
    };

    let mut out = Map::new();
    out.insert("id".into(), json!(id));
    out.insert(
        "name".into(),
        json!(described.get("name").and_then(Value::as_str).unwrap_or(id)),
    );
    for (theirs, ours) in [
        ("contextWindow", "context_window"),
        ("maxTokens", "max_output_tokens"),
        ("reasoning", "reasoning"),
        ("input", "input"),
        ("api", "api"),
        ("aliases", "aliases"),
    ] {
        if let Some(value) = described.get(theirs) {
            out.insert(ours.into(), value.clone());
        }
    }

    if let Some(cost) = described.get("cost").and_then(Value::as_object) {
        let mut prices = Map::new();
        for (theirs, ours) in [
            ("input", "input"),
            ("output", "output"),
            ("cacheRead", "cache_read"),
            ("cacheWrite", "cache_write"),
        ] {
            if let Some(price) = cost.get(theirs) {
                prices.insert(ours.into(), price.clone());
            }
        }
        out.insert("cost".into(), Value::Object(prices));
    }

    Ok(Value::Object(out))
}

/// The credential, from whichever of ohm's three spellings was used.
///
/// `$NAME` and `${NAME}` read the environment. A leading `!` runs a command and takes what
/// it printed, which is how a key kept in a password manager is fetched. Anything else is
/// the key itself.
pub fn resolve_key(written: &str) -> Result<String, String> {
    let written = written.trim();

    if let Some(command) = written.strip_prefix('!') {
        return from_command(command.trim());
    }

    let name = written
        .strip_prefix("${")
        .and_then(|rest| rest.strip_suffix('}'))
        .or_else(|| written.strip_prefix('$'));

    match name {
        Some(name) => std::env::var(name)
            .map_err(|_| format!("{name} is not set"))
            .map(|value| value.trim().to_string()),
        None => Ok(written.to_string()),
    }
}

/// Run a command and take what it printed as the key.
///
/// The command is the user's own configuration rather than anything a model wrote, and it
/// is written as a command line, so it is run as one.
fn from_command(command: &str) -> Result<String, String> {
    if command.is_empty() {
        return Err("no command to read the key from".to_string());
    }
    let finished = std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|error| format!("cannot run `{command}`: {error}"))?;

    if !finished.status.success() {
        let said = String::from_utf8_lossy(&finished.stderr);
        return Err(format!("`{command}` failed: {}", said.trim()));
    }
    Ok(String::from_utf8_lossy(&finished.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_declared_provider_becomes_a_catalog_the_models_crate_reads() {
        let declared = declare(
            "my-proxy",
            &json!({
                "name": "My Proxy",
                "baseUrl": "https://proxy.example.com",
                "api": "anthropic-messages",
                "headers": { "X-Team": "platform" },
                "models": [{
                    "id": "claude-sonnet-5",
                    "name": "Claude Sonnet 5 (proxy)",
                    "reasoning": true,
                    "input": ["text", "image"],
                    "contextWindow": 200000,
                    "maxTokens": 16384,
                    "cost": { "input": 3.0, "output": 15.0, "cacheRead": 0.3, "cacheWrite": 3.75 },
                }],
            }),
        )
        .unwrap();

        let provider = &declared.catalog["providers"]["my-proxy"];
        assert_eq!(provider["name"], "My Proxy");
        assert_eq!(provider["base_url"], "https://proxy.example.com");
        assert_eq!(provider["api"], "anthropic-messages");
        assert_eq!(provider["headers"]["X-Team"], "platform");

        let model = &provider["models"][0];
        assert_eq!(model["id"], "claude-sonnet-5");
        assert_eq!(model["name"], "Claude Sonnet 5 (proxy)");
        assert_eq!(model["context_window"], 200000);
        assert_eq!(model["max_output_tokens"], 16384);
        assert_eq!(model["cost"]["cache_read"], 0.3);
        assert_eq!(model["cost"]["cache_write"], 3.75);
    }

    /// The catalog it produces is one the catalog actually accepts.
    #[test]
    fn the_fragment_merges_into_a_real_catalog() {
        let declared = declare(
            "my-proxy",
            &json!({
                "baseUrl": "https://proxy.example.com",
                "api": "openai-completions",
                "models": [{ "id": "fast", "contextWindow": 128000, "maxTokens": 8192 }],
            }),
        )
        .unwrap();

        let mut catalog = micro_models::Catalog::bundled();
        catalog
            .apply_overrides(&declared.catalog.to_string())
            .expect("the catalog takes it");

        let model = catalog.get("my-proxy", "fast").expect("the model is there");
        assert_eq!(model.base_url, "https://proxy.example.com");
        assert_eq!(model.context_window, 128000);
        // A model that says nothing about its name is named after itself.
        assert_eq!(model.name, "fast");
    }

    /// Naming a provider that already exists changes it rather than adding a second one.
    #[test]
    fn a_base_url_alone_points_an_existing_provider_somewhere_else() {
        let declared = declare(
            "anthropic",
            &json!({ "baseUrl": "https://proxy.example.com" }),
        )
        .unwrap();

        let mut catalog = micro_models::Catalog::bundled();
        let before = catalog
            .models()
            .iter()
            .filter(|model| model.provider == "anthropic")
            .count();
        catalog
            .apply_overrides(&declared.catalog.to_string())
            .expect("the catalog takes it");

        let after: Vec<_> = catalog
            .models()
            .iter()
            .filter(|model| model.provider == "anthropic")
            .collect();
        assert_eq!(after.len(), before, "no models were added or lost");
        assert!(
            after
                .iter()
                .all(|model| model.base_url == "https://proxy.example.com"),
            "every one of them now goes through the proxy"
        );
    }

    #[test]
    fn a_declaration_with_nothing_in_it_is_refused() {
        assert!(declare("empty", &json!({})).is_err());
        assert!(declare("wrong", &json!("a string")).is_err());
        assert!(declare("no-id", &json!({ "models": [{ "name": "x" }] })).is_err());
    }

    #[test]
    fn a_key_can_be_written_out_or_named() {
        assert_eq!(resolve_key("sk-literal").unwrap(), "sk-literal");

        std::env::set_var("MICRO_TEST_PROVIDER_KEY", "sk-from-env");
        assert_eq!(
            resolve_key("$MICRO_TEST_PROVIDER_KEY").unwrap(),
            "sk-from-env"
        );
        assert_eq!(
            resolve_key("${MICRO_TEST_PROVIDER_KEY}").unwrap(),
            "sk-from-env"
        );
        std::env::remove_var("MICRO_TEST_PROVIDER_KEY");

        let missing = resolve_key("$MICRO_TEST_PROVIDER_KEY").expect_err("it is not set");
        assert!(missing.contains("is not set"), "{missing}");
    }

    #[test]
    fn a_key_can_come_from_a_command() {
        assert_eq!(
            resolve_key("!echo sk-from-a-command").unwrap(),
            "sk-from-a-command"
        );

        let failed = resolve_key("!exit 3").expect_err("it failed");
        assert!(failed.contains("failed"), "{failed}");
        assert!(resolve_key("!").is_err());
    }
}
