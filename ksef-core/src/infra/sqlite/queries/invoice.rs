use sqlx::{QueryBuilder, Row, Sqlite, SqliteExecutor};

use crate::domain::invoice::{
    Address, CountryCode, Currency, Direction, Invoice, InvoiceId, InvoiceStatus, InvoiceType,
    LineItem, Money, Party, PaymentMethod,
};
use crate::domain::nip::Nip;
use crate::domain::nip_account::NipAccountId;
use crate::domain::session::KSeFNumber;
use crate::error::RepositoryError;
use crate::ports::invoice_repository::InvoiceFilter;

#[derive(sqlx::FromRow)]
pub(crate) struct InvoiceRow {
    pub id: String,
    pub nip_account_id: String,
    pub direction: String,
    pub status: String,
    pub invoice_type: String,
    pub invoice_number: String,
    pub issue_date: String,
    pub sale_date: Option<String>,
    pub corrected_invoice_number: Option<String>,
    pub correction_reason: Option<String>,
    pub original_ksef_number: Option<String>,
    pub advance_payment_date: Option<String>,
    pub seller_nip: Option<String>,
    pub seller_name: String,
    pub seller_country: String,
    pub seller_address_line1: String,
    pub seller_address_line2: String,
    pub buyer_nip: Option<String>,
    pub buyer_name: String,
    pub buyer_country: String,
    pub buyer_address_line1: String,
    pub buyer_address_line2: String,
    pub currency: String,
    pub line_items: String,
    pub total_net_grosze: i64,
    pub total_vat_grosze: i64,
    pub total_gross_grosze: i64,
    pub payment_method: Option<i16>,
    pub payment_deadline: Option<String>,
    pub bank_account: Option<String>,
    pub ksef_number: Option<String>,
    pub ksef_error: Option<String>,
    pub raw_xml: Option<String>,
}

fn decode_err(msg: String) -> RepositoryError {
    RepositoryError::Database(sqlx::Error::Decode(msg.into()))
}

fn parse_optional_nip(raw: Option<&str>) -> Result<Option<Nip>, crate::error::DomainError> {
    match raw {
        Some(value) => Nip::parse(value).map(Some),
        None => Ok(None),
    }
}

fn parse_date(value: &str, field: &'static str) -> Result<chrono::NaiveDate, RepositoryError> {
    chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|e| decode_err(format!("invalid {field}: {e}")))
}

fn format_date(value: chrono::NaiveDate) -> String {
    value.format("%Y-%m-%d").to_string()
}

