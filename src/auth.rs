use std::{
    collections::HashMap,
    io::{self, Read},
    path::PathBuf,
    time::{Duration, Instant},
};

use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const SESSION_IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthFailure {
    AlreadyConfigured,
    SetupCodeInvalid,
    PassphraseTooShort,
    PassphraseTooLong,
    InvalidPassphrase,
    Persistence,
}

#[derive(Debug, Serialize, Deserialize)]
struct AuthDocument {
    version: u32,
    password_hash: String,
}

#[derive(Debug, Clone)]
pub struct AdminAuth {
    path: Option<PathBuf>,
    password_hash: Option<String>,
    setup_code: Option<String>,
    sessions: HashMap<String, Instant>,
}

impl AdminAuth {
    pub fn memory() -> Self {
        Self::new(None, None)
    }

    pub fn load(path: Option<PathBuf>) -> io::Result<Self> {
        let Some(path) = path else {
            return Ok(Self::new(None, None));
        };
        let mut file = match crate::api::open_private_file(&path, true) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(Self::new(Some(path), None));
            }
            Err(error) => return Err(error),
        };
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        let document: AuthDocument = serde_json::from_slice(&bytes).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("admin auth file is invalid: {error}"),
            )
        })?;
        Ok(Self {
            path: Some(path),
            password_hash: Some(document.password_hash),
            setup_code: None,
            sessions: HashMap::new(),
        })
    }

    fn new(path: Option<PathBuf>, password_hash: Option<String>) -> Self {
        let setup_code = password_hash
            .is_none()
            .then(|| Uuid::new_v4().simple().to_string().to_ascii_uppercase());
        Self {
            path,
            password_hash,
            setup_code,
            sessions: HashMap::new(),
        }
    }

    pub fn is_configured(&self) -> bool {
        self.password_hash.is_some()
    }

    pub fn setup_code(&self) -> Option<&str> {
        self.setup_code.as_deref()
    }

    pub fn setup(&mut self, code: &str, passphrase: &str) -> Result<String, AuthFailure> {
        if self.is_configured() {
            return Err(AuthFailure::AlreadyConfigured);
        }
        let Some(setup_code) = self.setup_code.as_deref() else {
            return Err(AuthFailure::SetupCodeInvalid);
        };
        if !constant_time_eq(setup_code.as_bytes(), code.as_bytes()) {
            return Err(AuthFailure::SetupCodeInvalid);
        }
        validate_passphrase(passphrase)?;
        let password_hash = hash_passphrase(passphrase).map_err(|_| AuthFailure::Persistence)?;
        self.persist(&password_hash)
            .map_err(|_| AuthFailure::Persistence)?;
        self.password_hash = Some(password_hash);
        self.setup_code = None;
        self.create_session()
    }

    pub fn login(&mut self, passphrase: &str) -> Result<String, AuthFailure> {
        if passphrase.len() > 256 {
            return Err(AuthFailure::InvalidPassphrase);
        }
        let Some(password_hash) = &self.password_hash else {
            return Err(AuthFailure::AlreadyConfigured);
        };
        if verify_passphrase(password_hash, passphrase) {
            self.create_session()
        } else {
            Err(AuthFailure::InvalidPassphrase)
        }
    }

    pub fn is_authenticated(&mut self, token: Option<&str>, now: Instant) -> bool {
        self.sessions.retain(|_, last_seen| {
            now.saturating_duration_since(*last_seen) < SESSION_IDLE_TIMEOUT
        });
        let Some(token) = token else {
            return false;
        };
        let Some(last_seen) = self.sessions.get_mut(token) else {
            return false;
        };
        *last_seen = now;
        true
    }

    pub fn logout(&mut self, token: Option<&str>) {
        if let Some(token) = token {
            self.sessions.remove(token);
        }
    }

    fn create_session(&mut self) -> Result<String, AuthFailure> {
        let token = Uuid::new_v4().simple().to_string();
        self.sessions.insert(token.clone(), Instant::now());
        Ok(token)
    }

    fn persist(&self, password_hash: &str) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let bytes = serde_json::to_vec_pretty(&AuthDocument {
            version: 1,
            password_hash: password_hash.to_owned(),
        })
        .map_err(io::Error::other)?;
        crate::api::atomic_write(path, &bytes)
    }
}

fn validate_passphrase(passphrase: &str) -> Result<(), AuthFailure> {
    if passphrase.len() < 12 {
        return Err(AuthFailure::PassphraseTooShort);
    }
    if passphrase.len() > 256 {
        return Err(AuthFailure::PassphraseTooLong);
    }
    Ok(())
}

fn hash_passphrase(passphrase: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut rand_core::OsRng);
    Argon2::default()
        .hash_password(passphrase.as_bytes(), &salt)
        .map(|hash| hash.to_string())
}

fn verify_passphrase(hash: &str, passphrase: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|hash| {
        Argon2::default()
            .verify_password(passphrase.as_bytes(), &hash)
            .is_ok()
    })
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

#[cfg(test)]
mod tests {
    use super::{AdminAuth, AuthFailure};
    use std::{fs, os::unix::fs::PermissionsExt};

    #[test]
    fn setup_persists_only_a_private_argon2_hash_and_consumes_the_code() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("admin-auth.json");
        let mut auth = AdminAuth::load(Some(path.clone())).unwrap();
        let code = auth.setup_code().unwrap().to_owned();
        let session = auth.setup(&code, "correct horse battery staple").unwrap();

        assert!(auth.is_configured());
        assert!(auth.setup_code().is_none());
        assert!(auth.is_authenticated(Some(&session), std::time::Instant::now()));
        let document: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let hash = document["password_hash"].as_str().unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert!(!hash.contains("correct horse"));
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn setup_code_is_one_time_and_passphrases_are_bounded() {
        let mut auth = AdminAuth::memory();
        let code = auth.setup_code().unwrap().to_owned();
        assert_eq!(
            auth.setup(&code, "short"),
            Err(AuthFailure::PassphraseTooShort)
        );
        auth.setup(&code, "correct horse battery staple").unwrap();
        assert_eq!(
            auth.setup(&code, "another good passphrase"),
            Err(AuthFailure::AlreadyConfigured)
        );
    }

    #[test]
    fn sessions_expire_after_idle_timeout_and_logout_revokes_them() {
        let mut auth = AdminAuth::memory();
        let code = auth.setup_code().unwrap().to_owned();
        let token = auth.setup(&code, "correct horse battery staple").unwrap();
        let now = std::time::Instant::now();
        assert!(auth.is_authenticated(Some(&token), now));
        assert!(!auth.is_authenticated(Some(&token), now + std::time::Duration::from_secs(1801)));
        let replacement = auth.login("correct horse battery staple").unwrap();
        auth.logout(Some(&replacement));
        assert!(!auth.is_authenticated(Some(&replacement), std::time::Instant::now()));
    }
}
