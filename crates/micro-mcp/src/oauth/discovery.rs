//! Finding the authorization server that guards a server, and what it says about itself.

use super::ResourceMetadata;
use super::ServerMetadata;
use crate::PROTOCOL_VERSION;
use reqwest::Url;
use serde::Deserialize;
use serde::Serialize;

/// The authorization server behind a resource, and what both say about themselves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Discovered {
    pub authorization_server: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<ServerMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<ResourceMetadata>,
}

/// Find the authorization server for `server`. `resource_metadata_url` comes from the server's
/// challenge. `metadata_override` names the authorization server's metadata document directly;
/// it is trusted as configured, so its issuer is not checked.
pub async fn discover(
    http: &reqwest::Client,
    server: &Url,
    resource_metadata_url: Option<&str>,
    metadata_override: Option<&str>,
) -> Result<Discovered, String> {
    let resource = resource_metadata(http, server, resource_metadata_url).await?;

    if let Some(url) = metadata_override {
        let response = fetch(http, url).await?;
        if !response.status().is_success() {
            return Err(format!(
                "HTTP {} loading authorization server metadata from {url}",
                response.status().as_u16()
            ));
        }
        let metadata = ServerMetadata::parse(&json(response).await?)?;
        return Ok(Discovered {
            authorization_server: metadata.issuer.clone(),
            metadata: Some(metadata),
            resource,
        });
    }

    let authorization_server = match resource
        .as_ref()
        .and_then(|resource| resource.authorization_servers.first())
    {
        Some(server) => server.clone(),
        None => server
            .join("/")
            .map_err(|error| error.to_string())?
            .to_string(),
    };
    let metadata = server_metadata(http, &authorization_server).await?;
    Ok(Discovered {
        authorization_server,
        metadata,
        resource,
    })
}

/// The server's protected resource metadata, or `None` when it publishes none.
async fn resource_metadata(
    http: &reqwest::Client,
    server: &Url,
    named: Option<&str>,
) -> Result<Option<ResourceMetadata>, String> {
    let first = match named {
        Some(url) => url.to_string(),
        None => well_known(
            server,
            "oauth-protected-resource",
            path_suffix(server.path()),
        ),
    };
    let mut response = fetch(http, &first).await?;
    if named.is_none() && server.path() != "/" && is_miss(response.status().as_u16()) {
        response = fetch(http, &well_known(server, "oauth-protected-resource", "")).await?;
    }
    if !response.status().is_success() {
        return Ok(None);
    }
    match json(response).await {
        Ok(value) => Ok(ResourceMetadata::parse(&value).ok()),
        Err(_) => Ok(None),
    }
}

/// The authorization server's metadata, from the first well-known place that has it. The
/// document must name the server it was asked about as its issuer.
async fn server_metadata(
    http: &reqwest::Client,
    authorization_server: &str,
) -> Result<Option<ServerMetadata>, String> {
    let issuer = Url::parse(authorization_server)
        .map_err(|_| format!("invalid authorization server URL {authorization_server}"))?;
    let path = path_suffix(issuer.path());
    let mut candidates = vec![
        well_known(&issuer, "oauth-authorization-server", path),
        well_known(&issuer, "openid-configuration", path),
    ];
    if !path.is_empty() {
        candidates.push(format!(
            "{}{path}/.well-known/openid-configuration",
            origin(&issuer)
        ));
    }

    for candidate in candidates {
        let response = fetch(http, &candidate).await?;
        let status = response.status().as_u16();
        if !response.status().is_success() {
            if is_miss(status) {
                continue;
            }
            return Err(format!(
                "HTTP {status} loading authorization server metadata from {candidate}"
            ));
        }
        let metadata = ServerMetadata::parse(&json(response).await?)?;
        if trim_slash(&metadata.issuer) != trim_slash(authorization_server) {
            return Err(format!(
                "the authorization server metadata names issuer {}, expected {authorization_server}",
                metadata.issuer
            ));
        }
        return Ok(Some(metadata));
    }
    Ok(None)
}

/// The resource indicator to send (RFC 8707): the metadata's resource, once it is known to
/// cover the server.
pub fn select_resource(
    server: &Url,
    metadata: Option<&ResourceMetadata>,
) -> Result<Option<String>, String> {
    let Some(metadata) = metadata else {
        return Ok(None);
    };
    let configured = Url::parse(&metadata.resource)
        .map_err(|_| format!("invalid protected resource {}", metadata.resource))?;
    let mismatch = || {
        format!(
            "the protected resource {} does not match the server {server}",
            metadata.resource
        )
    };
    if configured.origin() != server.origin() {
        return Err(mismatch());
    }
    let with_slash = |path: &str| match path.ends_with('/') {
        true => path.to_string(),
        false => format!("{path}/"),
    };
    if !with_slash(server.path()).starts_with(&with_slash(configured.path())) {
        return Err(mismatch());
    }
    Ok(Some(metadata.resource.clone()))
}

async fn fetch(http: &reqwest::Client, url: &str) -> Result<reqwest::Response, String> {
    http.get(url)
        .header("Accept", "application/json")
        .header("MCP-Protocol-Version", PROTOCOL_VERSION)
        .send()
        .await
        .map_err(|error| format!("cannot reach {url}: {error}"))
}

async fn json(response: reqwest::Response) -> Result<serde_json::Value, String> {
    response
        .json()
        .await
        .map_err(|error| format!("the metadata is not JSON: {error}"))
}

/// 4xx and 502 mean "not here", so the next candidate is tried.
fn is_miss(status: u16) -> bool {
    (400..500).contains(&status) || status == 502
}

fn path_suffix(path: &str) -> &str {
    path.strip_suffix('/').unwrap_or(path)
}

fn origin(url: &Url) -> String {
    url.origin().ascii_serialization()
}

fn well_known(url: &Url, kind: &str, path: &str) -> String {
    format!("{}/.well-known/{kind}{path}", origin(url))
}

fn trim_slash(value: &str) -> &str {
    value.strip_suffix('/').unwrap_or(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_resource_must_cover_the_server_it_guards() {
        let server = Url::parse("https://mcp.example.com/v1/mcp").unwrap();
        let metadata = |resource: &str| ResourceMetadata {
            resource: resource.to_string(),
            authorization_servers: Vec::new(),
            scopes_supported: None,
        };
        assert_eq!(
            select_resource(&server, Some(&metadata("https://mcp.example.com/v1"))).unwrap(),
            Some("https://mcp.example.com/v1".to_string())
        );
        assert!(select_resource(&server, Some(&metadata("https://other.example.com"))).is_err());
        assert!(select_resource(&server, Some(&metadata("https://mcp.example.com/v2"))).is_err());
        assert_eq!(select_resource(&server, None).unwrap(), None);
    }
}