impl InvoiceRow {
    pub(crate) fn into_domain(self) -> Result<Invoice, RepositoryError> {
        let direction: Direction = self
            .direction
            .parse()
            .map_err(|_| decode_err(format!("invalid direction: {}", self.direction)))?;
        let status: InvoiceStatus = self
            .status
            .parse()
            .map_err(|_| decode_err(format!("invalid status: {}", self.status)))?;
        let invoice_type: InvoiceType = self
            .invoice_type
            .parse()
            .map_err(|_| decode_err(format!("invalid invoice_type: {}", self.invoice_type)))?;
        let id = uuid::Uuid::parse_str(&self.id)
            .map_err(|e| decode_err(format!("invalid invoice id '{}': {e}", self.id)))?;
        let nip_account_id = self.nip_account_id.parse::<NipAccountId>().map_err(|e| {
            decode_err(format!(
                "invalid nip_account_id '{}': {e}",
                self.nip_account_id
            ))
        })?;

        let payment_method = match self.payment_method {
            Some(0) | None => None,
            Some(raw) => Some(
                PaymentMethod::try_from(raw)
                    .map_err(|_| decode_err(format!("invalid payment_method: {raw}")))?,
            ),
        };

        let line_items: Vec<LineItem> = serde_json::from_str(&self.line_items)
            .map_err(|e| decode_err(format!("invalid line_items JSON: {e}")))?;

        Ok(Invoice {
            id: InvoiceId::from_uuid(id),
            nip_account_id,
            direction,
            status,
            invoice_type,
            invoice_number: self.invoice_number,
            issue_date: parse_date(&self.issue_date, "issue_date")?,
            sale_date: self
                .sale_date
                .as_deref()
                .map(|v| parse_date(v, "sale_date"))
                .transpose()?,
            corrected_invoice_number: self.corrected_invoice_number,
            correction_reason: self.correction_reason,
            original_ksef_number: self.original_ksef_number.map(KSeFNumber::new),
            advance_payment_date: self
                .advance_payment_date
                .as_deref()
                .map(|v| parse_date(v, "advance_payment_date"))
                .transpose()?,
            seller: Party {
                nip: parse_optional_nip(self.seller_nip.as_deref())
                    .map_err(|e| decode_err(format!("invalid seller NIP: {e}")))?,
                name: self.seller_name,
                address: Address {
                    country_code: CountryCode::parse(&self.seller_country)
                        .map_err(|e| decode_err(format!("invalid seller country: {e}")))?,
                    line1: self.seller_address_line1,
                    line2: self.seller_address_line2,
                },
            },
            buyer: Party {
                nip: parse_optional_nip(self.buyer_nip.as_deref())
                    .map_err(|e| decode_err(format!("invalid buyer NIP: {e}")))?,
                name: self.buyer_name,
                address: Address {
                    country_code: CountryCode::parse(&self.buyer_country)
                        .map_err(|e| decode_err(format!("invalid buyer country: {e}")))?,
                    line1: self.buyer_address_line1,
                    line2: self.buyer_address_line2,
                },
            },
            currency: Currency::parse(&self.currency)
                .map_err(|e| decode_err(format!("invalid currency: {e}")))?,
            line_items,
            total_net: Money::from_grosze(self.total_net_grosze),
            total_vat: Money::from_grosze(self.total_vat_grosze),
            total_gross: Money::from_grosze(self.total_gross_grosze),
            payment_method,
            payment_deadline: self
                .payment_deadline
                .as_deref()
                .map(|v| parse_date(v, "payment_deadline"))
                .transpose()?,
            bank_account: self.bank_account,
            ksef_number: self.ksef_number.map(KSeFNumber::new),
            ksef_error: self.ksef_error,
            raw_xml: self.raw_xml,
        })
    }
}

