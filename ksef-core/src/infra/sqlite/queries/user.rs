use sqlx::SqliteExecutor;

use super::datetime::parse_sqlite_datetime;
use crate::domain::user::{User, UserId};
use crate::error::RepositoryError;

#[derive(sqlx::FromRow)]
pub(crate) struct UserRow {
    pub id: String,
    pub email: String,
    pub password_hash: String,
    pub created_at: String,
    pub updated_at: String,
}

fn decode_err(msg: String) -> RepositoryError {
    RepositoryError::Database(sqlx::Error::Decode(msg.into()))
}

fn parse_datetime(
    value: &str,
    field: &'static str,
) -> Result<chrono::DateTime<chrono::Utc>, RepositoryError> {
    parse_sqlite_datetime(value, field).map_err(decode_err)
}

impl UserRow {
    pub(crate) fn into_domain(self) -> Result<User, RepositoryError> {
        let id = uuid::Uuid::parse_str(&self.id)
            .map_err(|e| decode_err(format!("invalid user id '{}': {e}", self.id)))?;

        Ok(User {
            id: UserId::from_uuid(id),
            email: self.email,
            password_hash: self.password_hash,
            created_at: parse_datetime(&self.created_at, "created_at")?,
            updated_at: parse_datetime(&self.updated_at, "updated_at")?,
        })
    }
}

pub async fn create<'e>(
    exec: impl SqliteExecutor<'e>,
    user: &User,
) -> Result<UserId, RepositoryError> {
    sqlx::query(
        r"INSERT INTO users (id, email, password_hash, created_at, updated_at)
           VALUES (?1, ?2, ?3, ?4, ?5)",
    )
    .bind(user.id.to_string())
    .bind(&user.email)
    .bind(&user.password_hash)
    .bind(user.created_at.to_rfc3339())
    .bind(user.updated_at.to_rfc3339())
    .execute(exec)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db_err) if db_err.is_unique_violation() => {
            RepositoryError::Duplicate {
                entity: "User",
                key: user.email.clone(),
            }
        }
        _ => RepositoryError::Database(e),
    })?;

    Ok(user.id.clone())
}

pub async fn find_by_id<'e>(
    exec: impl SqliteExecutor<'e>,
    id: &UserId,
) -> Result<User, RepositoryError> {
    let row: UserRow = sqlx::query_as("SELECT * FROM users WHERE id = ?1")
        .bind(id.to_string())
        .fetch_optional(exec)
        .await?
        .ok_or_else(|| RepositoryError::NotFound {
            entity: "User",
            id: id.to_string(),
        })?;
    row.into_domain()
}

pub async fn find_by_email<'e>(
    exec: impl SqliteExecutor<'e>,
    email: &str,
) -> Result<Option<User>, RepositoryError> {
    let row: Option<UserRow> = sqlx::query_as("SELECT * FROM users WHERE email = ?1")
        .bind(email)
        .fetch_optional(exec)
        .await?;
    row.map(UserRow::into_domain).transpose()
}

pub async fn update_password<'e>(
    exec: impl SqliteExecutor<'e>,
    user: &User,
) -> Result<(), RepositoryError> {
    let result = sqlx::query(
        r"UPDATE users SET password_hash = ?1, updated_at = ?2 WHERE id = ?3",
    )
    .bind(&user.password_hash)
    .bind(user.updated_at.to_rfc3339())
    .bind(user.id.to_string())
    .execute(exec)
    .await?;

    if result.rows_affected() == 0 {
        return Err(RepositoryError::NotFound {
            entity: "User",
            id: user.id.to_string(),
        });
    }
    Ok(())
}
