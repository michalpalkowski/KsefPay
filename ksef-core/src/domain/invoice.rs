use std::fmt;
use std::str::FromStr;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::DomainError;

use super::nip::Nip;
use super::nip_account::NipAccountId;
use super::session::KSeFNumber;

/// Format an invoice number: `{prefix}/{year}/{month:02}/{seq:03}`.
///
/// Example: `format_invoice_number("FV", 2026, 4, 7)` → `"FV/2026/04/007"`.
#[must_use]
pub fn format_invoice_number(prefix: &str, year: i32, month: u32, sequence: u32) -> String {
    format!("{prefix}/{year}/{month:02}/{sequence:03}")
}

/// Unique identifier for an invoice in our system.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InvoiceId(Uuid);

impl InvoiceId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    #[must_use]
    pub fn from_uuid(id: Uuid) -> Self {
        Self(id)
    }

    #[must_use]
    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl Default for InvoiceId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for InvoiceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for InvoiceId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

/// Whether an invoice was sent by us or received from a counterparty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Outgoing,
    Incoming,
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Outgoing => write!(f, "outgoing"),
            Self::Incoming => write!(f, "incoming"),
        }
    }
}

impl FromStr for Direction {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "outgoing" => Ok(Self::Outgoing),
            "incoming" => Ok(Self::Incoming),
            other => Err(DomainError::InvalidParse {
                type_name: "Direction",
                value: other.to_string(),
            }),
        }
    }
}

/// FA(3) form code used in XML `<KodFormularza>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FormCode {
    Fa,
    Kor,
    Zal,
    Roz,
    Upr,
    VatPef,
    VatPefSp,
    KorPef,
    VatRr,
    KorVatRr,
    KorZal,
    KorRoz,
}

impl fmt::Display for FormCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fa => write!(f, "FA"),
            Self::Kor => write!(f, "KOR"),
            Self::Zal => write!(f, "ZAL"),
            Self::Roz => write!(f, "ROZ"),
            Self::Upr => write!(f, "UPR"),
            Self::VatPef => write!(f, "VAT_PEF"),
            Self::VatPefSp => write!(f, "VAT_PEF_SP"),
            Self::KorPef => write!(f, "KOR_PEF"),
            Self::VatRr => write!(f, "VAT_RR"),
            Self::KorVatRr => write!(f, "KOR_VAT_RR"),
            Self::KorZal => write!(f, "KOR_ZAL"),
            Self::KorRoz => write!(f, "KOR_ROZ"),
        }
    }
}

impl FromStr for FormCode {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_uppercase().as_str() {
            "FA" => Ok(Self::Fa),
            "KOR" => Ok(Self::Kor),
            "ZAL" => Ok(Self::Zal),
            "ROZ" => Ok(Self::Roz),
            "UPR" => Ok(Self::Upr),
            "VAT_PEF" => Ok(Self::VatPef),
            "VAT_PEF_SP" => Ok(Self::VatPefSp),
            "KOR_PEF" => Ok(Self::KorPef),
            "VAT_RR" => Ok(Self::VatRr),
            "KOR_VAT_RR" => Ok(Self::KorVatRr),
            "KOR_ZAL" => Ok(Self::KorZal),
            "KOR_ROZ" => Ok(Self::KorRoz),
            other => Err(DomainError::InvalidParse {
                type_name: "FormCode",
                value: other.to_string(),
            }),
        }
    }
}

/// Invoice semantic type used across parser/serializer and filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InvoiceType {
    Vat,
    Kor,
    Zal,
    Roz,
    Upr,
    VatPef,
    VatPefSp,
    KorPef,
    VatRr,
    KorVatRr,
    KorZal,
    KorRoz,
}

impl InvoiceType {
    #[must_use]
    pub fn form_code(self) -> FormCode {
        match self {
            Self::Vat => FormCode::Fa,
            Self::Kor => FormCode::Kor,
            Self::Zal => FormCode::Zal,
            Self::Roz => FormCode::Roz,
            Self::Upr => FormCode::Upr,
            Self::VatPef => FormCode::VatPef,
            Self::VatPefSp => FormCode::VatPefSp,
            Self::KorPef => FormCode::KorPef,
            Self::VatRr => FormCode::VatRr,
            Self::KorVatRr => FormCode::KorVatRr,
            Self::KorZal => FormCode::KorZal,
            Self::KorRoz => FormCode::KorRoz,
        }
    }
}

impl fmt::Display for InvoiceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Vat => write!(f, "vat"),
            Self::Kor => write!(f, "kor"),
            Self::Zal => write!(f, "zal"),
            Self::Roz => write!(f, "roz"),
            Self::Upr => write!(f, "upr"),
            Self::VatPef => write!(f, "vat_pef"),
            Self::VatPefSp => write!(f, "vat_pef_sp"),
            Self::KorPef => write!(f, "kor_pef"),
            Self::VatRr => write!(f, "vat_rr"),
            Self::KorVatRr => write!(f, "kor_vat_rr"),
            Self::KorZal => write!(f, "kor_zal"),
            Self::KorRoz => write!(f, "kor_roz"),
        }
    }
}

