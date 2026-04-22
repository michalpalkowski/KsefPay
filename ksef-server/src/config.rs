use ksef_core::domain::environment::KSeFEnvironment;

use crate::email::{SmtpAuthMode, SmtpEmailConfig, SmtpSecurityMode};
use crate::state::ApplicationAccessMode;

pub struct Config {
    pub database_url: String,
    pub server_host: String,
    pub server_port: u16,
    pub app_base_url: String,
    pub smtp: Option<SmtpEmailConfig>,
    pub ksef_environment: KSeFEnvironment,
    pub cert_storage_key: Option<String>,
    pub ksef_cert_pem: Option<String>,
    pub ksef_key_pem: Option<String>,
    /// Allowlist of emails permitted to register. Empty = registration closed.
    pub allowed_emails: Vec<String>,
    pub application_access_mode: ApplicationAccessMode,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let database_url = std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL not set")?;
        let app_base_url = std::env::var("APP_BASE_URL").map_err(|_| "APP_BASE_URL not set")?;
        if !(app_base_url.starts_with("https://") || app_base_url.starts_with("http://")) {
            return Err("APP_BASE_URL must start with http:// or https://".to_string());
        }

        let server_host = std::env::var("SERVER_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());

        let server_port: u16 = std::env::var("SERVER_PORT")
            .unwrap_or_else(|_| "3000".to_string())
            .parse()
            .map_err(|_| "SERVER_PORT must be a valid port number")?;

        let ksef_environment: KSeFEnvironment = std::env::var("KSEF_ENVIRONMENT")
            .unwrap_or_else(|_| "test".to_string())
            .parse()
            .map_err(|e| format!("invalid KSEF_ENVIRONMENT: {e}"))?;

        let ksef_cert_pem = std::env::var("KSEF_CERT_PEM").ok();
        let ksef_key_pem = std::env::var("KSEF_KEY_PEM").ok();
        let cert_storage_key = std::env::var("CERT_STORAGE_KEY").ok();

        let allowed_emails = std::env::var("ALLOWED_EMAILS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        let application_access_mode = match std::env::var("APPLICATION_ACCESS_MODE")
            .unwrap_or_else(|_| "email_invite".to_string())
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "email_invite" => ApplicationAccessMode::EmailInvite,
            "trusted_email" => ApplicationAccessMode::TrustedEmail,
            _ => {
                return Err(
                    "APPLICATION_ACCESS_MODE must be one of: email_invite, trusted_email"
                        .to_string(),
                );
            }
        };

        let smtp = if matches!(application_access_mode, ApplicationAccessMode::EmailInvite) {
            let smtp_server = std::env::var("SMTP_HOST").map_err(|_| "SMTP_HOST not set")?;
            let smtp_port: u16 = std::env::var("SMTP_PORT")
                .map_err(|_| "SMTP_PORT not set")?
                .parse()
                .map_err(|_| "SMTP_PORT must be a valid port number")?;
            let smtp_security = match std::env::var("SMTP_SECURITY")
                .unwrap_or_else(|_| "starttls".to_string())
                .trim()
                .to_ascii_lowercase()
                .as_str()
            {
                "starttls" => SmtpSecurityMode::StartTls,
                "plaintext" => SmtpSecurityMode::Plaintext,
                _ => return Err("SMTP_SECURITY must be one of: starttls, plaintext".to_string()),
            };
            let smtp_auth = match std::env::var("SMTP_AUTH")
                .unwrap_or_else(|_| "required".to_string())
                .trim()
                .to_ascii_lowercase()
                .as_str()
            {
                "required" => SmtpAuthMode::Required,
                "none" => SmtpAuthMode::None,
                _ => return Err("SMTP_AUTH must be one of: required, none".to_string()),
            };
            let smtp_username = std::env::var("SMTP_USERNAME").ok().filter(|v| !v.is_empty());
            let smtp_password = std::env::var("SMTP_PASSWORD").ok().filter(|v| !v.is_empty());
            if matches!(smtp_auth, SmtpAuthMode::Required)
                && (smtp_username.is_none() || smtp_password.is_none())
            {
                return Err(
                    "SMTP_USERNAME and SMTP_PASSWORD must be set when SMTP_AUTH=required"
                        .to_string(),
                );
            }
            let smtp_from_email =
                std::env::var("SMTP_FROM_EMAIL").map_err(|_| "SMTP_FROM_EMAIL not set")?;
            let smtp_from_name =
                std::env::var("SMTP_FROM_NAME").unwrap_or_else(|_| "KSeF Pay".to_string());

            Some(SmtpEmailConfig {
                server: smtp_server,
                port: smtp_port,
                security: smtp_security,
                auth: smtp_auth,
                username: smtp_username,
                password: smtp_password,
                from_email: smtp_from_email,
                from_name: smtp_from_name,
            })
        } else {
            None
        };

        Ok(Self {
            database_url,
            server_host,
            server_port,
            app_base_url,
            smtp,
            ksef_environment,
            cert_storage_key,
            ksef_cert_pem,
            ksef_key_pem,
            allowed_emails,
            application_access_mode,
        })
    }
}
