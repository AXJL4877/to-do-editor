//! API keys, owned by the app: entered in the AI settings panel and kept in
//! the Windows Credential Manager. The process environment
//! (`TODO_HUNIU_API_KEY`, `TODO_VOLC_API_KEY`) overrides them for development
//! and CI. Keys are never written to project or config files and never appear
//! in `Debug`/`Display` output.
use todo_production::Provider;

/// A key that cannot be printed by accident.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Option<Self> {
        let value = value.into().trim().to_owned();
        (!value.is_empty()).then_some(Self(value))
    }
    /// The raw key, only for building a request header.
    pub fn expose(&self) -> &str {
        &self.0
    }
    /// `sk-abc…wxyz` style hint for settings UI and logs.
    pub fn masked(&self) -> String {
        let chars: Vec<char> = self.0.chars().collect();
        if chars.len() <= 10 {
            return format!("{}…", chars.iter().take(3).collect::<String>());
        }
        let head: String = chars[..6].iter().collect();
        let tail: String = chars[chars.len() - 4..].iter().collect();
        format!("{head}…{tail}")
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(***)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretSource {
    Environment,
    /// Saved from the settings panel.
    Keychain,
}

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("credential store: {0}")]
    Store(String),
}

/// Where keys may be persisted when the toolkit `.env` is not available.
pub trait SecretStore: Send + Sync {
    fn read(&self, target: &str) -> Result<Option<Secret>, SecretError>;
    fn write(&self, target: &str, secret: &Secret) -> Result<(), SecretError>;
    fn delete(&self, target: &str) -> Result<(), SecretError>;
}

fn names(provider: Provider) -> (&'static str, &'static str) {
    match provider {
        Provider::Huniu => ("TODO_HUNIU_API_KEY", "todo-editor/huniu.api_key"),
        Provider::Volc => ("TODO_VOLC_API_KEY", "todo-editor/volc.api_key"),
    }
}

/// Keychain target name for `provider`.
pub fn credential_target(provider: Provider) -> &'static str {
    names(provider).1
}

pub struct Secrets<'a> {
    pub store: &'a dyn SecretStore,
    /// Process environment lookup; injectable for tests.
    pub environment: &'a dyn Fn(&str) -> Option<String>,
}

impl Secrets<'_> {
    pub fn resolve(
        &self,
        provider: Provider,
    ) -> Result<Option<(Secret, SecretSource)>, SecretError> {
        let (variable, target) = names(provider);
        if let Some(secret) = (self.environment)(variable).and_then(Secret::new) {
            return Ok(Some((secret, SecretSource::Environment)));
        }
        Ok(self
            .store
            .read(target)?
            .map(|secret| (secret, SecretSource::Keychain)))
    }

    /// Save (or with `None`, forget) the key entered in the settings panel.
    pub fn save(&self, provider: Provider, key: Option<&Secret>) -> Result<(), SecretError> {
        let target = credential_target(provider);
        match key {
            Some(key) => self.store.write(target, key),
            None => self.store.delete(target),
        }
    }
}

/// The platform keychain (Windows Credential Manager), or memory elsewhere.
pub fn default_store() -> Box<dyn SecretStore> {
    #[cfg(windows)]
    return Box::new(CredentialManager);
    #[cfg(not(windows))]
    return Box::new(MemoryStore::default());
}

pub fn process_environment(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// In-memory store for tests and for running without any persistent store.
#[derive(Default)]
pub struct MemoryStore(std::sync::Mutex<std::collections::HashMap<String, Secret>>);

impl SecretStore for MemoryStore {
    fn read(&self, target: &str) -> Result<Option<Secret>, SecretError> {
        Ok(self
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(target)
            .cloned())
    }
    fn write(&self, target: &str, secret: &Secret) -> Result<(), SecretError> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(target.into(), secret.clone());
        Ok(())
    }
    fn delete(&self, target: &str) -> Result<(), SecretError> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(target);
        Ok(())
    }
}

#[cfg(windows)]
pub use windows::CredentialManager;

