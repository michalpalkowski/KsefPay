use std::sync::Arc;

use uuid::Uuid;

use crate::domain::auth::{AuthStatus, ContextIdentifier, TokenPair};
use crate::domain::crypto::EncryptedInvoice;
use crate::domain::environment::KSeFEnvironment;
use crate::domain::nip::Nip;
use crate::domain::session::{SessionReference, Upo};
use crate::error::{CryptoError, KSeFError, RepositoryError};
use crate::ports::encryption::XadesSigner;
use crate::ports::ksef_auth::KSeFAuth;
use crate::ports::ksef_client::KSeFClient;
use crate::ports::nip_account_repository::NipAccountRepository;
use crate::ports::session_repository::{SessionRepository, StoredSession, StoredTokenPair};
use crate::ports::signer_factory::{SignerCredentials, SignerFactory};

const AUTH_POLL_MAX_ATTEMPTS: usize = 10;
#[cfg(not(test))]
const AUTH_POLL_DELAY: std::time::Duration = std::time::Duration::from_secs(2);
#[cfg(test)]
const AUTH_POLL_DELAY: std::time::Duration = std::time::Duration::from_millis(1);

/// Application service managing `KSeF` authentication and session lifecycle.
pub struct SessionService {
    auth: Arc<dyn KSeFAuth>,
    signer: Arc<dyn XadesSigner>,
    signer_factory: Option<Arc<dyn SignerFactory>>,
    nip_account_repo: Option<Arc<dyn NipAccountRepository>>,
    client: Arc<dyn KSeFClient>,
    repo: Arc<dyn SessionRepository>,
    environment: KSeFEnvironment,
    auth_method: AuthMethod,
}

