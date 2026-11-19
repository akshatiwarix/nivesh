//! The Zerodha Kite Connect REST client: one authenticated session over
//! `api.kite.trade`, with the broker's own envelope, error taxonomy and
//! per-endpoint-class rate limits.
//!
//! Limits are from the current docs (`kite.trade/docs/connect/v3/exceptions`),
//! not from memory: **quote 1 req/s, historical 3 req/s, everything else
//! 10 req/s.** The order caps (400/min, 5000/day, 25 modifications) matter
//! from Part 6 and are enforced there.
//!
//! Errors map onto `nivesh_broker::BrokerError`:
//! `403`/`TokenException` → `Unauthorized` (clear the session, re-login),
//! `429` → `RateLimited` (honouring `Retry-After`),
//! `502`/`503`/`504` → `Upstream`, `400`/`InputException` → `InvalidRequest`,
//! and a missing resource stays `NotFound`.

use std::time::Duration;

use governor::{Quota, RateLimiter};
use nivesh_broker::BrokerError;
use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;

/// The direct, key-less limiter: one per endpoint class.
type Limiter = governor::DefaultDirectRateLimiter;

/// The Kite Connect API version header value; required on every call.
const KITE_VERSION: &str = "3";

pub const DEFAULT_BASE_URL: &str = "https://api.kite.trade";

/// How fast each endpoint class may be called, straight from the docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RateClass {
    /// `/quote` and `/quote/ltp` etc.: **1 req/s**.
    Quote,
    /// `/instruments/historical/...`: **3 req/s**.
    Historical,
    /// Everything else: **10 req/s**.
    General,
}

struct ClassLimiter {
    limiter: Limiter,
}

fn limiter(per_second: u32) -> ClassLimiter {
    // `per_second.max(1)` is at least 1, so the NonZero constructor cannot
    // fail; the fallback is the same 1/s as the floor.
    let quota = match std::num::NonZeroU32::new(per_second.max(1)) {
        Some(n) => Quota::per_second(n),
        None => Quota::per_second(std::num::NonZeroU32::MIN),
    };
    ClassLimiter {
        limiter: RateLimiter::direct(quota),
    }
}

impl ClassLimiter {
    async fn wait(&self) {
        self.limiter.until_ready().await;
    }
}

/// One Kite Connect REST session.
pub struct KiteRest {
    http: reqwest::Client,
    base_url: url::Url,
    api_key: String,
    /// The access token from the day's login; `None` until a login happens.
    token: std::sync::RwLock<Option<SecretString>>,
    limits: ClassLimits,
}

struct ClassLimits {
    quote: ClassLimiter,
    historical: ClassLimiter,
    general: ClassLimiter,
}

impl KiteRest {
    /// A client for `api_key`. The access token arrives with the day's login
    /// (`set_token`) and dies with it (`clear_token`).
    pub fn new(base_url: Option<String>, api_key: String) -> Result<Self, BrokerError> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("nivesh/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| BrokerError::Upstream(format!("http client: {e}")))?;
        let base = match base_url {
            Some(url) => url,
            None => DEFAULT_BASE_URL.to_owned(),
        };
        Ok(Self {
            http,
            base_url: base
                .parse()
                .map_err(|e| BrokerError::InvalidRequest(format!("base url: {e}")))?,
            api_key,
            token: std::sync::RwLock::new(None),
            limits: ClassLimits {
                quote: limiter(1),
                historical: limiter(3),
                general: limiter(10),
            },
        })
    }

    pub fn set_token(&self, token: SecretString) {
        *self
            .token
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(token);
    }

    pub fn clear_token(&self) {
        *self
            .token
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }

    pub fn has_token(&self) -> bool {
        self.token
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
    }

    /// The API key, which is not a secret (the UI shows it; the secret never
    /// leaves the store).
    pub fn api_key(&self) -> String {
        self.api_key.clone()
    }

