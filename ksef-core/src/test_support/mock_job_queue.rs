use std::sync::Mutex;

use async_trait::async_trait;

use crate::domain::job::{Job, JobId, JobStatus};
use crate::error::QueueError;
use crate::ports::job_queue::JobQueue;

/// In-memory mock of `JobQueue` for unit tests.
pub struct MockJobQueue {
    jobs: Mutex<Vec<Job>>,
}

impl MockJobQueue {
    #[must_use]
    pub fn new() -> Self {
        Self {
            jobs: Mutex::new(Vec::new()),
        }
    }

    #[must_use]
    pub fn count(&self) -> usize {
        self.jobs.lock().unwrap().len()
    }
}

impl Default for MockJobQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl MockJobQueue {
    /// Inspect all jobs in the queue (for test assertions).
    #[must_use]
    pub fn snapshot(&self) -> Vec<Job> {
        self.jobs.lock().unwrap().clone()
    }
}

#[async_trait]
impl JobQueue for MockJobQueue {
    async fn enqueue(&self, job: Job) -> Result<JobId, QueueError> {
        let mut store = self.jobs.lock().unwrap();
        let id = job.id.clone();
        store.push(job);
        Ok(id)
    }

    async fn dequeue(&self) -> Result<Option<Job>, QueueError> {
        let mut store = self.jobs.lock().unwrap();
        let pos = store.iter().position(|j| j.status == JobStatus::Pending);
        match pos {
            Some(idx) => {
                store[idx].status = JobStatus::Running;
                Ok(Some(store[idx].clone()))
            }
            None => Ok(None),
        }
    }

    async fn complete(&self, id: &JobId) -> Result<(), QueueError> {
        let mut store = self.jobs.lock().unwrap();
        let job = store
            .iter_mut()
            .find(|j| j.id.as_uuid() == id.as_uuid())
            .ok_or_else(|| QueueError::JobNotFound(id.to_string()))?;
        job.status = JobStatus::Completed;
        Ok(())
    }

    async fn fail(&self, id: &JobId, error: &str) -> Result<(), QueueError> {
        let mut store = self.jobs.lock().unwrap();
        let job = store
            .iter_mut()
            .find(|j| j.id.as_uuid() == id.as_uuid())
            .ok_or_else(|| QueueError::JobNotFound(id.to_string()))?;
        job.attempts += 1;
        job.last_error = Some(error.to_string());
        if job.attempts >= job.max_attempts {
            job.status = JobStatus::DeadLetter;
        } else {
            job.status = JobStatus::Pending;
        }
        Ok(())
    }

    async fn dead_letter(&self, id: &JobId, error: &str) -> Result<(), QueueError> {
        let mut store = self.jobs.lock().unwrap();
        let job = store
            .iter_mut()
            .find(|j| j.id.as_uuid() == id.as_uuid())
            .ok_or_else(|| QueueError::JobNotFound(id.to_string()))?;
        job.status = JobStatus::DeadLetter;
        job.last_error = Some(error.to_string());
        Ok(())
    }

    async fn list_pending(&self) -> Result<Vec<Job>, QueueError> {
        let store = self.jobs.lock().unwrap();
        Ok(store
            .iter()
            .filter(|j| j.status == JobStatus::Pending)
            .cloned()
            .collect())
    }

    async fn list_dead_letter(&self) -> Result<Vec<Job>, QueueError> {
        let store = self.jobs.lock().unwrap();
        Ok(store
            .iter()
            .filter(|j| j.status == JobStatus::DeadLetter)
            .cloned()
            .collect())
    }
}

