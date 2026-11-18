//! The shared HTTP client: per-host rate limits, timeouts, jittered backoff for
//! idempotent GETs, `Retry-After` handling and redacted logging.
//!
//! Only GET is exposed. Nothing here retries a non-idempotent request, and
//! nothing tries to evade blocking: a 403 is reported as
//! [`DataError::Blocked`], not worked around.

use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher as _, Hasher as _};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};

use governor::clock::{Clock, DefaultClock};
use governor::middleware::NoOpMiddleware;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Quota, RateLimiter};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, RETRY_AFTER};
use reqwest::{StatusCode, Url};
use serde::de::DeserializeOwned;

use crate::DataError;

/// An ordinary desktop-browser `User-Agent`. Providers such as NSE reject
/// requests without one. This is the only header "disguise" we use.
pub const BROWSER_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

/// Prefix of the error text for a failed connection (refused, unreachable, DNS).
/// Such failures aren't retried: the machine is offline or the host is down, and
/// the next poll is the better retry.
pub(crate) const CONNECT_FAILED: &str = "cannot connect";

/// A sustained request rate with a burst allowance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limit {
    /// Time between permits once the burst is spent.
    pub period: Duration,
    /// Permits available at once.
    pub burst: u32,
}

impl Limit {
    pub const fn new(period: Duration, burst: u32) -> Self {
        Self { period, burst }
    }

    fn quota(self) -> Quota {
        let period = self.period.max(Duration::from_millis(1));
        let burst = NonZeroU32::new(self.burst).unwrap_or(NonZeroU32::MIN);
        // `with_period` only fails for a zero period, which is clamped above.
        Quota::with_period(period)
            .unwrap_or_else(|| Quota::per_second(NonZeroU32::MIN))
            .allow_burst(burst)
    }
}

/// A token bucket for one host. Generic over the clock so tests can drive it
/// with a fake one.
pub struct RateGate<C: Clock + Clone = DefaultClock> {
    limiter: RateLimiter<NotKeyed, InMemoryState, C, NoOpMiddleware<C::Instant>>,
    clock: C,
}

impl RateGate<DefaultClock> {
    pub fn new(limit: Limit) -> Self {
        Self::with_clock(limit, DefaultClock::default())
    }
}

impl<C: Clock + Clone> RateGate<C> {
    pub fn with_clock(limit: Limit, clock: C) -> Self {
        Self {
            limiter: RateLimiter::direct_with_clock(limit.quota(), clock.clone()),
            clock,
        }
    }

    /// Takes a permit now, or says how long to wait for the next one.
    pub fn try_acquire(&self) -> Result<(), Duration> {
        self.limiter
            .check()
            .map_err(|not_until| not_until.wait_time_from(self.clock.now()))
    }
}

impl RateGate<DefaultClock> {
    /// Waits until a permit is available and takes it.
    pub async fn acquire(&self) {
        while let Err(wait) = self.try_acquire() {
            tokio::time::sleep(wait).await;
        }
    }
}

/// Per-host rate limits, shared by every client so that all providers hitting
/// one host draw from the same bucket.
pub struct HostLimits {
    default: Limit,
    hosts: HashMap<String, Arc<RateGate>>,
    fallback: Arc<RateGate>,
}

impl HostLimits {
    pub fn new(default: Limit) -> Self {
        Self {
            default,
            hosts: HashMap::new(),
            fallback: Arc::new(RateGate::new(default)),
        }
    }

    pub fn with_host(mut self, host: &str, limit: Limit) -> Self {
        self.hosts
            .insert(host.to_ascii_lowercase(), Arc::new(RateGate::new(limit)));
        self
    }

