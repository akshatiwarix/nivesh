//! Broker session management for the app: which broker is set up, whether
//! the day's token is alive, and the login/logout flows. Wiring, not logic —
//! the Kite adapter and the secret store hold the rules.
//!
//! Secrets never leave this module: the setup command takes the API key and
//! secret as arguments, stores both in the keychain, and returns nothing
//! about them.

use std::sync::Arc;

use nivesh_broker::{BrokerAuth as _, BrokerError, SessionState, SessionStatus};
use nivesh_broker_kite::{
    DEFAULT_REDIRECT_PORT, KEY_API_KEY, KEY_API_SECRET, KiteAccount, KiteLogin, KiteRest,
    KiteStream,
};
use nivesh_secrets::SecretStore;
use nivesh_store::Store;
use tokio::sync::Mutex as AsyncMutex;

use crate::ipc::broker::BrokerSessionDto;

/// The keychain service under which Nivesh keeps its entries.
pub const KEYCHAIN_SERVICE: &str = "nivesh";

/// One broker's live wiring: shared clients the views and the hub will use.
/// Fields beyond `login` come into use with the live-data provider (4j).
#[derive(Clone)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct KiteWiring {
    #[allow(dead_code)]
    pub rest: Arc<KiteRest>,
    pub login: Arc<KiteLogin>,
    #[allow(dead_code)]
    pub account: Arc<KiteAccount>,
    #[allow(dead_code)]
    pub stream: Arc<KiteStream>,
}

impl KiteWiring {
    /// `base` is the API root: `None` for the real `api.kite.trade`, a mock
    /// server's URL in tests. Tests must never reach the real broker.
    fn new(
        store: Arc<Store>,
        api_key: String,
        secrets: Arc<dyn SecretStore>,
        base: Option<String>,
    ) -> Result<Self, BrokerError> {
        let rest = Arc::new(KiteRest::new(base, api_key)?);
        let login = Arc::new(KiteLogin::new(
            Arc::clone(&rest),
            Arc::clone(&secrets),
            DEFAULT_REDIRECT_PORT,
            nivesh_broker_kite::REDIRECT_PATH,
        ));
        let account = Arc::new(KiteAccount::new(Arc::clone(&rest), secrets));
        let stream = Arc::new(KiteStream::new(Arc::clone(&rest), store));
        Ok(Self {
            rest,
            login,
            account,
            stream,
        })
    }
}

/// Everything the broker commands need.
pub struct BrokerManager {
    store: Arc<Store>,
    secrets: Arc<dyn SecretStore>,
    /// The API root; tests point at a mock server and never at the real one.
    base: Option<String>,
    inner: AsyncMutex<Inner>,
}

struct Inner {
    kite: Option<KiteWiring>,
    /// One login flow at a time: the loopback port is single-use.
    logging_in: bool,
}

impl BrokerManager {
    /// The manager over the OS keychain (production). A debug-build hook
    /// (`NIVESH_KITE_URL`) points the clients at a local mock.
    pub fn new(store: Arc<Store>) -> Self {
        let base = if cfg!(debug_assertions) {
            std::env::var("NIVESH_KITE_URL").ok()
        } else {
            None
        };
        Self::build(
            store,
            Arc::new(nivesh_secrets::KeyringSecretStore::new(KEYCHAIN_SERVICE)),
            base,
        )
    }

