use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use sqlx::{PgPool, Row, SqlitePool};

#[derive(Debug, Clone)]
struct LegacyInvoice {
    id: String,
    direction: String,
    seller_nip: Option<String>,
    buyer_nip: Option<String>,
}

#[derive(Debug, Default)]
struct BackfillStats {
    total: usize,
    matched: usize,
    updated: usize,
    unmatched: usize,
}

fn is_dry_run() -> bool {
    std::env::args().skip(1).any(|arg| arg == "--dry-run")
}

fn normalize_nip(nip: &str) -> String {
    nip.chars().filter(|c| c.is_ascii_digit()).collect()
}

fn preferred_nips(row: &LegacyInvoice) -> Vec<String> {
    let seller = row
        .seller_nip
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(normalize_nip);
    let buyer = row
        .buyer_nip
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(normalize_nip);

    let mut nips = Vec::new();
    match row.direction.as_str() {
        "outgoing" => {
            if let Some(nip) = seller.clone() {
                nips.push(nip);
            }
            if let Some(nip) = buyer {
                nips.push(nip);
            }
        }
        "incoming" => {
            if let Some(nip) = buyer.clone() {
                nips.push(nip);
            }
            if let Some(nip) = seller {
                nips.push(nip);
            }
        }
        _ => {
            if let Some(nip) = seller.clone() {
                nips.push(nip);
            }
            if let Some(nip) = buyer {
                nips.push(nip);
            }
        }
    }

    nips
}

fn resolve_account_id<'a>(
    row: &LegacyInvoice,
    by_nip: &'a HashMap<String, String>,
) -> Option<&'a String> {
    preferred_nips(row)
        .into_iter()
        .find_map(|nip| by_nip.get(&nip))
}

async fn backfill_pg(database_url: &str, dry_run: bool) -> Result<()> {
    let pool = PgPool::connect(database_url)
        .await
        .context("connect postgres")?;

    let account_rows = sqlx::query("SELECT id::text AS id, nip FROM nip_accounts")
        .fetch_all(&pool)
        .await
        .context("load nip_accounts")?;

    let by_nip: HashMap<String, String> = account_rows
        .into_iter()
        .map(|r| {
            let id: String = r.get("id");
            let nip: String = r.get("nip");
            (normalize_nip(&nip), id)
        })
        .collect();

    let invoice_rows = sqlx::query(
        "SELECT id::text AS id, direction, seller_nip, buyer_nip FROM invoices WHERE nip_account_id IS NULL",
    )
    .fetch_all(&pool)
    .await
    .context("load legacy invoices")?;

    let invoices: Vec<LegacyInvoice> = invoice_rows
        .into_iter()
        .map(|r| LegacyInvoice {
            id: r.get("id"),
            direction: r.get("direction"),
            seller_nip: r.try_get("seller_nip").ok(),
            buyer_nip: r.try_get("buyer_nip").ok(),
        })
        .collect();

    let mut stats = BackfillStats {
        total: invoices.len(),
        ..BackfillStats::default()
    };
    let mut unmatched_ids = Vec::new();

    for invoice in invoices {
        match resolve_account_id(&invoice, &by_nip) {
            Some(account_id) => {
                stats.matched += 1;
                if dry_run {
                    println!(
                        "[dry-run] invoice {} => nip_account_id {}",
                        invoice.id, account_id
                    );
                } else {
                    sqlx::query(
                        "UPDATE invoices SET nip_account_id = $1::uuid WHERE id = $2::uuid",
                    )
                    .bind(account_id)
                    .bind(&invoice.id)
                    .execute(&pool)
                    .await
                    .with_context(|| format!("update invoice {}", invoice.id))?;
                    stats.updated += 1;
                }
            }
            None => {
                stats.unmatched += 1;
                unmatched_ids.push(invoice.id);
            }
        }
    }

    println!(
        "postgres backfill: total={}, matched={}, updated={}, unmatched={}",
        stats.total, stats.matched, stats.updated, stats.unmatched
    );

    if !unmatched_ids.is_empty() {
        println!("unmatched invoice ids:");
        for id in unmatched_ids {
            println!("- {id}");
        }
    }

    Ok(())
}

async fn backfill_sqlite(database_url: &str, dry_run: bool) -> Result<()> {
    let pool = SqlitePool::connect(database_url)
        .await
        .context("connect sqlite")?;

    let account_rows = sqlx::query("SELECT id, nip FROM nip_accounts")
        .fetch_all(&pool)
        .await
        .context("load nip_accounts")?;

    let by_nip: HashMap<String, String> = account_rows
        .into_iter()
        .map(|r| {
            let id: String = r.get("id");
            let nip: String = r.get("nip");
            (normalize_nip(&nip), id)
        })
        .collect();

    let invoice_rows = sqlx::query(
        "SELECT id, direction, seller_nip, buyer_nip FROM invoices WHERE nip_account_id IS NULL",
    )
    .fetch_all(&pool)
    .await
    .context("load legacy invoices")?;

    let invoices: Vec<LegacyInvoice> = invoice_rows
        .into_iter()
        .map(|r| LegacyInvoice {
            id: r.get("id"),
            direction: r.get("direction"),
            seller_nip: r.try_get("seller_nip").ok(),
            buyer_nip: r.try_get("buyer_nip").ok(),
        })
        .collect();

    let mut stats = BackfillStats {
        total: invoices.len(),
        ..BackfillStats::default()
    };
    let mut unmatched_ids = Vec::new();

    for invoice in invoices {
        match resolve_account_id(&invoice, &by_nip) {
            Some(account_id) => {
                stats.matched += 1;
                if dry_run {
                    println!(
                        "[dry-run] invoice {} => nip_account_id {}",
                        invoice.id, account_id
                    );
                } else {
                    sqlx::query("UPDATE invoices SET nip_account_id = ?1 WHERE id = ?2")
                        .bind(account_id)
                        .bind(&invoice.id)
                        .execute(&pool)
                        .await
                        .with_context(|| format!("update invoice {}", invoice.id))?;
                    stats.updated += 1;
                }
            }
            None => {
                stats.unmatched += 1;
                unmatched_ids.push(invoice.id);
            }
        }
    }

    println!(
        "sqlite backfill: total={}, matched={}, updated={}, unmatched={}",
        stats.total, stats.matched, stats.updated, stats.unmatched
    );

    if !unmatched_ids.is_empty() {
        println!("unmatched invoice ids:");
        for id in unmatched_ids {
            println!("- {id}");
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let database_url = std::env::var("DATABASE_URL")
        .context("DATABASE_URL is required (sqlite://... or postgres://...)")?;
    let dry_run = is_dry_run();

    if database_url.starts_with("postgres://") || database_url.starts_with("postgresql://") {
        backfill_pg(&database_url, dry_run).await?;
        return Ok(());
    }

    if database_url.starts_with("sqlite://") || database_url.starts_with("sqlite:") {
        backfill_sqlite(&database_url, dry_run).await?;
        return Ok(());
    }

    bail!("unsupported DATABASE_URL scheme: {database_url}")
}
