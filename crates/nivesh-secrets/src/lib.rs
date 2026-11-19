//! Secrets for Nivesh: broker API keys, API secrets and access tokens.
//!
//! Every secret is wrapped in [`SecretString`], which hides its contents from
//! `Debug` and `Display`, so a secret can only be read by explicitly calling
//! [`ExposeSecret::expose_secret`] — and never crosses IPC. Values live in the
//! OS keychain through the `keyring` crate (macOS Keychain today; Windows
//! Credential Manager and Linux Secret Service are compiled in too). Tests use
//! an in-memory store.
//!
//! Key names are stable strings such as `kite.api_secret`; the store prefixes
//! nothing, so callers own their namespace.

use secrecy::SecretString;

pub use secrecy::ExposeSecret;

/// Where secrets come from. One process-wide instance holds every key.
pub trait SecretStore: Send + Sync {
    /// Stores (or replaces) the secret for `key`.
    fn put(&self, key: &str, value: SecretString) -> Result<(), SecretError>;

    /// The secret for `key`, if one is stored.
    fn get(&self, key: &str) -> Result<Option<SecretString>, SecretError>;

    /// Removes the secret for `key`. Removing a key that is not stored is ok.
    fn delete(&self, key: &str) -> Result<(), SecretError>;
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SecretError {
    /// The OS keychain refused the operation (locked, denied, full).
    #[error("keychain refused: {0}")]
    Platform(String),
    /// No usable backend is available on this machine.
    #[error("no secret backend is available")]
    Unavailable,
    /// A key name this store refuses to handle.
    #[error("invalid key name: {0}")]
    InvalidKey(String),
}

/// Everything in memory. Tests, and the fixture/debug mode, use this.
#[derive(Default)]
pub struct InMemorySecretStore {
    entries: std::sync::Mutex<std::collections::HashMap<String, SecretString>>,
}

impl InMemorySecretStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for InMemorySecretStore {
    fn put(&self, key: &str, value: SecretString) -> Result<(), SecretError> {
        let mut guard = self.entries.lock().map_err(|_| poisoned())?;
        guard.insert(key.to_owned(), value);
        Ok(())
    }

    fn get(&self, key: &str) -> Result<Option<SecretString>, SecretError> {
        let guard = self.entries.lock().map_err(|_| poisoned())?;
        Ok(guard.get(key).cloned())
    }

    fn delete(&self, key: &str) -> Result<(), SecretError> {
        let mut guard = self.entries.lock().map_err(|_| poisoned())?;
        guard.remove(key);
        Ok(())
    }
}

fn poisoned() -> SecretError {
    SecretError::Platform("secret store lock poisoned".into())
}

/// The OS keychain. One `service` groups all of Nivesh's entries, so the
/// user sees them under one name ("Nivesh" in macOS Keychain Access).
pub struct KeyringSecretStore {
    service: String,
}

impl KeyringSecretStore {
    /// `service` is what the OS keychain shows as the item's group, e.g.
    /// `nivesh`.
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, key: &str) -> Result<keyring::Entry, SecretError> {
        if key.is_empty() || key.len() > 255 {
            return Err(SecretError::InvalidKey(key.to_owned()));
        }
        keyring::Entry::new(&self.service, key).map_err(|e| SecretError::Platform(e.to_string()))
    }
}

impl SecretStore for KeyringSecretStore {
    fn put(&self, key: &str, value: SecretString) -> Result<(), SecretError> {
        self.entry(key)?
            .set_password(value.expose_secret())
            .map_err(|e| SecretError::Platform(e.to_string()))
    }

    fn get(&self, key: &str) -> Result<Option<SecretString>, SecretError> {
        match self.entry(key)?.get_password() {
            Ok(text) => Ok(Some(SecretString::from(text))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(SecretError::Platform(e.to_string())),
        }
    }

    fn delete(&self, key: &str) -> Result<(), SecretError> {
        match self.entry(key)?.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(SecretError::Platform(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> InMemorySecretStore {
        InMemorySecretStore::new()
    }

    #[test]
    fn round_trips_and_deletes() {
        let store = store();
        assert!(store.get("kite.api_secret").unwrap().is_none());
        store
            .put("kite.api_secret", SecretString::from("shhh"))
            .unwrap();
        assert_eq!(
            store
                .get("kite.api_secret")
                .unwrap()
                .unwrap()
                .expose_secret(),
            "shhh"
        );
        // Replacing works.
        store
            .put("kite.api_secret", SecretString::from("newer"))
            .unwrap();
        assert_eq!(
            store
                .get("kite.api_secret")
                .unwrap()
                .unwrap()
                .expose_secret(),
            "newer"
        );
        store.delete("kite.api_secret").unwrap();
        assert!(store.get("kite.api_secret").unwrap().is_none());
        // Deleting an absent key is ok.
        store.delete("kite.api_secret").unwrap();
    }

    #[test]
    fn keys_are_namespaced_per_key() {
        let store = store();
        store.put("a", SecretString::from("one")).unwrap();
        store.put("b", SecretString::from("two")).unwrap();
        assert_eq!(store.get("a").unwrap().unwrap().expose_secret(), "one");
        assert_eq!(store.get("b").unwrap().unwrap().expose_secret(), "two");
    }

    #[test]
    fn debug_and_display_never_show_the_secret() {
        let secret = SecretString::from("hunter2-or-worse");
        // Display is not implemented at all: the compiler keeps secrets out
        // of `{}` formatting.
        let debug = format!("{secret:?}");
        assert!(!debug.contains("hunter2-or-worse"), "leaked: {debug}");

        // Even a struct that holds one (as payloads will) stays redacted.
        #[derive(Debug)]
        #[allow(dead_code)]
        struct Holder {
            key: SecretString,
        }
        let holder = Holder {
            key: SecretString::from("hunter2-or-worse"),
        };
        assert!(!format!("{holder:?}").contains("hunter2-or-worse"));
    }

    #[test]
    fn empty_keys_are_rejected_by_the_keyring_store() {
        let store = KeyringSecretStore::new("nivesh-test");
        assert!(matches!(store.get(""), Err(SecretError::InvalidKey(_))));
    }

    /// Only by hand: it touches the real keychain of the logged-in user.
    /// `cargo test -p nivesh-secrets -- --ignored --nocapture keychain_live`
    #[test]
    #[ignore = "touches the real OS keychain"]
    fn keychain_round_trip() {
        let store = KeyringSecretStore::new("nivesh-selftest");
        let key = "selftest.token";
        store.delete(key).unwrap();
        assert!(store.get(key).unwrap().is_none());
        store
            .put(key, SecretString::from("selftest-value"))
            .unwrap();
        assert_eq!(
            store.get(key).unwrap().unwrap().expose_secret(),
            "selftest-value"
        );
        store.delete(key).unwrap();
        assert!(store.get(key).unwrap().is_none());
    }
}
