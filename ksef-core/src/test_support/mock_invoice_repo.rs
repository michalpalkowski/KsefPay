use std::sync::Mutex;

use async_trait::async_trait;

use crate::domain::account_scope::AccountScope;
use crate::domain::invoice::{Invoice, InvoiceId, InvoiceStatus};
use crate::domain::session::KSeFNumber;
use crate::error::RepositoryError;
use crate::ports::invoice_repository::{InvoiceFilter, InvoiceRepository};

/// In-memory mock of `InvoiceRepository` for unit tests.
pub struct MockInvoiceRepo {
    invoices: Mutex<Vec<Invoice>>,
}

impl MockInvoiceRepo {
    #[must_use]
    pub fn new() -> Self {
        Self {
            invoices: Mutex::new(Vec::new()),
        }
    }

    #[must_use]
    pub fn count(&self) -> usize {
        self.invoices.lock().unwrap().len()
    }
}

impl Default for MockInvoiceRepo {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl InvoiceRepository for MockInvoiceRepo {
    async fn save(&self, invoice: &Invoice) -> Result<InvoiceId, RepositoryError> {
        let mut store = self.invoices.lock().unwrap();

        if store.iter().any(|i| i.id.as_uuid() == invoice.id.as_uuid()) {
            return Err(RepositoryError::Duplicate {
                entity: "Invoice",
                key: invoice.id.to_string(),
            });
        }

        let id = invoice.id.clone();
        store.push(invoice.clone());
        Ok(id)
    }

    async fn find_by_id(
        &self,
        id: &InvoiceId,
        scope: &AccountScope,
    ) -> Result<Invoice, RepositoryError> {
        let store = self.invoices.lock().unwrap();
        store
            .iter()
            .find(|i| i.id.as_uuid() == id.as_uuid() && i.nip_account_id == *scope.id())
            .cloned()
            .ok_or_else(|| RepositoryError::NotFound {
                entity: "Invoice",
                id: id.to_string(),
            })
    }

    async fn update_status(
        &self,
        id: &InvoiceId,
        scope: &AccountScope,
        status: InvoiceStatus,
    ) -> Result<(), RepositoryError> {
        let mut store = self.invoices.lock().unwrap();
        let invoice = store
            .iter_mut()
            .find(|i| i.id.as_uuid() == id.as_uuid() && i.nip_account_id == *scope.id())
            .ok_or_else(|| RepositoryError::NotFound {
                entity: "Invoice",
                id: id.to_string(),
            })?;
        invoice.status = status;
        Ok(())
    }

    async fn set_ksef_number(
        &self,
        id: &InvoiceId,
        scope: &AccountScope,
        ksef_number: &str,
    ) -> Result<(), RepositoryError> {
        let mut store = self.invoices.lock().unwrap();
        let invoice = store
            .iter_mut()
            .find(|i| i.id.as_uuid() == id.as_uuid() && i.nip_account_id == *scope.id())
            .ok_or_else(|| RepositoryError::NotFound {
                entity: "Invoice",
                id: id.to_string(),
            })?;
        invoice.ksef_number = Some(crate::domain::session::KSeFNumber::new(
            ksef_number.to_string(),
        ));
        Ok(())
    }

    async fn set_ksef_error(
        &self,
        id: &InvoiceId,
        scope: &AccountScope,
        error: &str,
    ) -> Result<(), RepositoryError> {
        let mut store = self.invoices.lock().unwrap();
        let invoice = store
            .iter_mut()
            .find(|i| i.id.as_uuid() == id.as_uuid() && i.nip_account_id == *scope.id())
            .ok_or_else(|| RepositoryError::NotFound {
                entity: "Invoice",
                id: id.to_string(),
            })?;
        invoice.ksef_error = Some(error.to_string());
        Ok(())
    }

    async fn find_by_ksef_number(
        &self,
        ksef_number: &KSeFNumber,
    ) -> Result<Option<Invoice>, RepositoryError> {
        let store = self.invoices.lock().unwrap();
        Ok(store
            .iter()
            .find(|i| {
                i.ksef_number
                    .as_ref()
                    .is_some_and(|n| n.as_str() == ksef_number.as_str())
            })
            .cloned())
    }

