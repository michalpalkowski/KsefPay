use sqlx::SqliteExecutor;

use crate::domain::nip::Nip;
use crate::error::RepositoryError;

/// Atomically increment and return next invoice number.
///
/// SQLite `ON CONFLICT ... DO UPDATE` + `RETURNING` achieves the same
/// single-statement atomicity as the Postgres version.
pub async fn next_number<'e>(
    exec: impl SqliteExecutor<'e>,
    seller_nip: &Nip,
    year: i32,
    month: u32,
) -> Result<u32, RepositoryError> {
    let row: (i32,) = sqlx::query_as(
        r"INSERT INTO invoice_sequences (seller_nip, year, month, last_number)
        VALUES (?, ?, ?, 1)
        ON CONFLICT (seller_nip, year, month)
        DO UPDATE SET last_number = invoice_sequences.last_number + 1
        RETURNING last_number",
    )
    .bind(seller_nip.as_str())
    .bind(year)
    .bind(month as i32)
    .fetch_one(exec)
    .await?;

    u32::try_from(row.0).map_err(|_| {
        RepositoryError::Database(sqlx::Error::Decode(
            format!("invoice sequence overflow: {}", row.0).into(),
        ))
    })
}
