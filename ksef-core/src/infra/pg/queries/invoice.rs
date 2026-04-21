use std::fmt::Write;

use sqlx::{PgExecutor, Row};

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
    pub id: uuid::Uuid,
    pub nip_account_id: uuid::Uuid,
    pub direction: String,
    pub status: String,
    pub invoice_type: String,
    pub invoice_number: String,
    pub issue_date: chrono::NaiveDate,
    pub sale_date: Option<chrono::NaiveDate>,
    pub corrected_invoice_number: Option<String>,
    pub correction_reason: Option<String>,
    pub original_ksef_number: Option<String>,
    pub advance_payment_date: Option<chrono::NaiveDate>,
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
    pub line_items: serde_json::Value,
    pub total_net_grosze: i64,
    pub total_vat_grosze: i64,
    pub total_gross_grosze: i64,
    pub payment_method: Option<i16>,
    pub payment_deadline: Option<chrono::NaiveDate>,
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
        let payment_method = match self.payment_method {
            Some(0) | None => None,
            Some(raw) => Some(
                PaymentMethod::try_from(raw)
                    .map_err(|_| decode_err(format!("invalid payment_method: {raw}")))?,
            ),
        };
        let line_items: Vec<LineItem> = serde_json::from_value(self.line_items)
            .map_err(|e| decode_err(format!("invalid line_items JSON: {e}")))?;

        Ok(Invoice {
            id: InvoiceId::from_uuid(self.id),
            nip_account_id: NipAccountId::from_uuid(self.nip_account_id),
            direction,
            status,
            invoice_type,
            invoice_number: self.invoice_number,
            issue_date: self.issue_date,
            sale_date: self.sale_date,
            corrected_invoice_number: self.corrected_invoice_number,
            correction_reason: self.correction_reason,
            original_ksef_number: self.original_ksef_number.map(KSeFNumber::new),
            advance_payment_date: self.advance_payment_date,
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
            payment_deadline: self.payment_deadline,
            bank_account: self.bank_account,
            ksef_number: self.ksef_number.map(KSeFNumber::new),
            ksef_error: self.ksef_error,
            raw_xml: self.raw_xml,
        })
    }
}

pub async fn save<'e>(
    exec: impl PgExecutor<'e>,
    invoice: &Invoice,
) -> Result<InvoiceId, RepositoryError> {
    let line_items_json = serde_json::to_value(&invoice.line_items)
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
            $1, $2, $3, $4, $5, $6, $7, $8,
            $9, $10, $11, $12,
            $13, $14, $15, $16, $17,
            $18, $19, $20, $21, $22,
            $23, $24, $25, $26, $27, $28,
            $29, $30, $31, $32, $33
        )",
    )
    .bind(invoice.id.as_uuid())
    .bind(invoice.nip_account_id.as_uuid())
    .bind(invoice.direction.to_string())
    .bind(invoice.status.to_string())
    .bind(invoice.invoice_type.to_string())
    .bind(&invoice.invoice_number)
    .bind(invoice.issue_date)
    .bind(invoice.sale_date)
    .bind(&invoice.corrected_invoice_number)
    .bind(&invoice.correction_reason)
    .bind(
        invoice
            .original_ksef_number
            .as_ref()
            .map(KSeFNumber::as_str),
    )
    .bind(invoice.advance_payment_date)
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
    .bind(&line_items_json)
    .bind(invoice.total_net.grosze())
    .bind(invoice.total_vat.grosze())
    .bind(invoice.total_gross.grosze())
    .bind(invoice.payment_method.map(|m| i16::from(m.fa3_code())))
    .bind(invoice.payment_deadline)
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
    exec: impl PgExecutor<'e>,
    id: &InvoiceId,
    account_id: &NipAccountId,
) -> Result<Invoice, RepositoryError> {
    let row: InvoiceRow =
        sqlx::query_as("SELECT * FROM invoices WHERE id = $1 AND nip_account_id = $2")
            .bind(id.as_uuid())
            .bind(account_id.as_uuid())
            .fetch_optional(exec)
            .await?
            .ok_or_else(|| RepositoryError::NotFound {
                entity: "Invoice",
                id: id.to_string(),
            })?;
    row.into_domain()
}

