//! The Kite login flow: the browser opens Kite's login page, Kite redirects
//! to the app's registered loopback URL with a `request_token`, and the app
//! exchanges that token — signed with the SHA-256 checksum
//! `api_key + request_token + api_secret` — for the day's access token,
//! which goes straight into the secret store (the keychain in production).
//!
//! Kite does not echo a `state` nonce through the redirect, so the flow's
//! protection is the registered loopback redirect URL plus a one-shot
//! listener that accepts exactly one callback and then closes.

use std::time::Duration;

use async_trait::async_trait;
use nivesh_broker::{
    BrokerAuth, BrokerError, LoginCallback, LoginState, SessionInfo, SessionState, SessionStatus,
};
use nivesh_secrets::{InMemorySecretStore, SecretStore};
use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest, Sha256};
use url::Url;

use crate::rest::KiteRest;

/// Where Kite's login page lives; the user opens this in their browser.
pub const LOGIN_PAGE: &str = "https://kite.zerodha.com/connect/login";

/// How long the loopback listener waits for the redirect.
pub const LISTEN_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Kite flushes access tokens at the daily reset, around 06:00 IST the next
/// morning; the session treats that as its expiry.
const TOKEN_FLUSH_UTC: (u32, u32) = (0, 30); // 06:00 IST is 00:30 UTC

/// Keys in the secret store. The API secret is entered once and stored; the
/// access token arrives with each login and is replaced daily.
pub const KEY_API_SECRET: &str = "kite.api_secret";
pub const KEY_ACCESS_TOKEN: &str = "kite.access_token";

/// The login half of the Kite adapter.
pub struct KiteLogin {
    rest: std::sync::Arc<KiteRest>,
    /// Secrets (api_secret in, access_token out). Tests use the memory store;
    /// production uses the keychain.
    secrets: std::sync::Arc<dyn SecretStore>,
    /// The redirect port the developer app registered, e.g. 7801.
    redirect_port: u16,
    redirect_path: String,
}

impl KiteLogin {
    /// A keychain problem (locked, denied, absent) surfaces as an upstream
    /// problem: it is a real failure the user may be able to fix, and
    /// retrying is reasonable once the keychain is unlocked.
    fn secret_err(e: nivesh_secrets::SecretError) -> BrokerError {
        BrokerError::Upstream(format!("secret store: {e}"))
    }

    /// Wires the flow around the REST client and a secret store. The API
    /// secret must already be stored under [`KEY_API_SECRET`] (the setup
    /// command, Part 4h, puts it there).
    pub fn new(
        rest: std::sync::Arc<KiteRest>,
        secrets: std::sync::Arc<dyn SecretStore>,
        redirect_port: u16,
        redirect_path: impl Into<String>,
    ) -> Self {
        Self {
            rest,
            secrets,
            redirect_port,
            redirect_path: redirect_path.into(),
        }
    }

    /// The redirect URL the developer app must register, shown in the docs
    /// and in the setup UI so the user can copy it.
    pub fn registered_redirect_url(&self) -> Result<Url, BrokerError> {
        let mut url = Url::parse("http://127.0.0.1")
            .map_err(|e| BrokerError::InvalidRequest(format!("loopback base: {e}")))?;
        url.set_port(Some(self.redirect_port)).map_err(|_| {
            BrokerError::InvalidRequest(format!("bad redirect port {}", self.redirect_port))
        })?;
        url.set_path(&self.redirect_path);
        Ok(url)
    }

