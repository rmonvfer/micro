//! Google Vertex AI: the Gemini shape, under a project and a location.
//!
//! Vertex serves the same models Google's own endpoint does, but addresses them by the
//! project and region they were provisioned in rather than by name alone, and
//! authenticates with a Google Cloud credential rather than an API key.
//!
//! A credential can be three things, and which one it is decides how a token is got:
//!
//! - a plain access token, sent as it is;
//! - a refresh token, which is what `gcloud auth application-default login` leaves behind,
//!   exchanged for an access token;
//! - an API key, sent as a bearer, which is what Vertex's express mode takes.
//!
//! Whichever it is, the request goes out as a bearer token, which is what keeps the rest
//! of the client the same as the one talking to Google directly.

use serde_json::Value;

/// The provider id Vertex is listed under.
pub const PROVIDER: &str = "google-vertex";
/// Where Vertex is served when the account says nothing else.
const DEFAULT_LOCATION: &str = "us-central1";
/// Where a refresh token is exchanged for one that can be used.
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

const PROJECT_ENV: &str = "GOOGLE_CLOUD_PROJECT";
const LEGACY_PROJECT_ENV: &str = "GCLOUD_PROJECT";
const LOCATION_ENV: &str = "GOOGLE_CLOUD_LOCATION";
const CREDENTIALS_ENV: &str = "GOOGLE_APPLICATION_CREDENTIALS";

/// Which project and region serve this account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub project: String,
    pub location: String,
}

/// The account a request belongs to, from the environment or from the address.
///
/// Vertex cannot be reached without a project: unlike every other service here, the
/// address is not the same for two customers. Saying so plainly is better than a request
/// that fails with a message about a malformed URL.
pub fn account(base_url: &str) -> Result<Account, String> {
    if let Some(found) = account_in(base_url) {
        return Ok(found);
    }

    let project = [PROJECT_ENV, LEGACY_PROJECT_ENV]
        .iter()
        .find_map(|name| std::env::var(name).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            format!("Vertex needs a project: set {PROJECT_ENV} to the one serving the model")
        })?;

    let location = std::env::var(LOCATION_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_LOCATION.to_string());

    Ok(Account { project, location })
}

/// The project and location named in a Vertex address, if it names them.
fn account_in(base_url: &str) -> Option<Account> {
    let (_, rest) = base_url.split_once("/projects/")?;
    let (project, rest) = rest.split_once("/locations/")?;
    let location = rest.split('/').next()?;
    if project.is_empty() || location.is_empty() || project.contains('{') {
        return None;
    }
    Some(Account {
        project: project.to_string(),
        location: location.to_string(),
    })
}

/// Where a model's stream is asked for.
pub fn endpoint(account: &Account, model_id: &str) -> String {
    let id = model_id.trim_start_matches("models/");
    format!(
        "https://{}-aiplatform.googleapis.com/v1/projects/{}/locations/{}/publishers/google/models/{id}:streamGenerateContent?alt=sse",
        account.location, account.project, account.location
    )
}

/// A token the request can be made with.
///
/// A credential that is already usable is returned as it is. One that is a refresh token,
/// or the file `gcloud` leaves behind, is exchanged for a usable one first.
pub async fn access_token(client: &reqwest::Client, credential: &str) -> Result<String, String> {
    let credential = credential.trim();

    // The file gcloud writes, named by the conventional variable.
    if let Some(refresh) = application_default_credentials() {
        return exchange(client, &refresh).await;
    }
    if credential.is_empty() {
        return Err(format!(
            "no Google Cloud credential: run `gcloud auth application-default login`, or \
             set {CREDENTIALS_ENV} to a credentials file"
        ));
    }
    Ok(credential.to_string())
}

/// What a refresh token needs alongside it to be exchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshCredential {
    pub client_id: String,
    pub client_secret: String,
    pub refresh_token: String,
}

