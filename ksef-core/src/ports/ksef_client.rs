use async_trait::async_trait;

use crate::domain::auth::AccessToken;
use crate::domain::crypto::{EncryptedInvoice, KSeFPublicKey};
use crate::domain::session::{InvoiceMetadata, InvoiceQuery, KSeFNumber, SessionReference, Upo};
use crate::domain::xml::UntrustedInvoiceXml;
use crate::error::KSeFError;

/// Port: `KSeF` session and invoice operations.
#[async_trait]
pub trait KSeFClient: Send + Sync {
    async fn open_session(
        &self,
        access_token: &AccessToken,
        session_encryption: &EncryptedInvoice,
    ) -> Result<SessionReference, KSeFError>;

    async fn send_invoice(
        &self,
        access_token: &AccessToken,
        session: &SessionReference,
        encrypted_invoice: &EncryptedInvoice,
    ) -> Result<KSeFNumber, KSeFError>;

    async fn close_session(
        &self,
        access_token: &AccessToken,
        session: &SessionReference,
    ) -> Result<Upo, KSeFError>;

    async fn get_upo(
        &self,
        access_token: &AccessToken,
        session: &SessionReference,
    ) -> Result<Upo, KSeFError>;

    async fn fetch_invoice(
        &self,
        access_token: &AccessToken,
        ksef_number: &KSeFNumber,
    ) -> Result<UntrustedInvoiceXml, KSeFError>;

    async fn query_invoices(
        &self,
        access_token: &AccessToken,
        criteria: &InvoiceQuery,
    ) -> Result<Vec<InvoiceMetadata>, KSeFError>;

    async fn fetch_public_keys(&self) -> Result<Vec<KSeFPublicKey>, KSeFError>;
}
