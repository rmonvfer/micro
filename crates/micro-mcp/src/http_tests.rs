//! Servers reached over streamable HTTP, with and without signing in.

use crate::config::OAuthConfig;
use crate::config::Scope;
use crate::config::ServerConfig;
use crate::config::Transport;
use crate::oauth::CredentialStore;
use crate::oauth::SignIn;
use crate::test_server::Request;
use crate::test_server::Response;
use crate::test_server::TestServer;
use crate::LoadedConfig;
use crate::ServerEntry;
use crate::Servers;
use crate::Status;
use serde_json::json;
use serde_json::Value;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

fn scratch(label: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("micro-mcp-http-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

fn servers_with(name: &str, config: ServerConfig) -> Servers {
    Servers::new(
        LoadedConfig {
            servers: vec![ServerEntry {
                name: name.to_string(),
                config,
                source: "mcp.json".into(),
                scope: Scope::Global,
            }],
            errors: Vec::new(),
        },
        Path::new("."),
    )
}

/// How a test server answers MCP itself, once a request is allowed in.
fn speak_mcp(request: &Request, tools_as_events: bool) -> Response {
    if request.method == "GET" {
        return Response::status(405);
    }
    if request.method == "DELETE" {
        return Response::status(200);
    }
    let message = request.json();
    let id = message["id"].clone();
    match message["method"].as_str().unwrap_or_default() {
        "initialize" => Response::json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "protocolVersion": "2025-06-18", "capabilities": {} },
        }))
        .with_header("Mcp-Session-Id", "session-1"),
        "notifications/initialized" => Response::status(202),
        "tools/list" => {
            let answer = json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "tools": [{ "name": "look-up", "description": "Look something up" }] },
            });
            match tools_as_events {
                true => Response::events(&[
                    json!({ "jsonrpc": "2.0", "method": "notifications/message", "params": {} }),
                    answer,
                ]),
                false => Response::json(answer),
            }
        }
        "tools/call" => Response::json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": { "content": [{ "type": "text", "text": format!("found {}", message["params"]["arguments"]["q"]) }] },
        })),
        _ => Response::status(400),
    }
}

#[tokio::test]
async fn a_streamable_http_server_keeps_its_session() {
    let server = TestServer::start(|request| {
        let initializing = request.body.contains("\"initialize\"");
        if request.method == "POST"
            && !initializing
            && request.header("mcp-session-id") != Some("session-1")
        {
            return Response::status(400);
        }
        speak_mcp(request, true)
    })
    .await;

    let mut config = ServerConfig::http(server.url("/mcp"));
    if let Transport::Http { headers, .. } = &mut config.transport {
        headers.insert("X-Team".into(), "${MICRO_MCP_HTTP_TEST_TEAM}".into());
    }
    std::env::set_var("MICRO_MCP_HTTP_TEST_TEAM", "blue");
    let servers = servers_with("lookup", config);

    let tools = servers.connect("lookup").await.expect("it connects");
    assert_eq!(tools[0].definition().name, "mcp__lookup__look_up");
    let said = tools[0].execute(&json!({ "q": "x" })).await.unwrap();
    assert_eq!(said, "found \"x\"");

    let requests = server.requests();
    let call = requests
        .iter()
        .find(|request| request.body.contains("tools/call"))
        .unwrap();
    assert_eq!(call.header("mcp-protocol-version"), Some("2025-06-18"));
    assert_eq!(call.header("x-team"), Some("blue"));
    assert!(call
        .header("accept")
        .is_some_and(|accept| accept.contains("text/event-stream")));
}

#[tokio::test]
async fn a_provider_credential_is_sent_as_the_bearer_token() {
    let server = TestServer::start(|request| match request.header("authorization") {
        Some("Bearer sk-provider") => speak_mcp(request, false),
        _ => Response::status(401),
    })
    .await;
    let store = micro_auth::AuthStore::open_at(scratch("provider").join("auth.json")).unwrap();
    store
        .set("openai", micro_auth::Credential::api_key("sk-provider"))
        .unwrap();

    let mut config = ServerConfig::http(server.url("/mcp"));
    if let Transport::Http { provider, .. } = &mut config.transport {
        *provider = Some("openai".into());
    }
    let servers = servers_with("hosted", config).with_providers(Arc::new(store));

    let tools = servers
        .connect("hosted")
        .await
        .expect("the credential is accepted");
    assert_eq!(tools.len(), 1);
}