    /// Limits tuned for the hosts Nivesh talks to. NSE gets about one request
    /// per second with a small burst; Yahoo is a little more generous.
    pub fn defaults() -> Self {
        let nse = Limit::new(Duration::from_millis(1000), 3);
        Self::new(Limit::new(Duration::from_millis(500), 2))
            .with_host("www.nseindia.com", nse)
            .with_host("nsearchives.nseindia.com", nse)
            .with_host(
                "query1.finance.yahoo.com",
                Limit::new(Duration::from_millis(500), 4),
            )
            .with_host(
                "query2.finance.yahoo.com",
                Limit::new(Duration::from_millis(500), 4),
            )
    }

    pub fn default_limit(&self) -> Limit {
        self.default
    }

    fn gate(&self, host: &str) -> &Arc<RateGate> {
        self.hosts
            .get(&host.to_ascii_lowercase())
            .unwrap_or(&self.fallback)
    }
}

#[derive(Debug, Clone)]
pub struct HttpConfig {
    pub user_agent: String,
    pub timeout: Duration,
    pub connect_timeout: Duration,
    /// Total tries per request (1 = no retry).
    pub max_attempts: u32,
    /// First backoff delay; doubles each retry, plus jitter up to the same amount.
    pub backoff_base: Duration,
    /// A `Retry-After` longer than this is reported as `RateLimited` instead of
    /// being slept on.
    pub max_retry_after: Duration,
    /// Keep cookies between requests (NSE needs this).
    pub cookies: bool,
    /// Responses larger than this are rejected.
    pub max_body_bytes: usize,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            user_agent: BROWSER_USER_AGENT.to_owned(),
            timeout: Duration::from_secs(12),
            connect_timeout: Duration::from_secs(6),
            max_attempts: 3,
            backoff_base: Duration::from_millis(400),
            max_retry_after: Duration::from_secs(10),
            cookies: false,
            max_body_bytes: 16 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn json<T: DeserializeOwned>(&self) -> Result<T, DataError> {
        serde_json::from_slice(&self.body).map_err(|e| DataError::Parse(e.to_string()))
    }
}

/// A GET-only HTTP client with a cookie jar of its own (when enabled) and a
/// shared set of host limits.
#[derive(Clone)]
pub struct HttpClient {
    client: reqwest::Client,
    limits: Arc<HostLimits>,
    config: Arc<HttpConfig>,
}

impl HttpClient {
    pub fn new(config: HttpConfig, limits: Arc<HostLimits>) -> Result<Self, DataError> {
        let client = reqwest::Client::builder()
            .user_agent(&config.user_agent)
            .timeout(config.timeout)
            .connect_timeout(config.connect_timeout)
            .cookie_store(config.cookies)
            .build()
            .map_err(|e| {
                DataError::Upstream(format!("building http client: {}", e.without_url()))
            })?;
        Ok(Self {
            client,
            limits,
            config: Arc::new(config),
        })
    }

    /// GET with the given extra headers. Waits for the host's rate limit before
    /// every attempt, including retries.
    pub async fn get(
        &self,
        url: &str,
        headers: &[(&str, &str)],
    ) -> Result<HttpResponse, DataError> {
        let url = Url::parse(url).map_err(|e| DataError::Upstream(format!("bad url: {e}")))?;
        let host = url.host_str().unwrap_or_default().to_owned();
        let gate = Arc::clone(self.limits.gate(&host));
        let extra = build_headers(headers)?;
        let shown = redact_url(&url);

        let mut attempt = 0;
        loop {
            attempt += 1;
            gate.acquire().await;
            let started = Instant::now();
            let result = self.attempt(&url, &extra).await;
            let elapsed_ms = started.elapsed().as_millis();
            match &result {
                Ok(r) => {
                    tracing::debug!(url = %shown, status = r.status, elapsed_ms, attempt, "http get")
                }
                Err(e) => {
                    tracing::debug!(url = %shown, error = %e, elapsed_ms, attempt, "http get failed")
                }
            }
            let err = match result {
                Ok(r) => return Ok(r),
                Err(e) => e,
            };
            if attempt >= self.config.max_attempts {
                return Err(err);
            }
            match self.retry_delay(&err, attempt) {
                Some(delay) => tokio::time::sleep(delay).await,
                None => return Err(err),
            }
        }
    }