#[cfg(test)]
fn make_job(job_type: &str) -> Job {
    Job {
        id: JobId::new(),
        job_type: job_type.to_string(),
        payload: serde_json::json!({"invoice_id": "test-123"}),
        status: JobStatus::Pending,
        attempts: 0,
        max_attempts: 3,
        last_error: None,
        created_at: chrono::Utc::now(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Contract test: enqueue then dequeue returns the job.
    #[tokio::test]
    async fn enqueue_then_dequeue_returns_job() {
        let queue = MockJobQueue::new();
        let job = make_job("submit_invoice");
        let id = queue.enqueue(job).await.unwrap();

        let dequeued = queue.dequeue().await.unwrap().unwrap();
        assert_eq!(dequeued.id.as_uuid(), id.as_uuid());
        assert_eq!(dequeued.status, JobStatus::Running);
    }

    /// Contract test: dequeue on empty queue returns None.
    #[tokio::test]
    async fn dequeue_empty_returns_none() {
        let queue = MockJobQueue::new();
        assert!(queue.dequeue().await.unwrap().is_none());
    }

    /// Contract test: complete marks job as completed.
    #[tokio::test]
    async fn complete_marks_completed() {
        let queue = MockJobQueue::new();
        let job = make_job("submit_invoice");
        let id = queue.enqueue(job).await.unwrap();
        queue.dequeue().await.unwrap(); // transition to Running

        queue.complete(&id).await.unwrap();

        // Should not appear in pending or dead letter
        assert!(queue.list_pending().await.unwrap().is_empty());
        assert!(queue.list_dead_letter().await.unwrap().is_empty());
    }

    /// Contract test: fail increments attempts, preserves error, and requeues.
    #[tokio::test]
    async fn fail_increments_attempts() {
        let queue = MockJobQueue::new();
        let job = make_job("submit_invoice");
        let id = queue.enqueue(job).await.unwrap();
        queue.dequeue().await.unwrap();

        queue.fail(&id, "connection refused").await.unwrap();

        let jobs = queue.snapshot();
        let job = jobs
            .iter()
            .find(|j| j.id.as_uuid() == id.as_uuid())
            .unwrap();
        assert_eq!(job.attempts, 1);
        assert_eq!(job.last_error.as_deref(), Some("connection refused"));
        assert_eq!(job.status, JobStatus::Pending);
    }

    /// Contract test: fail after max_attempts moves to dead letter.
    #[tokio::test]
    async fn fail_after_max_attempts_dead_letters() {
        let queue = MockJobQueue::new();
        let mut job = make_job("submit_invoice");
        job.max_attempts = 2;
        let id = queue.enqueue(job).await.unwrap();
        queue.dequeue().await.unwrap();

        queue.fail(&id, "error 1").await.unwrap();
        queue.fail(&id, "error 2").await.unwrap();

        let dead = queue.list_dead_letter().await.unwrap();
        assert_eq!(dead.len(), 1);
        assert_eq!(dead[0].id.as_uuid(), id.as_uuid());
        assert_eq!(dead[0].last_error.as_deref(), Some("error 2"));
    }

    /// Contract test: dead_letter explicitly moves to dead letter.
    #[tokio::test]
    async fn dead_letter_explicit() {
        let queue = MockJobQueue::new();
        let job = make_job("submit_invoice");
        let id = queue.enqueue(job).await.unwrap();

        queue.dead_letter(&id, "permanent failure").await.unwrap();

        let dead = queue.list_dead_letter().await.unwrap();
        assert_eq!(dead.len(), 1);
    }

    /// Contract test: operations on missing job return error.
    #[tokio::test]
    async fn operations_on_missing_job_return_error() {
        let queue = MockJobQueue::new();
        let missing = JobId::new();

        assert!(queue.complete(&missing).await.is_err());
        assert!(queue.fail(&missing, "err").await.is_err());
        assert!(queue.dead_letter(&missing, "err").await.is_err());
    }

    /// Contract test: dequeue skips non-pending jobs.
    #[tokio::test]
    async fn dequeue_skips_non_pending() {
        let queue = MockJobQueue::new();
        let job1 = make_job("job1");
        let id1 = queue.enqueue(job1).await.unwrap();
        queue.dequeue().await.unwrap(); // job1 is now Running
        queue.complete(&id1).await.unwrap(); // job1 is now Completed

        let job2 = make_job("job2");
        queue.enqueue(job2).await.unwrap();

        let dequeued = queue.dequeue().await.unwrap().unwrap();
        assert_eq!(dequeued.job_type, "job2");
    }

    /// Contract test: list_pending only returns pending jobs.
    #[tokio::test]
    async fn list_pending_filters_correctly() {
        let queue = MockJobQueue::new();
        queue.enqueue(make_job("pending1")).await.unwrap();
        queue.enqueue(make_job("pending2")).await.unwrap();
        let _id3 = queue.enqueue(make_job("will_complete")).await.unwrap();
        queue.dequeue().await.unwrap(); // takes pending1
        queue.dequeue().await.unwrap(); // takes pending2
        // pending1 and pending2 are Running, will_complete is still Pending... wait
        // Actually dequeue takes first pending: pending1 -> Running, then pending2 -> Running
        // will_complete is still Pending

        // Hmm, the 3rd dequeue would get will_complete
        // Let's just verify list_pending
        let pending = queue.list_pending().await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].job_type, "will_complete");
    }
}
