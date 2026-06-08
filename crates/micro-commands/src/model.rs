//! `/model` and `/provider`: choosing what answers.

use crate::CommandContext;
use crate::CommandOutcome;
use crate::Picker;
use crate::PickerItem;
use crate::PickerLayout;
use micro_models::ModelDef;
use micro_models::Resolution;

fn step_direction(query: &str) -> Option<bool> {
    match query.trim().to_ascii_lowercase().as_str() {
        "next" => Some(true),
        "previous" | "prev" => Some(false),
        _ => None,
    }
}

/// The model one place along from the one in use, wrapping at either end.
fn offered(context: &CommandContext<'_>) -> Vec<ModelDef> {
    context
        .catalog
        .models()
        .iter()
        .filter(|model| context.auth.status_of(&model.provider).is_authenticated())
        .cloned()
        .collect()
}

/// What is said about the ones left out.
const ONLY_CONFIGURED: &str =
    "Only showing models from configured providers. Use /login to add providers.";

fn neighbour(context: &CommandContext<'_>, forward: bool) -> Option<ModelDef> {
    let models = offered(context);
    if models.is_empty() {
        return None;
    }
    let current = context.model.and_then(|model| {
        models
            .iter()
            .position(|candidate| candidate.qualified_id() == model.qualified_id())
    });
    let index = match (current, forward) {
        (None, _) => 0,
        (Some(index), true) => (index + 1) % models.len(),
        (Some(index), false) => (index + models.len() - 1) % models.len(),
    };
    models.get(index).cloned()
}

pub(crate) fn model(argument: Option<&str>, context: &CommandContext<'_>) -> CommandOutcome {
    let Some(query) = argument else {
        let offered = offered(context);
        if offered.is_empty() {
            return CommandOutcome::error(
                "no provider is signed in - run `/login` to add one".to_string(),
            );
        }
        let all: Vec<_> = ordered(&offered, context.model)
            .iter()
            .map(|model| item(model, context.model))
            .collect();

        let shortlist: Vec<_> = ordered(
            &on_shortlist(&offered, context.scoped_models),
            context.model,
        )
        .iter()
        .map(|model| item(model, context.model))
        .collect();

        return CommandOutcome::Choose(
            Picker::new("Select a model", all)
                .refreshing()
                .scoping(shortlist)
                .saying(ONLY_CONFIGURED)
                .searchable()
                .laid_out(PickerLayout::Badges),
        );
    };

    if let Some(forward) = step_direction(query) {
        return match neighbour(context, forward) {
            Some(model) => CommandOutcome::SetModel {
                model: Box::new(model),
            },
            None => CommandOutcome::error("the catalog holds no other model"),
        };
    }

    match context.catalog.resolve(query) {
        Resolution::Match(model) => CommandOutcome::SetModel {
            model: Box::new(model.clone()),
        },

        Resolution::Ambiguous(candidates) => CommandOutcome::Choose(
            Picker::new(
                format!("{} models match \"{query}\"", candidates.len()),
                candidates
                    .iter()
                    .map(|model| item(model, context.model))
                    .collect(),
            )
            .searchable()
            .laid_out(PickerLayout::Badges),
        ),
        Resolution::NotFound => CommandOutcome::error(format!(
            "no model matches \"{query}\" - /model on its own lists them"
        )),
    }
}

pub(crate) fn provider(argument: Option<&str>, context: &CommandContext<'_>) -> CommandOutcome {
    let Some(name) = argument else {
        return CommandOutcome::Choose(
            Picker::new(
                "Select a provider",
                micro_provider::known_providers()
                    .iter()
                    .map(|info| {
                        PickerItem::new(
                            info.id,
                            format!("{} · {}", info.label, credential_note(info.id, context)),
                            format!("/provider {}", info.id),
                        )
                        .current(info.id == context.provider)
                    })
                    .collect(),
            )
            .searchable(),
        );
    };

    match micro_provider::provider_info(name) {
        Some(info) => CommandOutcome::SetProvider { provider: info.id },
        None => CommandOutcome::error(format!("unknown provider \"{name}\" - try one of: {}", {
            let ids: Vec<&str> = micro_provider::known_providers()
                .iter()
                .map(|info| info.id)
                .collect();
            ids.join(", ")
        })),
    }
}

fn item(model: &ModelDef, current: Option<&ModelDef>) -> PickerItem {
    let qualified = model.qualified_id();
    let is_current = current.is_some_and(|model| model.qualified_id() == qualified);

    let item = PickerItem::new(
        &model.id,
        format!("[{}]", model.provider),
        format!("/model {qualified}"),
    )
    .current(is_current)
    .found_by(search_text(model));

    match model.name.is_empty() {
        true => item,
        false => item.noting(format!("Model Name: {}", model.name)),
    }
}

