use async_trait::async_trait;

use crate::domain::auth::AccessToken;
use crate::domain::rate_limit::{ContextLimits, EffectiveApiRateLimits, SubjectLimits};
use crate::error::KSeFError;

/// Port: `KSeF` effective/context/subject rate-limits.
#[async_trait]
pub trait KSeFRateLimits: Send + Sync {
    async fn get_effective_limits(
        &self,
        access_token: &AccessToken,
    ) -> Result<EffectiveApiRateLimits, KSeFError>;

    async fn get_context_limits(
        &self,
        access_token: &AccessToken,
    ) -> Result<Vec<ContextLimits>, KSeFError>;

    async fn get_subject_limits(
        &self,
        access_token: &AccessToken,
    ) -> Result<Vec<SubjectLimits>, KSeFError>;
}
