use ksef_core::domain::environment::KSeFEnvironment;
use ksef_core::domain::nip::Nip;

pub struct Config {
    pub database_url: String,
    pub server_host: String,
    pub server_port: u16,
    pub ksef_environment: KSeFEnvironment,
    pub ksef_nip: Nip,
    pub ksef_cert_pem: Option<String>,
    pub ksef_key_pem: Option<String>,
    pub ksef_auth_method: String,
    pub ksef_auth_token: Option<String>,
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

        let ksef_nip = Nip::parse(
            &std::env::var("KSEF_NIP").map_err(|_| "KSEF_NIP not set (your company NIP)")?,
        )
        .map_err(|e| format!("invalid KSEF_NIP: {e}"))?;

        let ksef_cert_pem = std::env::var("KSEF_CERT_PEM").ok();
        let ksef_key_pem = std::env::var("KSEF_KEY_PEM").ok();
        let ksef_auth_method =
            std::env::var("KSEF_AUTH_METHOD").unwrap_or_else(|_| "xades".to_string());
        let ksef_auth_token = std::env::var("KSEF_AUTH_TOKEN").ok();

        Ok(Self {
            database_url,
            server_host,
            server_port,
            ksef_environment,
            ksef_nip,
            ksef_cert_pem,
            ksef_key_pem,
            ksef_auth_method,
            ksef_auth_token,
        })
    }
}