#[cfg(windows)]
mod windows {
    use super::{Secret, SecretError, SecretStore};
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_NOT_FOUND};
    use windows_sys::Win32::Security::Credentials::{
        CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE,
        CRED_TYPE_GENERIC,
    };

    /// Generic credentials of the current Windows user.
    #[derive(Debug, Default, Clone, Copy)]
    pub struct CredentialManager;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn last_error(action: &str) -> SecretError {
        // SAFETY: reads the calling thread's last-error value.
        SecretError::Store(format!("{action} failed: {}", unsafe { GetLastError() }))
    }

    impl SecretStore for CredentialManager {
        fn read(&self, target: &str) -> Result<Option<Secret>, SecretError> {
            let target = wide(target);
            let mut credential: *mut CREDENTIALW = std::ptr::null_mut();
            // SAFETY: valid NUL-terminated target; on success the buffer is freed below.
            let ok = unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) };
            if ok == 0 {
                // SAFETY: as above.
                if unsafe { GetLastError() } == ERROR_NOT_FOUND {
                    return Ok(None);
                }
                return Err(last_error("CredReadW"));
            }
            // SAFETY: CredReadW returned a valid credential whose blob holds
            // `CredentialBlobSize` bytes written by `write` (UTF-8).
            let value = unsafe {
                let credential = &*credential;
                let bytes = std::slice::from_raw_parts(
                    credential.CredentialBlob,
                    credential.CredentialBlobSize as usize,
                );
                String::from_utf8_lossy(bytes).into_owned()
            };
            // SAFETY: frees the buffer returned by CredReadW exactly once.
            unsafe { CredFree(credential.cast()) };
            Ok(Secret::new(value))
        }

        fn write(&self, target: &str, secret: &Secret) -> Result<(), SecretError> {
            let target = wide(target);
            let mut blob = secret.expose().as_bytes().to_vec();
            // SAFETY: all-zero is a valid CREDENTIALW; the fields set below point
            // into buffers that outlive the CredWriteW call.
            let mut credential: CREDENTIALW = unsafe { std::mem::zeroed() };
            credential.Type = CRED_TYPE_GENERIC;
            credential.TargetName = target.as_ptr() as *mut u16;
            credential.CredentialBlobSize = blob.len() as u32;
            credential.CredentialBlob = blob.as_mut_ptr();
            credential.Persist = CRED_PERSIST_LOCAL_MACHINE;
            // SAFETY: `credential` is fully initialized for a generic credential.
            if unsafe { CredWriteW(&credential, 0) } == 0 {
                return Err(last_error("CredWriteW"));
            }
            Ok(())
        }

        fn delete(&self, target: &str) -> Result<(), SecretError> {
            let target = wide(target);
            // SAFETY: valid NUL-terminated target.
            if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } == 0 {
                // SAFETY: reads the thread's last error.
                if unsafe { GetLastError() } == ERROR_NOT_FOUND {
                    return Ok(());
                }
                return Err(last_error("CredDeleteW"));
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_environment(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn secrets_never_print() {
        let secret = Secret::new("sk-1234567890abcdef").unwrap();
        assert_eq!(format!("{secret:?}"), "Secret(***)");
        assert_eq!(secret.masked(), "sk-123…cdef");
        assert!(Secret::new("   ").is_none());
    }

    #[test]
    fn environment_overrides_the_saved_key() {
        let store = MemoryStore::default();
        let plain = Secrets {
            store: &store,
            environment: &no_environment,
        };
        assert!(plain.resolve(Provider::Huniu).unwrap().is_none());
        plain
            .save(
                Provider::Huniu,
                Some(&Secret::new("from-settings").unwrap()),
            )
            .unwrap();
        let (secret, source) = plain.resolve(Provider::Huniu).unwrap().unwrap();
        assert_eq!(
            (secret.expose(), source),
            ("from-settings", SecretSource::Keychain)
        );
        assert!(plain.resolve(Provider::Volc).unwrap().is_none());

        let env_var = |name: &str| (name == "TODO_HUNIU_API_KEY").then(|| "from-env".to_owned());
        let with_env = Secrets {
            store: &store,
            environment: &env_var,
        };
        let (secret, source) = with_env.resolve(Provider::Huniu).unwrap().unwrap();
        assert_eq!(
            (secret.expose(), source),
            ("from-env", SecretSource::Environment)
        );

        plain.save(Provider::Huniu, None).unwrap();
        assert!(plain.resolve(Provider::Huniu).unwrap().is_none());
    }

    /// Writes to the real Windows credential store; run with `--ignored`.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn credential_manager_round_trip() {
        let target = format!("todo-editor/test.{}", std::process::id());
        let store = CredentialManager;
        assert!(store.read(&target).unwrap().is_none());
        store
            .write(&target, &Secret::new("sk-roundtrip-秘").unwrap())
            .unwrap();
        assert_eq!(
            store.read(&target).unwrap().unwrap().expose(),
            "sk-roundtrip-秘"
        );
        store.delete(&target).unwrap();
        assert!(store.read(&target).unwrap().is_none());
        store.delete(&target).unwrap();
    }
}
