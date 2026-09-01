use bytes::Bytes;
use futures::SinkExt;
use futures::StreamExt;
use http_body_util::BodyExt;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::header::CONNECTION;
use hyper::header::SEC_WEBSOCKET_ACCEPT;
use hyper::header::SEC_WEBSOCKET_KEY;
use hyper::header::UPGRADE;
use hyper::service::service_fn;
use hyper::Method;
use hyper::Request;
use hyper::Response;
use hyper::StatusCode;
use hyper_util::rt::TokioIo;
use rcgen::generate_simple_self_signed;
use rcgen::CertifiedKey;
use rustls::pki_types::PrivatePkcs8KeyDer;
use rustls::ClientConfig;
use rustls::RootCertStore;
use rustls::ServerConfig;
use serde_json::Value;
use sha2::Digest;
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::Role;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::Connector;
use tokio_tungstenite::WebSocketStream;

#[derive(Default)]
struct Enrolment {
    pairing_id: String,
    machine_public: String,
    phone_public: Option<String>,
}

#[derive(Default)]
struct Verifiers {
    machine: String,
    phone: String,
}

#[derive(Default)]
struct Peers {
    machine: Option<mpsc::UnboundedSender<Message>>,
    phone: Option<mpsc::UnboundedSender<Message>>,
}

#[derive(Default)]
struct State {
    enrolments: HashMap<String, Enrolment>,
    verifiers: HashMap<String, Verifiers>,
    peers: HashMap<String, Peers>,
}

pub struct RelayFixture {
    pub url: String,
    pub http: reqwest::Client,
    #[allow(dead_code)]
    pub connector: Connector,
    task: tokio::task::JoinHandle<()>,
}

impl RelayFixture {
    pub async fn start() -> RelayFixture {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let CertifiedKey { cert, signing_key } =
            generate_simple_self_signed(vec!["127.0.0.1".to_string()]).unwrap();
        let server = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![cert.der().clone()],
                PrivatePkcs8KeyDer::from(signing_key.serialize_der()).into(),
            )
            .unwrap();
        let acceptor = TlsAcceptor::from(Arc::new(server));

        let http = reqwest::Client::builder()
            .add_root_certificate(reqwest::Certificate::from_der(cert.der()).unwrap())
            .build()
            .unwrap();
        let mut roots = RootCertStore::empty();
        roots.add(cert.der().clone()).unwrap();
        let client = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let connector = Connector::Rustls(Arc::new(client));

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let state = Arc::new(Mutex::new(State::default()));
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let acceptor = acceptor.clone();
                let state = Arc::clone(&state);
                tokio::spawn(async move {
                    let Ok(stream) = acceptor.accept(stream).await else {
                        return;
                    };
                    let connection = hyper::server::conn::http1::Builder::new()
                        .serve_connection(
                            TokioIo::new(stream),
                            service_fn(move |request| handle(request, Arc::clone(&state))),
                        )
                        .with_upgrades();
                    let _ = connection.await;
                });
            }
        });

        RelayFixture {
            url: format!("https://{address}"),
            http,
            connector,
            task,
        }
    }
}

impl Drop for RelayFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn handle(
    request: Request<Incoming>,
    state: Arc<Mutex<State>>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let path = request.uri().path().to_string();
    let method = request.method().clone();

    if method == Method::POST && path == "/enrol/start" {
        let body = json_body(request).await;
        let Some(code) = string(&body, "code") else {
            return Ok(response(StatusCode::BAD_REQUEST, ""));
        };
        let Some(pairing_id) = string(&body, "pairingId") else {
            return Ok(response(StatusCode::BAD_REQUEST, ""));
        };
        let Some(machine_public) = string(&body, "machinePublic") else {
            return Ok(response(StatusCode::BAD_REQUEST, ""));
        };
        state.lock().await.enrolments.insert(
            code,
            Enrolment {
                pairing_id,
                machine_public,
                phone_public: None,
            },
        );
        return Ok(response(StatusCode::NO_CONTENT, ""));
    }

    if method == Method::POST && path == "/enrol/claim" {
        let body = json_body(request).await;
        let Some(code) = string(&body, "code") else {
            return Ok(response(StatusCode::BAD_REQUEST, ""));
        };
        let Some(phone_public) = string(&body, "phonePublic") else {
            return Ok(response(StatusCode::BAD_REQUEST, ""));
        };
        let mut state = state.lock().await;
        let Some(enrolment) = state.enrolments.get_mut(&code) else {
            return Ok(response(StatusCode::NOT_FOUND, ""));
        };
        if enrolment.phone_public.is_some() {
            return Ok(response(StatusCode::CONFLICT, ""));
        }
        enrolment.phone_public = Some(phone_public);
        return Ok(json_response(serde_json::json!({
            "pairingId": enrolment.pairing_id,
            "machinePublic": enrolment.machine_public,
        })));
    }

    if method == Method::GET && path == "/enrol/await" {
        let code = query(&request, "code").unwrap_or_default();
        let state = state.lock().await;
        let Some(enrolment) = state.enrolments.get(&code) else {
            return Ok(response(StatusCode::NOT_FOUND, ""));
        };
        return match &enrolment.phone_public {
            Some(phone_public) => Ok(json_response(serde_json::json!({
                "phonePublic": phone_public,
            }))),
            None => Ok(response(StatusCode::NO_CONTENT, "")),
        };
    }

    if method == Method::POST && path == "/pairings" {
        let body = json_body(request).await;
        let Some(pairing_id) = string(&body, "pairingId") else {
            return Ok(response(StatusCode::BAD_REQUEST, ""));
        };
        let Some(machine) = string(&body, "machineVerifier") else {
            return Ok(response(StatusCode::BAD_REQUEST, ""));
        };
        let Some(phone) = string(&body, "phoneVerifier") else {
            return Ok(response(StatusCode::BAD_REQUEST, ""));
        };
        state
            .lock()
            .await
            .verifiers
            .insert(pairing_id, Verifiers { machine, phone });
        return Ok(response(StatusCode::NO_CONTENT, ""));
    }

    if method == Method::GET && path.starts_with("/channel/") {
        return Ok(upgrade(request, state).await);
    }

    Ok(response(StatusCode::NOT_FOUND, ""))
}