impl FromStr for InvoiceType {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "vat" => Ok(Self::Vat),
            "kor" => Ok(Self::Kor),
            "zal" => Ok(Self::Zal),
            "roz" => Ok(Self::Roz),
            "upr" => Ok(Self::Upr),
            "vat_pef" => Ok(Self::VatPef),
            "vat_pef_sp" => Ok(Self::VatPefSp),
            "kor_pef" => Ok(Self::KorPef),
            "vat_rr" => Ok(Self::VatRr),
            "kor_vat_rr" => Ok(Self::KorVatRr),
            "kor_zal" => Ok(Self::KorZal),
            "kor_roz" => Ok(Self::KorRoz),
            other => Err(DomainError::InvalidParse {
                type_name: "InvoiceType",
                value: other.to_string(),
            }),
        }
    }
}

/// Lifecycle status of an invoice in the `KSeF` submission pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InvoiceStatus {
    Draft,
    Queued,
    Submitted,
    Accepted,
    Rejected,
    Failed,
    /// Downloaded from `KSeF` via fetch/query. Terminal state for incoming invoices.
    Fetched,
}

impl InvoiceStatus {
    /// Validate that a status transition is allowed.
    ///
    /// Valid transitions:
    /// - Draft -> Queued (user submits)
    /// - Queued -> Submitted (worker picks up)
    /// - Submitted -> Accepted | Rejected (`KSeF` responds)
    /// - Queued -> Failed (permanent failure after retries)
    /// - Submitted -> Failed (permanent failure)
    ///
    /// Terminal states (no transitions out): Accepted, Rejected, Failed, Fetched.
    /// Fetched is set directly on insert for invoices downloaded from `KSeF`.
    pub fn transition_to(self, target: Self) -> Result<Self, DomainError> {
        let valid = matches!(
            (self, target),
            (Self::Draft, Self::Queued)
                | (Self::Queued, Self::Submitted | Self::Failed)
                | (
                    Self::Submitted,
                    Self::Accepted | Self::Rejected | Self::Failed
                )
        );

        if valid {
            Ok(target)
        } else {
            Err(DomainError::InvalidStatusTransition {
                from: self.to_string(),
                to: target.to_string(),
            })
        }
    }
}

impl fmt::Display for InvoiceStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Draft => write!(f, "draft"),
            Self::Queued => write!(f, "queued"),
            Self::Submitted => write!(f, "submitted"),
            Self::Accepted => write!(f, "accepted"),
            Self::Rejected => write!(f, "rejected"),
            Self::Failed => write!(f, "failed"),
            Self::Fetched => write!(f, "fetched"),
        }
    }
}

impl FromStr for InvoiceStatus {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "draft" => Ok(Self::Draft),
            "queued" => Ok(Self::Queued),
            "submitted" => Ok(Self::Submitted),
            "accepted" => Ok(Self::Accepted),
            "rejected" => Ok(Self::Rejected),
            "failed" => Ok(Self::Failed),
            "fetched" => Ok(Self::Fetched),
            other => Err(DomainError::InvalidParse {
                type_name: "InvoiceStatus",
                value: other.to_string(),
            }),
        }
    }
}

/// VAT rate as used in Polish invoices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VatRate {
    /// 23% standard rate
    Rate23,
    /// 8% reduced rate
    Rate8,
    /// 7% old reduced rate (still valid for some goods)
    Rate7,
    /// 5% reduced rate
    Rate5,
    /// 4% reduced rate (for farmers)
    Rate4,
    /// 3% reduced rate (ryczalt)
    Rate3,
    /// 0% zero rate
    Rate0,
    /// VAT exempt (zwolniony)
    Exempt,
    /// Not subject to VAT (nie podlega opodatkowaniu)
    NotSubject,
    /// Reverse charge / not applicable (np)
    ReverseCharge,
}

impl VatRate {
    #[must_use]
    pub fn percentage(self) -> Option<u8> {
        match self {
            Self::Rate23 => Some(23),
            Self::Rate8 => Some(8),
            Self::Rate7 => Some(7),
            Self::Rate5 => Some(5),
            Self::Rate4 => Some(4),
            Self::Rate3 => Some(3),
            Self::Rate0 => Some(0),
            Self::Exempt | Self::NotSubject | Self::ReverseCharge => None,
        }
    }

    /// FA(3) XML element suffix for this VAT rate's summary block.
    #[must_use]
    pub fn fa3_suffix(self) -> &'static str {
        match self {
            Self::Rate23 => "1",
            Self::Rate8 | Self::Rate7 => "2",
            Self::Rate5 => "3",
            Self::Rate4 => "4",
            Self::Rate3 => "5",
            Self::Rate0 => "6_1",
            Self::Exempt | Self::NotSubject | Self::ReverseCharge => "7",
        }
    }
}

