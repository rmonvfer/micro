//! `/login`, `/logout`, and `/auth`: who micro is allowed to talk to.

use crate::CommandContext;
use crate::CommandOutcome;
use crate::Picker;
use crate::PickerItem;
use micro_auth::canonical_provider;
use micro_auth::AuthMethod;
use micro_auth::CredentialSource;
use micro_auth::LoginFlow;
use micro_auth::LoginOptions;
use micro_auth::ProviderStatus;
use micro_auth::METHOD_API_KEY;

pub(crate) async fn login(argument: Option<&str>, context: &CommandContext<'_>) -> CommandOutcome {
    let Some(argument) = argument else {
        return CommandOutcome::Choose(
            Picker::new(
                "Select provider to configure:",
                micro_auth::provider_table()
                    .iter()
                    .map(|entry| {
                        PickerItem::new(
                            entry.id.clone(),
                            login_note(&context.auth.status_of(&entry.id)),
                            format!("/login {}", entry.id),
                        )
                        .current(entry.id == context.provider)
                    })
                    .collect(),
            )
            .searchable(),
        );
    };

    let mut words = argument.split_whitespace();
    let name = words.next().unwrap_or_default();
    let method = words.next();
    let Some(provider) = known(name) else {
        return CommandOutcome::error(unknown_provider(name));
    };

    let options = match needs_device_id(provider, method) {
        true => match micro_config::Config::device_id(micro_auth::oauth::random_uuid) {
            Ok(device_id) => LoginOptions {
                device_id: Some(device_id),
            },
            Err(error) => {
                return CommandOutcome::error(format!(
                    "could not record this installation's device id: {error}"
                ))
            }
        },
        false => LoginOptions::default(),
    };

    match context
        .auth
        .begin_login_with(provider, method, &options)
        .await
    {
        Ok(LoginFlow::ApiKey {
            provider,
            env_names,
        }) => CommandOutcome::PromptForApiKey {
            provider,
            env_names,
        },
        Ok(LoginFlow::Choose {
            provider,
            title,
            options,
        }) => CommandOutcome::Choose(Picker::new(
            title,
            options
                .into_iter()
                .map(|option| {
                    PickerItem::new(
                        option.label,
                        String::new(),
                        format!("/login {provider} {}", option.id),
                    )
                })
                .collect(),
        )),
        Ok(LoginFlow::DeviceCode(pending)) => CommandOutcome::DeviceLogin {
            pending: Box::new(pending),
        },
        Ok(LoginFlow::Browser(pending)) => CommandOutcome::BrowserLogin {
            pending: Box::new(pending),
        },
        Err(error) => CommandOutcome::error(format!("could not start the login: {error}")),
    }
}

/// Sign in with ChatGPT registers this installation, so it is the one login that needs its id.
fn needs_device_id(provider: &str, method: Option<&str>) -> bool {
    provider == micro_auth::OPENAI && method.is_some_and(|method| method != METHOD_API_KEY)
}

pub(crate) fn logout(argument: Option<&str>, context: &CommandContext<'_>) -> CommandOutcome {
    let Some(name) = argument else {
        let signed_in: Vec<PickerItem> = context
            .auth
            .status()
            .into_iter()
            .filter(|status| status.source == CredentialSource::Stored)
            .map(|status| {
                PickerItem::new(
                    status.provider.clone(),
                    micro_auth::credential_kind(
                        &status.provider,
                        status.method == AuthMethod::OAuth,
                    ),
                    format!("/logout {}", status.provider),
                )
            })
            .collect();

        if signed_in.is_empty() {
            return CommandOutcome::info("no provider has a stored credential");
        }
        return CommandOutcome::Choose(
            Picker::new("Select provider to logout:", signed_in).searchable(),
        );
    };

    let Some(provider) = known(name) else {
        return CommandOutcome::error(unknown_provider(name));
    };

    if context.auth.get(provider).is_none() {
        return CommandOutcome::info(format!("{provider} has no stored credential"));
    }

    match context.auth.logout(provider) {
        Ok(()) => CommandOutcome::info(format!("signed out of {provider}")),
        Err(error) => CommandOutcome::error(format!("could not sign out of {provider}: {error}")),
    }
}

