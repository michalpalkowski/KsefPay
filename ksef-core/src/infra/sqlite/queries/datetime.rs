use chrono::{DateTime, NaiveDateTime, Utc};

/// Parse a datetime string stored in SQLite (RFC3339 or common NaiveDateTime formats).
///
/// Returns `Err(message)` on failure — callers wrap this in their own error type.
pub fn parse_sqlite_datetime(value: &str, field: &'static str) -> Result<DateTime<Utc>, String> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(value) {
        return Ok(dt.with_timezone(&Utc));
    }
    if let Ok(naive) = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S") {
        return Ok(DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc));
    }
    if let Ok(naive) = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f") {
        return Ok(DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc));
    }
    Err(format!("invalid datetime in {field}: '{value}'"))
}
