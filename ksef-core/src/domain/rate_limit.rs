use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RateLimitCategory {
    Auth,
    Session,
    Invoice,
    Query,
    PublicKey,
    TestData,
    Default,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextLimits {
    pub category: RateLimitCategory,
    pub per_second: u32,
    pub per_minute: u32,
    pub per_hour: u32,
    pub burst: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubjectLimits {
    pub subject_identifier: String,
    pub limits: Vec<ContextLimits>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectiveApiRateLimits {
    pub contexts: Vec<ContextLimits>,
    pub subjects: Vec<SubjectLimits>,
}
