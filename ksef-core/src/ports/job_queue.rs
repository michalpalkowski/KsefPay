use async_trait::async_trait;

use crate::domain::job::{Job, JobId};
use crate::error::QueueError;

/// Port: background job queue.
#[async_trait]
pub trait JobQueue: Send + Sync {
    async fn enqueue(&self, job: Job) -> Result<JobId, QueueError>;

    async fn dequeue(&self) -> Result<Option<Job>, QueueError>;

    async fn complete(&self, id: &JobId) -> Result<(), QueueError>;

    async fn fail(&self, id: &JobId, error: &str) -> Result<(), QueueError>;

    async fn dead_letter(&self, id: &JobId, error: &str) -> Result<(), QueueError>;

    async fn list_pending(&self) -> Result<Vec<Job>, QueueError>;

    async fn list_dead_letter(&self) -> Result<Vec<Job>, QueueError>;
}