/// A server that is briefly unavailable is tried again; one that refuses outright is not.
#[tokio::test]
async fn connecting_retries_a_server_that_is_briefly_unavailable() {
    let refused = Arc::new(Mutex::new(0));
    let counted = Arc::clone(&refused);
    let server = TestServer::start(move |request| {
        if request.body.contains("\"initialize\"") {
            let mut refused = counted.lock().unwrap();
            if *refused < 2 {
                *refused += 1;
                return Response::status(503);
            }
        }
        speak_mcp(request, false)
    })
    .await;
    let servers = servers_with("flaky", ServerConfig::http(server.url("/mcp")));

    let tools = servers
        .connect("flaky")
        .await
        .expect("the third attempt connects");
    assert_eq!(tools.len(), 1);
    assert_eq!(*refused.lock().unwrap(), 2);

    let broken = TestServer::start(|_| Response::status(400)).await;
    let servers = servers_with("broken", ServerConfig::http(broken.url("/mcp")));
    assert!(servers.connect("broken").await.is_err());
    assert_eq!(broken.requests().len(), 1, "a refusal is not retried");
}

/// An answer stream that breaks off is resumed from its last event, past a transient failure.
#[tokio::test]
async fn a_broken_answer_stream_is_resumed_from_its_last_event() {
    let reopened = Arc::new(Mutex::new(0));
    let counted = Arc::clone(&reopened);
    let server = TestServer::start(move |request| {
        let stream = |body: String| Response {
            status: 200,
            headers: vec![("Content-Type".into(), "text/event-stream".into())],
            body,
        };
        if request.method == "GET" {
            if request.header("last-event-id") != Some("e1") {
                return Response::status(405);
            }
            let mut reopened = counted.lock().unwrap();
            *reopened += 1;
            if *reopened == 1 {
                return Response::status(503);
            }
            let answer = json!({
                "jsonrpc": "2.0",
                "id": 3,
                "result": { "content": [{ "type": "text", "text": "resumed" }] },
            });
            return stream(format!("id: e2\nevent: message\ndata: {answer}\n\n"));
        }
        if request.body.contains("tools/call") {
            return stream("id: e1\ndata: \n\n".to_string());
        }
        speak_mcp(request, false)
    })
    .await;
    let servers = servers_with("resuming", ServerConfig::http(server.url("/mcp")));
    let tools = servers.connect("resuming").await.unwrap();

    let said = tools[0].execute(&json!({ "q": "x" })).await.unwrap();
    assert_eq!(said, "resumed");
    assert_eq!(*reopened.lock().unwrap(), 2);
}

/// An authorization server and resource that sign micro in, recording what it was sent.
struct OAuthWorld {
    server: TestServer,
    valid: Arc<Mutex<Vec<String>>>,
    /// Scopes a tool call needs, beyond which the server asks for more.
    needs: Arc<Mutex<Option<String>>>,
}

async fn oauth_world() -> OAuthWorld {
    let valid: Arc<Mutex<Vec<String>>> = Arc::default();
    let needs: Arc<Mutex<Option<String>>> = Arc::default();
    let accepted = Arc::clone(&valid);
    let wanted = Arc::clone(&needs);
    let server = TestServer::start(move |request| {
        let base = format!("http://{}", request.header("host").unwrap());
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/.well-known/oauth-protected-resource/mcp") => Response::json(json!({
                "resource": format!("{base}/mcp"),
                "authorization_servers": [base],
                "scopes_supported": ["read"],
            })),
            ("GET", "/.well-known/oauth-authorization-server") => Response::json(json!({
                "issuer": base,
                "authorization_endpoint": format!("{base}/authorize"),
                "token_endpoint": format!("{base}/token"),
                "registration_endpoint": format!("{base}/register"),
                "response_types_supported": ["code"],
                "code_challenge_methods_supported": ["S256"],
                "authorization_response_iss_parameter_supported": true,
            })),
            ("POST", "/register") => Response::json(json!({
                "client_id": "client-1",
                "client_secret": null,
                "redirect_uris": request.json()["redirect_uris"],
            })),
            ("POST", "/token") => {
                let form = request.form();
                let (access, refresh) = match form.get("grant_type").map(String::as_str) {
                    Some("authorization_code") if form.get("code").map(String::as_str) == Some("the-code") => ("at-1", "rt-1"),
                    Some("refresh_token") if form.get("refresh_token").map(String::as_str) == Some("rt-1") => ("at-2", "rt-2"),
                    _ => return Response::json(json!({ "error": "invalid_grant" })).with_status(400),
                };
                accepted.lock().unwrap().push(access.to_string());
                Response::json(json!({
                    "access_token": access,
                    "token_type": "Bearer",
                    "expires_in": 3600,
                    "scope": "",
                    "refresh_token": refresh,
                    "id_token": null,
                }))
            }
            (_, "/mcp") => {
                let token = request
                    .header("authorization")
                    .and_then(|value| value.strip_prefix("Bearer "))
                    .map(str::to_string);
                if !token.is_some_and(|token| accepted.lock().unwrap().contains(&token)) {
                    return Response::status(401).with_header(
                        "WWW-Authenticate",
                        &format!(
                            "Bearer resource_metadata=\"{base}/.well-known/oauth-protected-resource/mcp\""
                        ),
                    );
                }
                if request.body.contains("tools/call") {
                    if let Some(scope) = wanted.lock().unwrap().clone() {
                        return Response::status(403).with_header(
                            "WWW-Authenticate",
                            &format!("Bearer error=\"insufficient_scope\", scope=\"{scope}\""),
                        );
                    }
                }
                speak_mcp(request, false)
            }
            _ => Response::status(404),
        }
    })
    .await;
    OAuthWorld {
        server,
        valid,
        needs,
    }
}