impl fmt::Display for VatRate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rate23 => write!(f, "23"),
            Self::Rate8 => write!(f, "8"),
            Self::Rate7 => write!(f, "7"),
            Self::Rate5 => write!(f, "5"),
            Self::Rate4 => write!(f, "4"),
            Self::Rate3 => write!(f, "3"),
            Self::Rate0 => write!(f, "0"),
            Self::Exempt => write!(f, "zw"),
            Self::NotSubject => write!(f, "oo"),
            Self::ReverseCharge => write!(f, "np"),
        }
    }
}

impl FromStr for VatRate {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let normalized = s.split_whitespace().collect::<Vec<_>>().join(" ");
        let normalized_lower = normalized.to_ascii_lowercase();

        let parse_qualified_code = || -> Option<Result<Self, DomainError>> {
            let mut parts = normalized_lower.split_whitespace();
            let code = parts.next()?;
            let qualifier = parts.next()?;
            if parts.next().is_some() {
                return Some(Err(DomainError::InvalidVatRate(normalized.clone())));
            }

            if !qualifier.chars().all(|c| c.is_ascii_alphanumeric()) {
                return Some(Err(DomainError::InvalidVatRate(normalized.clone())));
            }

            let mapped = match code {
                "zw" => Self::Exempt,
                "oo" => Self::NotSubject,
                "np" => Self::ReverseCharge,
                _ => return Some(Err(DomainError::InvalidVatRate(normalized.clone()))),
            };
            Some(Ok(mapped))
        };

        match normalized_lower.as_str() {
            "23" => Ok(Self::Rate23),
            "8" => Ok(Self::Rate8),
            "7" => Ok(Self::Rate7),
            "5" => Ok(Self::Rate5),
            "4" => Ok(Self::Rate4),
            "3" => Ok(Self::Rate3),
            "0" => Ok(Self::Rate0),
            "zw" | "exempt" => Ok(Self::Exempt),
            "oo" => Ok(Self::NotSubject),
            "np" | "0 kr" => Ok(Self::ReverseCharge),
            // Some KSeF documents include an additional qualifier, e.g. "np I".
            _ => parse_qualified_code().unwrap_or(Err(DomainError::InvalidVatRate(normalized))),
        }
    }
}

/// Payment method codes as defined in FA(3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaymentMethod {
    /// 1 - gotówka (cash)
    Cash,
    /// 2 - karta (card)
    Card,
    /// 3 - bon (voucher)
    Voucher,
    /// 4 - czek (check)
    Check,
    /// 5 - kredyt (credit)
    Credit,
    /// 6 - przelew (bank transfer)
    Transfer,
    /// 7 - płatność mobilna (mobile payment)
    Mobile,
}

impl PaymentMethod {
    #[must_use]
    pub fn fa3_code(self) -> u8 {
        match self {
            Self::Cash => 1,
            Self::Card => 2,
            Self::Voucher => 3,
            Self::Check => 4,
            Self::Credit => 5,
            Self::Transfer => 6,
            Self::Mobile => 7,
        }
    }
}

impl TryFrom<u8> for PaymentMethod {
    type Error = DomainError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Cash),
            2 => Ok(Self::Card),
            3 => Ok(Self::Voucher),
            4 => Ok(Self::Check),
            5 => Ok(Self::Credit),
            6 => Ok(Self::Transfer),
            7 => Ok(Self::Mobile),
            other => Err(DomainError::InvalidParse {
                type_name: "PaymentMethod",
                value: other.to_string(),
            }),
        }
    }
}

impl TryFrom<i16> for PaymentMethod {
    type Error = DomainError;

    fn try_from(value: i16) -> Result<Self, Self::Error> {
        let code = u8::try_from(value).map_err(|_| DomainError::InvalidParse {
            type_name: "PaymentMethod",
            value: value.to_string(),
        })?;
        Self::try_from(code)
    }
}

impl fmt::Display for PaymentMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cash => write!(f, "gotówka"),
            Self::Card => write!(f, "karta"),
            Self::Voucher => write!(f, "bon"),
            Self::Check => write!(f, "czek"),
            Self::Credit => write!(f, "kredyt"),
            Self::Transfer => write!(f, "przelew"),
            Self::Mobile => write!(f, "mobilna"),
        }
    }
}

/// Monetary amount in grosze (1/100 PLN) to avoid floating-point issues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Money(i64);

impl Money {
    #[must_use]
    pub fn from_grosze(grosze: i64) -> Self {
        Self(grosze)
    }

    #[must_use]
    pub fn from_pln(zloty: i64, grosze: i64) -> Self {
        Self(zloty * 100 + grosze)
    }