    /// The checksum that signs the token exchange:
    /// SHA-256 of `api_key + request_token + api_secret`, hex-encoded.
    pub fn checksum(api_key: &str, request_token: &str, api_secret: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(api_key.as_bytes());
        hasher.update(request_token.as_bytes());
        hasher.update(api_secret.as_bytes());
        let digest = hasher.finalize();
        digest.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Exchanges `request_token` for the day's session and stores the access
    /// token under [`KEY_ACCESS_TOKEN`]. The API secret comes from the store
    /// under [`KEY_API_SECRET`]; without it the exchange cannot be signed and
    /// login fails cleanly.
    async fn exchange(&self, request_token: &str) -> Result<SessionInfo, BrokerError> {
        let api_secret = self
            .secrets
            .get(KEY_API_SECRET)
            .map_err(Self::secret_err)?
            .ok_or_else(|| {
                BrokerError::InvalidRequest(
                    "no Kite API secret stored; run the broker setup first".into(),
                )
            })?;
        let checksum = Self::checksum(
            &self.rest.api_key(),
            request_token,
            api_secret.expose_secret(),
        );
        let form = [
            ("api_key".to_owned(), self.rest.api_key()),
            ("request_token".to_owned(), request_token.to_owned()),
            ("checksum".to_owned(), checksum),
        ];
        let data = self.rest.post_form("/session/token", &form).await?;
        let session = parse_session(&data)?;
        self.rest.set_token(SecretString::from(
            session.access_token.expose_secret().to_owned(),
        ));
        self.secrets
            .put(KEY_ACCESS_TOKEN, session.access_token.clone())
            .map_err(Self::secret_err)?;
        Ok(session)
    }

    /// Binds the registered loopback address and waits for the redirect,
    /// answering the browser and returning the callback it delivered.
    /// One-shot: the listener is dropped afterwards, so only the login that
    /// started it can complete.
    pub async fn wait_for_redirect(&self) -> Result<LoginCallback, BrokerError> {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", self.redirect_port))
            .await
            .map_err(|e| {
                BrokerError::InvalidRequest(format!(
                    "cannot listen on 127.0.0.1:{} (is another login in progress?): {e}",
                    self.redirect_port
                ))
            })?;
        let (mut socket, _) = tokio::time::timeout(LISTEN_TIMEOUT, listener.accept())
            .await
            .map_err(|_| BrokerError::Timeout)?
            .map_err(|e| BrokerError::Upstream(format!("accept: {e}")))?;
        let request = read_request(&mut socket).await?;
        answer_browser(&mut socket).await;
        parse_redirect(&request, &self.redirect_path)
    }

    /// Restores today's session from the secret store: the token back into
    /// the REST client, its computed expiry into the status.
    pub fn restore_session(&self) -> Result<SessionStatus, BrokerError> {
        match self
            .secrets
            .get(KEY_ACCESS_TOKEN)
            .map_err(Self::secret_err)?
        {
            Some(token) => {
                self.rest.set_token(token);
                Ok(SessionStatus {
                    state: SessionState::Connected,
                    expires_at: Some(next_token_flush()),
                })
            }
            None => Ok(SessionStatus {
                state: SessionState::LoggedOut,
                expires_at: None,
            }),
        }
    }

    /// Drops the stored token and clears the REST client's copy.
    pub fn forget_session(&self) -> Result<(), BrokerError> {
        self.rest.clear_token();
        self.secrets
            .delete(KEY_ACCESS_TOKEN)
            .map_err(Self::secret_err)?;
        Ok(())
    }
}

/// Reads one HTTP request from the socket (Kite's redirect is a plain GET
/// with a query), stopping at the header blank line. Does not close the
/// socket; [`answer_browser`] replies and closes.
async fn read_request(socket: &mut tokio::net::TcpStream) -> Result<String, BrokerError> {
    use tokio::io::AsyncReadExt;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let n = socket
            .read(&mut chunk)
            .await
            .map_err(|e| BrokerError::Upstream(format!("reading redirect: {e}")))?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") || buf.len() > 16 * 1024 {
            break;
        }
    }
    String::from_utf8(buf).map_err(|_| BrokerError::InvalidRequest("redirect was not UTF-8".into()))
}

