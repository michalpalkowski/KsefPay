use async_trait::async_trait;
use chrono::Utc;
use reqwest::Client;
use serde::Deserialize;

use crate::domain::company::{CompanyInfo, VatStatus};
use crate::domain::nip::Nip;
use crate::ports::company_lookup::{CompanyLookup, CompanyLookupError};

const BASE_URL: &str = "https://wl-api.mf.gov.pl/api/search/nip";

/// HTTP client for Biała Lista VAT (wl-api.mf.gov.pl).
pub struct WhiteListClient {
    client: Client,
}

impl WhiteListClient {
    #[must_use]
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }
}

impl Default for WhiteListClient {
    fn default() -> Self {
        Self::new()
    }
}

// --- API response types (match real wl-api.mf.gov.pl JSON) ---

#[derive(Deserialize)]
struct WlResponse {
    result: WlResult,
}

#[derive(Deserialize)]
struct WlResult {
    subject: Option<WlSubject>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WlSubject {
    name: String,
    nip: String,
    status_vat: Option<String>,
    working_address: Option<String>,
    residence_address: Option<String>,
    #[serde(default)]
    account_numbers: Vec<String>,
}

fn parse_subject(subject: WlSubject, nip: &Nip) -> CompanyInfo {
    let address = subject
        .working_address
        .or(subject.residence_address)
        .unwrap_or_default();

    let vat_status = subject
        .status_vat
        .as_deref()
        .map(VatStatus::from_whitelist)
        .unwrap_or(VatStatus::Unregistered);

    CompanyInfo {
        nip: nip.clone(),
        name: subject.name,
        address,
        bank_accounts: subject.account_numbers,
        vat_status,
        fetched_at: Utc::now(),
    }
}

#[async_trait]
impl CompanyLookup for WhiteListClient {
    async fn lookup(&self, nip: &Nip) -> Result<CompanyInfo, CompanyLookupError> {
        let date = Utc::now().format("%Y-%m-%d");
        let url = format!("{BASE_URL}/{nip}?date={date}");

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| CompanyLookupError::ApiError(format!("HTTP request failed: {e}")))?;

        let status = response.status();
        if status.as_u16() == 400 {
            return Err(CompanyLookupError::NotFound(nip.clone()));
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(CompanyLookupError::ApiError(format!(
                "HTTP {status}: {body}"
            )));
        }

        let wl: WlResponse = response
            .json()
            .await
            .map_err(|e| CompanyLookupError::ApiError(format!("invalid JSON: {e}")))?;

        let subject = wl
            .result
            .subject
            .ok_or_else(|| CompanyLookupError::NotFound(nip.clone()))?;

        Ok(parse_subject(subject, nip))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_subject_maps_all_fields() {
        let subject = WlSubject {
            name: "FIRMA SP. Z O.O.".to_string(),
            nip: "5260250274".to_string(),
            status_vat: Some("Czynny".to_string()),
            working_address: Some("ul. Testowa 1, 00-001 Warszawa".to_string()),
            residence_address: None,
            account_numbers: vec!["PL61109010140000071219812874".to_string()],
        };
        let nip = Nip::parse("5260250274").unwrap();

        let info = parse_subject(subject, &nip);

        assert_eq!(info.name, "FIRMA SP. Z O.O.");
        assert_eq!(info.address, "ul. Testowa 1, 00-001 Warszawa");
        assert_eq!(info.vat_status, VatStatus::Active);
        assert_eq!(info.bank_accounts.len(), 1);
    }

    #[test]
    fn parse_subject_falls_back_to_residence_address() {
        let subject = WlSubject {
            name: "Test".to_string(),
            nip: "5260250274".to_string(),
            status_vat: None,
            working_address: None,
            residence_address: Some("ul. Domowa 5".to_string()),
            account_numbers: vec![],
        };
        let nip = Nip::parse("5260250274").unwrap();

        let info = parse_subject(subject, &nip);

        assert_eq!(info.address, "ul. Domowa 5");
        assert_eq!(info.vat_status, VatStatus::Unregistered);
    }

    #[tokio::test]
    #[ignore = "requires network access to wl-api.mf.gov.pl"]
    async fn lookup_real_nip_returns_company_info() {
        let client = WhiteListClient::new();
        let nip = Nip::parse("5260250274").unwrap();

        let info = client.lookup(&nip).await.unwrap();

        assert_eq!(info.nip, nip);
        assert!(
            info.name.contains("MINISTERSTWO FINANSÓW") || info.name.contains("MINISTERSTWO"),
            "unexpected name: {}",
            info.name
        );
        assert_eq!(info.vat_status, VatStatus::Active);
        assert!(!info.bank_accounts.is_empty());
    }

    #[tokio::test]
    #[ignore = "requires network access to wl-api.mf.gov.pl"]
    async fn lookup_nonexistent_nip_returns_not_found() {
        let client = WhiteListClient::new();
        // Valid checksum NIP but not a real company
        let nip = Nip::parse("9990000008").unwrap();

        let result = client.lookup(&nip).await;

        assert!(
            matches!(result, Err(CompanyLookupError::NotFound(_))),
            "expected NotFound, got: {result:?}"
        );
    }
}