pub async fn save<'e>(
    exec: impl SqliteExecutor<'e>,
    invoice: &Invoice,
) -> Result<InvoiceId, RepositoryError> {
    let line_items_json = serde_json::to_string(&invoice.line_items)
        .map_err(|e| decode_err(format!("failed to serialize line_items: {e}")))?;

    sqlx::query(
        r"INSERT INTO invoices (
            id, nip_account_id, direction, status, invoice_type, invoice_number, issue_date, sale_date,
            corrected_invoice_number, correction_reason, original_ksef_number, advance_payment_date,
            seller_nip, seller_name, seller_country, seller_address_line1, seller_address_line2,
            buyer_nip, buyer_name, buyer_country, buyer_address_line1, buyer_address_line2,
            currency, line_items, total_net_grosze, total_vat_grosze, total_gross_grosze,
            payment_method, payment_deadline, bank_account, ksef_number, ksef_error, raw_xml
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
            ?9, ?10, ?11, ?12,
            ?13, ?14, ?15, ?16, ?17,
            ?18, ?19, ?20, ?21, ?22,
            ?23, ?24, ?25, ?26, ?27, ?28,
            ?29, ?30, ?31, ?32, ?33
        )",
    )
    .bind(invoice.id.to_string())
    .bind(invoice.nip_account_id.to_string())
    .bind(invoice.direction.to_string())
    .bind(invoice.status.to_string())
    .bind(invoice.invoice_type.to_string())
    .bind(&invoice.invoice_number)
    .bind(format_date(invoice.issue_date))
    .bind(invoice.sale_date.map(format_date))
    .bind(&invoice.corrected_invoice_number)
    .bind(&invoice.correction_reason)
    .bind(
        invoice
            .original_ksef_number
            .as_ref()
            .map(KSeFNumber::as_str),
    )
    .bind(invoice.advance_payment_date.map(format_date))
    .bind(invoice.seller.nip.as_ref().map(Nip::as_str))
    .bind(&invoice.seller.name)
    .bind(invoice.seller.address.country_code.as_str())
    .bind(&invoice.seller.address.line1)
    .bind(&invoice.seller.address.line2)
    .bind(invoice.buyer.nip.as_ref().map(Nip::as_str))
    .bind(&invoice.buyer.name)
    .bind(invoice.buyer.address.country_code.as_str())
    .bind(&invoice.buyer.address.line1)
    .bind(&invoice.buyer.address.line2)
    .bind(invoice.currency.as_str())
    .bind(line_items_json)
    .bind(invoice.total_net.grosze())
    .bind(invoice.total_vat.grosze())
    .bind(invoice.total_gross.grosze())
    .bind(invoice.payment_method.map(|m| i16::from(m.fa3_code())))
    .bind(invoice.payment_deadline.map(format_date))
    .bind(&invoice.bank_account)
    .bind(invoice.ksef_number.as_ref().map(KSeFNumber::as_str))
    .bind(&invoice.ksef_error)
    .bind(&invoice.raw_xml)
    .execute(exec)
    .await
    .map_err(|e| match &e {
        sqlx::Error::Database(db_err) if db_err.is_unique_violation() => {
            RepositoryError::Duplicate {
                entity: "Invoice",
                key: invoice.id.to_string(),
            }
        }
        _ => RepositoryError::Database(e),
    })?;

    Ok(invoice.id.clone())
}

pub async fn find_by_id<'e>(
    exec: impl SqliteExecutor<'e>,
    id: &InvoiceId,
    account_id: &NipAccountId,
) -> Result<Invoice, RepositoryError> {
    let row: InvoiceRow =
        sqlx::query_as("SELECT * FROM invoices WHERE id = ?1 AND nip_account_id = ?2")
            .bind(id.to_string())
            .bind(account_id.to_string())
            .fetch_optional(exec)
            .await?
            .ok_or_else(|| RepositoryError::NotFound {
                entity: "Invoice",
                id: id.to_string(),
            })?;
    row.into_domain()
}

pub async fn update_status<'e>(
    exec: impl SqliteExecutor<'e>,
    id: &InvoiceId,
    account_id: &NipAccountId,
    status: InvoiceStatus,
) -> Result<(), RepositoryError> {
    let result = sqlx::query(
        "UPDATE invoices SET status = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = ?2 AND nip_account_id = ?3",
    )
    .bind(status.to_string())
    .bind(id.to_string())
    .bind(account_id.as_uuid().to_string())
    .execute(exec)
    .await?;

    if result.rows_affected() == 0 {
        return Err(RepositoryError::NotFound {
            entity: "Invoice",
            id: id.to_string(),
        });
    }
    Ok(())
}

pub async fn set_ksef_number<'e>(
    exec: impl SqliteExecutor<'e>,
    id: &InvoiceId,
    account_id: &NipAccountId,
    ksef_number: &str,
) -> Result<(), RepositoryError> {
    let result = sqlx::query(
        "UPDATE invoices SET ksef_number = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = ?2 AND nip_account_id = ?3",
    )
    .bind(ksef_number)
    .bind(id.to_string())
    .bind(account_id.as_uuid().to_string())
    .execute(exec)
    .await?;
    if result.rows_affected() == 0 {
        return Err(RepositoryError::NotFound {
            entity: "Invoice",
            id: id.to_string(),
        });
    }
    Ok(())
}