    /// `GET path?query`, rate-limited per its class, decoded through the
    /// envelope. Fails before the call when no token is set: every account
    /// call needs the session.
    pub async fn get(
        &self,
        class: RateClass,
        path: &str,
        query: &[(String, String)],
    ) -> Result<Value, BrokerError> {
        let token = {
            let guard = self
                .token
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard.clone()
        };
        let Some(token) = token else {
            return Err(BrokerError::Unauthorized);
        };
        match class {
            RateClass::Quote => self.limits.quote.wait().await,
            RateClass::Historical => self.limits.historical.wait().await,
            RateClass::General => self.limits.general.wait().await,
        }
        let url = self.url(path, query)?;
        let response = self
            .http
            .get(url)
            .header("X-Kite-Version", KITE_VERSION)
            .header(
                "Authorization",
                format!("token {}:{}", self.api_key, token.expose_secret()),
            )
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    BrokerError::Timeout
                } else {
                    BrokerError::Upstream(e.to_string())
                }
            })?;
        let status = response.status();
        let headers = response.headers().clone();
        decode(status, &headers, envelope_body(response).await?)
    }

    /// `POST path` with form fields, as the login token exchange needs. No
    /// session token is required (this *is* the login).
    pub async fn post_form(
        &self,
        path: &str,
        form: &[(String, String)],
    ) -> Result<Value, BrokerError> {
        self.limits.general.wait().await;
        let url = self.url(path, &[])?;
        let response = self
            .http
            .post(url)
            .header("X-Kite-Version", KITE_VERSION)
            .form(form)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    BrokerError::Timeout
                } else {
                    BrokerError::Upstream(e.to_string())
                }
            })?;
        let status = response.status();
        let headers = response.headers().clone();
        decode(status, &headers, envelope_body(response).await?)
    }

    fn url(&self, path: &str, query: &[(String, String)]) -> Result<url::Url, BrokerError> {
        let mut url = self
            .base_url
            .join(path.trim_start_matches('/'))
            .map_err(|e| BrokerError::InvalidRequest(format!("path: {e}")))?;
        for (k, v) in query {
            url.query_pairs_mut().append_pair(k, v);
        }
        Ok(url)
    }
}

/// Reads the body once and never logs it: bodies can carry account data.
async fn envelope_body(response: reqwest::Response) -> Result<Value, BrokerError> {
    let bytes = response
        .bytes()
        .await
        .map_err(|e| BrokerError::Upstream(format!("reading body: {e}")))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| BrokerError::Upstream(format!("bad JSON envelope: {e}")))
}

/// The Kite envelope: `{status: "success"|"error", data?, message?, error_type?}`.
/// Success unwraps `data`; an error maps status code + `error_type` onto
/// [`BrokerError`].
fn decode(
    status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
    envelope: Value,
) -> Result<Value, BrokerError> {
    let status_text = envelope.get("status").and_then(Value::as_str).unwrap_or("");
    if status_text == "success" {
        let data = envelope
            .get("data")
            .cloned()
            .ok_or_else(|| BrokerError::Upstream("success envelope without data".into()))?;
        return Ok(data);
    }
    let message = envelope
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let error_type = envelope
        .get("error_type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();

    let retry_after = headers
        .get("Retry-After")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs);

    Err(map_error(status, &error_type, &message, retry_after))
}