/// Answers the browser so its tab closes cleanly, whatever the callback says.
async fn answer_browser(socket: &mut tokio::net::TcpStream) {
    use tokio::io::AsyncWriteExt;
    let body = "Nivesh: login received. You can close this window.";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = socket.write_all(response.as_bytes()).await;
    let _ = socket.shutdown().await;
}

/// Parses the redirect request line: `GET <path>?status=..&request_token=..`.
/// Anything that is not the registered path, or carries no token, is an
/// error the UI shows — never silently accepted.
fn parse_redirect(request: &str, path: &str) -> Result<LoginCallback, BrokerError> {
    let request_line = request.lines().next().unwrap_or_default();
    let url_part = request_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| BrokerError::InvalidRequest("empty redirect request".into()))?;
    let (seen_path, query) = url_part.split_once('?').unwrap_or((url_part, ""));
    if seen_path.trim_end_matches('/') != path.trim_end_matches('/') {
        return Err(BrokerError::InvalidRequest(format!(
            "redirect to unexpected path {seen_path:?}"
        )));
    }
    let mut request_token = None;
    let mut status_ok = false;
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        match k {
            "request_token" if !v.is_empty() => request_token = Some(form_value(v)),
            "status" => status_ok = v == "success",
            _ => {}
        }
    }
    let Some(request_token) = request_token else {
        return Err(BrokerError::InvalidRequest(
            "redirect carried no request_token".into(),
        ));
    };
    Ok(LoginCallback {
        request_token,
        state: if status_ok {
            "success".into()
        } else {
            String::new()
        },
    })
}

/// Decodes one form value (`%XX` and `+` as space).
fn form_value(value: &str) -> String {
    url::form_urlencoded::parse(format!("x={value}").as_bytes())
        .next()
        .map(|(_, v)| v.into_owned())
        .unwrap_or_else(|| value.to_owned())
}

/// Decodes the token exchange response into a session. Kite reports no
/// explicit expiry, so the session carries the next daily flush (06:00 IST).
fn parse_session(data: &serde_json::Value) -> Result<SessionInfo, BrokerError> {
    let access_token = data
        .get("access_token")
        .and_then(serde_json::Value::as_str)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| BrokerError::Upstream("token exchange had no access_token".into()))?;
    let user_id = data
        .get("user_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    Ok(SessionInfo {
        access_token: SecretString::from(access_token.to_owned()),
        expires_at: next_token_flush(),
        user_id,
    })
}

/// The next 06:00 IST (00:30 UTC) strictly after `now`, when Kite flushes
/// tokens. When the clock or calendar misbehaves, a day from now.
pub fn next_token_flush_value() -> chrono::DateTime<chrono::Utc> {
    next_token_flush()
}

fn next_token_flush() -> chrono::DateTime<chrono::Utc> {
    use chrono::{Duration, NaiveTime, Utc};
    let now = Utc::now();
    let flush = match NaiveTime::from_hms_opt(TOKEN_FLUSH_UTC.0, TOKEN_FLUSH_UTC.1, 0) {
        Some(t) => t,
        None => return now + Duration::hours(24),
    };
    let candidate = |day: chrono::NaiveDate| {
        day.and_time(flush)
            .and_local_timezone(Utc)
            .single()
            .unwrap_or(now + Duration::hours(24))
    };
    let today = candidate(now.date_naive());
    if today > now {
        today
    } else {
        match now.date_naive().succ_opt() {
            Some(tomorrow) => candidate(tomorrow),
            None => now + Duration::hours(24),
        }
    }
}

#[async_trait]
impl BrokerAuth for KiteLogin {
    fn login_url(&self, _state: &LoginState) -> Result<Url, BrokerError> {
        let mut url = Url::parse(LOGIN_PAGE)
            .map_err(|e| BrokerError::InvalidRequest(format!("login page: {e}")))?;
        url.query_pairs_mut()
            .append_pair("v", "3")
            .append_pair("api_key", &self.rest.api_key());
        Ok(url)
    }

