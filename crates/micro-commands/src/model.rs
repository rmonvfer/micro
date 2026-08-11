//! `/model` and `/provider`: choosing what answers.

use crate::CommandContext;
use crate::CommandOutcome;
use crate::Picker;
use crate::PickerItem;
use micro_models::ModelDef;
use micro_models::Resolution;

/// Whether a query is a step through the catalog rather than a model to look up.
fn step_direction(query: &str) -> Option<bool> {
    match query.trim().to_ascii_lowercase().as_str() {
        "next" => Some(true),
        "previous" | "prev" => Some(false),
        _ => None,
    }
}

/// The model one place along from the one in use, wrapping at either end.
///
/// With nothing in use yet the first model is what a step lands on, so the keys do
/// something sensible before a model has been chosen.
/// The models on offer: those served by something the user is signed in to.
///
/// A catalog lists every service micro can speak to, which is far more than any one person
/// has an account with. Offering all of them would bury the handful that can actually
/// answer, so the rest are left out until there is a credential for them.
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
const ONLY_CONFIGURED: &str = "Only showing models from configured providers. Use /login to add providers.";

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
        return CommandOutcome::Choose(
            Picker::new(
                "Select a model",
                offered
                    .iter()
                    .map(|model| item(model, context.model))
                    .collect(),
            )
            .saying(ONLY_CONFIGURED),
        );
    };

    // `next` and `previous` step through the catalog from wherever it currently is, which
    // is what the cycle keys send rather than a model name.
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
        // Several models match equally well, so the choice is handed back rather than
        // guessed at.
        Resolution::Ambiguous(candidates) => CommandOutcome::Choose(Picker::new(
            format!("{} models match \"{query}\"", candidates.len()),
            candidates
                .iter()
                .map(|model| item(model, context.model))
                .collect(),
        )),
        Resolution::NotFound => CommandOutcome::error(format!(
            "no model matches \"{query}\" - /model on its own lists them"
        )),
    }
}

pub(crate) fn provider(argument: Option<&str>, context: &CommandContext<'_>) -> CommandOutcome {
    let Some(name) = argument else {
        return CommandOutcome::Choose(Picker::new(
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
        ));
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

    PickerItem::new(
        &qualified,
        format!(
            "{} · {} context · {}",
            model.name,
            tokens(model.context_window),
            price(model)
        ),
        format!("/model {qualified}"),
    )
    .current(is_current)
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

/// A round token count, the way a person says it.
fn tokens(count: u32) -> String {
    match count {
        0 => "unknown".to_string(),
        count if count >= 1_000_000 => format!("{}M", count / 1_000_000),
        count if count >= 1_000 => format!("{}k", count / 1_000),
        count => count.to_string(),
    }
}

fn price(model: &ModelDef) -> String {
    if model.cost.is_free() {
        return "included".to_string();
    }
    format!(
        "${:.2}/${:.2} per Mtok",
        model.cost.input, model.cost.output
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch;
    use crate::testing::*;

    /// The list offers what can answer, not everything micro can speak to. Which
    /// providers those are depends on the environment, so the property is asserted.
    #[tokio::test]
    async fn model_with_no_argument_offers_what_is_signed_in() {
        let harness = Harness::new("model-picker");
        harness.auth.store_api_key("anthropic", "sk-ant-test").unwrap();

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
            picker.items.iter().any(|item| item.label.starts_with("anthropic/")),
            "the signed-in provider is offered"
        );
        for item in &picker.items {
            let provider = item.label.split('/').next().unwrap_or_default();
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
            .map(|item| item.label.split('/').next().unwrap())
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
        harness.auth.store_api_key("anthropic", "sk-ant-test").unwrap();
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
        assert_eq!(marked, vec!["anthropic/claude-opus-5"]);
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

    #[test]
    fn token_counts_are_written_the_way_people_say_them() {
        assert_eq!(tokens(200_000), "200k");
        assert_eq!(tokens(1_000_000), "1M");
        assert_eq!(tokens(512), "512");
        assert_eq!(tokens(0), "unknown");
    }

    #[test]
    fn a_subscription_model_shows_no_price() {
        let mut model = ModelDef {
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
        assert_eq!(price(&model), "included");

        model.cost.input = 3.0;
        model.cost.output = 15.0;
        assert_eq!(price(&model), "$3.00/$15.00 per Mtok");
    }
}
