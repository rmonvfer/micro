//! One JSON request to a model service that answers in one JSON body, retried while the service
//! says it is busy.

use micro_models::ModelDef;
use serde_json::Value;
use std::time::Duration;

/// How many times a request is sent again after the service said it was busy or failing.
const RETRIES: u32 = 2;

/// The first wait between attempts; each later one doubles it.
const FIRST_DELAY: Duration = Duration::from_millis(500);

/// How much of a failed response's body an error message quotes.
const QUOTED_BODY: usize = 500;

/// Send `body` to `url` and read the JSON it answers with.
///
/// The model's own headers go on every request, and the credential, when there is one, as a bearer
/// token. Rate limits and server errors are retried; anything else is reported at once.
pub(crate) async fn post_json(
    http: &reqwest::Client,
    model: &ModelDef,
    url: &str,
    api_key: Option<&str>,
    body: &Value,
) -> Result<Value, String> {
    let mut attempt = 0;
    loop {
        let mut request = crate::with_attribution(http.post(url), &model.base_url)
            .header("content-type", "application/json")
            .json(body);
        if let Some(key) = api_key.filter(|key| !key.trim().is_empty()) {
            request = request.bearer_auth(key);
        }
        for (name, value) in &model.headers {
            request = request.header(name.as_str(), value.as_str());
        }

        let failure = match request.send().await {
            Ok(response) if response.status().is_success() => {
                return response.json::<Value>().await.map_err(|error| {
                    format!("{url} answered with something other than JSON: {error}")
                });
            }
            Ok(response) => {
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                let quoted: String = text.chars().take(QUOTED_BODY).collect();
                let message = match quoted.trim().is_empty() {
                    true => format!("{} returned {status}", model.provider),
                    false => format!("{} returned {status}: {}", model.provider, quoted.trim()),
                };
                (status.as_u16() == 429 || status.is_server_error(), message)
            }
            Err(error) => (
                error.is_connect() || error.is_timeout(),
                format!("cannot reach {url}: {error}"),
            ),
        };

        let (retryable, message) = failure;
        if !retryable || attempt >= RETRIES {
            return Err(message);
        }
        tokio::time::sleep(FIRST_DELAY * 2u32.pow(attempt)).await;
        attempt += 1;
    }
}

/// An address with every `{NAME}` filled in from the environment, or the name that is missing.
///
/// Some services are addressed under an account, which the catalog can only write as a
/// placeholder.
pub(crate) fn expand_placeholders(url: &str) -> Result<String, String> {
    let mut expanded = String::with_capacity(url.len());
    let mut rest = url;
    while let Some(open) = rest.find('{') {
        let Some(close) = rest[open..].find('}') else {
            break;
        };
        let name = &rest[open + 1..open + close];
        let value = std::env::var(name)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| format!("set {name} to reach {url}"))?;
        expanded.push_str(&rest[..open]);
        expanded.push_str(value.trim());
        rest = &rest[open + close + 1..];
    }
    expanded.push_str(rest);
    Ok(expanded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_without_placeholders_is_left_alone() {
        assert_eq!(
            expand_placeholders("https://openrouter.ai/api/v1").unwrap(),
            "https://openrouter.ai/api/v1"
        );
    }

    #[test]
    fn a_missing_placeholder_names_what_to_set() {
        let error =
            expand_placeholders("https://x.example/{MICRO_TEST_UNSET_ACCOUNT}/ai").unwrap_err();
        assert!(error.contains("MICRO_TEST_UNSET_ACCOUNT"), "{error}");
    }
}
