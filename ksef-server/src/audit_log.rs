use ksef_core::domain::audit::AuditAction;
use ksef_core::domain::nip::Nip;
use ksef_core::domain::user::UserId;

use crate::state::AppState;

/// Best-effort audit logging. Failures are logged but never block the user flow.
pub async fn log_action(
    state: &AppState,
    user_id: &UserId,
    user_email: &str,
    nip: Option<&Nip>,
    action: AuditAction,
    details: Option<String>,
    ip_address: Option<String>,
) {
    if let Err(err) = state
        .audit_service
        .log_action(user_id, user_email, nip, action, details, ip_address)
        .await
    {
        tracing::warn!(
            action = %action,
            user_id = %user_id,
            error = %err,
            "failed to write audit log"
        );
    }
}