    async fn find_by_ksef_number_and_account(
        &self,
        ksef_number: &KSeFNumber,
        scope: &AccountScope,
    ) -> Result<Option<Invoice>, RepositoryError> {
        let store = self.invoices.lock().unwrap();
        Ok(store
            .iter()
            .find(|i| {
                i.ksef_number
                    .as_ref()
                    .is_some_and(|n| n.as_str() == ksef_number.as_str())
                    && i.nip_account_id == *scope.id()
            })
            .cloned())
    }

    async fn upsert_by_ksef_number(&self, invoice: &Invoice) -> Result<InvoiceId, RepositoryError> {
        let mut store = self.invoices.lock().unwrap();
        let ksef_num = invoice
            .ksef_number
            .as_ref()
            .expect("upsert_by_ksef_number requires ksef_number");

        if let Some(existing) = store.iter_mut().find(|i| {
            i.ksef_number
                .as_ref()
                .is_some_and(|n| n.as_str() == ksef_num.as_str())
                && i.nip_account_id == invoice.nip_account_id
        }) {
            // Update all fields except id and nip_account_id
            existing.direction = invoice.direction;
            existing.invoice_number = invoice.invoice_number.clone();
            existing.issue_date = invoice.issue_date;
            existing.sale_date = invoice.sale_date;
            existing.seller = invoice.seller.clone();
            existing.buyer = invoice.buyer.clone();
            existing.currency = invoice.currency.clone();
            existing.line_items = invoice.line_items.clone();
            existing.total_net = invoice.total_net;
            existing.total_vat = invoice.total_vat;
            existing.total_gross = invoice.total_gross;
            existing.payment_method = invoice.payment_method;
            existing.payment_deadline = invoice.payment_deadline;
            existing.bank_account = invoice.bank_account.clone();
            existing.ksef_error = invoice.ksef_error.clone();
            existing.raw_xml = invoice.raw_xml.clone();
            return Ok(existing.id.clone());
        }

        let id = invoice.id.clone();
        store.push(invoice.clone());
        Ok(id)
    }