/// The credentials `gcloud auth application-default login` leaves behind.
///
/// Read from the file the conventional variable names, falling back to the place gcloud
/// writes it, which is where every other Google tool looks for it.
fn application_default_credentials() -> Option<RefreshCredential> {
    let named = std::env::var(CREDENTIALS_ENV)
        .ok()
        .map(std::path::PathBuf::from);
    let conventional = std::env::var("HOME").ok().map(|home| {
        std::path::Path::new(&home)
            .join(".config")
            .join("gcloud")
            .join("application_default_credentials.json")
    });

    for path in [named, conventional].into_iter().flatten() {
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Some(found) = read_refresh_credential(&contents) {
            return Some(found);
        }
    }
    None
}

/// A refresh credential out of a credentials file, if that is what the file holds.
///
/// A service account file holds a private key instead, which is a different exchange and
/// is not read here.
pub fn read_refresh_credential(contents: &str) -> Option<RefreshCredential> {
    let parsed: Value = serde_json::from_str(contents).ok()?;
    let field = |name: &str| {
        parsed
            .get(name)
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|value| !value.is_empty())
    };
    Some(RefreshCredential {
        client_id: field("client_id")?,
        client_secret: field("client_secret")?,
        refresh_token: field("refresh_token")?,
    })
}

/// Trade a refresh token for one that can be used now.
async fn exchange(
    client: &reqwest::Client,
    credential: &RefreshCredential,
) -> Result<String, String> {
    let response = client
        .post(TOKEN_URL)
        .form(&[
            ("client_id", credential.client_id.as_str()),
            ("client_secret", credential.client_secret.as_str()),
            ("refresh_token", credential.refresh_token.as_str()),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .map_err(|error| format!("Google token exchange failed: {error}"))?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "Google token exchange returned {}: {}",
            status.as_u16(),
            body.trim()
        ));
    }

    serde_json::from_str::<Value>(&body)
        .ok()
        .and_then(|parsed| {
            parsed
                .get("access_token")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .ok_or_else(|| "Google token exchange returned no access token".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Vertex model lives under a project and a region, which is what makes its address
    /// different for every customer.
    #[test]
    fn a_model_is_addressed_under_its_project() {
        let account = Account {
            project: "my-project".into(),
            location: "europe-west4".into(),
        };
        assert_eq!(
            endpoint(&account, "gemini-3-pro"),
            "https://europe-west4-aiplatform.googleapis.com/v1/projects/my-project/locations/europe-west4/publishers/google/models/gemini-3-pro:streamGenerateContent?alt=sse"
        );
        // A `models/` prefix belongs to the other endpoint's spelling.
        assert!(endpoint(&account, "models/gemini-3-pro").contains("/models/gemini-3-pro:"));
    }

    /// An address that already names the account is read rather than second-guessed.
    #[test]
    fn the_address_can_name_the_account() {
        let found = account_in(
            "https://us-central1-aiplatform.googleapis.com/v1/projects/acme/locations/us-central1/publishers/google",
        )
        .expect("it names both");
        assert_eq!(found.project, "acme");
        assert_eq!(found.location, "us-central1");
    }

    /// A template address names nothing, so the environment decides.
    #[test]
    fn a_template_address_names_nothing() {
        assert!(account_in("https://{location}-aiplatform.googleapis.com/v1").is_none());
        assert!(account_in("https://example.test").is_none());
    }

    /// The credentials gcloud writes hold a refresh token; a service account file holds a
    /// key instead, and is a different exchange.
    #[test]
    fn a_refresh_credential_is_read_from_what_gcloud_writes() {
        let written = r#"{
            "client_id": "id.apps.googleusercontent.com",
            "client_secret": "secret",
            "refresh_token": "1//refresh",
            "type": "authorized_user"
        }"#;
        let found = read_refresh_credential(written).expect("it holds one");
        assert_eq!(found.client_id, "id.apps.googleusercontent.com");
        assert_eq!(found.refresh_token, "1//refresh");

        let service_account = r#"{
            "type": "service_account",
            "project_id": "acme",
            "private_key": "-----BEGIN PRIVATE KEY-----"
        }"#;
        assert!(read_refresh_credential(service_account).is_none());
        assert!(read_refresh_credential("not json").is_none());
    }
}