pub async fn set_ksef_error<'e>(
    exec: impl SqliteExecutor<'e>,
    id: &InvoiceId,
    account_id: &NipAccountId,
    error: &str,
) -> Result<(), RepositoryError> {
    let result = sqlx::query(
        "UPDATE invoices SET ksef_error = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = ?2 AND nip_account_id = ?3",
    )
    .bind(error)
    .bind(id.to_string())
    .bind(account_id.as_uuid().to_string())
    .execute(exec)
    .await?;
    if result.rows_affected() == 0 {
        return Err(RepositoryError::NotFound {
            entity: "Invoice",
            id: id.to_string(),
        });
    }
    Ok(())
}

pub async fn find_by_ksef_number<'e>(
    exec: impl SqliteExecutor<'e>,
    ksef_number: &KSeFNumber,
) -> Result<Option<Invoice>, RepositoryError> {
    let row: Option<InvoiceRow> = sqlx::query_as("SELECT * FROM invoices WHERE ksef_number = ?1")
        .bind(ksef_number.as_str())
        .fetch_optional(exec)
        .await?;
    row.map(InvoiceRow::into_domain).transpose()
}

pub async fn find_by_ksef_number_and_account<'e>(
    exec: impl SqliteExecutor<'e>,
    ksef_number: &KSeFNumber,
    account_id: &NipAccountId,
) -> Result<Option<Invoice>, RepositoryError> {
    let row: Option<InvoiceRow> =
        sqlx::query_as("SELECT * FROM invoices WHERE ksef_number = ?1 AND nip_account_id = ?2")
            .bind(ksef_number.as_str())
            .bind(account_id.as_uuid().to_string())
            .fetch_optional(exec)
            .await?;
    row.map(InvoiceRow::into_domain).transpose()
}