async fn upgrade(
    mut request: Request<Incoming>,
    state: Arc<Mutex<State>>,
) -> Response<Full<Bytes>> {
    let pairing_id = request
        .uri()
        .path()
        .strip_prefix("/channel/")
        .unwrap_or_default()
        .to_string();
    let role = query(&request, "role").unwrap_or_default();
    let token = query(&request, "token").unwrap_or_default();
    let verifier = hex_sha256(&token);
    let authorized = state
        .lock()
        .await
        .verifiers
        .get(&pairing_id)
        .map(|expected| match role.as_str() {
            "machine" => expected.machine == verifier,
            "phone" => expected.phone == verifier,
            _ => false,
        })
        .unwrap_or(false);
    if !authorized {
        return response(StatusCode::UNAUTHORIZED, "");
    }

    let Some(key) = request.headers().get(SEC_WEBSOCKET_KEY).cloned() else {
        return response(StatusCode::BAD_REQUEST, "");
    };
    let upgraded = hyper::upgrade::on(&mut request);
    let state_for_socket = Arc::clone(&state);
    tokio::spawn(async move {
        let Ok(upgraded) = upgraded.await else {
            return;
        };
        let socket =
            WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, None).await;
        serve_socket(socket, state_for_socket, pairing_id, role).await;
    });

    Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(UPGRADE, "websocket")
        .header(CONNECTION, "Upgrade")
        .header(SEC_WEBSOCKET_ACCEPT, derive_accept_key(key.as_bytes()))
        .body(Full::new(Bytes::new()))
        .unwrap()
}

async fn serve_socket<S>(
    socket: WebSocketStream<S>,
    state: Arc<Mutex<State>>,
    pairing_id: String,
    role: String,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (mut writer, mut reader) = socket.split();
    let (outgoing, mut incoming) = mpsc::unbounded_channel();
    {
        let mut state = state.lock().await;
        let peers = state.peers.entry(pairing_id.clone()).or_default();
        match role.as_str() {
            "machine" => {
                peers.machine = Some(outgoing);
                if peers.phone.is_some() {
                    let _ = peers.machine.as_ref().unwrap().send(Message::Text(
                        r#"{"relay":"peer","role":"phone","connected":true}"#.into(),
                    ));
                }
            }
            "phone" => {
                peers.phone = Some(outgoing);
                if let Some(machine) = &peers.machine {
                    let _ = machine.send(Message::Text(
                        r#"{"relay":"peer","role":"phone","connected":true}"#.into(),
                    ));
                }
            }
            _ => return,
        }
    }

    loop {
        tokio::select! {
            message = incoming.recv() => {
                let Some(message) = message else { return };
                if writer.send(message).await.is_err() {
                    return;
                }
            }
            message = reader.next() => {
                let Some(Ok(message)) = message else { return };
                let destination = {
                    let state = state.lock().await;
                    state.peers.get(&pairing_id).and_then(|peers| match role.as_str() {
                        "machine" => peers.phone.clone(),
                        "phone" => peers.machine.clone(),
                        _ => None,
                    })
                };
                if let Some(destination) = destination {
                    let _ = destination.send(message);
                }
            }
        }
    }
}

async fn json_body(request: Request<Incoming>) -> Value {
    let body = request.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&body).unwrap_or(Value::Null)
}

fn string(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_string)
}

fn query(request: &Request<Incoming>, name: &str) -> Option<String> {
    request.uri().query()?.split('&').find_map(|part| {
        let (key, value) = part.split_once('=')?;
        (key == name).then(|| value.to_string())
    })
}

fn hex_sha256(value: &str) -> String {
    sha2::Sha256::digest(value.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn json_response(value: Value) -> Response<Full<Bytes>> {
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(value.to_string())))
        .unwrap()
}

fn response(status: StatusCode, body: &'static str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .body(Full::new(Bytes::from_static(body.as_bytes())))
        .unwrap()
}