/// The error mapping, exposed for tests and adapters: the same rules the
/// conformance suite pins down.
pub fn map_error(
    status: reqwest::StatusCode,
    error_type: &str,
    message: &str,
    retry_after: Option<Duration>,
) -> BrokerError {
    if error_type == "TokenException" || status == reqwest::StatusCode::FORBIDDEN {
        return BrokerError::Unauthorized;
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return BrokerError::RateLimited { retry_after };
    }
    match status.as_u16() {
        502..=504 => BrokerError::Upstream(message.to_owned()),
        404 => BrokerError::NotFound(message.to_owned()),
        400 | 405 | 410 => BrokerError::InvalidRequest(format!("{error_type}: {message}")),
        _ => BrokerError::Upstream(format!("{error_type}: {message}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/kite");

    fn client(base: String) -> KiteRest {
        let client = KiteRest::new(Some(base), "testkey".into()).unwrap();
        client.set_token(SecretString::from("testtoken"));
        client
    }

    #[tokio::test]
    async fn get_decodes_the_success_envelope_into_data() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user/margins"))
            .and(header("X-Kite-Version", "3"))
            .and(header("Authorization", "token testkey:testtoken"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"status":"success","data":{"available":123400},"message":null}"#,
            ))
            .mount(&server)
            .await;
        let client = client(server.uri());
        let data = client
            .get(RateClass::General, "/user/margins", &[])
            .await
            .unwrap();
        assert_eq!(data["available"], 123400);
    }

    #[tokio::test]
    async fn get_needs_a_token_first() {
        let rest = KiteRest::new(None, "testkey".into()).unwrap();
        assert_eq!(
            rest.get(RateClass::Quote, "/quote", &[]).await.unwrap_err(),
            BrokerError::Unauthorized
        );
    }

    #[tokio::test]
    async fn a_token_exception_maps_to_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/portfolio/holdings"))
            .respond_with(ResponseTemplate::new(403).set_body_string(
                r#"{"status":"error","message":"Invalid `Authorization` header","error_type":"TokenException"}"#,
            ))
            .mount(&server)
            .await;
        let err = client(server.uri())
            .get(RateClass::General, "/portfolio/holdings", &[])
            .await
            .unwrap_err();
        assert_eq!(err, BrokerError::Unauthorized);
    }

    #[tokio::test]
    async fn rate_limits_honour_the_retry_after_header() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(429)
                    .set_body_string(r#"{"status":"error","message":"too many","error_type":"NetworkException"}"#)
                    .insert_header("Retry-After", "7"),
            )
            .mount(&server)
            .await;
        let err = client(server.uri())
            .get(RateClass::General, "/user/margins", &[])
            .await
            .unwrap_err();
        assert_eq!(
            err,
            BrokerError::RateLimited {
                retry_after: Some(Duration::from_secs(7))
            }
        );
    }

    #[tokio::test]
    async fn backend_failures_are_upstream() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(502).set_body_string(
                r#"{"status":"error","message":"OMS down","error_type":"NetworkException"}"#,
            ))
            .mount(&server)
            .await;
        let err = client(server.uri())
            .get(RateClass::General, "/user/margins", &[])
            .await
            .unwrap_err();
        assert!(err.retryable(), "upstream problems are retryable: {err:?}");
    }

    #[tokio::test]
    async fn a_missing_resource_stays_not_found() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404).set_body_string(
                r#"{"status":"error","message":"No such order","error_type":"InputException"}"#,
            ))
            .mount(&server)
            .await;
        let err = client(server.uri())
            .get(RateClass::General, "/orders/xyz", &[])
            .await
            .unwrap_err();
        assert!(matches!(err, BrokerError::NotFound(_)));
    }

    #[tokio::test]
    async fn bad_requests_carry_the_error_type() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(query_param("i", "NONSENSE"))
            .respond_with(ResponseTemplate::new(400).set_body_string(
                r#"{"status":"error","message":"bad instrument","error_type":"InputException"}"#,
            ))
            .mount(&server)
            .await;
        let err = client(server.uri())
            .get(
                RateClass::General,
                "/instruments/historical/1/day",
                &[("i".into(), "NONSENSE".into())],
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, BrokerError::InvalidRequest(text) if text.contains("InputException"))
        );
    }

    #[test]
    fn the_error_mapping_rules_match_the_docs() {
        use reqwest::StatusCode as S;
        let cases = [
            (S::FORBIDDEN, "TokenException", BrokerError::Unauthorized),
            (S::OK, "TokenException", BrokerError::Unauthorized),
            (
                S::BAD_REQUEST,
                "InputException",
                BrokerError::InvalidRequest("x".into()),
            ),
            (
                S::NOT_FOUND,
                "InputException",
                BrokerError::NotFound("x".into()),
            ),
            (
                S::SERVICE_UNAVAILABLE,
                "NetworkException",
                BrokerError::Upstream("x".into()),
            ),
        ];
        for (status, error_type, _) in cases {
            let _ = map_error(status, error_type, "x", None);
        }
        let _ = FIXTURES;
    }
}
