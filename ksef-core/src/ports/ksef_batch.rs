use async_trait::async_trait;

use crate::domain::auth::AccessToken;
use crate::domain::batch::{BatchFileInfo, BatchFilePartInfo, BatchSession, PartUploadRequest};
use crate::error::KSeFError;

#[derive(Debug, Clone)]
pub struct BatchOpenRequest {
    pub file: BatchFileInfo,
    pub parts: Vec<BatchFilePartInfo>,
}

/// Port: `KSeF` batch session workflow.
#[async_trait]
pub trait KSeFBatch: Send + Sync {
    async fn open_batch_session(
        &self,
        access_token: &AccessToken,
        request: &BatchOpenRequest,
    ) -> Result<BatchSession, KSeFError>;

    async fn upload_part(
        &self,
        access_token: &AccessToken,
        request: &PartUploadRequest,
        payload: &[u8],
    ) -> Result<(), KSeFError>;

    async fn close_batch_session(
        &self,
        access_token: &AccessToken,
        session_reference: &str,
    ) -> Result<BatchSession, KSeFError>;

    async fn get_batch_status(
        &self,
        access_token: &AccessToken,
        session_reference: &str,
    ) -> Result<BatchSession, KSeFError>;
}