pub async fn update_status<'e>(
    exec: impl PgExecutor<'e>,
    id: &InvoiceId,
    account_id: &NipAccountId,
    status: InvoiceStatus,
) -> Result<(), RepositoryError> {
    let result = sqlx::query(
        "UPDATE invoices SET status = $1, updated_at = NOW() WHERE id = $2 AND nip_account_id = $3",
    )
    .bind(status.to_string())
    .bind(id.as_uuid())
    .bind(account_id.as_uuid())
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
    exec: impl PgExecutor<'e>,
    id: &InvoiceId,
    account_id: &NipAccountId,
    ksef_number: &str,
) -> Result<(), RepositoryError> {
    let result = sqlx::query(
        "UPDATE invoices SET ksef_number = $1, updated_at = NOW() WHERE id = $2 AND nip_account_id = $3",
    )
    .bind(ksef_number)
    .bind(id.as_uuid())
    .bind(account_id.as_uuid())
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
    exec: impl PgExecutor<'e>,
    id: &InvoiceId,
    account_id: &NipAccountId,
    error: &str,
) -> Result<(), RepositoryError> {
    let result = sqlx::query(
        "UPDATE invoices SET ksef_error = $1, updated_at = NOW() WHERE id = $2 AND nip_account_id = $3",
    )
    .bind(error)
    .bind(id.as_uuid())
    .bind(account_id.as_uuid())
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
    exec: impl PgExecutor<'e>,
    ksef_number: &KSeFNumber,
) -> Result<Option<Invoice>, RepositoryError> {
    let row: Option<InvoiceRow> = sqlx::query_as("SELECT * FROM invoices WHERE ksef_number = $1")
        .bind(ksef_number.as_str())
        .fetch_optional(exec)
        .await?;
    row.map(InvoiceRow::into_domain).transpose()
}

pub async fn find_by_ksef_number_and_account<'e>(
    exec: impl PgExecutor<'e>,
    ksef_number: &KSeFNumber,
    account_id: &NipAccountId,
) -> Result<Option<Invoice>, RepositoryError> {
    let row: Option<InvoiceRow> =
        sqlx::query_as("SELECT * FROM invoices WHERE ksef_number = $1 AND nip_account_id = $2")
            .bind(ksef_number.as_str())
            .bind(account_id.as_uuid())
            .fetch_optional(exec)
            .await?;
    row.map(InvoiceRow::into_domain).transpose()
}