    #[must_use]
    pub fn grosze(self) -> i64 {
        self.0
    }

    #[must_use]
    pub fn zloty_part(self) -> i64 {
        self.0 / 100
    }

    #[must_use]
    pub fn grosze_part(self) -> i64 {
        self.0 % 100
    }

    #[must_use]
    pub fn is_negative(self) -> bool {
        self.0 < 0
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{:02}", self.0 / 100, (self.0 % 100).abs())
    }
}

impl FromStr for Money {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split('.').collect();
        match parts.as_slice() {
            [whole] => {
                let zloty: i64 = whole
                    .parse()
                    .map_err(|_| DomainError::InvalidAmount(s.to_string()))?;
                Ok(Self(zloty * 100))
            }
            [whole, frac] => {
                let zloty: i64 = whole
                    .parse()
                    .map_err(|_| DomainError::InvalidAmount(s.to_string()))?;
                // Accept any precision, round to grosze (2 decimal places).
                // "1500.00000000" → 1500.00, "10.5" → 10.50, "3.999" → 4.00
                let grosze: i64 = if frac.is_empty() {
                    0
                } else {
                    // Parse all fractional digits as a big number, then scale to centesimal
                    let frac_val: i128 = frac
                        .parse()
                        .map_err(|_| DomainError::InvalidAmount(s.to_string()))?;
                    let frac_len = u32::try_from(frac.len())
                        .map_err(|_| DomainError::InvalidAmount(s.to_string()))?;
                    if frac_len <= 2 {
                        // 1 or 2 digits — scale up to grosze
                        let scale = 10_i128.pow(2 - frac_len);
                        i64::try_from(frac_val * scale)
                            .map_err(|_| DomainError::InvalidAmount(s.to_string()))?
                    } else {
                        // >2 digits — round to grosze
                        let divisor = 10_i128.pow(frac_len - 2);
                        let rounded = (frac_val + divisor / 2) / divisor;
                        i64::try_from(rounded)
                            .map_err(|_| DomainError::InvalidAmount(s.to_string()))?
                    }
                };
                // For "-0.50", zloty is 0 (not negative) so we check the leading '-'
                let sign = if zloty < 0 || s.starts_with('-') {
                    -1
                } else {
                    1
                };
                Ok(Self(zloty * 100 + sign * grosze))
            }
            _ => Err(DomainError::InvalidAmount(s.to_string())),
        }
    }
}

impl std::ops::Add for Money {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}

impl std::ops::Sub for Money {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self(self.0 - rhs.0)
    }
}

/// Quantity as a decimal string (e.g., "160", "2.5").
///
/// Stored as integer + scale to avoid floating-point.
/// Supports up to 6 decimal places (FA(3) allows decimals).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quantity {
    /// Value in smallest units (e.g., 2500 with scale 3 = 2.500)
    value: i64,
    /// Number of decimal places (0-6)
    scale: u8,
}

impl Quantity {
    #[must_use]
    pub fn integer(value: i64) -> Self {
        Self { value, scale: 0 }
    }

    #[must_use]
    pub fn value(&self) -> i64 {
        self.value
    }

    #[must_use]
    pub fn scale(&self) -> u8 {
        self.scale
    }

    pub fn parse(s: &str) -> Result<Self, DomainError> {
        let parts: Vec<&str> = s.split('.').collect();
        match parts.as_slice() {
            [whole] => {
                let value: i64 = whole.parse().map_err(|_| DomainError::InvalidParse {
                    type_name: "Quantity",
                    value: s.to_string(),
                })?;
                Ok(Self { value, scale: 0 })
            }
            [whole, frac] => {
                if frac.len() > 6 {
                    return Err(DomainError::InvalidParse {
                        type_name: "Quantity",
                        value: s.to_string(),
                    });
                }
                let scale = u8::try_from(frac.len()).map_err(|_| DomainError::InvalidParse {
                    type_name: "Quantity",
                    value: s.to_string(),
                })?;
                let whole_val: i64 = whole.parse().map_err(|_| DomainError::InvalidParse {
                    type_name: "Quantity",
                    value: s.to_string(),
                })?;
                let frac_val: i64 = frac.parse().map_err(|_| DomainError::InvalidParse {
                    type_name: "Quantity",
                    value: s.to_string(),
                })?;
                let multiplier = 10_i64.pow(u32::from(scale));
                let sign = if whole_val < 0 || s.starts_with('-') {
                    -1
                } else {
                    1
                };
                Ok(Self {
                    value: whole_val * multiplier + sign * frac_val,
                    scale,
                })
            }
            _ => Err(DomainError::InvalidParse {
                type_name: "Quantity",
                value: s.to_string(),
            }),
        }
    }
}

