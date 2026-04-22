use ksef_core::domain::environment::KSeFEnvironment;

pub struct Config {
    pub database_url: String,
    pub server_host: String,
    pub server_port: u16,
    pub ksef_environment: KSeFEnvironment,
    pub cert_storage_key: Option<String>,
    pub ksef_cert_pem: Option<String>,
    pub ksef_key_pem: Option<String>,
    /// Allowlist of emails permitted to register. Empty = registration closed.
    pub allowed_emails: Vec<String>,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let database_url = std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL not set")?;

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

        Ok(Self {
            database_url,
            server_host,
            server_port,
            ksef_environment,
            cert_storage_key,
            ksef_cert_pem,
            ksef_key_pem,
            allowed_emails,
        })
    }
}