impl Response {
    fn with_status(mut self, status: u16) -> Response {
        self.status = status;
        self
    }
}

fn oauth_servers(world: &OAuthWorld, store: &CredentialStore) -> Servers {
    let mut config = ServerConfig::http(world.server.url("/mcp"));
    if let Transport::Http { oauth, .. } = &mut config.transport {
        *oauth = OAuthConfig {
            client_name: Some("Custom Name".into()),
            ..OAuthConfig::default()
        };
    }
    servers_with("guarded", config).with_credentials(store.clone())
}

fn parameter(url: &reqwest::Url, name: &str) -> Option<String> {
    url.query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

/// Play the browser: follow the authorization URL's redirect back with a code.
async fn approve(authorization_url: &reqwest::Url, iss: &str) {
    let mut back =
        reqwest::Url::parse(&parameter(authorization_url, "redirect_uri").unwrap()).unwrap();
    back.query_pairs_mut()
        .append_pair("code", "the-code")
        .append_pair("state", &parameter(authorization_url, "state").unwrap())
        .append_pair("iss", iss);
    reqwest::get(back).await.unwrap();
}

#[tokio::test]
async fn a_server_that_wants_a_sign_in_gets_one() {
    let world = oauth_world().await;
    let store = CredentialStore::at(scratch("sign-in").join("mcp-auth.json"));
    let servers = oauth_servers(&world, &store);

    let refused = servers
        .connect("guarded")
        .await
        .err()
        .expect("no token yet");
    assert!(
        refused.to_string().contains("micro mcp login guarded"),
        "{refused}"
    );
    assert_eq!(servers.status("guarded"), Some(Status::NeedsSignIn));

    let SignIn::Pending(pending) = servers.begin_sign_in("guarded").await.unwrap() else {
        panic!("nothing was stored, so the browser is needed");
    };
    let url = pending.authorization_url.clone();
    assert_eq!(parameter(&url, "client_id").as_deref(), Some("client-1"));
    assert_eq!(
        parameter(&url, "code_challenge_method").as_deref(),
        Some("S256")
    );
    assert_eq!(parameter(&url, "scope").as_deref(), Some("read"));
    assert_eq!(parameter(&url, "resource"), Some(world.server.url("/mcp")));

    approve(&url, &world.server.base).await;
    pending
        .finish(Duration::from_secs(5))
        .await
        .expect("signed in");

    let registered = world
        .server
        .requests()
        .into_iter()
        .find(|request| request.path == "/register")
        .unwrap();
    assert_eq!(registered.json()["client_name"], "Custom Name");

    let stored = store
        .server("guarded", &world.server.url("/mcp"))
        .load()
        .unwrap();
    let tokens = stored.tokens.unwrap();
    assert_eq!(tokens.access_token, "at-1");
    assert_eq!(
        tokens.scope.as_deref(),
        Some("read"),
        "an empty scope in the answer grants what was asked for"
    );

    assert_eq!(servers.reconnect("guarded").await.unwrap(), 1);
    assert!(servers.arrivals().find("mcp__guarded__look_up").is_some());
}

#[tokio::test]
async fn a_code_from_another_issuer_is_never_exchanged() {
    let world = oauth_world().await;
    let store = CredentialStore::at(scratch("issuer").join("mcp-auth.json"));
    let servers = oauth_servers(&world, &store);
    let _ = servers.connect("guarded").await;

    let SignIn::Pending(pending) = servers.begin_sign_in("guarded").await.unwrap() else {
        panic!("the browser is needed");
    };
    approve(
        &pending.authorization_url.clone(),
        "https://elsewhere.example",
    )
    .await;
    let error = pending.finish(Duration::from_secs(5)).await.unwrap_err();
    assert!(error.contains("issuer"), "{error}");
    assert!(
        !world
            .server
            .requests()
            .iter()
            .any(|request| request.path == "/token"),
        "the code never reached the token endpoint"
    );
}

#[tokio::test]
async fn a_lapsing_token_is_refreshed_before_it_is_sent() {
    let world = oauth_world().await;
    let store = CredentialStore::at(scratch("refresh").join("mcp-auth.json"));
    let servers = oauth_servers(&world, &store);
    let _ = servers.connect("guarded").await;
    let SignIn::Pending(pending) = servers.begin_sign_in("guarded").await.unwrap() else {
        panic!("the browser is needed");
    };
    approve(&pending.authorization_url.clone(), &world.server.base).await;
    pending.finish(Duration::from_secs(5)).await.unwrap();

    let credentials = store.server("guarded", &world.server.url("/mcp"));
    let mut stored = credentials.load().unwrap();
    stored.tokens_expire_at = Some(0);
    credentials.save(stored).unwrap();
    world.valid.lock().unwrap().retain(|token| token != "at-1");

    servers
        .connect("guarded")
        .await
        .expect("the refreshed token is accepted");
    let refreshed = credentials.load().unwrap().tokens.unwrap();
    assert_eq!(refreshed.access_token, "at-2");
    assert_eq!(refreshed.refresh_token.as_deref(), Some("rt-2"));
    assert_eq!(
        refreshed.scope.as_deref(),
        Some("read"),
        "a refresh keeps the granted scope"
    );
}

#[tokio::test]
async fn asking_for_more_scope_keeps_the_scope_already_granted() {
    let world = oauth_world().await;
    let store = CredentialStore::at(scratch("step-up").join("mcp-auth.json"));
    let servers = oauth_servers(&world, &store);
    let _ = servers.connect("guarded").await;
    let SignIn::Pending(pending) = servers.begin_sign_in("guarded").await.unwrap() else {
        panic!("the browser is needed");
    };
    approve(&pending.authorization_url.clone(), &world.server.base).await;
    pending.finish(Duration::from_secs(5)).await.unwrap();

    let tools = servers.connect("guarded").await.unwrap();
    *world.needs.lock().unwrap() = Some("write".into());
    let refused = tools[0].execute(&json!({ "q": "x" })).await.unwrap_err();
    assert!(refused.contains("needs sign-in"), "{refused}");

    let SignIn::Pending(pending) = servers.begin_sign_in("guarded").await.unwrap() else {
        panic!("more scope needs the browser, not a refresh");
    };
    assert_eq!(
        parameter(&pending.authorization_url, "scope").as_deref(),
        Some("read write")
    );
}

#[tokio::test]
async fn signing_out_forgets_the_credentials() {
    let world = oauth_world().await;
    let store = CredentialStore::at(scratch("sign-out").join("mcp-auth.json"));
    let servers = oauth_servers(&world, &store);
    let _ = servers.connect("guarded").await;
    let SignIn::Pending(pending) = servers.begin_sign_in("guarded").await.unwrap() else {
        panic!("the browser is needed");
    };
    approve(&pending.authorization_url.clone(), &world.server.base).await;
    pending.finish(Duration::from_secs(5)).await.unwrap();
    assert!(servers.report()[0].signed_in);

    assert!(servers.sign_out("guarded").unwrap());
    assert!(!servers.sign_out("guarded").unwrap());
    assert_eq!(servers.status("guarded"), Some(Status::SignedOut));
    let report: Vec<Value> = servers
        .report()
        .iter()
        .map(|report| json!(report.signed_in))
        .collect();
    assert_eq!(report, vec![json!(false)]);
}