    /// How long to wait before retrying after `err`, or `None` if it isn't
    /// worth retrying.
    fn retry_delay(&self, err: &DataError, attempt: u32) -> Option<Duration> {
        let backoff = || {
            let base = self.config.backoff_base;
            let exp = base.saturating_mul(1 << (attempt - 1).min(6));
            exp + jitter(base)
        };
        match err {
            DataError::RateLimited {
                retry_after: Some(wait),
            } => (*wait <= self.config.max_retry_after).then_some(*wait),
            // Offline or host down: the next poll is the better retry.
            DataError::Upstream(message) if message.starts_with(CONNECT_FAILED) => None,
            DataError::RateLimited { retry_after: None }
            | DataError::Timeout
            | DataError::Upstream(_) => Some(backoff()),
            _ => None,
        }
    }

    async fn attempt(&self, url: &Url, headers: &HeaderMap) -> Result<HttpResponse, DataError> {
        let response = self
            .client
            .get(url.clone())
            .headers(headers.clone())
            .send()
            .await
            .map_err(map_transport_error)?;
        let status = response.status();
        let response_headers = response.headers().clone();
        if let Some(err) = status_error(status, &response_headers) {
            return Err(err);
        }
        if response
            .content_length()
            .is_some_and(|n| n as usize > self.config.max_body_bytes)
        {
            return Err(DataError::Upstream("response too large".into()));
        }
        let body = response.bytes().await.map_err(map_transport_error)?;
        if body.len() > self.config.max_body_bytes {
            return Err(DataError::Upstream("response too large".into()));
        }
        Ok(HttpResponse {
            status: status.as_u16(),
            headers: response_headers,
            body: body.to_vec(),
        })
    }
}

fn build_headers(headers: &[(&str, &str)]) -> Result<HeaderMap, DataError> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|e| DataError::Upstream(format!("bad header name: {e}")))?;
        let value = HeaderValue::from_str(value)
            .map_err(|e| DataError::Upstream(format!("bad header value: {e}")))?;
        map.insert(name, value);
    }
    Ok(map)
}

fn map_transport_error(err: reqwest::Error) -> DataError {
    // `without_url`: error text must never carry a query string.
    let err = err.without_url();
    if err.is_timeout() {
        DataError::Timeout
    } else if err.is_connect() {
        DataError::Upstream(format!("{CONNECT_FAILED}: {err}"))
    } else {
        DataError::Upstream(err.to_string())
    }
}

/// Maps a non-success status to the error taxonomy.
fn status_error(status: StatusCode, headers: &HeaderMap) -> Option<DataError> {
    if status.is_success() {
        return None;
    }
    Some(match status {
        StatusCode::NOT_FOUND => DataError::NotFound,
        StatusCode::UNAUTHORIZED => DataError::Unauthorized,
        StatusCode::FORBIDDEN => DataError::Blocked,
        StatusCode::TOO_MANY_REQUESTS => DataError::RateLimited {
            retry_after: parse_retry_after(headers),
        },
        s => DataError::Upstream(format!("HTTP {}", s.as_u16())),
    })
}

