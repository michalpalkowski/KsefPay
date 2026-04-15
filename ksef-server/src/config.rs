use ksef_core::domain::environment::KSeFEnvironment;

pub struct Config {
    pub database_url: String,
    pub server_host: String,
    pub server_port: u16,
    pub ksef_environment: KSeFEnvironment,
    pub ksef_cert_pem: Option<String>,
    pub ksef_key_pem: Option<String>,
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

        Ok(Self {
            database_url,
            server_host,
            server_port,
            ksef_environment,
            ksef_cert_pem,
            ksef_key_pem,
        })
    }
}