    async fn list(
        &self,
        scope: &AccountScope,
        filter: &InvoiceFilter,
    ) -> Result<Vec<Invoice>, RepositoryError> {
        let store = self.invoices.lock().unwrap();
        let mut result: Vec<Invoice> = store
            .iter()
            .filter(|inv| {
                inv.nip_account_id == *scope.id()
                    && filter.direction.map_or(true, |d| inv.direction == d)
                    && filter.status.map_or(true, |s| inv.status == s)
            })
            .cloned()
            .collect();

        if let Some(offset) = filter.offset {
            result = result.into_iter().skip(offset as usize).collect();
        }
        if let Some(limit) = filter.limit {
            result.truncate(limit as usize);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    use crate::domain::nip::Nip;
    use crate::domain::nip_account::NipAccountId;
    use crate::test_support::fixtures::sample_invoice;

    fn test_scope() -> AccountScope {
        let id = NipAccountId::from_uuid(Uuid::from_u128(1));
        let nip = Nip::parse("5260250274").unwrap();
        AccountScope::new(id, nip)
    }

    /// Contract test: save then find_by_id returns the same invoice.
    #[tokio::test]
    async fn save_and_find_by_id() {
        let repo = MockInvoiceRepo::new();
        let invoice = sample_invoice();
        let scope = AccountScope::new(
            invoice.nip_account_id.clone(),
            Nip::parse("5260250274").unwrap(),
        );
        let id = repo.save(&invoice).await.unwrap();

        let found = repo.find_by_id(&id, &scope).await.unwrap();
        assert_eq!(found.id.as_uuid(), invoice.id.as_uuid());
        assert_eq!(found.invoice_number, invoice.invoice_number);
    }

    /// Contract test: find_by_id on missing id returns NotFound.
    #[tokio::test]
    async fn find_by_id_not_found() {
        let repo = MockInvoiceRepo::new();
        let missing_id = InvoiceId::new();
        let err = repo
            .find_by_id(&missing_id, &test_scope())
            .await
            .unwrap_err();
        assert!(matches!(err, RepositoryError::NotFound { .. }));
    }

    /// Contract test: duplicate save returns Duplicate error.
    #[tokio::test]
    async fn save_duplicate_returns_error() {
        let repo = MockInvoiceRepo::new();
        let invoice = sample_invoice();
        repo.save(&invoice).await.unwrap();
        let err = repo.save(&invoice).await.unwrap_err();
        assert!(matches!(err, RepositoryError::Duplicate { .. }));
    }

    /// Contract test: update_status changes the status.
    #[tokio::test]
    async fn update_status_changes_status() {
        let repo = MockInvoiceRepo::new();
        let invoice = sample_invoice();
        let scope = AccountScope::new(
            invoice.nip_account_id.clone(),
            Nip::parse("5260250274").unwrap(),
        );
        let id = repo.save(&invoice).await.unwrap();

        repo.update_status(&id, &scope, InvoiceStatus::Queued)
            .await
            .unwrap();

        let found = repo.find_by_id(&id, &scope).await.unwrap();
        assert_eq!(found.status, InvoiceStatus::Queued);
    }

    /// Contract test: update_status on missing id returns NotFound.
    #[tokio::test]
    async fn update_status_not_found() {
        let repo = MockInvoiceRepo::new();
        let err = repo
            .update_status(&InvoiceId::new(), &test_scope(), InvoiceStatus::Queued)
            .await
            .unwrap_err();
        assert!(matches!(err, RepositoryError::NotFound { .. }));
    }

    /// Contract test: set_ksef_number persists.
    #[tokio::test]
    async fn set_ksef_number_persists() {
        let repo = MockInvoiceRepo::new();
        let invoice = sample_invoice();
        let scope = AccountScope::new(
            invoice.nip_account_id.clone(),
            Nip::parse("5260250274").unwrap(),
        );
        let id = repo.save(&invoice).await.unwrap();

        repo.set_ksef_number(&id, &scope, "KSeF-12345")
            .await
            .unwrap();

        let found = repo.find_by_id(&id, &scope).await.unwrap();
        assert_eq!(found.ksef_number.unwrap().as_str(), "KSeF-12345");
    }

    /// Contract test: list with direction filter.
    #[tokio::test]
    async fn list_filters_by_direction() {
        let repo = MockInvoiceRepo::new();

        let mut outgoing = sample_invoice();
        outgoing.direction = crate::domain::invoice::Direction::Outgoing;
        repo.save(&outgoing).await.unwrap();

        let mut incoming = sample_invoice();
        incoming.direction = crate::domain::invoice::Direction::Incoming;
        repo.save(&incoming).await.unwrap();

        let scope = AccountScope::new(
            outgoing.nip_account_id.clone(),
            Nip::parse("5260250274").unwrap(),
        );
        let filter =
            InvoiceFilter::new().with_direction(crate::domain::invoice::Direction::Outgoing);
        let result = repo.list(&scope, &filter).await.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].direction,
            crate::domain::invoice::Direction::Outgoing
        );
    }

    /// Contract test: list with limit and offset.
    #[tokio::test]
    async fn list_with_limit_and_offset() {
        let repo = MockInvoiceRepo::new();

        for _ in 0..5 {
            repo.save(&sample_invoice()).await.unwrap();
        }

        let mut filter = InvoiceFilter::new();
        filter.limit = Some(2);
        filter.offset = Some(1);
        let result = repo.list(&test_scope(), &filter).await.unwrap();
        assert_eq!(result.len(), 2);
    }

    /// Contract test: empty list returns empty vec.
    #[tokio::test]
    async fn list_empty_returns_empty() {
        let repo = MockInvoiceRepo::new();
        let result = repo
            .list(&test_scope(), &InvoiceFilter::new())
            .await
            .unwrap();
        assert!(result.is_empty());
    }
}
