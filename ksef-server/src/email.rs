use std::sync::Arc;

use lettre::message::{Mailbox, Message};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{SmtpTransport, Transport};
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct SmtpEmailConfig {
    pub server: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub from_email: String,
    pub from_name: String,
}

#[derive(Debug, Clone)]
pub struct WorkspaceInviteEmail {
    pub recipient_email: String,
    pub workspace_name: String,
    pub inviter_email: String,
    pub role_label: String,
    pub invite_url: String,
}

#[derive(Debug, Error)]
pub enum EmailConfigError {
    #[error("invalid sender email address: {0}")]
    InvalidSender(String),
    #[error("smtp transport initialization failed: {0}")]
    Transport(String),
}

#[derive(Debug, Error)]
pub enum EmailSendError {
    #[error("invalid recipient email address: {0}")]
    InvalidRecipient(String),
    #[error("message build failed: {0}")]
    Build(String),
    #[error("smtp delivery failed: {0}")]
    Transport(String),
    #[error("email worker join failed: {0}")]
    Join(String),
}

pub trait EmailSender: Send + Sync {
    fn send_workspace_invite(&self, invite: WorkspaceInviteEmail) -> Result<(), EmailSendError>;
}

pub type SharedEmailSender = Arc<dyn EmailSender>;

pub struct NoopEmailSender;

impl EmailSender for NoopEmailSender {
    fn send_workspace_invite(&self, _invite: WorkspaceInviteEmail) -> Result<(), EmailSendError> {
        Ok(())
    }
}

pub struct SmtpEmailSender {
    mailer: SmtpTransport,
    from: Mailbox,
}

impl SmtpEmailSender {
    pub fn new(config: &SmtpEmailConfig) -> Result<Self, EmailConfigError> {
        let from = Mailbox::new(
            Some(config.from_name.clone()),
            config
                .from_email
                .parse()
                .map_err(|e| EmailConfigError::InvalidSender(format!("{e}")))?,
        );
        let credentials = Credentials::new(config.username.clone(), config.password.clone());
        let mailer = SmtpTransport::starttls_relay(&config.server)
            .map_err(|e| EmailConfigError::Transport(format!("{e}")))?
            .port(config.port)
            .credentials(credentials)
            .build();

        Ok(Self { mailer, from })
    }
}

impl EmailSender for SmtpEmailSender {
    fn send_workspace_invite(&self, invite: WorkspaceInviteEmail) -> Result<(), EmailSendError> {
        let to = Mailbox::new(
            None,
            invite
                .recipient_email
                .parse()
                .map_err(|e| EmailSendError::InvalidRecipient(format!("{e}")))?,
        );
        let subject = format!("Zaproszenie do workspace {}", invite.workspace_name);
        let body = format!(
            "Otrzymujesz zaproszenie do workspace \"{}\" jako {}.\n\nZaprasza: {}\n\nDokończ rejestrację lub zaloguj się przez ten link:\n{}\n\nJeśli nie oczekujesz tego zaproszenia, zignoruj tę wiadomość.",
            invite.workspace_name, invite.role_label, invite.inviter_email, invite.invite_url
        );
        let message = Message::builder()
            .from(self.from.clone())
            .to(to)
            .subject(subject)
            .body(body)
            .map_err(|e| EmailSendError::Build(format!("{e}")))?;

        self.mailer
            .send(&message)
            .map_err(|e| EmailSendError::Transport(format!("{e}")))?;
        Ok(())
    }
}

pub async fn dispatch_workspace_invite(
    sender: SharedEmailSender,
    invite: WorkspaceInviteEmail,
) -> Result<(), EmailSendError> {
    tokio::task::spawn_blocking(move || sender.send_workspace_invite(invite))
        .await
        .map_err(|e| EmailSendError::Join(e.to_string()))?
}
