//! The loopback page the browser is sent back to once the user has authorized micro.

use tokio::io::AsyncReadExt as _;
use tokio::io::AsyncWriteExt as _;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

/// The longest request line and headers read from the browser.
const MAX_REQUEST_BYTES: usize = 16 * 1024;

/// What the authorization server sent back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Callback {
    pub code: String,
    /// The `iss` parameter (RFC 9207), when the server sends one.
    pub iss: Option<String>,
}

/// A server on a loopback port, waiting for one authorization response.
pub struct CallbackServer {
    redirect_uri: String,
    answer: oneshot::Receiver<Result<Callback, String>>,
    task: JoinHandle<()>,
}

impl Drop for CallbackServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl CallbackServer {
    /// Listen on `host` at `port`, or a free port. The redirect URI names `redirect_host`, which
    /// may be `localhost` while the server listens on `127.0.0.1`. Only a response carrying
    /// `state` is accepted.
    pub async fn listen(
        host: &str,
        redirect_host: &str,
        port: Option<u16>,
        path: &str,
        state: String,
    ) -> std::io::Result<CallbackServer> {
        let listener = tokio::net::TcpListener::bind((host, port.unwrap_or(0))).await?;
        let bound = listener.local_addr()?.port();
        let shown_host = match redirect_host.contains(':') && !redirect_host.starts_with('[') {
            true => format!("[{redirect_host}]"),
            false => redirect_host.to_string(),
        };
        let redirect_uri = format!("http://{shown_host}:{bound}{path}");

        let (sender, answer) = oneshot::channel();
        let path = path.to_string();
        let task = tokio::spawn(async move {
            let mut sender = Some(sender);
            while let Ok((stream, _)) = listener.accept().await {
                if let Some(outcome) = serve(stream, &path, &state).await {
                    if let Some(sender) = sender.take() {
                        let _ = sender.send(outcome);
                    }
                    return;
                }
            }
        });

        Ok(CallbackServer {
            redirect_uri,
            answer,
            task,
        })
    }

    pub fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }

    /// Wait for the browser, or give up after `within`.
    pub async fn wait(&mut self, within: std::time::Duration) -> Result<Callback, String> {
        match tokio::time::timeout(within, &mut self.answer).await {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(_)) => Err("the sign-in page closed".to_string()),
            Err(_) => Err("timed out waiting for the browser".to_string()),
        }
    }
}

/// Answer one request. `None` means it was not the response being waited for.
async fn serve(
    mut stream: tokio::net::TcpStream,
    path: &str,
    state: &str,
) -> Option<Result<Callback, String>> {
    let mut request = Vec::new();
    let mut buffer = [0u8; 2048];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        match stream.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(read) => request.extend_from_slice(&buffer[..read]),
        }
        if request.len() > MAX_REQUEST_BYTES {
            break;
        }
    }
    let text = String::from_utf8_lossy(&request);
    let target = text
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");
    let url = reqwest::Url::parse(&format!("http://localhost{target}")).ok()?;
    if url.path() != path {
        reply(&mut stream, 404, "Not found", None).await;
        return None;
    }
    let parameter = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    };
    if parameter("state").as_deref() != Some(state) {
        reply(&mut stream, 400, "Invalid or expired sign-in state.", None).await;
        return None;
    }
    if let Some(error) = parameter("error") {
        let description = parameter("error_description").unwrap_or(error);
        reply(
            &mut stream,
            200,
            "Authorization failed. You may close this window.",
            Some(&description),
        )
        .await;
        return Some(Err(description));
    }
    let Some(code) = parameter("code") else {
        reply(&mut stream, 400, "Missing authorization code.", None).await;
        return Some(Err("the authorization response has no code".to_string()));
    };
    reply(
        &mut stream,
        200,
        "Signed in to the MCP server. You may close this window.",
        None,
    )
    .await;
    Some(Ok(Callback {
        code,
        iss: parameter("iss"),
    }))
}

async fn reply(
    stream: &mut tokio::net::TcpStream,
    status: u16,
    message: &str,
    details: Option<&str>,
) {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        _ => "Not Found",
    };
    let details = details
        .map(|details| format!("<p>{}</p>", escape(details)))
        .unwrap_or_default();
    let body = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>micro</title></head>\
         <body style=\"font-family: sans-serif; margin: 3em\"><p>{}</p>{details}</body></html>",
        escape(message)
    );
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\n\
         Cache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn visit(url: &str) -> u16 {
        reqwest::get(url).await.unwrap().status().as_u16()
    }

    #[tokio::test]
    async fn only_the_matching_state_is_taken() {
        let server =
            CallbackServer::listen("127.0.0.1", "localhost", None, "/callback", "s1".into())
                .await
                .unwrap();
        let redirect = server.redirect_uri().to_string();
        assert!(redirect.starts_with("http://localhost:"), "{redirect}");
        let port = redirect
            .split(':')
            .nth(2)
            .unwrap()
            .split('/')
            .next()
            .unwrap()
            .to_string();

        assert_eq!(
            visit(&format!("http://127.0.0.1:{port}/elsewhere")).await,
            404
        );
        assert_eq!(
            visit(&format!(
                "http://127.0.0.1:{port}/callback?state=other&code=x"
            ))
            .await,
            400
        );
        assert_eq!(
            visit(&format!(
                "http://127.0.0.1:{port}/callback?state=s1&code=the-code&iss=https%3A%2F%2Fauth"
            ))
            .await,
            200
        );

        let mut server = server;
        let callback = server
            .wait(std::time::Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(callback.code, "the-code");
        assert_eq!(callback.iss.as_deref(), Some("https://auth"));
    }
}