pub async fn upsert_by_ksef_number<'e>(
    exec: impl PgExecutor<'e>,
    invoice: &Invoice,
) -> Result<InvoiceId, RepositoryError> {
    let line_items_json = serde_json::to_value(&invoice.line_items)
        .map_err(|e| decode_err(format!("failed to serialize line_items: {e}")))?;

    let ksef_number = invoice
        .ksef_number
        .as_ref()
        .map(KSeFNumber::as_str)
        .ok_or_else(|| decode_err("upsert_by_ksef_number requires ksef_number".to_string()))?;

    let query = sqlx::query(
        r"INSERT INTO invoices (
            id, nip_account_id, direction, status, invoice_type, invoice_number, issue_date, sale_date,
            corrected_invoice_number, correction_reason, original_ksef_number, advance_payment_date,
            seller_nip, seller_name, seller_country, seller_address_line1, seller_address_line2,
            buyer_nip, buyer_name, buyer_country, buyer_address_line1, buyer_address_line2,
            currency, line_items, total_net_grosze, total_vat_grosze, total_gross_grosze,
            payment_method, payment_deadline, bank_account, ksef_number, ksef_error, raw_xml
        ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8,
            $9, $10, $11, $12,
            $13, $14, $15, $16, $17,
            $18, $19, $20, $21, $22,
            $23, $24, $25, $26, $27, $28,
            $29, $30, $31, $32, $33
        )
        ON CONFLICT (ksef_number, nip_account_id) DO UPDATE SET
            direction = EXCLUDED.direction,
            status = EXCLUDED.status,
            invoice_type = EXCLUDED.invoice_type,
            invoice_number = EXCLUDED.invoice_number,
            issue_date = EXCLUDED.issue_date,
            sale_date = EXCLUDED.sale_date,
            corrected_invoice_number = EXCLUDED.corrected_invoice_number,
            correction_reason = EXCLUDED.correction_reason,
            original_ksef_number = EXCLUDED.original_ksef_number,
            advance_payment_date = EXCLUDED.advance_payment_date,
            seller_nip = EXCLUDED.seller_nip,
            seller_name = EXCLUDED.seller_name,
            seller_country = EXCLUDED.seller_country,
            seller_address_line1 = EXCLUDED.seller_address_line1,
            seller_address_line2 = EXCLUDED.seller_address_line2,
            buyer_nip = EXCLUDED.buyer_nip,
            buyer_name = EXCLUDED.buyer_name,
            buyer_country = EXCLUDED.buyer_country,
            buyer_address_line1 = EXCLUDED.buyer_address_line1,
            buyer_address_line2 = EXCLUDED.buyer_address_line2,
            currency = EXCLUDED.currency,
            line_items = EXCLUDED.line_items,
            total_net_grosze = EXCLUDED.total_net_grosze,
            total_vat_grosze = EXCLUDED.total_vat_grosze,
            total_gross_grosze = EXCLUDED.total_gross_grosze,
            payment_method = EXCLUDED.payment_method,
            payment_deadline = EXCLUDED.payment_deadline,
            bank_account = EXCLUDED.bank_account,
            ksef_error = EXCLUDED.ksef_error,
            raw_xml = EXCLUDED.raw_xml,
            updated_at = NOW()
        RETURNING id",
    )
    .bind(invoice.id.as_uuid())
    .bind(invoice.nip_account_id.as_uuid())
    .bind(invoice.direction.to_string())
    .bind(invoice.status.to_string())
    .bind(invoice.invoice_type.to_string())
    .bind(&invoice.invoice_number)
    .bind(invoice.issue_date)
    .bind(invoice.sale_date)
    .bind(&invoice.corrected_invoice_number)
    .bind(&invoice.correction_reason)
    .bind(
        invoice
            .original_ksef_number
            .as_ref()
            .map(KSeFNumber::as_str),
    )
    .bind(invoice.advance_payment_date)
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
    .bind(&line_items_json)
    .bind(invoice.total_net.grosze())
    .bind(invoice.total_vat.grosze())
    .bind(invoice.total_gross.grosze())
    .bind(invoice.payment_method.map(|m| i16::from(m.fa3_code())))
    .bind(invoice.payment_deadline)
    .bind(&invoice.bank_account)
    .bind(ksef_number)
    .bind(&invoice.ksef_error)
    .bind(&invoice.raw_xml);

    let row = query.fetch_one(exec).await?;

    Ok(InvoiceId::from_uuid(row.get("id")))
}

pub async fn list<'e>(
    exec: impl PgExecutor<'e>,
    filter: &InvoiceFilter,
) -> Result<Vec<Invoice>, RepositoryError> {
    let mut query = String::from("SELECT * FROM invoices WHERE nip_account_id = $1");
    let mut param_idx = 2u32;

    if filter.direction.is_some() {
        write!(query, " AND direction = ${param_idx}").unwrap();
        param_idx += 1;
    }
    if filter.status.is_some() {
        write!(query, " AND status = ${param_idx}").unwrap();
    }

    query.push_str(" ORDER BY created_at DESC");

    if let Some(limit) = filter.limit {
        write!(query, " LIMIT {limit}").unwrap();
    }
    if let Some(offset) = filter.offset {
        write!(query, " OFFSET {offset}").unwrap();
    }

    let mut q = sqlx::query_as::<_, InvoiceRow>(&query).bind(filter.account_id.as_uuid());

    if let Some(ref d) = filter.direction {
        q = q.bind(d.to_string());
    }
    if let Some(ref s) = filter.status {
        q = q.bind(s.to_string());
    }
    let rows: Vec<InvoiceRow> = q.fetch_all(exec).await?;
    rows.into_iter().map(InvoiceRow::into_domain).collect()
}