impl fmt::Display for Quantity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.scale == 0 {
            write!(f, "{}", self.value)
        } else {
            let multiplier = 10_i64.pow(u32::from(self.scale));
            let whole = self.value / multiplier;
            let frac = (self.value % multiplier).abs();
            write!(f, "{whole}.{frac:0>width$}", width = self.scale as usize)
        }
    }
}

impl FromStr for Quantity {
    type Err = DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// ISO 3166-1 alpha-2 country code (e.g., "PL", "DE").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CountryCode(String);

impl CountryCode {
    pub fn parse(s: &str) -> Result<Self, DomainError> {
        let upper = s.trim().to_uppercase();
        if upper.len() == 2 && upper.chars().all(|c| c.is_ascii_uppercase()) {
            Ok(Self(upper))
        } else {
            Err(DomainError::InvalidParse {
                type_name: "CountryCode",
                value: s.to_string(),
            })
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Poland.
    #[must_use]
    pub fn pl() -> Self {
        Self("PL".to_string())
    }
}

impl fmt::Display for CountryCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for CountryCode {
    type Err = DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// ISO 4217 currency code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Currency(String);

impl Currency {
    pub fn parse(s: &str) -> Result<Self, DomainError> {
        let upper = s.trim().to_uppercase();
        if upper.len() == 3 && upper.chars().all(|c| c.is_ascii_uppercase()) {
            Ok(Self(upper))
        } else {
            Err(DomainError::InvalidParse {
                type_name: "Currency",
                value: s.to_string(),
            })
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Polish zloty.
    #[must_use]
    pub fn pln() -> Self {
        Self("PLN".to_string())
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for Currency {
    type Err = DomainError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// Address in FA(3) simplified format (two lines).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Address {
    pub country_code: CountryCode,
    pub line1: String,
    pub line2: String,
}

/// Party on an invoice (seller or buyer).
///
/// NIP is optional: buyers may be individuals (no NIP), foreign entities,
/// or identified by other means (`BrakID` in FA(3)).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Party {
    pub nip: Option<Nip>,
    pub name: String,
    pub address: Address,
}

/// Single line item on an invoice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineItem {
    pub line_number: u32,
    pub description: String,
    pub unit: Option<String>,
    pub quantity: Quantity,
    pub unit_net_price: Option<Money>,
    pub net_value: Money,
    pub vat_rate: VatRate,
    pub vat_amount: Money,
    pub gross_value: Money,
}

/// A complete invoice in our domain model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invoice {
    pub id: InvoiceId,
    /// Owning NIP account — tenant isolation boundary.
    pub nip_account_id: NipAccountId,
    pub direction: Direction,
    pub status: InvoiceStatus,
    pub invoice_type: InvoiceType,
    pub invoice_number: String,
    pub issue_date: NaiveDate,
    pub sale_date: Option<NaiveDate>,
    pub corrected_invoice_number: Option<String>,
    pub correction_reason: Option<String>,
    pub original_ksef_number: Option<KSeFNumber>,
    pub advance_payment_date: Option<NaiveDate>,
    pub seller: Party,
    pub buyer: Party,
    pub currency: Currency,
    pub line_items: Vec<LineItem>,
    pub total_net: Money,
    pub total_vat: Money,
    pub total_gross: Money,
    pub payment_method: Option<PaymentMethod>,
    pub payment_deadline: Option<NaiveDate>,
    pub bank_account: Option<String>,
    pub ksef_number: Option<KSeFNumber>,
    pub ksef_error: Option<String>,
    /// Original FA(3) XML from `KSeF` (stored for audit, not fallback).
    pub raw_xml: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- InvoiceId ---

    #[test]
    fn invoice_id_new_generates_unique_ids() {
        let a = InvoiceId::new();
        let b = InvoiceId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn invoice_id_display_and_from_str_round_trip() {
        let id = InvoiceId::new();
        let displayed = id.to_string();
        let parsed: InvoiceId = displayed.parse().unwrap();
        assert_eq!(id, parsed);
    }

    // --- Direction ---

    #[test]
    fn direction_display_and_from_str_round_trip() {
        for dir in [Direction::Outgoing, Direction::Incoming] {
            let s = dir.to_string();
            let parsed: Direction = s.parse().unwrap();
            assert_eq!(dir, parsed);
        }
    }

    #[test]
    fn direction_from_str_invalid_returns_invalid_parse() {
        let err = "sideways".parse::<Direction>().unwrap_err();
        assert!(matches!(
            err,
            DomainError::InvalidParse {
                type_name: "Direction",
                ..
            }
        ));
    }

    // --- InvoiceType ---

    #[test]
    fn invoice_type_display_and_from_str_round_trip() {
        for invoice_type in [
            InvoiceType::Vat,
            InvoiceType::Kor,
            InvoiceType::Zal,
            InvoiceType::Roz,
            InvoiceType::Upr,
            InvoiceType::VatPef,
            InvoiceType::VatPefSp,
            InvoiceType::KorPef,
            InvoiceType::VatRr,
            InvoiceType::KorVatRr,
            InvoiceType::KorZal,
            InvoiceType::KorRoz,
        ] {
            let s = invoice_type.to_string();
            let parsed: InvoiceType = s.parse().unwrap();
            assert_eq!(invoice_type, parsed);
        }
    }

    #[test]
    fn invoice_type_form_code_mapping_is_correct() {
        assert_eq!(InvoiceType::Vat.form_code(), FormCode::Fa);
        assert_eq!(InvoiceType::Kor.form_code(), FormCode::Kor);
        assert_eq!(InvoiceType::Zal.form_code(), FormCode::Zal);
        assert_eq!(InvoiceType::Roz.form_code(), FormCode::Roz);
        assert_eq!(InvoiceType::Upr.form_code(), FormCode::Upr);
        assert_eq!(InvoiceType::VatPef.form_code(), FormCode::VatPef);
        assert_eq!(InvoiceType::VatPefSp.form_code(), FormCode::VatPefSp);
        assert_eq!(InvoiceType::KorPef.form_code(), FormCode::KorPef);
        assert_eq!(InvoiceType::VatRr.form_code(), FormCode::VatRr);
        assert_eq!(InvoiceType::KorVatRr.form_code(), FormCode::KorVatRr);
        assert_eq!(InvoiceType::KorZal.form_code(), FormCode::KorZal);
        assert_eq!(InvoiceType::KorRoz.form_code(), FormCode::KorRoz);
    }

    #[test]
    fn invoice_type_unknown_value_returns_error() {
        assert!(matches!(
            "unknown".parse::<InvoiceType>(),
            Err(DomainError::InvalidParse {
                type_name: "InvoiceType",
                ..
            })
        ));
    }

    #[test]
    fn form_code_display_and_from_str_round_trip() {
        for form_code in [
            FormCode::Fa,
            FormCode::Kor,
            FormCode::Zal,
            FormCode::Roz,
            FormCode::Upr,
            FormCode::VatPef,
            FormCode::VatPefSp,
            FormCode::KorPef,
            FormCode::VatRr,
            FormCode::KorVatRr,
            FormCode::KorZal,
            FormCode::KorRoz,
        ] {
            let s = form_code.to_string();
            let parsed: FormCode = s.parse().unwrap();
            assert_eq!(form_code, parsed);
        }
    }

    // --- InvoiceStatus transitions ---

    #[test]
    fn status_draft_to_queued_is_valid() {
        let result = InvoiceStatus::Draft.transition_to(InvoiceStatus::Queued);
        assert_eq!(result.unwrap(), InvoiceStatus::Queued);
    }

    #[test]
    fn status_queued_to_submitted_is_valid() {
        let result = InvoiceStatus::Queued.transition_to(InvoiceStatus::Submitted);
        assert_eq!(result.unwrap(), InvoiceStatus::Submitted);
    }

    #[test]
    fn status_submitted_to_accepted_is_valid() {
        let result = InvoiceStatus::Submitted.transition_to(InvoiceStatus::Accepted);
        assert_eq!(result.unwrap(), InvoiceStatus::Accepted);
    }

    #[test]
    fn status_submitted_to_rejected_is_valid() {
        let result = InvoiceStatus::Submitted.transition_to(InvoiceStatus::Rejected);
        assert_eq!(result.unwrap(), InvoiceStatus::Rejected);
    }

    #[test]
    fn status_queued_to_failed_is_valid() {
        let result = InvoiceStatus::Queued.transition_to(InvoiceStatus::Failed);
        assert_eq!(result.unwrap(), InvoiceStatus::Failed);
    }

    #[test]
    fn status_submitted_to_failed_is_valid() {
        let result = InvoiceStatus::Submitted.transition_to(InvoiceStatus::Failed);
        assert_eq!(result.unwrap(), InvoiceStatus::Failed);
    }

    #[test]
    fn status_draft_to_accepted_is_invalid() {
        assert!(
            InvoiceStatus::Draft
                .transition_to(InvoiceStatus::Accepted)
                .is_err()
        );
    }

    #[test]
    fn status_accepted_to_draft_is_invalid() {
        assert!(
            InvoiceStatus::Accepted
                .transition_to(InvoiceStatus::Draft)
                .is_err()
        );
    }

    #[test]
    fn status_failed_to_anything_is_invalid() {
        for target in [
            InvoiceStatus::Draft,
            InvoiceStatus::Queued,
            InvoiceStatus::Submitted,
            InvoiceStatus::Accepted,
        ] {
            assert!(InvoiceStatus::Failed.transition_to(target).is_err());
        }
    }

    #[test]
    fn status_rejected_to_anything_is_invalid() {
        for target in [
            InvoiceStatus::Draft,
            InvoiceStatus::Queued,
            InvoiceStatus::Submitted,
        ] {
            assert!(InvoiceStatus::Rejected.transition_to(target).is_err());
        }
    }

    #[test]
    fn status_fetched_to_anything_is_invalid() {
        for target in [
            InvoiceStatus::Draft,
            InvoiceStatus::Queued,
            InvoiceStatus::Submitted,
            InvoiceStatus::Accepted,
        ] {
            assert!(InvoiceStatus::Fetched.transition_to(target).is_err());
        }
    }

    #[test]
    fn status_display_and_from_str_round_trip() {
        for status in [
            InvoiceStatus::Draft,
            InvoiceStatus::Queued,
            InvoiceStatus::Submitted,
            InvoiceStatus::Accepted,
            InvoiceStatus::Rejected,
            InvoiceStatus::Failed,
            InvoiceStatus::Fetched,
        ] {
            let s = status.to_string();
            let parsed: InvoiceStatus = s.parse().unwrap();
            assert_eq!(status, parsed);
        }
    }

    #[test]
    fn status_from_str_invalid_returns_invalid_parse() {
        let err = "bogus".parse::<InvoiceStatus>().unwrap_err();
        assert!(matches!(
            err,
            DomainError::InvalidParse {
                type_name: "InvoiceStatus",
                ..
            }
        ));
    }

    // --- VatRate ---

    #[test]
    fn vat_rate_percentage() {
        assert_eq!(VatRate::Rate23.percentage(), Some(23));
        assert_eq!(VatRate::Rate8.percentage(), Some(8));
        assert_eq!(VatRate::Rate5.percentage(), Some(5));
        assert_eq!(VatRate::Rate0.percentage(), Some(0));
        assert_eq!(VatRate::Exempt.percentage(), None);
    }

    #[test]
    fn vat_rate_fa3_suffix() {
        assert_eq!(VatRate::Rate23.fa3_suffix(), "1");
        assert_eq!(VatRate::Rate8.fa3_suffix(), "2");
        assert_eq!(VatRate::Rate5.fa3_suffix(), "3");
        assert_eq!(VatRate::Rate0.fa3_suffix(), "6_1");
        assert_eq!(VatRate::Exempt.fa3_suffix(), "7");
    }

    #[test]
    fn vat_rate_display_and_from_str_round_trip() {
        for rate in [
            VatRate::Rate23,
            VatRate::Rate8,
            VatRate::Rate5,
            VatRate::Rate0,
            VatRate::Exempt,
        ] {
            let s = rate.to_string();
            let parsed: VatRate = s.parse().unwrap();
            assert_eq!(rate, parsed);
        }
    }

    #[test]
    fn vat_rate_invalid_string_returns_error() {
        assert!("99".parse::<VatRate>().is_err());
        assert!("".parse::<VatRate>().is_err());
    }

    #[test]
    fn vat_rate_accepts_qualified_non_numeric_codes() {
        assert_eq!("np I".parse::<VatRate>().unwrap(), VatRate::ReverseCharge);
        assert_eq!("ZW A".parse::<VatRate>().unwrap(), VatRate::Exempt);
        assert_eq!("oo x".parse::<VatRate>().unwrap(), VatRate::NotSubject);
    }

    #[test]
    fn vat_rate_rejects_invalid_qualified_non_numeric_codes() {
        assert!("np I A".parse::<VatRate>().is_err());
        assert!("np <x>".parse::<VatRate>().is_err());
        assert!("zw ???".parse::<VatRate>().is_err());
    }

    // --- Money ---

    #[test]
    fn money_from_grosze() {
        let m = Money::from_grosze(12345);
        assert_eq!(m.zloty_part(), 123);
        assert_eq!(m.grosze_part(), 45);
    }

    #[test]
    fn money_from_pln() {
        let m = Money::from_pln(123, 45);
        assert_eq!(m.grosze(), 12345);
    }

    #[test]
    fn money_display() {
        assert_eq!(Money::from_grosze(12345).to_string(), "123.45");
        assert_eq!(Money::from_grosze(100).to_string(), "1.00");
        assert_eq!(Money::from_grosze(5).to_string(), "0.05");
        assert_eq!(Money::from_grosze(0).to_string(), "0.00");
    }

    #[test]
    fn money_display_negative() {
        assert_eq!(Money::from_grosze(-12345).to_string(), "-123.45");
    }

    #[test]
    fn money_from_str() {
        assert_eq!(
            "123.45".parse::<Money>().unwrap(),
            Money::from_grosze(12345)
        );
        assert_eq!("0.05".parse::<Money>().unwrap(), Money::from_grosze(5));
        assert_eq!("100".parse::<Money>().unwrap(), Money::from_grosze(10000));
        assert_eq!("1.5".parse::<Money>().unwrap(), Money::from_grosze(150));
    }

    #[test]
    fn money_from_str_invalid() {
        assert!("abc".parse::<Money>().is_err());
        assert!("1.2.3".parse::<Money>().is_err());
    }

    #[test]
    fn money_from_str_rounds_extra_decimals() {
        // FA(3) XML may have >2 decimal places — round to grosze
        assert_eq!("1.234".parse::<Money>().unwrap(), Money::from_grosze(123));
        assert_eq!(
            "1500.00000000".parse::<Money>().unwrap(),
            Money::from_grosze(150_000)
        );
        assert_eq!(
            "10000.0000".parse::<Money>().unwrap(),
            Money::from_grosze(1_000_000)
        );
        assert_eq!("3.999".parse::<Money>().unwrap(), Money::from_grosze(400)); // rounds up
    }

    #[test]
    fn money_display_and_from_str_round_trip() {
        for amount in [0, 1, 99, 100, 12345, 999999] {
            let m = Money::from_grosze(amount);
            let s = m.to_string();
            let parsed: Money = s.parse().unwrap();
            assert_eq!(m, parsed);
        }
    }

    #[test]
    fn money_arithmetic() {
        let a = Money::from_grosze(10000);
        let b = Money::from_grosze(2300);
        assert_eq!((a + b).grosze(), 12300);
        assert_eq!((a - b).grosze(), 7700);
    }

    // --- PaymentMethod ---

    #[test]
    fn payment_method_fa3_code() {
        assert_eq!(PaymentMethod::Cash.fa3_code(), 1);
        assert_eq!(PaymentMethod::Transfer.fa3_code(), 6);
        assert_eq!(PaymentMethod::Mobile.fa3_code(), 7);
    }

    #[test]
    fn payment_method_try_from_code() {
        assert_eq!(PaymentMethod::try_from(1u8).unwrap(), PaymentMethod::Cash);
        assert_eq!(PaymentMethod::try_from(7u8).unwrap(), PaymentMethod::Mobile);
        assert!(PaymentMethod::try_from(99u8).is_err());
    }

    // --- Quantity ---

    #[test]
    fn quantity_integer_display() {
        assert_eq!(Quantity::integer(160).to_string(), "160");
    }

    #[test]
    fn quantity_decimal_parse_and_display() {
        let q = Quantity::parse("2.5").unwrap();
        assert_eq!(q.to_string(), "2.5");
    }

    #[test]
    fn quantity_display_and_from_str_round_trip() {
        for s in ["1", "100", "2.5", "0.25", "3.141"] {
            let q: Quantity = s.parse().unwrap();
            assert_eq!(q.to_string(), s);
        }
    }

    #[test]
    fn quantity_invalid_returns_error() {
        assert!(Quantity::parse("abc").is_err());
        assert!(Quantity::parse("1.2.3").is_err());
        assert!(Quantity::parse("1.1234567").is_err()); // > 6 decimal places
    }

    // --- CountryCode ---

    #[test]
    fn country_code_valid() {
        let pl = CountryCode::parse("PL").unwrap();
        assert_eq!(pl.as_str(), "PL");
    }

    #[test]
    fn country_code_normalizes_case() {
        let pl = CountryCode::parse("pl").unwrap();
        assert_eq!(pl.as_str(), "PL");
    }

    #[test]
    fn country_code_invalid() {
        assert!(CountryCode::parse("").is_err());
        assert!(CountryCode::parse("P").is_err());
        assert!(CountryCode::parse("POL").is_err());
        assert!(CountryCode::parse("12").is_err());
    }

    #[test]
    fn country_code_pl_shortcut() {
        assert_eq!(CountryCode::pl().as_str(), "PL");
    }

    // --- Currency ---

    #[test]
    fn currency_valid() {
        let pln = Currency::parse("PLN").unwrap();
        assert_eq!(pln.as_str(), "PLN");
    }

    #[test]
    fn currency_normalizes_case() {
        let pln = Currency::parse("pln").unwrap();
        assert_eq!(pln.as_str(), "PLN");
    }

    #[test]
    fn currency_invalid() {
        assert!(Currency::parse("").is_err());
        assert!(Currency::parse("PL").is_err());
        assert!(Currency::parse("EURO").is_err());
        assert!(Currency::parse("123").is_err());
    }

    #[test]
    fn currency_pln_shortcut() {
        assert_eq!(Currency::pln().as_str(), "PLN");
    }

    #[test]
    fn format_invoice_number_standard() {
        assert_eq!(
            super::format_invoice_number("FV", 2026, 4, 7),
            "FV/2026/04/007"
        );
    }

    #[test]
    fn format_invoice_number_january_first() {
        assert_eq!(
            super::format_invoice_number("FV", 2026, 1, 1),
            "FV/2026/01/001"
        );
    }

    #[test]
    fn format_invoice_number_custom_prefix() {
        assert_eq!(
            super::format_invoice_number("PROJ", 2026, 12, 42),
            "PROJ/2026/12/042"
        );
    }
}
