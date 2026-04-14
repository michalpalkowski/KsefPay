use chrono::{DateTime, NaiveDateTime, Utc};
use sqlx::SqliteExecutor;

use crate::domain::job::{Job, JobId, JobStatus};
use crate::error::QueueError;

#[derive(sqlx::FromRow)]
pub(crate) struct JobRow {
    pub id: String,
    pub job_type: String,
    pub payload: String,
    pub status: String,
    pub attempts: i32,
    pub max_attempts: i32,
    pub last_error: Option<String>,
    pub created_at: String,
}

fn parse_datetime(value: &str, field: &'static str) -> Result<DateTime<Utc>, QueueError> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(value) {
        return Ok(dt.with_timezone(&Utc));
    }
    if let Ok(naive) = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S") {
        return Ok(DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc));
    }
    if let Ok(naive) = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f") {
        return Ok(DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc));
    }
    Err(QueueError::DequeueFailed(format!(
        "invalid datetime in {field}: '{value}'"
    )))
}

impl JobRow {
    pub(crate) fn try_into_domain(self) -> Result<Job, QueueError> {
        let status = match self.status.as_str() {
            "pending" => JobStatus::Pending,
            "running" => JobStatus::Running,
            "completed" => JobStatus::Completed,
            "dead_letter" => JobStatus::DeadLetter,
            "failed" => JobStatus::Failed,
            other => {
                return Err(QueueError::DequeueFailed(format!(
                    "unknown job status '{other}' for job {}",
                    self.id
                )));
            }
        };

        let id = uuid::Uuid::parse_str(&self.id)
            .map_err(|e| QueueError::DequeueFailed(format!("invalid job id '{}': {e}", self.id)))?;

        let payload: serde_json::Value = serde_json::from_str(&self.payload).map_err(|e| {
            QueueError::DequeueFailed(format!("invalid payload JSON for job {}: {e}", self.id))
        })?;

        let attempts = u32::try_from(self.attempts).map_err(|_| {
            QueueError::DequeueFailed(format!(
                "job {} has invalid attempts value {}",
                self.id, self.attempts
            ))
        })?;

        let max_attempts = u32::try_from(self.max_attempts).map_err(|_| {
            QueueError::DequeueFailed(format!(
                "job {} has invalid max_attempts value {}",
                self.id, self.max_attempts
            ))
        })?;

        Ok(Job {
            id: JobId::from_uuid(id),
            job_type: self.job_type,
            payload,
            status,
            attempts,
            max_attempts,
            last_error: self.last_error,
            created_at: parse_datetime(&self.created_at, "created_at")?,
        })
    }
}

pub async fn enqueue<'e>(exec: impl SqliteExecutor<'e>, job: &Job) -> Result<JobId, QueueError> {
    let attempts = i32::try_from(job.attempts).map_err(|_| {
        QueueError::EnqueueFailed(format!(
            "job {} attempts value {} exceeds i32 range",
            job.id, job.attempts
        ))
    })?;
    let max_attempts = i32::try_from(job.max_attempts).map_err(|_| {
        QueueError::EnqueueFailed(format!(
            "job {} max_attempts value {} exceeds i32 range",
            job.id, job.max_attempts
        ))
    })?;

    sqlx::query(
        r"INSERT INTO jobs (id, job_type, payload, status, attempts, max_attempts, last_error, created_at)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )
    .bind(job.id.to_string())
    .bind(&job.job_type)
    .bind(job.payload.to_string())
    .bind(job.status.to_string())
    .bind(attempts)
    .bind(max_attempts)
    .bind(&job.last_error)
    .bind(job.created_at.to_rfc3339())
    .execute(exec)
    .await?;

    Ok(job.id.clone())
}

pub async fn dequeue<'e>(exec: impl SqliteExecutor<'e>) -> Result<Option<Job>, QueueError> {
    let row: Option<JobRow> = sqlx::query_as(
        r"UPDATE jobs
           SET status = 'running', started_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
           WHERE id = (
               SELECT id FROM jobs
               WHERE status = 'pending' AND datetime(scheduled_at) <= datetime('now')
               ORDER BY datetime(scheduled_at) ASC
               LIMIT 1
           )
           RETURNING id, job_type, payload, status, attempts, max_attempts, last_error, created_at",
    )
    .fetch_optional(exec)
    .await?;

    row.map(JobRow::try_into_domain).transpose()
}

pub async fn complete<'e>(exec: impl SqliteExecutor<'e>, id: &JobId) -> Result<(), QueueError> {
    let result = sqlx::query(
        "UPDATE jobs SET status = 'completed', completed_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = ?1",
    )
    .bind(id.to_string())
    .execute(exec)
    .await?;

    if result.rows_affected() == 0 {
        return Err(QueueError::JobNotFound(id.to_string()));
    }
    Ok(())
}

pub async fn fail<'e>(
    exec: impl SqliteExecutor<'e>,
    id: &JobId,
    error: &str,
) -> Result<(), QueueError> {
    let result = sqlx::query(
        r"UPDATE jobs SET
            attempts = attempts + 1,
            last_error = ?2,
            status = CASE
                WHEN attempts + 1 >= max_attempts THEN 'dead_letter'
                ELSE 'pending'
            END,
            scheduled_at = CASE
                WHEN attempts + 1 >= max_attempts THEN scheduled_at
                ELSE strftime('%Y-%m-%dT%H:%M:%fZ','now', '+' || CAST((1 << attempts) AS TEXT) || ' seconds')
            END,
            started_at = CASE
                WHEN attempts + 1 >= max_attempts THEN started_at
                ELSE NULL
            END
           WHERE id = ?1",
    )
    .bind(id.to_string())
    .bind(error)
    .execute(exec)
    .await?;

    if result.rows_affected() == 0 {
        return Err(QueueError::JobNotFound(id.to_string()));
    }
    Ok(())
}

pub async fn dead_letter<'e>(
    exec: impl SqliteExecutor<'e>,
    id: &JobId,
    error: &str,
) -> Result<(), QueueError> {
    let result =
        sqlx::query("UPDATE jobs SET status = 'dead_letter', last_error = ?2 WHERE id = ?1")
            .bind(id.to_string())
            .bind(error)
            .execute(exec)
            .await?;

    if result.rows_affected() == 0 {
        return Err(QueueError::JobNotFound(id.to_string()));
    }
    Ok(())
}

pub async fn list_pending<'e>(exec: impl SqliteExecutor<'e>) -> Result<Vec<Job>, QueueError> {
    let rows: Vec<JobRow> = sqlx::query_as(
        "SELECT id, job_type, payload, status, attempts, max_attempts, last_error, created_at
         FROM jobs WHERE status = 'pending' ORDER BY datetime(scheduled_at) ASC",
    )
    .fetch_all(exec)
    .await?;

    rows.into_iter().map(JobRow::try_into_domain).collect()
}

pub async fn list_dead_letter<'e>(exec: impl SqliteExecutor<'e>) -> Result<Vec<Job>, QueueError> {
    let rows: Vec<JobRow> = sqlx::query_as(
        "SELECT id, job_type, payload, status, attempts, max_attempts, last_error, created_at
         FROM jobs WHERE status = 'dead_letter' ORDER BY datetime(created_at) DESC",
    )
    .fetch_all(exec)
    .await?;

    rows.into_iter().map(JobRow::try_into_domain).collect()
}
