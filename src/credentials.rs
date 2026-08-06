use keyring::v1::{Entry, Error as KeyringError};
use secrecy::{ExposeSecret, SecretString};

use crate::error::{AppError, Result};

const SERVICE: &str = "dev.juber.codex-deepseek-switcher";
const ACCOUNT: &str = "deepseek-api-key";

pub trait CredentialStore {
    fn set(&self, secret: &SecretString) -> Result<()>;
    fn get(&self) -> Result<SecretString>;
    fn delete(&self) -> Result<bool>;

    fn contains_key(&self) -> Result<bool> {
        match self.get() {
            Ok(_) => Ok(true),
            Err(AppError::CredentialMissing) => Ok(false),
            Err(error) => Err(error),
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NativeCredentialStore;

impl NativeCredentialStore {
    fn entry() -> Result<Entry> {
        Entry::new(SERVICE, ACCOUNT).map_err(map_keyring_error)
    }
}

impl CredentialStore for NativeCredentialStore {
    fn set(&self, secret: &SecretString) -> Result<()> {
        validate_api_key(secret.expose_secret())?;
        Self::entry()?
            .set_password(secret.expose_secret())
            .map_err(map_keyring_error)
    }

    fn get(&self) -> Result<SecretString> {
        Self::entry()?
            .get_password()
            .map(SecretString::from)
            .map_err(map_keyring_error)
    }

    fn delete(&self) -> Result<bool> {
        match Self::entry()?.delete_credential() {
            Ok(()) => Ok(true),
            Err(KeyringError::NoEntry) => Ok(false),
            Err(error) => Err(map_keyring_error(error)),
        }
    }
}

pub fn validate_api_key(api_key: &str) -> Result<()> {
    if api_key.is_empty() || api_key.trim() != api_key || !api_key.starts_with("sk-") {
        return Err(AppError::InvalidApiKey);
    }
    Ok(())
}

fn map_keyring_error(error: KeyringError) -> AppError {
    if matches!(error, KeyringError::NoEntry) {
        return AppError::CredentialMissing;
    }

    #[cfg(target_os = "linux")]
    let guidance = " Ensure a Secret Service provider (for example GNOME Keyring or KeePassXC) is running and DBUS_SESSION_BUS_ADDRESS is available; WSL users may need to start and unlock one first.";

    #[cfg(target_os = "macos")]
    let guidance = " Ensure the login Keychain is available and unlocked.";

    #[cfg(target_os = "windows")]
    let guidance = " Ensure Windows Credential Manager is available for this user.";

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    let guidance = "";

    AppError::CredentialStore(format!("{error}.{guidance}"))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct MockCredentialStore {
        value: Mutex<Option<String>>,
    }

    impl CredentialStore for MockCredentialStore {
        fn set(&self, secret: &SecretString) -> Result<()> {
            validate_api_key(secret.expose_secret())?;
            *self.value.lock().expect("mock lock") = Some(secret.expose_secret().to_owned());
            Ok(())
        }

        fn get(&self) -> Result<SecretString> {
            self.value
                .lock()
                .expect("mock lock")
                .clone()
                .map(SecretString::from)
                .ok_or(AppError::CredentialMissing)
        }

        fn delete(&self) -> Result<bool> {
            Ok(self.value.lock().expect("mock lock").take().is_some())
        }
    }

    #[test]
    fn rejects_malformed_keys() {
        for key in ["", "abc", " sk-valid", "sk-valid "] {
            assert!(matches!(
                validate_api_key(key),
                Err(AppError::InvalidApiKey)
            ));
        }
    }

    #[test]
    fn accepts_deepseek_key_shape() {
        assert!(validate_api_key("sk-test-value").is_ok());
    }

    #[test]
    fn credential_contract_handles_missing_set_and_delete() {
        let store = MockCredentialStore::default();
        assert!(!store.contains_key().expect("missing key status"));

        let secret = SecretString::from("sk-test-value".to_owned());
        store.set(&secret).expect("set key");
        assert!(store.contains_key().expect("stored key status"));
        assert_eq!(
            store.get().expect("get key").expose_secret(),
            "sk-test-value"
        );

        assert!(store.delete().expect("delete key"));
        assert!(!store.delete().expect("delete missing key"));
        assert!(matches!(store.get(), Err(AppError::CredentialMissing)));
    }
}