pub(crate) fn status(context: &CommandContext<'_>) -> CommandOutcome {
    let entries = context.auth.status();
    let width = entries
        .iter()
        .map(|status| status.provider.chars().count())
        .max()
        .unwrap_or(0);

    let text = entries
        .iter()
        .map(|status| {
            format!(
                "{:width$}  {}",
                status.provider,
                describe(status),
                width = width
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    CommandOutcome::info(text)
}

/// One provider's standing, in a phrase.
fn describe(status: &ProviderStatus) -> String {
    match &status.source {
        CredentialSource::Stored if status.needs_refresh => {
            "signed in - token refreshes on the next request".to_string()
        }
        CredentialSource::Stored => "signed in".to_string(),
        CredentialSource::Environment { variable } => format!("signed in via {variable}"),
        CredentialSource::Federation => "signed in via workload identity federation".to_string(),
        CredentialSource::Missing => "not signed in".to_string(),
    }
}

/// How a provider stands, as the login picker says it: whether it is configured, and with what.
fn login_note(status: &ProviderStatus) -> String {
    match &status.source {
        CredentialSource::Stored => format!(
            "{} configured",
            micro_auth::credential_kind(&status.provider, status.method == AuthMethod::OAuth)
        ),
        CredentialSource::Environment { variable } => format!("env: {variable}"),
        CredentialSource::Federation => "workload identity federation".to_string(),
        CredentialSource::Missing => "not configured".to_string(),
    }
}

/// The canonical id for a name micro knows, or nothing.
fn known(name: &str) -> Option<&'static str> {
    let canonical = canonical_provider(name);
    micro_auth::providers()
        .into_iter()
        .find(|provider| *provider == canonical)
}

fn unknown_provider(name: &str) -> String {
    format!(
        "unknown provider \"{name}\" - try one of: {}",
        micro_auth::providers().join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch;
    use crate::testing::*;
    use micro_auth::Credential;

    #[tokio::test]
    async fn login_with_no_argument_offers_every_provider() {
        let harness = Harness::new("login-picker");
        let outcome = dispatch("/login", &harness.context()).await.unwrap();
        let picker = picker(&outcome);

        let labels: Vec<&str> = picker
            .items
            .iter()
            .map(|item| item.label.as_str())
            .collect();
        assert_eq!(labels, micro_auth::providers());
        assert!(picker
            .items
            .iter()
            .all(|item| item.command.starts_with("/login ")));
    }

    #[tokio::test]
    async fn logging_in_to_a_key_provider_asks_for_the_key() {
        let harness = Harness::new("login-key");
        let outcome = dispatch("/login openrouter api_key", &harness.context())
            .await
            .unwrap();

        let CommandOutcome::PromptForApiKey {
            provider,
            env_names,
        } = outcome
        else {
            panic!("expected a key prompt");
        };
        assert_eq!(provider, "openrouter");
        assert_eq!(env_names, vec!["OPENROUTER_API_KEY"]);
    }

    #[tokio::test]
    async fn a_provider_with_an_account_and_a_key_asks_which_to_use() {
        let harness = Harness::new("login-choose");
        let outcome = dispatch("/login openrouter", &harness.context())
            .await
            .unwrap();
        let choices = picker(&outcome);

        let labels: Vec<&str> = choices
            .items
            .iter()
            .map(|item| item.label.as_str())
            .collect();
        assert_eq!(
            labels,
            vec!["Sign in with OpenRouter", "Sign in with an API key"]
        );
        assert_eq!(choices.command_at(0), Some("/login openrouter oauth"));
        assert_eq!(choices.command_at(1), Some("/login openrouter api_key"));

        let outcome = dispatch("/login anthropic oauth", &harness.context())
            .await
            .unwrap();
        let methods = picker(&outcome);
        assert_eq!(methods.command_at(0), Some("/login anthropic browser"));
        assert_eq!(methods.command_at(1), Some("/login anthropic copy_code"));
    }

    #[tokio::test]
    async fn a_method_a_provider_does_not_have_is_refused() {
        let harness = Harness::new("login-bad-method");
        let outcome = dispatch("/login github-copilot api_key", &harness.context())
            .await
            .unwrap();
        assert!(outcome.is_error(), "{outcome:?}");
    }

    #[tokio::test]
    async fn the_login_picker_says_what_is_configured_and_with_what() {
        let harness = Harness::new("login-notes");
        harness.auth.store_api_key("openrouter", "sk-or").unwrap();
        harness
            .auth
            .set(
                "xai",
                Credential::OAuth(micro_auth::OAuthCredential {
                    access_token: "xa".into(),
                    refresh_token: "xr".into(),
                    expires: i64::MAX / 2,
                    client_id: None,
                }),
            )
            .unwrap();

        let outcome = dispatch("/login", &harness.context()).await.unwrap();
        let picker = picker(&outcome);
        let note = |id: &str| {
            picker
                .items
                .iter()
                .find(|item| item.label == id)
                .map(|item| item.detail.clone())
                .unwrap()
        };
        assert_eq!(note("openrouter"), "API key configured");
        assert_eq!(note("xai"), "subscription configured");
        assert_eq!(note("mistral"), "not configured");
    }

    #[tokio::test]
    async fn the_logout_picker_names_the_kind_of_credential() {
        let harness = Harness::new("logout-kinds");
        harness
            .auth
            .set(
                "openrouter",
                Credential::OAuth(micro_auth::OAuthCredential {
                    access_token: "sk-or-v1".into(),
                    refresh_token: String::new(),
                    expires: micro_auth::openrouter::NEVER_EXPIRES,
                    client_id: None,
                }),
            )
            .unwrap();
        harness.auth.store_api_key("mistral", "key").unwrap();

        let outcome = dispatch("/logout", &harness.context()).await.unwrap();
        let picker = picker(&outcome);
        let details: Vec<(&str, &str)> = picker
            .items
            .iter()
            .map(|item| (item.label.as_str(), item.detail.as_str()))
            .collect();
        assert!(details.contains(&("openrouter", "account")), "{details:?}");
        assert!(details.contains(&("mistral", "API key")), "{details:?}");
    }

    #[tokio::test]
    async fn an_alias_reaches_the_provider_it_names() {
        let harness = Harness::new("login-alias");
        let outcome = dispatch("/login google", &harness.context()).await.unwrap();

        let CommandOutcome::PromptForApiKey { provider, .. } = outcome else {
            panic!("expected a key prompt");
        };
        assert_eq!(provider, "google");
    }

    #[tokio::test]
    async fn logging_in_to_a_provider_micro_does_not_know_lists_the_ones_it_does() {
        let harness = Harness::new("login-unknown");
        let outcome = dispatch("/login nowhere", &harness.context())
            .await
            .unwrap();

        assert!(outcome.is_error());
        assert!(text(&outcome).contains("github-copilot"), "{outcome:?}");
    }

    #[tokio::test]
    async fn logging_out_forgets_the_stored_credential() {
        let harness = Harness::new("logout");
        harness.auth.store_api_key("openrouter", "sk-or").unwrap();

        let outcome = dispatch("/logout openrouter", &harness.context())
            .await
            .unwrap();
        assert_eq!(text(&outcome), "signed out of openrouter");
        assert!(harness.auth.get("openrouter").is_none());
    }

    #[tokio::test]
    async fn logging_out_of_a_provider_with_nothing_stored_says_so() {
        let harness = Harness::new("logout-empty");
        let outcome = dispatch("/logout anthropic", &harness.context())
            .await
            .unwrap();

        assert!(!outcome.is_error());
        assert!(text(&outcome).contains("no stored credential"));
    }

    #[tokio::test]
    async fn logout_with_no_argument_offers_only_what_is_stored() {
        let harness = Harness::new("logout-picker");
        let outcome = dispatch("/logout", &harness.context()).await.unwrap();
        assert!(text(&outcome).contains("no provider"), "{outcome:?}");

        harness.auth.store_api_key("openrouter", "sk-or").unwrap();
        let outcome = dispatch("/logout", &harness.context()).await.unwrap();
        let picker = picker(&outcome);

        assert_eq!(picker.items.len(), 1);
        assert_eq!(picker.command_at(0), Some("/logout openrouter"));
    }

    #[tokio::test]
    async fn auth_reports_every_provider() {
        let harness = Harness::new("auth-status");
        harness.auth.store_api_key("openrouter", "sk-or").unwrap();

        let outcome = dispatch("/auth", &harness.context()).await.unwrap();
        let report = text(&outcome);

        for provider in micro_auth::providers() {
            assert!(report.contains(provider), "missing {provider}");
        }

        let line = report
            .lines()
            .find(|line| line.starts_with("openrouter"))
            .unwrap_or_else(|| panic!("no openrouter line in:\n{report}"));
        assert_eq!(line.split_once("  ").unwrap().1.trim(), "signed in");
    }

    #[tokio::test]
    async fn a_stored_token_past_its_expiry_is_reported_as_refreshing() {
        let harness = Harness::new("auth-expired");
        harness
            .auth
            .set(
                "github-copilot",
                Credential::OAuth(micro_auth::OAuthCredential {
                    access_token: "stale".into(),
                    refresh_token: "gho".into(),
                    expires: 1,
                    client_id: None,
                }),
            )
            .unwrap();

        let outcome = dispatch("/auth", &harness.context()).await.unwrap();
        assert!(text(&outcome).contains("refreshes on the next request"));
    }

    #[test]
    fn only_providers_micro_knows_are_accepted() {
        assert_eq!(known("copilot"), Some("github-copilot"));
        assert_eq!(known("GEMINI"), Some("google"));

        assert_eq!(
            known("azure-openai-responses"),
            Some("azure-openai-responses")
        );

        assert_eq!(known("mistral"), Some("mistral"));

        assert_eq!(known("amazon-bedrock"), Some("amazon-bedrock"));

        assert_eq!(known("google-vertex"), Some("google-vertex"));

        assert_eq!(known("not-a-service"), None);
    }
}
