use std::sync::Arc;

use lettre::message::{Mailbox, Message};
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::Tls;
use lettre::{SmtpTransport, Transport};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtpSecurityMode {
    StartTls,
    Plaintext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtpAuthMode {
    Required,
    None,
}

#[derive(Debug, Clone)]
pub struct SmtpEmailConfig {
    pub server: String,
    pub port: u16,
    pub security: SmtpSecurityMode,
    pub auth: SmtpAuthMode,
    pub username: Option<String>,
    pub password: Option<String>,
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

#[derive(Debug, Clone)]
pub struct ApplicationAccessInviteEmail {
    pub recipient_email: String,
    pub inviter_email: String,
    pub invite_url: String,
}

#[derive(Debug, Error)]
pub enum EmailConfigError {
    #[error("invalid sender email address: {0}")]
    InvalidSender(String),
    #[error("smtp authentication requires username and password")]
    MissingCredentials,
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
    fn send_application_access_invite(
        &self,
        invite: ApplicationAccessInviteEmail,
    ) -> Result<(), EmailSendError>;
}

pub type SharedEmailSender = Arc<dyn EmailSender>;

#[cfg_attr(not(test), allow(dead_code))]
pub struct NoopEmailSender;

impl EmailSender for NoopEmailSender {
    fn send_workspace_invite(&self, _invite: WorkspaceInviteEmail) -> Result<(), EmailSendError> {
        Ok(())
    }

    fn send_application_access_invite(
        &self,
        _invite: ApplicationAccessInviteEmail,
    ) -> Result<(), EmailSendError> {
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
        let mut builder = match config.security {
            SmtpSecurityMode::StartTls => SmtpTransport::starttls_relay(&config.server)
                .map_err(|e| EmailConfigError::Transport(format!("{e}")))?
                .port(config.port),
            SmtpSecurityMode::Plaintext => SmtpTransport::builder_dangerous(&config.server)
                .port(config.port)
                .tls(Tls::None),
        };

        if config.auth == SmtpAuthMode::Required {
            let username = config
                .username
                .clone()
                .ok_or(EmailConfigError::MissingCredentials)?;
            let password = config
                .password
                .clone()
                .ok_or(EmailConfigError::MissingCredentials)?;
            builder = builder.credentials(Credentials::new(username, password));
        }

        let mailer = builder.build();

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
        let subject = format!(
            "Zaproszenie do współdzielonego workspace {}",
            invite.workspace_name
        );
        let body = format!(
            "Otrzymujesz zaproszenie do istniejącego współdzielonego workspace \"{}\" jako {}.\n\nTo zaproszenie nie daje tylko dostępu do aplikacji. Po zaakceptowaniu zobaczysz dane tego workspace, w tym przypisane do niego NIP-y i faktury, zgodnie z nadaną rolą.\n\nZaprasza: {}\n\nJeśli potrzebujesz osobnego, niezależnego workspace z własnymi danymi, nie używaj tego linku i poproś o osobny bootstrap workspace.\n\nDokończ rejestrację lub zaloguj się przez ten link:\n{}\n\nJeśli nie oczekujesz tego zaproszenia, zignoruj tę wiadomość.",
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

    fn send_application_access_invite(
        &self,
        invite: ApplicationAccessInviteEmail,
    ) -> Result<(), EmailSendError> {
        let to = Mailbox::new(
            None,
            invite
                .recipient_email
                .parse()
                .map_err(|e| EmailSendError::InvalidRecipient(format!("{e}")))?,
        );
        let subject = "Dostęp do aplikacji KSeF Pay".to_string();
        let body = format!(
            "Otrzymujesz zaproszenie do aplikacji KSeF Pay.\n\nTo zaproszenie nie doda Cię do cudzego workspace i nie udostępni cudzych danych. Po zaakceptowaniu uzyskasz dostęp do aplikacji i utworzysz własny, niezależny workspace.\n\nZaprasza: {}\n\nDokończ rejestrację lub zaloguj się przez ten link:\n{}\n\nJeśli nie oczekujesz tego zaproszenia, zignoruj tę wiadomość.",
            invite.inviter_email, invite.invite_url
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

pub async fn dispatch_application_access_invite(
    sender: SharedEmailSender,
    invite: ApplicationAccessInviteEmail,
) -> Result<(), EmailSendError> {
    tokio::task::spawn_blocking(move || sender.send_application_access_invite(invite))
        .await
        .map_err(|e| EmailSendError::Join(e.to_string()))?
}
