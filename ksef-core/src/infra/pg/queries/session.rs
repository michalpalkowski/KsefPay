use sqlx::PgExecutor;

use crate::domain::auth::{AccessToken, RefreshToken, TokenPair};
use crate::domain::environment::KSeFEnvironment;
use crate::domain::nip::Nip;
use crate::domain::session::SessionReference;
use crate::error::RepositoryError;
use crate::ports::session_repository::{StoredSession, StoredTokenPair};

#[derive(sqlx::FromRow)]
pub(crate) struct TokenRow {
    pub id: uuid::Uuid,
    pub nip: String,
    pub environment: String,
    pub access_token: String,
    pub refresh_token: String,
    pub access_token_expires_at: chrono::DateTime<chrono::Utc>,
    pub refresh_token_expires_at: chrono::DateTime<chrono::Utc>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(sqlx::FromRow)]
pub(crate) struct SessionRow {
    pub id: uuid::Uuid,
    pub session_reference: String,
    pub nip: String,
    pub environment: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
    pub terminated_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn parse_env(s: &str) -> Result<KSeFEnvironment, RepositoryError> {
    s.parse().map_err(|_| {
        RepositoryError::Database(sqlx::Error::Decode(
            format!("invalid KSeF environment in database: '{s}'").into(),
        ))
    })
}

fn decode_nip(s: &str, ctx: &str) -> Result<Nip, RepositoryError> {
    Nip::parse(s).map_err(|e| {
        RepositoryError::Database(sqlx::Error::Decode(
            format!("invalid NIP in {ctx}: {e}").into(),
        ))
    })
}

impl TokenRow {
    pub(crate) fn into_domain(self) -> Result<StoredTokenPair, RepositoryError> {
        Ok(StoredTokenPair {
            id: self.id,
            nip: decode_nip(&self.nip, "token row")?,
            environment: parse_env(&self.environment)?,
            token_pair: TokenPair {
                access_token: AccessToken::new(self.access_token),
                refresh_token: RefreshToken::new(self.refresh_token),
                access_token_expires_at: self.access_token_expires_at,
                refresh_token_expires_at: self.refresh_token_expires_at,
            },
            created_at: self.created_at,
        })
    }
}

impl SessionRow {
    pub(crate) fn into_domain(self) -> Result<StoredSession, RepositoryError> {
        Ok(StoredSession {
            id: self.id,
            session_reference: SessionReference::new(self.session_reference),
            nip: decode_nip(&self.nip, "session row")?,
            environment: parse_env(&self.environment)?,
            created_at: self.created_at,
            expires_at: self.expires_at,
            terminated_at: self.terminated_at,
        })
    }
}

pub async fn save_token_pair<'e>(
    exec: impl PgExecutor<'e>,
    token: &StoredTokenPair,
) -> Result<(), RepositoryError> {
    sqlx::query(
        r"INSERT INTO ksef_auth_tokens (id, nip, environment, access_token, refresh_token,
            access_token_expires_at, refresh_token_expires_at, created_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(token.id)
    .bind(token.nip.as_str())
    .bind(token.environment.to_string())
    .bind(token.token_pair.access_token.as_str())
    .bind(token.token_pair.refresh_token.as_str())
    .bind(token.token_pair.access_token_expires_at)
    .bind(token.token_pair.refresh_token_expires_at)
    .bind(token.created_at)
    .execute(exec)
    .await?;
    Ok(())
}

pub async fn find_active_token<'e>(
    exec: impl PgExecutor<'e>,
    nip: &Nip,
    environment: KSeFEnvironment,
) -> Result<Option<StoredTokenPair>, RepositoryError> {
    let row: Option<TokenRow> = sqlx::query_as(
        r"SELECT * FROM ksef_auth_tokens
           WHERE nip = $1 AND environment = $2 AND refresh_token_expires_at > NOW()
           ORDER BY created_at DESC LIMIT 1",
    )
    .bind(nip.as_str())
    .bind(environment.to_string())
    .fetch_optional(exec)
    .await?;
    row.map(TokenRow::into_domain).transpose()
}

pub async fn save_session<'e>(
    exec: impl PgExecutor<'e>,
    session: &StoredSession,
) -> Result<(), RepositoryError> {
    sqlx::query(
        r"INSERT INTO ksef_sessions (id, session_reference, nip, environment, created_at, expires_at, terminated_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(session.id)
    .bind(session.session_reference.as_str())
    .bind(session.nip.as_str())
    .bind(session.environment.to_string())
    .bind(session.created_at)
    .bind(session.expires_at)
    .bind(session.terminated_at)
    .execute(exec)
    .await?;
    Ok(())
}

pub async fn find_active_session<'e>(
    exec: impl PgExecutor<'e>,
    nip: &Nip,
    environment: KSeFEnvironment,
) -> Result<Option<StoredSession>, RepositoryError> {
    let row: Option<SessionRow> = sqlx::query_as(
        r"SELECT * FROM ksef_sessions
           WHERE nip = $1 AND environment = $2 AND terminated_at IS NULL AND expires_at > NOW()
           ORDER BY created_at DESC LIMIT 1",
    )
    .bind(nip.as_str())
    .bind(environment.to_string())
    .fetch_optional(exec)
    .await?;
    row.map(SessionRow::into_domain).transpose()
}

pub async fn terminate_session<'e>(
    exec: impl PgExecutor<'e>,
    session_id: uuid::Uuid,
) -> Result<(), RepositoryError> {
    let result = sqlx::query(
        "UPDATE ksef_sessions SET terminated_at = NOW() WHERE id = $1 AND terminated_at IS NULL",
    )
    .bind(session_id)
    .execute(exec)
    .await?;
    if result.rows_affected() == 0 {
        return Err(RepositoryError::NotFound {
            entity: "Session",
            id: session_id.to_string(),
        });
    }
    Ok(())
}
