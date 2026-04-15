use async_trait::async_trait;

use crate::domain::user::{User, UserId};
use crate::error::RepositoryError;

/// Port: user account persistence.
#[async_trait]
pub trait UserRepository: Send + Sync {
    /// Create a new user. Returns `Duplicate` if email already exists.
    async fn create(&self, user: &User) -> Result<UserId, RepositoryError>;

    /// Find user by ID. Returns `NotFound` if missing.
    async fn find_by_id(&self, id: &UserId) -> Result<User, RepositoryError>;

    /// Find user by email. Returns `None` if not found (not an error — used for login).
    async fn find_by_email(&self, email: &str) -> Result<Option<User>, RepositoryError>;

    /// Update user's password hash. Returns `NotFound` if user missing.
    async fn update_password(&self, user: &User) -> Result<(), RepositoryError>;
}
