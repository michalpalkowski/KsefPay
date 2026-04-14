use std::sync::Arc;

use crate::domain::auth::AccessToken;
use crate::domain::batch::{BatchFileInfo, BatchSession, PartUploadRequest};
use crate::error::KSeFError;
use crate::infra::batch::zip_builder::{BatchArchive, BatchFileBuilder};
use crate::ports::ksef_batch::{BatchOpenRequest, KSeFBatch};

pub struct BatchService {
    port: Arc<dyn KSeFBatch>,
    builder: BatchFileBuilder,
}

#[derive(Debug, thiserror::Error)]
pub enum BatchServiceError {
    #[error(transparent)]
    KSeF(#[from] KSeFError),
}

impl BatchService {
    #[must_use]
    pub fn new(port: Arc<dyn KSeFBatch>, builder: BatchFileBuilder) -> Self {
        Self { port, builder }
    }

    pub fn build_archive(
        &self,
        files: &[(String, Vec<u8>)],
    ) -> Result<BatchArchive, BatchServiceError> {
        Ok(self.builder.build(files)?)
    }

    pub async fn upload_archive(
        &self,
        access_token: &AccessToken,
        archive: &BatchArchive,
    ) -> Result<BatchSession, BatchServiceError> {
        let opened = self
            .port
            .open_batch_session(
                access_token,
                &BatchOpenRequest {
                    file: BatchFileInfo {
                        file_name: archive.file_info.file_name.clone(),
                        file_size_bytes: archive.file_info.file_size_bytes,
                        file_hash_sha256_base64: archive.file_info.file_hash_sha256_base64.clone(),
                    },
                },
            )
            .await?;

        for part in &archive.parts {
            let offset = usize::try_from(part.offset_bytes).map_err(|_| {
                KSeFError::InvoiceSubmissionFailed("part offset exceeds usize".to_string())
            })?;
            let size = usize::try_from(part.size_bytes).map_err(|_| {
                KSeFError::InvoiceSubmissionFailed("part size exceeds usize".to_string())
            })?;
            let payload = archive
                .zip_bytes
                .get(offset..offset + size)
                .ok_or_else(|| {
                    KSeFError::InvoiceSubmissionFailed(
                        "part boundaries exceed archive data".to_string(),
                    )
                })?;
            self.port
                .upload_part(
                    access_token,
                    &PartUploadRequest {
                        session_reference: opened.reference_number.clone(),
                        upload_url: String::new(),
                        part: part.clone(),
                    },
                    payload,
                )
                .await?;
        }

        let closed = self
            .port
            .close_batch_session(access_token, &opened.reference_number)
            .await?;
        Ok(closed)
    }

    pub async fn build_and_upload(
        &self,
        access_token: &AccessToken,
        files: &[(String, Vec<u8>)],
    ) -> Result<BatchSession, BatchServiceError> {
        let archive = self.build_archive(files)?;
        self.upload_archive(access_token, &archive).await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use chrono::Utc;

    use super::*;
    use crate::domain::batch::{BatchSessionStatus, PartUploadRequest};
    use crate::ports::ksef_batch::{BatchOpenRequest, KSeFBatch};

    #[derive(Default)]
    struct MockBatchPort {
        uploads: Mutex<HashMap<String, usize>>,
    }

    #[async_trait]
    impl KSeFBatch for MockBatchPort {
        async fn open_batch_session(
            &self,
            _access_token: &AccessToken,
            _request: &BatchOpenRequest,
        ) -> Result<BatchSession, KSeFError> {
            Ok(BatchSession {
                reference_number: "batch-1".to_string(),
                status: BatchSessionStatus::Created,
                created_at: Utc::now(),
                files: Vec::new(),
            })
        }

        async fn upload_part(
            &self,
            _access_token: &AccessToken,
            request: &PartUploadRequest,
            payload: &[u8],
        ) -> Result<(), KSeFError> {
            if payload.is_empty() {
                return Err(KSeFError::HttpError {
                    status: 400,
                    body: "empty payload".to_string(),
                });
            }
            let mut guard = self.uploads.lock().unwrap();
            let entry = guard.entry(request.session_reference.clone()).or_insert(0);
            *entry += 1;
            Ok(())
        }

        async fn close_batch_session(
            &self,
            _access_token: &AccessToken,
            session_reference: &str,
        ) -> Result<BatchSession, KSeFError> {
            Ok(BatchSession {
                reference_number: session_reference.to_string(),
                status: BatchSessionStatus::Closed,
                created_at: Utc::now(),
                files: Vec::new(),
            })
        }

        async fn get_batch_status(
            &self,
            _access_token: &AccessToken,
            session_reference: &str,
        ) -> Result<BatchSession, KSeFError> {
            Ok(BatchSession {
                reference_number: session_reference.to_string(),
                status: BatchSessionStatus::Uploading,
                created_at: Utc::now(),
                files: Vec::new(),
            })
        }
    }

    fn access_token() -> AccessToken {
        AccessToken::new("mock-access-token".to_string())
    }

    #[tokio::test]
    async fn build_and_upload_happy_path() {
        let service = BatchService::new(
            Arc::new(MockBatchPort::default()),
            BatchFileBuilder::new(1024),
        );
        let files = vec![
            ("a.xml".to_string(), b"<a/>".to_vec()),
            ("b.xml".to_string(), b"<b/>".to_vec()),
        ];

        let result = service
            .build_and_upload(&access_token(), &files)
            .await
            .unwrap();
        assert_eq!(result.reference_number, "batch-1");
        assert!(matches!(result.status, BatchSessionStatus::Closed));
    }

    #[tokio::test]
    async fn empty_input_fails_fast() {
        let service = BatchService::new(
            Arc::new(MockBatchPort::default()),
            BatchFileBuilder::default(),
        );
        let files = Vec::new();
        let err = service
            .build_and_upload(&access_token(), &files)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            BatchServiceError::KSeF(KSeFError::InvoiceSubmissionFailed(_))
        ));
    }
}
