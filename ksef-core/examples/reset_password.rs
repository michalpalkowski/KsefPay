use anyhow::{Context, Result, bail};
use argon2::password_hash::SaltString;
use argon2::password_hash::rand_core::OsRng;
use argon2::{Argon2, PasswordHasher};
use rand::Rng;
use sqlx::{PgPool, SqlitePool};

fn usage() {
    eprintln!("Usage: cargo run -p ksef-core --example reset_password -- <email>");
}

fn generate_password() -> String {
    const LOWER: &[u8] = b"abcdefghjkmnpqrstuvwxyz";
    const UPPER: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ";
    const DIGITS: &[u8] = b"23456789";
    const SPECIAL: &[u8] = b"!@#$%^&*()-_=+";

    let mut rng = rand::thread_rng();
    let mut chars = vec![
        *LOWER.get(rng.gen_range(0..LOWER.len())).unwrap_or(&b'a') as char,
        *UPPER.get(rng.gen_range(0..UPPER.len())).unwrap_or(&b'A') as char,
        *DIGITS.get(rng.gen_range(0..DIGITS.len())).unwrap_or(&b'2') as char,
        *SPECIAL
            .get(rng.gen_range(0..SPECIAL.len()))
            .unwrap_or(&b'!') as char,
    ];

    let all: Vec<u8> = [LOWER, UPPER, DIGITS, SPECIAL].concat();
    for _ in 0..12 {
        chars.push(*all.get(rng.gen_range(0..all.len())).unwrap_or(&b'x') as char);
    }

    // Fisher-Yates shuffle
    for i in (1..chars.len()).rev() {
        let j = rng.gen_range(0..=i);
        chars.swap(i, j);
    }

    chars.into_iter().collect()
}

fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("password hashing failed: {e}"))?
        .to_string();
    Ok(hash)
}

async fn reset_pg(database_url: &str, email: &str, password_hash: &str) -> Result<u64> {
    let pool = PgPool::connect(database_url)
        .await
        .context("failed to connect to PostgreSQL")?;

    let result = sqlx::query(
        r"UPDATE users
           SET password_hash = $1, updated_at = NOW()
           WHERE lower(email) = lower($2)",
    )
    .bind(password_hash)
    .bind(email)
    .execute(&pool)
    .await
    .context("failed to update user password in PostgreSQL")?;

    Ok(result.rows_affected())
}

async fn reset_sqlite(database_url: &str, email: &str, password_hash: &str) -> Result<u64> {
    let pool = SqlitePool::connect(database_url)
        .await
        .context("failed to connect to SQLite")?;

    let result = sqlx::query(
        r"UPDATE users
           SET password_hash = ?1, updated_at = ?2
           WHERE lower(email) = lower(?3)",
    )
    .bind(password_hash)
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(email)
    .execute(&pool)
    .await
    .context("failed to update user password in SQLite")?;

    Ok(result.rows_affected())
}

#[tokio::main]
async fn main() -> Result<()> {
    let email = match std::env::args().nth(1) {
        Some(value) if !value.trim().is_empty() => value.trim().to_string(),
        _ => {
            usage();
            std::process::exit(1);
        }
    };

    let database_url = std::env::var("DATABASE_URL")
        .context("DATABASE_URL is required (sqlite://... or postgres://...)")?;

    let new_password = generate_password();
    let password_hash = hash_password(&new_password)?;

    let rows_affected =
        if database_url.starts_with("postgres://") || database_url.starts_with("postgresql://") {
            reset_pg(&database_url, &email, &password_hash).await?
        } else if database_url.starts_with("sqlite://") || database_url.starts_with("sqlite:") {
            reset_sqlite(&database_url, &email, &password_hash).await?
        } else {
            bail!("unsupported DATABASE_URL scheme: {database_url}");
        };

    if rows_affected == 0 {
        bail!("user not found for email: {email}");
    }

    println!("Email: {email}");
    println!("Nowe hasło: {new_password}");

    Ok(())
}
