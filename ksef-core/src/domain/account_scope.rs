use crate::domain::nip::Nip;
use crate::domain::nip_account::NipAccountId;

/// Proof-of-authorization token for a NIP account.
///
/// `AccountScope` can only be constructed inside `ksef-core` — its constructor is
/// `pub(crate)`.  Every instance guarantees that
/// [`NipAccountRepository::verify_access`] was called with a valid `(UserId, Nip)`
/// pair and returned `Some`, or that trusted internal code (e.g. the job worker
/// reconstructing context from a previously authorized job payload) constructed it.
///
/// All per-account service and repository methods require `&AccountScope`.
/// Passing a raw [`NipAccountId`] is not enough — the compiler rejects it.
#[derive(Debug, Clone)]
pub struct AccountScope {
    id: NipAccountId,
    nip: Nip,
}

impl AccountScope {
    /// Only callable from within `ksef-core`.
    pub(crate) fn new(id: NipAccountId, nip: Nip) -> Self {
        Self { id, nip }
    }

    /// The authorized account ID.
    pub fn id(&self) -> &NipAccountId {
        &self.id
    }

    /// The NIP of the authorized account (used for KSeF API calls).
    pub fn nip(&self) -> &Nip {
        &self.nip
    }
}