pub async fn upsert_by_ksef_number<'e>(
    exec: impl SqliteExecutor<'e>,
    invoice: &Invoice,
) -> Result<InvoiceId, RepositoryError> {
    let line_items_json = serde_json::to_string(&invoice.line_items)
        .map_err(|e| decode_err(format!("failed to serialize line_items: {e}")))?;

    let ksef_number = invoice
        .ksef_number
        .as_ref()
        .map(KSeFNumber::as_str)
        .ok_or_else(|| decode_err("upsert_by_ksef_number requires ksef_number".to_string()))?;

    let row = sqlx::query(
        r"INSERT INTO invoices (
            id, nip_account_id, direction, status, invoice_type, invoice_number, issue_date, sale_date,
            corrected_invoice_number, correction_reason, original_ksef_number, advance_payment_date,
            seller_nip, seller_name, seller_country, seller_address_line1, seller_address_line2,
            buyer_nip, buyer_name, buyer_country, buyer_address_line1, buyer_address_line2,
            currency, line_items, total_net_grosze, total_vat_grosze, total_gross_grosze,
            payment_method, payment_deadline, bank_account, ksef_number, ksef_error, raw_xml
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
            ?9, ?10, ?11, ?12,
            ?13, ?14, ?15, ?16, ?17,
            ?18, ?19, ?20, ?21, ?22,
            ?23, ?24, ?25, ?26, ?27, ?28,
            ?29, ?30, ?31, ?32, ?33
        )
        ON CONFLICT (ksef_number, nip_account_id) DO UPDATE SET
            direction = excluded.direction,
            status = excluded.status,
            invoice_type = excluded.invoice_type,
            invoice_number = excluded.invoice_number,
            issue_date = excluded.issue_date,
            sale_date = excluded.sale_date,
            corrected_invoice_number = excluded.corrected_invoice_number,
            correction_reason = excluded.correction_reason,
            original_ksef_number = excluded.original_ksef_number,
            advance_payment_date = excluded.advance_payment_date,
            seller_nip = excluded.seller_nip,
            seller_name = excluded.seller_name,
            seller_country = excluded.seller_country,
            seller_address_line1 = excluded.seller_address_line1,
            seller_address_line2 = excluded.seller_address_line2,
            buyer_nip = excluded.buyer_nip,
            buyer_name = excluded.buyer_name,
            buyer_country = excluded.buyer_country,
            buyer_address_line1 = excluded.buyer_address_line1,
            buyer_address_line2 = excluded.buyer_address_line2,
            currency = excluded.currency,
            line_items = excluded.line_items,
            total_net_grosze = excluded.total_net_grosze,
            total_vat_grosze = excluded.total_vat_grosze,
            total_gross_grosze = excluded.total_gross_grosze,
            payment_method = excluded.payment_method,
            payment_deadline = excluded.payment_deadline,
            bank_account = excluded.bank_account,
            ksef_error = excluded.ksef_error,
            raw_xml = excluded.raw_xml,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
        RETURNING id",
    )
    .bind(invoice.id.to_string())
    .bind(invoice.nip_account_id.to_string())
    .bind(invoice.direction.to_string())
    .bind(invoice.status.to_string())
    .bind(invoice.invoice_type.to_string())
    .bind(&invoice.invoice_number)
    .bind(format_date(invoice.issue_date))
    .bind(invoice.sale_date.map(format_date))
    .bind(&invoice.corrected_invoice_number)
    .bind(&invoice.correction_reason)
    .bind(
        invoice
            .original_ksef_number
            .as_ref()
            .map(KSeFNumber::as_str),
    )
    .bind(invoice.advance_payment_date.map(format_date))
    .bind(invoice.seller.nip.as_ref().map(Nip::as_str))
    .bind(&invoice.seller.name)
    .bind(invoice.seller.address.country_code.as_str())
    .bind(&invoice.seller.address.line1)
    .bind(&invoice.seller.address.line2)
    .bind(invoice.buyer.nip.as_ref().map(Nip::as_str))
    .bind(&invoice.buyer.name)
    .bind(invoice.buyer.address.country_code.as_str())
    .bind(&invoice.buyer.address.line1)
    .bind(&invoice.buyer.address.line2)
    .bind(invoice.currency.as_str())
    .bind(line_items_json)
    .bind(invoice.total_net.grosze())
    .bind(invoice.total_vat.grosze())
    .bind(invoice.total_gross.grosze())
    .bind(invoice.payment_method.map(|m| i16::from(m.fa3_code())))
    .bind(invoice.payment_deadline.map(format_date))
    .bind(&invoice.bank_account)
    .bind(ksef_number)
    .bind(&invoice.ksef_error)
    .bind(&invoice.raw_xml)
    .fetch_one(exec)
    .await?;
    let id: String = row.try_get("id")?;
    let uuid = uuid::Uuid::parse_str(&id)
        .map_err(|e| decode_err(format!("invalid invoice id '{id}' after upsert: {e}")))?;
    Ok(InvoiceId::from_uuid(uuid))
}

pub async fn list<'e>(
    exec: impl SqliteExecutor<'e>,
    filter: &InvoiceFilter,
) -> Result<Vec<Invoice>, RepositoryError> {
    let mut qb: QueryBuilder<'_, Sqlite> =
        QueryBuilder::new("SELECT * FROM invoices WHERE nip_account_id = ");
    qb.push_bind(filter.account_id.to_string());

    if let Some(ref direction) = filter.direction {
        qb.push(" AND direction = ")
            .push_bind(direction.to_string());
    }
    if let Some(ref status) = filter.status {
        qb.push(" AND status = ").push_bind(status.to_string());
    }

    qb.push(" ORDER BY datetime(created_at) DESC");

    if let Some(limit) = filter.limit {
        qb.push(" LIMIT ").push_bind(i64::from(limit));
    }
    if let Some(offset) = filter.offset {
        qb.push(" OFFSET ").push_bind(i64::from(offset));
    }

    let rows: Vec<InvoiceRow> = qb.build_query_as().fetch_all(exec).await?;
    rows.into_iter().map(InvoiceRow::into_domain).collect()
}