    /// The manager over an arbitrary secret store (tests).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn with_secrets(
        store: Arc<Store>,
        secrets: Arc<dyn SecretStore>,
        base: Option<String>,
    ) -> Self {
        Self::build(store, secrets, base)
    }

    fn build(store: Arc<Store>, secrets: Arc<dyn SecretStore>, base: Option<String>) -> Self {
        Self {
            store,
            secrets,
            base,
            inner: AsyncMutex::new(Inner {
                kite: None,
                logging_in: false,
            }),
        }
    }

    /// Restores a stored session, if any: a valid keychain token flows back
    /// into the clients so LIVE data works after a restart until the daily
    /// flush. Called once at startup.
    pub async fn restore(&self) -> Result<BrokerSessionDto, BrokerError> {
        let inner = self.inner.lock().await;
        let Some(wiring) = &inner.kite else {
            return Ok(self.logged_out());
        };
        let status = wiring.login.restore_session()?;
        Ok(BrokerSessionDto::new(
            self.broker_name(true),
            status,
            self.user_id(),
        ))
    }

    /// Stores the API credentials and builds the wiring. Secrets go only to
    /// the keychain; nothing about them comes back.
    pub async fn setup(&self, api_key: &str, api_secret: &str) -> Result<(), BrokerError> {
        let api_key = api_key.trim();
        let api_secret = api_secret.trim();
        if api_key.len() < 6 || api_secret.is_empty() {
            return Err(BrokerError::InvalidRequest(
                "the API key looks too short, or the API secret is missing".into(),
            ));
        }
        self.secrets
            .put(KEY_API_KEY, api_key.to_owned().into())
            .map_err(BrokerManager::secret_err)?;
        self.secrets
            .put(KEY_API_SECRET, api_secret.to_owned().into())
            .map_err(BrokerManager::secret_err)?;
        let wiring = KiteWiring::new(
            Arc::clone(&self.store),
            api_key.to_owned(),
            Arc::clone(&self.secrets),
            self.base.clone(),
        )?;
        let mut inner = self.inner.lock().await;
        inner.kite = Some(wiring);
        Ok(())
    }

    /// Whether the login flow can start: a setup exists and no login is
    /// running. The URL the caller gets opens in the user's browser.
    pub async fn begin_login(&self) -> Result<url::Url, BrokerError> {
        let mut inner = self.inner.lock().await;
        if inner.logging_in {
            return Err(BrokerError::InvalidRequest(
                "a login is already in progress".into(),
            ));
        }
        let Some(wiring) = &inner.kite else {
            return Err(BrokerError::InvalidRequest(
                "no broker is set up; run the broker setup first".into(),
            ));
        };
        let url = wiring
            .login
            .login_url(&nivesh_broker::LoginState(nivesh_broker::new_nonce()))?;
        inner.logging_in = true;
        Ok(url)
    }

    /// Completes the login the listener captured, marks the flow over, and
    /// reports the session. `user_id` lands in the store for the status bar.
    pub async fn complete_login(
        &self,
        callback: nivesh_broker::LoginCallback,
    ) -> Result<BrokerSessionDto, BrokerError> {
        let outcome = self.complete_login_inner(callback).await;
        let mut inner = self.inner.lock().await;
        inner.logging_in = false;
        outcome
    }

    async fn complete_login_inner(
        &self,
        callback: nivesh_broker::LoginCallback,
    ) -> Result<BrokerSessionDto, BrokerError> {
        let wiring = {
            let inner = self.inner.lock().await;
            inner.kite.as_ref().cloned()
        };
        let Some(wiring) = wiring else {
            return Err(BrokerError::InvalidRequest("no broker is set up".into()));
        };
        let session = wiring.login.complete_login(callback).await?;
        self.store
            .set_setting("kite.user_id", &session.user_id)
            .map_err(|e| BrokerError::Upstream(format!("storing the user id: {e}")))?;
        Ok(BrokerSessionDto::connected(
            &session.user_id,
            session.expires_at.timestamp_millis(),
        ))
    }

    /// Marks a login attempt over without a session (a timeout, a foreign
    /// redirect). The UI shows the current state.
    pub async fn cancel_login(&self) -> BrokerSessionDto {
        let mut inner = self.inner.lock().await;
        inner.logging_in = false;
        self.current_status(&inner)
    }

    /// Ends the session: the broker invalidates the day's token and the
    /// keychain copy goes with it.
    pub async fn logout(&self) -> Result<BrokerSessionDto, BrokerError> {
        let wiring = {
            let mut inner = self.inner.lock().await;
            inner.logging_in = false;
            inner.kite.as_ref().cloned()
        };
        if let Some(wiring) = wiring {
            wiring.login.logout().await?;
        }
        Ok(self.logged_out())
    }

    /// The session as it stands right now, checked against the token's
    /// stored state (not a live call — `broker_status` may be polled).
    pub async fn status(&self) -> BrokerSessionDto {
        let inner = self.inner.lock().await;
        self.current_status(&inner)
    }

    /// The stored token, if the wiring has one, for the live-data provider.
    pub async fn kite_wiring(&self) -> Option<KiteWiring> {
        self.inner.lock().await.kite.clone()
    }

    fn current_status(&self, inner: &Inner) -> BrokerSessionDto {
        match &inner.kite {
            Some(wiring) => match wiring.login.restore_session() {
                Ok(status) => BrokerSessionDto::new(self.broker_name(true), status, self.user_id()),
                Err(_) => BrokerSessionDto::degraded(),
            },
            None => self.logged_out(),
        }
    }

    /// Which broker the wiring (or stored credentials) name. "kite" or none;
    /// never a secret.
    fn broker_name(&self, has_kite: bool) -> Option<String> {
        if has_kite {
            return Some("kite".to_owned());
        }
        self.secrets
            .get(KEY_API_KEY)
            .ok()
            .flatten()
            .map(|_| "kite".to_owned())
    }

    fn user_id(&self) -> Option<String> {
        self.store.setting("kite.user_id").ok().flatten()
    }

    fn logged_out(&self) -> BrokerSessionDto {
        let broker = self.broker_name(self.has_any_credentials());
        BrokerSessionDto::new(
            broker,
            SessionStatus {
                state: SessionState::LoggedOut,
                expires_at: None,
            },
            None,
        )
    }

    /// Whether a setup has ever happened, even without a live session.
    fn has_any_credentials(&self) -> bool {
        self.secrets.get(KEY_API_KEY).ok().flatten().is_some()
    }

    fn secret_err(e: nivesh_secrets::SecretError) -> BrokerError {
        BrokerError::Upstream(format!("secret store: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::broker::SessionStateDto;
    use nivesh_secrets::{ExposeSecret as _, InMemorySecretStore};
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn manager() -> (MockServer, Arc<BrokerManager>, Arc<InMemorySecretStore>) {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/session/token"))
            .and(body_string_contains("checksum="))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"status":"success","data":{"access_token":"day-token","user_id":"XY1234"}}"#,
            ))
            .mount(&server)
            .await;
        let store = Arc::new(Store::open_in_memory().unwrap());
        let secrets = Arc::new(InMemorySecretStore::new());
        let base = server.uri();
        (
            server,
            Arc::new(BrokerManager::with_secrets(
                store,
                secrets.clone(),
                Some(base),
            )),
            secrets,
        )
    }

    #[tokio::test]
    async fn setup_stores_the_credentials_and_builds_the_wiring() {
        let (_server, manager, secrets) = manager().await;
        let status = manager.status().await;
        assert_eq!(status.state, SessionStateDto::LoggedOut);
        assert_eq!(status.broker, None, "no setup yet");

        manager.setup("key1234", "secret").await.unwrap();
        assert_eq!(
            secrets.get(KEY_API_KEY).unwrap().unwrap().expose_secret(),
            "key1234"
        );
        assert_eq!(
            secrets
                .get(KEY_API_SECRET)
                .unwrap()
                .unwrap()
                .expose_secret(),
            "secret"
        );
        // Nothing about the credentials comes back on the status.
        let status = manager.status().await;
        assert_eq!(status.broker.as_deref(), Some("kite"));
        assert_eq!(status.state, SessionStateDto::LoggedOut);
        assert!(status.user_id.is_none());
        assert!(manager.kite_wiring().await.is_some());
    }

    #[tokio::test]
    async fn setup_rejects_a_short_key_and_an_empty_secret() {
        let (_server, manager, _) = manager().await;
        assert!(manager.setup("abc", "secret").await.is_err());
        assert!(manager.setup("key1234", "").await.is_err());
    }

    #[tokio::test]
    async fn begin_login_needs_setup_and_guards_reentry() {
        let (_server, manager, _) = manager().await;
        let err = manager.begin_login().await.unwrap_err();
        assert!(matches!(err, BrokerError::InvalidRequest(text) if text.contains("setup")));

        manager.setup("key1234", "secret").await.unwrap();
        let url = manager.begin_login().await.unwrap();
        assert!(url.as_str().contains("api_key=key1234"), "{url}");
        // A second login while one is running is refused.
        assert!(manager.begin_login().await.is_err());
        // Cancelling frees the flow.
        manager.cancel_login().await;
        assert!(manager.begin_login().await.is_ok());
    }

    #[tokio::test]
    async fn complete_login_exchanges_and_reports_connected() {
        let (_server, manager, secrets) = manager().await;
        manager.setup("key1234", "secret").await.unwrap();
        manager.begin_login().await.unwrap();
        let session = manager
            .complete_login(nivesh_broker::LoginCallback {
                request_token: "token-for-tests".into(),
                state: "success".into(),
            })
            .await
            .unwrap();
        assert_eq!(session.state, SessionStateDto::Connected);
        assert_eq!(session.user_id.as_deref(), Some("XY1234"));
        assert!(session.expires_at_ms.is_some());
        // The token landed in the secret store...
        assert_eq!(
            secrets
                .get(nivesh_broker_kite::KEY_ACCESS_TOKEN)
                .unwrap()
                .unwrap()
                .expose_secret(),
            "day-token"
        );
        // ...and the user id in the store.
        assert_eq!(
            manager.store.setting("kite.user_id").unwrap().as_deref(),
            Some("XY1234")
        );
        // The flow is over; another can start.
        assert!(manager.begin_login().await.is_ok());
    }

    #[tokio::test]
    async fn logout_clears_the_session() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/session/token"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"status":"success","data":{"access_token":"day-token","user_id":"XY1234"}}"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/session/invalidate_token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(r#"{"status":"success","data":{"access_token":"day-token"}}"#),
            )
            .mount(&server)
            .await;
        let store = Arc::new(Store::open_in_memory().unwrap());
        let secrets = Arc::new(InMemorySecretStore::new());
        let manager =
            BrokerManager::with_secrets(store, secrets.clone(), Some(server.uri().clone()));
        manager.setup("key1234", "secret").await.unwrap();
        manager
            .complete_login(nivesh_broker::LoginCallback {
                request_token: "token-for-tests".into(),
                state: "success".into(),
            })
            .await
            .unwrap();

        let session = manager.logout().await.unwrap();
        assert_eq!(session.state, SessionStateDto::LoggedOut);
        assert!(
            secrets
                .get(nivesh_broker_kite::KEY_ACCESS_TOKEN)
                .unwrap()
                .is_none()
        );
    }
}
