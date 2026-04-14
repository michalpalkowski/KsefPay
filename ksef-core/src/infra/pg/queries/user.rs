use sqlx::PgExecutor;

use crate::domain::user::{User, UserId};
use crate::error::RepositoryError;

#[derive(sqlx::FromRow)]
struct UserRow {
    id: uuid::Uuid,
    email: String,
    password_hash: String,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl UserRow {
    fn into_domain(self) -> User {
        User {
            id: UserId::from_uuid(self.id),
            email: self.email,
            password_hash: self.password_hash,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

pub async fn create<'e>(
    exec: impl PgExecutor<'e>,
    user: &User,
) -> Result<UserId, RepositoryError> {
    sqlx::query(
        r"INSERT INTO users (id, email, password_hash, created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(user.id.as_uuid())
    .bind(&user.email)
    .bind(&user.password_hash)
    .bind(user.created_at)
    .bind(user.updated_at)
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
    exec: impl PgExecutor<'e>,
    id: &UserId,
) -> Result<User, RepositoryError> {
    let row: UserRow = sqlx::query_as("SELECT * FROM users WHERE id = $1")
        .bind(id.as_uuid())
        .fetch_optional(exec)
        .await?
        .ok_or_else(|| RepositoryError::NotFound {
            entity: "User",
            id: id.to_string(),
        })?;
    Ok(row.into_domain())
}

pub async fn find_by_email<'e>(
    exec: impl PgExecutor<'e>,
    email: &str,
) -> Result<Option<User>, RepositoryError> {
    let row: Option<UserRow> = sqlx::query_as("SELECT * FROM users WHERE email = $1")
        .bind(email)
        .fetch_optional(exec)
        .await?;
    Ok(row.map(UserRow::into_domain))
}