/// What a query for a model is matched against.
fn search_text(model: &ModelDef) -> String {
    let provider = &model.provider;
    let id = &model.id;
    let name = match model.name.is_empty() {
        true => String::new(),
        false => format!(" {}", model.name),
    };
    format!("{provider} {provider}/{id} {provider} {id}{name}")
}

/// The models a workspace named, matched by prefix so a pattern can name a provider, a family, or
/// one exact model.
fn on_shortlist(models: &[ModelDef], patterns: &[String]) -> Vec<ModelDef> {
    if patterns.is_empty() {
        return Vec::new();
    }
    models
        .iter()
        .filter(|model| {
            patterns.iter().any(|pattern| {
                model.qualified_id().starts_with(pattern.as_str())
                    || model.id.starts_with(pattern.as_str())
            })
        })
        .cloned()
        .collect()
}

/// The models a picker offers, in the order they should be read: whatever is running first, then
/// grouped by who serves it.
fn ordered(models: &[ModelDef], current: Option<&ModelDef>) -> Vec<ModelDef> {
    let mut ordered = models.to_vec();
    let current = current.map(ModelDef::qualified_id);
    ordered.sort_by(|left, right| {
        let is_current = |model: &ModelDef| current.as_deref() == Some(&model.qualified_id());
        match (is_current(left), is_current(right)) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => left.provider.cmp(&right.provider),
        }
    });
    ordered
}