    async fn complete_login(&self, callback: LoginCallback) -> Result<SessionInfo, BrokerError> {
        self.exchange(&callback.request_token).await
    }

    async fn logout(&self) -> Result<(), BrokerError> {
        // Kite invalidates the day's token server-side; the local copy goes too.
        self.rest
            .post_form("/session/invalidate_token", &[])
            .await
            .ok();
        self.forget_session()
    }
}

/// The login wired against an in-memory secret store (tests and demos).
pub fn memory_login(rest: std::sync::Arc<KiteRest>, redirect_port: u16) -> KiteLogin {
    KiteLogin::new(
        rest,
        std::sync::Arc::new(InMemorySecretStore::new()),
        redirect_port,
        "/nivesh/login",
    )
}

#[cfg(test)]
fn memory_secrets() -> std::sync::Arc<dyn SecretStore> {
    std::sync::Arc::new(InMemorySecretStore::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nivesh_broker::BrokerError;
    use std::sync::Arc;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const GOLDEN_CHECKSUM: &str =
        "9d682da34f8bd7c827ea118f8a2e2365f89d2f7718651a2970b0e8969a20f02f";

    #[test]
    fn the_checksum_is_sha256_of_key_token_secret() {
        assert_eq!(
            KiteLogin::checksum("testkey", "token", "secret"),
            GOLDEN_CHECKSUM
        );
    }

    #[test]
    fn the_login_url_carries_the_api_key() {
        let rest = Arc::new(KiteRest::new(None, "testkey".into()).unwrap());
        let login = KiteLogin::new(rest, memory_secrets(), 7801, "/nivesh/login");
        let url = login.login_url(&LoginState("x".into())).unwrap();
        assert_eq!(
            url.as_str(),
            "https://kite.zerodha.com/connect/login?v=3&api_key=testkey"
        );
    }

    #[test]
    fn the_redirect_url_is_what_the_app_registers() {
        let rest = Arc::new(KiteRest::new(None, "k".into()).unwrap());
        let login = KiteLogin::new(rest, memory_secrets(), 7801, "/nivesh/login");
        assert_eq!(
            login.registered_redirect_url().unwrap().as_str(),
            "http://127.0.0.1:7801/nivesh/login"
        );
    }

    #[test]
    fn parses_a_successful_redirect() {
        let request = "GET /nivesh/login?status=success&request_token=abc123 HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        let callback = parse_redirect(request, "/nivesh/login").unwrap();
        assert_eq!(callback.request_token, "abc123");
        assert_eq!(callback.state, "success");
    }

    #[test]
    fn decodes_percent_and_plus_in_the_token() {
        let request = "GET /nivesh/login?status=success&request_token=a+b%2Fc HTTP/1.1\r\n\r\n";
        let callback = parse_redirect(request, "/nivesh/login").unwrap();
        assert_eq!(callback.request_token, "a b/c");
    }

    #[test]
    fn rejects_redirects_that_are_not_ours() {
        let wrong_path = "GET /evil?status=success&request_token=x HTTP/1.1\r\n\r\n";
        assert!(parse_redirect(wrong_path, "/nivesh/login").is_err());
        let no_token = "GET /nivesh/login?status=success HTTP/1.1\r\n\r\n";
        assert!(parse_redirect(no_token, "/nivesh/login").is_err());
        let empty = "";
        assert!(parse_redirect(empty, "/nivesh/login").is_err());
    }

    #[tokio::test]
    async fn exchange_signs_the_checksum_and_stores_the_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/session/token"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"status":"success","data":{"access_token":"day-token","user_id":"XY1234"}}"#,
            ))
            .mount(&server)
            .await;
        let rest = Arc::new(KiteRest::new(Some(server.uri()), "testkey".into()).unwrap());
        let secrets = memory_secrets();
        secrets
            .put(KEY_API_SECRET, SecretString::from("secret"))
            .unwrap();
        let login = KiteLogin::new(rest, secrets.clone(), 7801, "/nivesh/login");
        let session = login
            .complete_login(LoginCallback {
                request_token: "token".into(),
                state: "success".into(),
            })
            .await
            .unwrap();
        assert_eq!(session.user_id, "XY1234");
        // The token went to the secret store and the client.
        assert_eq!(
            secrets
                .get(KEY_ACCESS_TOKEN)
                .unwrap()
                .unwrap()
                .expose_secret(),
            "day-token"
        );
        assert!(login.rest.has_token());

        // The signed form fields were exactly api_key + request_token + checksum.
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        let body = String::from_utf8(requests[0].body.clone()).unwrap();
        let expected_checksum = KiteLogin::checksum("testkey", "token", "secret");
        assert!(body.contains("api_key=testkey"), "{body}");
        assert!(body.contains("request_token=token"), "{body}");
        assert!(
            body.contains(&format!("checksum={expected_checksum}")),
            "{body}"
        );
    }

    #[tokio::test]
    async fn exchange_without_a_stored_secret_fails_cleanly() {
        let server = MockServer::start().await;
        let rest = Arc::new(KiteRest::new(Some(server.uri()), "testkey".into()).unwrap());
        let login = KiteLogin::new(rest, memory_secrets(), 7801, "/nivesh/login");
        let err = login
            .complete_login(LoginCallback {
                request_token: "token".into(),
                state: String::new(),
            })
            .await
            .unwrap_err();
        assert!(matches!(err, BrokerError::InvalidRequest(text) if text.contains("API secret")));
    }

    #[tokio::test]
    async fn restore_and_forget_move_the_session_state() {
        let rest = Arc::new(KiteRest::new(None, "k".into()).unwrap());
        let secrets = memory_secrets();
        let login = KiteLogin::new(rest, secrets.clone(), 7801, "/nivesh/login");

        let status = login.restore_session().unwrap();
        assert_eq!(status.state, SessionState::LoggedOut);

        secrets
            .put(KEY_ACCESS_TOKEN, SecretString::from("day-token"))
            .unwrap();
        let status = login.restore_session().unwrap();
        assert_eq!(status.state, SessionState::Connected);
        assert!(login.rest.has_token());
        // The expiry is the next daily flush, 06:00 IST = 00:30 UTC.
        let expiry = status.expires_at.unwrap();
        assert_eq!(expiry.format("%H:%M").to_string(), "00:30");
        let now = chrono::Utc::now();
        assert!(expiry > now, "expiry is in the future");
        assert!(expiry - now <= chrono::Duration::hours(24) + chrono::Duration::minutes(30));

        login.forget_session().unwrap();
        assert!(!login.rest.has_token());
        assert!(secrets.get(KEY_ACCESS_TOKEN).unwrap().is_none());
    }

    #[tokio::test]
    async fn the_listener_answers_and_hands_over_the_callback() {
        // Take an ephemeral port, free it, and use it for the one-shot listener.
        let probe = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);

        let rest = Arc::new(KiteRest::new(None, "k".into()).unwrap());
        let login = KiteLogin::new(rest, memory_secrets(), port, "/nivesh/login");
        let waiter = tokio::spawn(async move { login.wait_for_redirect().await });

        // Let the listener bind first.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        socket
            .write_all(
                b"GET /nivesh/login?status=success&request_token=tok1 HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
            )
            .await
            .unwrap();
        let mut response = Vec::new();
        let _ = socket.read_to_end(&mut response).await;
        let response = String::from_utf8_lossy(&response);

        let callback = waiter.await.unwrap().unwrap();
        assert_eq!(callback.request_token, "tok1");
        assert!(
            response.contains("200 OK") && response.contains("close this window"),
            "the browser gets a clean answer: {response}"
        );
    }
}