#[derive(Debug, Clone)]
pub enum AuthMethod {
    Xades,
    Token {
        context: ContextIdentifier,
        token: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum SessionServiceError {
    #[error("KSeF authentication failed: {0}")]
    AuthFailed(String),

    #[error(transparent)]
    KSeF(#[from] KSeFError),

    #[error(transparent)]
    Crypto(#[from] CryptoError),

    #[error(transparent)]
    Repository(#[from] RepositoryError),
}

impl SessionService {
    #[must_use]
    pub fn new(
        auth: Arc<dyn KSeFAuth>,
        signer: Arc<dyn XadesSigner>,
        client: Arc<dyn KSeFClient>,
        repo: Arc<dyn SessionRepository>,
        environment: KSeFEnvironment,
    ) -> Self {
        Self::with_auth_method(auth, signer, client, repo, environment, AuthMethod::Xades)
    }

    #[must_use]
    pub fn with_auth_method(
        auth: Arc<dyn KSeFAuth>,
        signer: Arc<dyn XadesSigner>,
        client: Arc<dyn KSeFClient>,
        repo: Arc<dyn SessionRepository>,
        environment: KSeFEnvironment,
        auth_method: AuthMethod,
    ) -> Self {
        Self {
            auth,
            signer,
            signer_factory: None,
            nip_account_repo: None,
            client,
            repo,
            environment,
            auth_method,
        }
    }

    /// Create with a per-NIP signer factory for multi-tenant mode.
    ///
    /// When `signer_factory` + `nip_account_repo` are set, `authenticate()`
    /// loads the cert from the NIP account and creates a per-NIP signer
    /// instead of using the global fallback.
    #[must_use]
    pub fn with_signer_factory(
        auth: Arc<dyn KSeFAuth>,
        fallback_signer: Arc<dyn XadesSigner>,
        signer_factory: Arc<dyn SignerFactory>,
        nip_account_repo: Arc<dyn NipAccountRepository>,
        client: Arc<dyn KSeFClient>,
        repo: Arc<dyn SessionRepository>,
        environment: KSeFEnvironment,
        auth_method: AuthMethod,
    ) -> Self {
        Self {
            auth,
            signer: fallback_signer,
            signer_factory: Some(signer_factory),
            nip_account_repo: Some(nip_account_repo),
            client,
            repo,
            environment,
            auth_method,
        }
    }

    /// Check if a valid token exists (without refreshing).
    pub async fn has_valid_token(&self, nip: &Nip) -> bool {
        self.repo
            .find_active_token(nip, self.environment)
            .await
            .ok()
            .flatten()
            .is_some()
    }

    /// Check if an online session is currently open.
    pub async fn has_active_session(&self, nip: &Nip) -> bool {
        self.repo
            .find_active_session(nip, self.environment)
            .await
            .ok()
            .flatten()
            .is_some()
    }

    /// Resolve the signer for a NIP: factory with per-NIP cert, or global fallback.
    async fn resolve_signer(&self, nip: &Nip) -> Result<Arc<dyn XadesSigner>, SessionServiceError> {
        let (Some(factory), Some(nip_repo)) = (&self.signer_factory, &self.nip_account_repo) else {
            return Ok(self.signer.clone());
        };

        // Load cert from NIP account if stored
        let account = nip_repo.find_by_nip(nip).await?;
        let credentials = match account {
            Some(ref acc) if acc.cert_pem.is_some() && acc.key_pem.is_some() => {
                SignerCredentials::Pem {
                    cert_pem: acc.cert_pem.as_ref().unwrap(),
                    key_pem: acc.key_pem.as_ref().unwrap(),
                }
            }
            _ => {
                if self.environment == KSeFEnvironment::Production {
                    return Err(SessionServiceError::AuthFailed(format!(
                        "no certificate stored for NIP {nip} — required in production"
                    )));
                }
                SignerCredentials::AutoGenerate
            }
        };

        factory
            .create_signer(nip, credentials)
            .map_err(SessionServiceError::Crypto)
    }

    /// Full auth flow: challenge -> sign -> submit -> poll -> redeem -> persist.
    pub async fn authenticate(&self, nip: &Nip) -> Result<TokenPair, SessionServiceError> {
        let auth_ref = match &self.auth_method {
            AuthMethod::Xades => {
                let signer = self.resolve_signer(nip).await?;
                let challenge = self.auth.request_challenge(nip).await?;
                let signed = signer.sign_auth_request(&challenge, nip).await?;
                self.auth.authenticate_xades(&signed).await?
            }
            AuthMethod::Token { context, token } => {
                self.auth.authenticate_token(context, token).await?
            }
        };

        let mut completed = false;
        for attempt in 1..=AUTH_POLL_MAX_ATTEMPTS {
            match self.auth.poll_auth_status(&auth_ref).await? {
                AuthStatus::Completed => {
                    completed = true;
                    break;
                }
                AuthStatus::Failed { reason } => {
                    return Err(SessionServiceError::AuthFailed(reason));
                }
                AuthStatus::Processing => {
                    if attempt < AUTH_POLL_MAX_ATTEMPTS {
                        tokio::time::sleep(AUTH_POLL_DELAY).await;
                    }
                }
            }
        }
        if !completed {
            return Err(SessionServiceError::AuthFailed(format!(
                "auth still processing after {AUTH_POLL_MAX_ATTEMPTS} polls"
            )));
        }

        let token_pair = self.auth.redeem_token(&auth_ref).await?;

        let stored = StoredTokenPair {
            id: Uuid::new_v4(),
            nip: nip.clone(),
            environment: self.environment,
            token_pair: token_pair.clone(),
            created_at: chrono::Utc::now(),
        };
        self.repo.save_token_pair(&stored).await?;

        Ok(token_pair)
    }

    /// Get a valid access token. Refreshes if expired. Authenticates from scratch if needed.
    ///
    /// If refresh fails (stale token, `KSeF` revoked it, etc.), falls through to full
    /// re-authentication rather than propagating a stale-token error.
    pub async fn ensure_token(&self, nip: &Nip) -> Result<TokenPair, SessionServiceError> {
        if let Some(stored) = self.repo.find_active_token(nip, self.environment).await? {
            if !stored.token_pair.is_access_expired() {
                return Ok(stored.token_pair);
            }
            if !stored.token_pair.is_refresh_expired() {
                match self
                    .auth
                    .refresh_token(&stored.token_pair.refresh_token)
                    .await
                {
                    Ok(new_pair) => {
                        let new_stored = StoredTokenPair {
                            id: Uuid::new_v4(),
                            nip: nip.clone(),
                            environment: self.environment,
                            token_pair: new_pair.clone(),
                            created_at: chrono::Utc::now(),
                        };
                        self.repo.save_token_pair(&new_stored).await?;
                        return Ok(new_pair);
                    }
                    Err(err) => {
                        tracing::warn!("token refresh failed, re-authenticating: {err}");
                    }
                }
            }
        }

        self.authenticate(nip).await
    }

    /// Open an interactive `KSeF` session with encryption material for invoice payloads.
    pub async fn ensure_session(
        &self,
        nip: &Nip,
        session_encryption: &EncryptedInvoice,
    ) -> Result<SessionReference, SessionServiceError> {
        let token_pair = self.ensure_token(nip).await?;
        if let Some(existing) = self.repo.find_active_session(nip, self.environment).await? {
            self.client
                .close_session(&token_pair.access_token, &existing.session_reference)
                .await?;
            self.repo.terminate_session(existing.id).await?;
        }

        let session_ref = self
            .client
            .open_session(&token_pair.access_token, session_encryption)
            .await?;

        let stored = StoredSession {
            id: Uuid::new_v4(),
            session_reference: session_ref.clone(),
            nip: nip.clone(),
            environment: self.environment,
            created_at: chrono::Utc::now(),
            expires_at: chrono::Utc::now() + chrono::Duration::hours(12),
            terminated_at: None,
        };
        self.repo.save_session(&stored).await?;

        Ok(session_ref)
    }

    /// Close the active session and retrieve UPO.
    pub async fn close_session(&self, nip: &Nip) -> Result<Upo, SessionServiceError> {
        let session = self
            .repo
            .find_active_session(nip, self.environment)
            .await?
            .ok_or_else(|| {
                SessionServiceError::AuthFailed("no active session to close".to_string())
            })?;

        let token_pair = self.ensure_token(nip).await?;
        let upo = self
            .client
            .close_session(&token_pair.access_token, &session.session_reference)
            .await?;
        self.repo.terminate_session(session.id).await?;

        Ok(upo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::crypto::EncryptedInvoice;
    use crate::domain::nip::Nip;
    use crate::test_support::mock_ksef::{MockKSeFAuth, MockKSeFClient, MockXadesSigner};
    use crate::test_support::mock_session_repo::MockSessionRepo;

    fn test_nip() -> Nip {
        Nip::parse("5260250274").unwrap()
    }

    fn test_encrypted_invoice() -> EncryptedInvoice {
        EncryptedInvoice::new(
            b"mock-encrypted-key".to_vec(),
            b"mock-iv-16bytes!".to_vec(),
            b"mock-encrypted-data".to_vec(),
            "mock-plaintext-hash".to_string(),
            123,
            "mock-encrypted-hash".to_string(),
            456,
        )
    }

    fn make_service() -> (SessionService, Arc<MockKSeFAuth>, Arc<MockSessionRepo>) {
        let auth = Arc::new(MockKSeFAuth::new());
        let signer = Arc::new(MockXadesSigner);
        let client = Arc::new(MockKSeFClient::new());
        let repo = Arc::new(MockSessionRepo::new());

        let service = SessionService::new(
            auth.clone(),
            signer,
            client,
            repo.clone(),
            KSeFEnvironment::Test,
        );
        (service, auth, repo)
    }

    fn make_service_with_auth_method(
        auth_method: AuthMethod,
    ) -> (SessionService, Arc<MockKSeFAuth>, Arc<MockSessionRepo>) {
        let auth = Arc::new(MockKSeFAuth::new());
        let signer = Arc::new(MockXadesSigner);
        let client = Arc::new(MockKSeFClient::new());
        let repo = Arc::new(MockSessionRepo::new());

        let service = SessionService::with_auth_method(
            auth.clone(),
            signer,
            client,
            repo.clone(),
            KSeFEnvironment::Test,
            auth_method,
        );
        (service, auth, repo)
    }

    // --- authenticate ---

    #[tokio::test]
    async fn authenticate_performs_full_flow_and_persists_token() {
        let (service, auth, repo) = make_service();
        let nip = test_nip();

        let token_pair = service.authenticate(&nip).await.unwrap();

        assert_eq!(token_pair.access_token.as_str(), "mock-access-token");
        assert_eq!(*auth.challenge_count.lock().unwrap(), 1);
        assert_eq!(*auth.redeem_count.lock().unwrap(), 1);

        let stored = repo
            .find_active_token(&nip, KSeFEnvironment::Test)
            .await
            .unwrap();
        assert!(stored.is_some());
    }

    #[tokio::test]
    async fn authenticate_retries_processing_until_completed() {
        let (service, auth, _) = make_service();
        let nip = test_nip();
        auth.set_poll_statuses(vec![
            AuthStatus::Processing,
            AuthStatus::Processing,
            AuthStatus::Completed,
        ]);

        let token_pair = service.authenticate(&nip).await.unwrap();
        assert_eq!(token_pair.access_token.as_str(), "mock-access-token");
        assert_eq!(*auth.redeem_count.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn authenticate_returns_error_after_processing_timeout() {
        let (service, auth, _) = make_service();
        let nip = test_nip();
        auth.set_poll_statuses(vec![AuthStatus::Processing; AUTH_POLL_MAX_ATTEMPTS + 1]);

        let err = service.authenticate(&nip).await.unwrap_err();
        assert!(matches!(err, SessionServiceError::AuthFailed(_)));
    }

    #[tokio::test]
    async fn authenticate_with_token_method_skips_challenge_signing() {
        let nip = test_nip();
        let (service, auth, _) = make_service_with_auth_method(AuthMethod::Token {
            context: ContextIdentifier::Nip(nip.clone()),
            token: "bootstrap-token".to_string(),
        });

        let token_pair = service.authenticate(&nip).await.unwrap();
        assert_eq!(token_pair.access_token.as_str(), "mock-access-token");
        assert_eq!(*auth.challenge_count.lock().unwrap(), 0);
        assert_eq!(*auth.token_auth_count.lock().unwrap(), 1);
        assert_eq!(*auth.redeem_count.lock().unwrap(), 1);
    }

    // --- ensure_token ---

    #[tokio::test]
    async fn ensure_token_returns_cached_when_valid() {
        let (service, auth, _) = make_service();
        let nip = test_nip();

        // First call authenticates
        service.ensure_token(&nip).await.unwrap();
        assert_eq!(*auth.challenge_count.lock().unwrap(), 1);

        // Second call returns cached — no new challenge
        let token = service.ensure_token(&nip).await.unwrap();
        assert_eq!(*auth.challenge_count.lock().unwrap(), 1);
        assert_eq!(token.access_token.as_str(), "mock-access-token");
    }

    #[tokio::test]
    async fn ensure_token_authenticates_when_no_token_exists() {
        let (service, auth, _) = make_service();
        let nip = test_nip();

        service.ensure_token(&nip).await.unwrap();
        assert_eq!(*auth.challenge_count.lock().unwrap(), 1);
    }

    // --- ensure_session ---

    #[tokio::test]
    async fn ensure_session_opens_new_session_and_persists() {
        let (service, _, repo) = make_service();
        let nip = test_nip();
        let encrypted = test_encrypted_invoice();

        let session_ref = service.ensure_session(&nip, &encrypted).await.unwrap();
        assert_eq!(session_ref.as_str(), "mock-session-ref");

        let stored = repo
            .find_active_session(&nip, KSeFEnvironment::Test)
            .await
            .unwrap();
        assert!(stored.is_some());
    }

    #[tokio::test]
    async fn ensure_session_closes_existing_and_opens_new_session() {
        let (service, auth, _) = make_service();
        let nip = test_nip();
        let encrypted = test_encrypted_invoice();

        let ref1 = service.ensure_session(&nip, &encrypted).await.unwrap();
        let ref2 = service.ensure_session(&nip, &encrypted).await.unwrap();

        assert_eq!(ref1.as_str(), ref2.as_str());
        // Access token was cached between calls.
        assert_eq!(*auth.challenge_count.lock().unwrap(), 1);
    }

    // --- close_session ---

    #[tokio::test]
    async fn close_session_terminates_and_persists() {
        let (service, _, repo) = make_service();
        let nip = test_nip();
        let encrypted = test_encrypted_invoice();

        service.ensure_session(&nip, &encrypted).await.unwrap();
        let upo = service.close_session(&nip).await.unwrap();
        assert_eq!(upo.reference, "mock-upo-ref");

        let active = repo
            .find_active_session(&nip, KSeFEnvironment::Test)
            .await
            .unwrap();
        assert!(active.is_none());
    }

    #[tokio::test]
    async fn close_session_without_active_session_returns_error() {
        let (service, _, _) = make_service();
        let nip = test_nip();

        let err = service.close_session(&nip).await.unwrap_err();
        assert!(matches!(err, SessionServiceError::AuthFailed(_)));
    }

    // --- full lifecycle ---

    #[tokio::test]
    async fn full_lifecycle_auth_session_close() {
        let (service, _, repo) = make_service();
        let nip = test_nip();
        let encrypted = test_encrypted_invoice();

        // Authenticate
        let token = service.authenticate(&nip).await.unwrap();
        assert!(!token.is_access_expired());

        // Open session
        let session = service.ensure_session(&nip, &encrypted).await.unwrap();
        assert_eq!(session.as_str(), "mock-session-ref");

        // Close session
        let upo = service.close_session(&nip).await.unwrap();
        assert_eq!(upo.reference, "mock-upo-ref");

        // Session is terminated
        let active = repo
            .find_active_session(&nip, KSeFEnvironment::Test)
            .await
            .unwrap();
        assert!(active.is_none());
    }
}