/// How a provider's credential stands, in a few words for a picker line.
fn credential_note(provider: &str, context: &CommandContext<'_>) -> String {
    let status = context.auth.status_of(provider);
    match status.source {
        micro_auth::CredentialSource::Stored => "signed in".to_string(),
        micro_auth::CredentialSource::Environment { variable } => format!("via {variable}"),
        micro_auth::CredentialSource::Missing => "not signed in".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch;
    use crate::testing::*;

    /// The list offers what can answer, not everything micro can speak to.
    #[tokio::test]
    async fn model_with_no_argument_offers_what_is_signed_in() {
        let harness = Harness::new("model-picker");
        harness
            .auth
            .store_api_key("anthropic", "sk-ant-test")
            .unwrap();

        let outcome = dispatch("/model", &harness.context()).await.unwrap();
        let picker = picker(&outcome);

        assert_eq!(picker.title, "Select a model");
        assert_eq!(
            picker.hint.as_deref(),
            Some(ONLY_CONFIGURED),
            "the list should say what it leaves out"
        );
        assert!(picker
            .items
            .iter()
            .all(|item| item.command.starts_with("/model ")));
        assert!(
            picker.items.iter().any(|item| item.detail == "[anthropic]"),
            "the signed-in provider is offered"
        );
        for item in &picker.items {
            let provider = item.detail.trim_matches(['[', ']']);
            assert!(
                harness.auth.status_of(provider).is_authenticated(),
                "{} is offered without a credential",
                item.label
            );
        }
    }

    #[tokio::test]
    async fn a_picked_item_dispatches_back_to_the_model_it_names() {
        let harness = Harness::new("model-roundtrip");
        harness
            .auth
            .store_api_key("anthropic", "sk-ant-test")
            .unwrap();
        let outcome = dispatch("/model", &harness.context()).await.unwrap();
        let line = picker(&outcome).command_at(0).unwrap().to_string();

        let outcome = dispatch(&line, &harness.context()).await.unwrap();
        let CommandOutcome::SetModel { model } = outcome else {
            panic!("expected a model to be set, got {outcome:?}");
        };
        assert_eq!(format!("/model {}", model.qualified_id()), line);
    }

    #[tokio::test]
    async fn an_unambiguous_query_switches_model() {
        let harness = Harness::new("model-resolve");
        let outcome = dispatch("/model anthropic/claude-opus-5", &harness.context())
            .await
            .unwrap();

        let CommandOutcome::SetModel { model } = outcome else {
            panic!("expected a model to be set");
        };
        assert_eq!(model.provider, "anthropic");
        assert_eq!(model.id, "claude-opus-5");
    }

    #[tokio::test]
    async fn an_ambiguous_query_reports_its_candidates_instead_of_guessing() {
        let harness = Harness::new("model-ambiguous");
        let outcome = dispatch("/model claude-opus-5", &harness.context())
            .await
            .unwrap();
        let picker = picker(&outcome);

        assert!(picker.title.contains("claude-opus-5"), "{}", picker.title);
        assert!(picker.items.len() >= 2);
        let providers: Vec<&str> = picker
            .items
            .iter()
            .map(|item| item.detail.trim_matches(['[', ']']))
            .collect();
        assert!(providers.contains(&"anthropic"));
    }

    #[tokio::test]
    async fn an_unknown_model_is_reported_rather_than_approximated() {
        let harness = Harness::new("model-unknown");
        let outcome = dispatch("/model llama-9000", &harness.context())
            .await
            .unwrap();

        assert!(outcome.is_error());
        assert!(text(&outcome).contains("llama-9000"));
    }

    #[tokio::test]
    async fn the_model_in_use_is_marked_in_the_picker() {
        let harness = Harness::new("model-current");
        harness
            .auth
            .store_api_key("anthropic", "sk-ant-test")
            .unwrap();
        let current = harness
            .catalog
            .resolve("anthropic/claude-opus-5")
            .model()
            .unwrap();
        let context = CommandContext {
            model: Some(current),
            ..harness.context()
        };

        let outcome = dispatch("/model", &context).await.unwrap();
        let marked: Vec<&str> = picker(&outcome)
            .items
            .iter()
            .filter(|item| item.current)
            .map(|item| item.label.as_str())
            .collect();
        assert_eq!(marked, vec!["claude-opus-5"]);
    }

    #[tokio::test]
    async fn provider_with_no_argument_offers_every_provider() {
        let harness = Harness::new("provider-picker");
        let outcome = dispatch("/provider", &harness.context()).await.unwrap();
        let picker = picker(&outcome);

        let labels: Vec<&str> = picker
            .items
            .iter()
            .map(|item| item.label.as_str())
            .collect();
        let known: Vec<&str> = micro_provider::known_providers()
            .iter()
            .map(|info| info.id)
            .collect();
        assert_eq!(labels, known);
        assert!(picker.items.iter().any(|item| item.current));
    }

    #[tokio::test]
    async fn a_provider_is_switched_by_any_name_it_answers_to() {
        let harness = Harness::new("provider-set");
        let outcome = dispatch("/provider copilot", &harness.context())
            .await
            .unwrap();

        assert!(matches!(
            outcome,
            CommandOutcome::SetProvider {
                provider: "github-copilot"
            }
        ));
    }

    #[tokio::test]
    async fn an_unknown_provider_lists_the_ones_that_exist() {
        let harness = Harness::new("provider-unknown");
        let outcome = dispatch("/provider nowhere", &harness.context())
            .await
            .unwrap();

        assert!(outcome.is_error());
        assert!(text(&outcome).contains("openrouter"), "{outcome:?}");
    }

    /// A model row includes its provider.
    #[test]
    fn a_model_row_names_the_model_and_who_serves_it() {
        let model = ModelDef {
            id: "gpt-5".into(),
            name: "GPT-5".into(),
            provider: "github-copilot".into(),
            api: micro_models::WireApi::OpenaiCompletions,
            base_url: "https://example.invalid".into(),
            context_window: 128_000,
            max_output_tokens: 16_000,
            reasoning: false,
            input: Vec::new(),
            headers: Default::default(),
            aliases: Vec::new(),
            cost: Default::default(),
            compat: Default::default(),
            thinking: Default::default(),
        };

        let row = item(&model, None);
        assert_eq!(row.label, "gpt-5");
        assert_eq!(row.detail, "[github-copilot]");
        assert!(!row.current);

        let row = item(&model, Some(&model));
        assert!(row.current, "the one running is marked");
    }

    /// Whatever is running is offered first, and the rest are grouped by who serves them.
    #[test]
    fn the_running_model_is_offered_first() {
        let make = |id: &str, provider: &str| ModelDef {
            id: id.into(),
            name: id.into(),
            provider: provider.into(),
            api: micro_models::WireApi::OpenaiCompletions,
            base_url: "https://example.invalid".into(),
            context_window: 1000,
            max_output_tokens: 100,
            reasoning: false,
            input: Vec::new(),
            headers: Default::default(),
            aliases: Vec::new(),
            cost: Default::default(),
            compat: Default::default(),
            thinking: Default::default(),
        };

        let models = vec![make("a", "zed"), make("b", "acme"), make("c", "middle")];
        let running = make("c", "middle");

        let sorted = ordered(&models, Some(&running));
        assert_eq!(sorted[0].id, "c", "what is running comes first");
        assert_eq!(sorted[1].provider, "acme", "then grouped by provider");
        assert_eq!(sorted[2].provider, "zed");
    }
}