/// `Retry-After` in delta-seconds. HTTP-date values are ignored.
fn parse_retry_after(headers: &HeaderMap) -> Option<Duration> {
    let secs: u64 = headers
        .get(RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some(Duration::from_secs(secs))
}

/// A random duration in `0..max`, without a `rand` dependency.
fn jitter(max: Duration) -> Duration {
    let nanos = u64::try_from(max.as_nanos()).unwrap_or(u64::MAX).max(1);
    Duration::from_nanos(RandomState::new().build_hasher().finish() % nanos)
}

/// `host/path` with no scheme, userinfo, query or fragment: safe to log even
/// when the query carries a token.
pub fn redact_url(url: &Url) -> String {
    format!("{}{}", url.host_str().unwrap_or("?"), url.path())
}

#[cfg(test)]
mod tests {
    use governor::clock::FakeRelativeClock;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn fast_config() -> HttpConfig {
        HttpConfig {
            backoff_base: Duration::from_millis(2),
            timeout: Duration::from_secs(2),
            ..HttpConfig::default()
        }
    }

    fn client(config: HttpConfig) -> HttpClient {
        // A generous limit so tests that aren't about limiting never wait.
        let limits = HostLimits::new(Limit::new(Duration::from_millis(1), 1000));
        HttpClient::new(config, Arc::new(limits)).unwrap()
    }

    #[test]
    fn redaction_drops_query_userinfo_and_fragment() {
        let url = Url::parse("https://user:pw@api.example.com/v1/q?token=SECRET&x=1#frag").unwrap();
        let shown = redact_url(&url);
        assert_eq!(shown, "api.example.com/v1/q");
        assert!(!shown.contains("SECRET") && !shown.contains("pw"));
    }

    #[test]
    fn rate_gate_never_exceeds_burst_plus_rate_on_a_fake_clock() {
        let clock = FakeRelativeClock::default();
        let gate = RateGate::with_clock(Limit::new(Duration::from_millis(1000), 3), clock.clone());
        // Hammer it: 100 attempts per simulated second for 10 seconds.
        let mut granted = 0;
        for _tick in 0..1000 {
            if gate.try_acquire().is_ok() {
                granted += 1;
            }
            clock.advance(Duration::from_millis(10));
        }
        // burst (3) + one per second for the 10 seconds simulated.
        assert!((10..=13).contains(&granted), "granted {granted}");
    }

    #[test]
    fn rate_gate_reports_how_long_to_wait() {
        let clock = FakeRelativeClock::default();
        let gate = RateGate::with_clock(Limit::new(Duration::from_millis(500), 1), clock.clone());
        assert!(gate.try_acquire().is_ok());
        let wait = gate.try_acquire().unwrap_err();
        assert!(
            wait > Duration::from_millis(400) && wait <= Duration::from_millis(500),
            "{wait:?}"
        );
        clock.advance(wait);
        assert!(gate.try_acquire().is_ok());
    }

    #[test]
    fn hosts_have_separate_buckets_and_unknown_hosts_use_the_default() {
        let limits = HostLimits::new(Limit::new(Duration::from_secs(60), 1))
            .with_host("a.test", Limit::new(Duration::from_secs(60), 2));
        let a = limits.gate("A.test");
        assert!(a.try_acquire().is_ok() && a.try_acquire().is_ok() && a.try_acquire().is_err());
        let other = limits.gate("b.test");
        assert!(other.try_acquire().is_ok());
        assert!(other.try_acquire().is_err());
    }

    #[test]
    fn zero_burst_and_zero_period_are_clamped_not_panics() {
        let gate = RateGate::new(Limit::new(Duration::ZERO, 0));
        assert!(gate.try_acquire().is_ok());
    }

    #[tokio::test]
    async fn client_waits_for_the_host_limit() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        let limits = HostLimits::new(Limit::new(Duration::from_millis(60), 1));
        let c = HttpClient::new(fast_config(), Arc::new(limits)).unwrap();
        let started = Instant::now();
        for _ in 0..3 {
            c.get(&server.uri(), &[]).await.unwrap();
        }
        // First is free; the next two wait ~60 ms each.
        assert!(
            started.elapsed() >= Duration::from_millis(100),
            "{:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn returns_body_and_sends_extra_headers() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/q"))
            .and(header("referer", "https://example.test/"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{\"a\":1}"))
            .mount(&server)
            .await;
        let r = client(fast_config())
            .get(
                &format!("{}/q?x=1", server.uri()),
                &[("Referer", "https://example.test/")],
            )
            .await
            .unwrap();
        assert_eq!(r.status, 200);
        let v: serde_json::Value = r.json().unwrap();
        assert_eq!(v["a"], 1);
    }

    #[tokio::test]
    async fn invalid_json_is_a_parse_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<html>"))
            .mount(&server)
            .await;
        let r = client(fast_config()).get(&server.uri(), &[]).await.unwrap();
        assert!(matches!(
            r.json::<serde_json::Value>(),
            Err(DataError::Parse(_))
        ));
    }

    #[tokio::test]
    async fn status_codes_map_to_the_taxonomy_without_retrying_client_errors() {
        for (status, want) in [
            (404, DataError::NotFound),
            (401, DataError::Unauthorized),
            (403, DataError::Blocked),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(status))
                .expect(1)
                .mount(&server)
                .await;
            let err = client(fast_config())
                .get(&server.uri(), &[])
                .await
                .unwrap_err();
            assert_eq!(err, want, "status {status}");
            // `expect(1)` verifies on drop that there was no retry.
        }
    }

    #[tokio::test]
    async fn retries_server_errors_then_succeeds() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(2)
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .expect(1)
            .mount(&server)
            .await;
        let r = client(fast_config()).get(&server.uri(), &[]).await.unwrap();
        assert_eq!(r.text(), "ok");
    }

    #[tokio::test]
    async fn gives_up_after_max_attempts() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .expect(3)
            .mount(&server)
            .await;
        let err = client(fast_config())
            .get(&server.uri(), &[])
            .await
            .unwrap_err();
        assert_eq!(err, DataError::Upstream("HTTP 500".into()));
    }

    #[tokio::test]
    async fn honours_a_short_retry_after() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            client(fast_config())
                .get(&server.uri(), &[])
                .await
                .unwrap()
                .status,
            200
        );
    }

    #[tokio::test]
    async fn a_long_retry_after_is_reported_not_slept_on() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "120"))
            .expect(1)
            .mount(&server)
            .await;
        let started = Instant::now();
        let err = client(fast_config())
            .get(&server.uri(), &[])
            .await
            .unwrap_err();
        assert_eq!(
            err,
            DataError::RateLimited {
                retry_after: Some(Duration::from_secs(120))
            }
        );
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn cookies_persist_across_requests_when_enabled() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/warm"))
            .respond_with(ResponseTemplate::new(200).insert_header("Set-Cookie", "sid=abc; Path=/"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api"))
            .and(header("cookie", "sid=abc"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let c = client(HttpConfig {
            cookies: true,
            ..fast_config()
        });
        c.get(&format!("{}/warm", server.uri()), &[]).await.unwrap();
        assert_eq!(
            c.get(&format!("{}/api", server.uri()), &[])
                .await
                .unwrap()
                .status,
            200
        );
    }

    #[tokio::test]
    async fn a_refused_connection_is_reported_at_once_without_retries() {
        // Grab a free port, then close it so connecting is refused.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let slow_backoff = HttpConfig {
            backoff_base: Duration::from_millis(400),
            ..fast_config()
        };
        let started = Instant::now();
        let err = client(slow_backoff)
            .get(&format!("http://127.0.0.1:{port}/"), &[])
            .await
            .unwrap_err();
        assert!(
            matches!(&err, DataError::Upstream(m) if m.starts_with(CONNECT_FAILED)),
            "{err:?}"
        );
        assert!(
            started.elapsed() < Duration::from_millis(300),
            "retried: {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn oversized_bodies_are_rejected() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("x".repeat(100)))
            .mount(&server)
            .await;
        let c = client(HttpConfig {
            max_body_bytes: 10,
            ..fast_config()
        });
        assert!(matches!(
            c.get(&server.uri(), &[]).await,
            Err(DataError::Upstream(_))
        ));
    }

    #[tokio::test]
    async fn bad_urls_and_headers_are_errors() {
        let c = client(fast_config());
        assert!(c.get("not a url", &[]).await.is_err());
        assert!(
            c.get("http://127.0.0.1:1/", &[("bad header", "x")])
                .await
                .is_err()
        );
    }
}
