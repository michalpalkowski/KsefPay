use std::sync::Arc;
use std::time::Duration;
use std::{future::IntoFuture, pin::pin};

use tokio::net::TcpListener;
use tokio::sync::watch;
use tower_http::services::ServeDir;
use tower_sessions::Expiry;
use tower_sessions::SessionManagerLayer;
use tower_sessions::cookie::SameSite;
use tower_sessions::cookie::time::Duration as CookieDuration;
use tracing_subscriber::EnvFilter;

use ksef_core::infra::batch::zip_builder::BatchFileBuilder;
use ksef_core::infra::crypto::{AesCbcEncryptor, OpenSslSignerFactory, OpenSslXadesSigner};
use ksef_core::infra::fa3::Fa3XmlConverter;
use ksef_core::infra::http::rate_limiter::TokenBucketRateLimiter;
use ksef_core::infra::http::retry::RetryPolicy;
use ksef_core::infra::ksef::KSeFApiClient;
use ksef_core::infra::qr::generator::QRCodeGenerator;
use ksef_core::services::audit_service::AuditService;
use ksef_core::services::batch_service::BatchService;
use ksef_core::services::export_service::ExportService;
use ksef_core::services::fetch_service::FetchService;
use ksef_core::services::invoice_service::InvoiceService;
use ksef_core::services::offline_service::{OfflineConfig, OfflineService};
use ksef_core::services::permission_service::PermissionService;
use ksef_core::services::qr_service::QRService;
use ksef_core::services::session_service::{AuthMethod, SessionService};
use ksef_core::services::token_mgmt_service::TokenMgmtService;
use ksef_core::workers::job_worker::JobWorker;

mod audit_log;
mod auth_rate_limit;
mod config;
mod csrf;
mod db_backend;
mod extractors;
mod request_meta;
mod routes;
mod state;

use state::AppState;

fn read_pem_env(var: &str, raw: &str) -> anyhow::Result<Vec<u8>> {
    let path = std::path::Path::new(raw);
    if !raw.contains("-----BEGIN") && path.exists() {
        return std::fs::read(path)
            .map_err(|e| anyhow::anyhow!("{var} points to unreadable file '{raw}': {e}"));
    }

    let normalized = raw.replace("\\n", "\n");
    if !normalized.contains("-----BEGIN") {
        return Err(anyhow::anyhow!(
            "{var} must contain PEM content or a path to a PEM file"
        ));
    }

    Ok(normalized.into_bytes())
}

fn load_fallback_signer(config: &config::Config) -> anyhow::Result<OpenSslXadesSigner> {
    // If explicit cert provided in env, use it as fallback signer.
    // Otherwise generate a dummy — the SignerFactory handles per-NIP signing.
    match (&config.ksef_cert_pem, &config.ksef_key_pem) {
        (Some(cert_raw), Some(key_raw)) => {
            let cert_pem = read_pem_env("KSEF_CERT_PEM", cert_raw)?;
            let key_pem = read_pem_env("KSEF_KEY_PEM", key_raw)?;
            tracing::info!("using provided KSeF certificate as fallback signer");
            Ok(OpenSslXadesSigner::from_pem(key_pem, cert_pem))
        }
        _ => {
            // Generate a placeholder cert — SignerFactory creates per-NIP signers at runtime.
            let placeholder_nip =
                ksef_core::domain::nip::Nip::parse("5260250274").expect("well-known NIP is valid");
            OpenSslXadesSigner::generate_self_signed_for_nip(&placeholder_nip)
                .map_err(|e| anyhow::anyhow!("fallback signer generation failed: {e}"))
        }
    }
}

#[tokio::main]
#[allow(clippy::too_many_lines)]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,ksef_core=debug,ksef_server=debug".into()),
        )
        .init();

    let config = config::Config::from_env().map_err(|e| anyhow::anyhow!(e))?;

    tracing::info!(environment = %config.ksef_environment, "initializing database");
    let db = db_backend::connect(&config.database_url).await?;
    tracing::info!(backend = ?db.kind, "database backend ready");

    let ksef = Arc::new(KSeFApiClient::with_http_controls(
        config.ksef_environment,
        Arc::new(TokenBucketRateLimiter::default()),
        RetryPolicy::default(),
    ));
    let fallback_signer = Arc::new(load_fallback_signer(&config)?);
    let signer_factory = Arc::new(OpenSslSignerFactory);
    // Multi-tenant: auth method is always xades (per-NIP cert from SignerFactory).
    // Token auth from env is no longer supported — tokens are managed per-NIP account.
    let auth_method = AuthMethod::Xades;
    let session_service = Arc::new(SessionService::with_signer_factory(
        ksef.clone(),
        fallback_signer,
        signer_factory,
        db.nip_account_repo.clone(),
        ksef.clone(),
        db.session_repo.clone(),
        config.ksef_environment,
        auth_method,
    ));
    let invoice_service = Arc::new(InvoiceService::with_atomic(
        db.invoice_repo.clone(),
        db.job_queue.clone(),
        db.atomic_scope_factory.clone(),
    ));
    let encryptor = Arc::new(AesCbcEncryptor);
    let decryptor = Arc::new(AesCbcEncryptor);
    let xml_converter = Arc::new(Fa3XmlConverter);
    let qr_renderer = Arc::new(QRCodeGenerator);

    let fetch_service = Arc::new(FetchService::new(
        session_service.clone(),
        ksef.clone(),
        db.invoice_repo.clone(),
        xml_converter.clone(),
    ));

    let whitelist_client = Arc::new(ksef_core::infra::whitelist::WhiteListClient::new());
    let company_lookup_service = Arc::new(
        ksef_core::services::company_lookup_service::CompanyLookupService::new(
            db.company_cache.clone(),
            whitelist_client,
        ),
    );

    let permission_service = Arc::new(PermissionService::new(ksef.clone()));
    let token_mgmt_service = Arc::new(TokenMgmtService::new(ksef.clone()));
    let export_service = Arc::new(ExportService::new(ksef.clone(), decryptor));
    let audit_service = Arc::new(AuditService::new(db.audit_log_repo.clone()));
    let batch_service = Arc::new(BatchService::new(
        ksef.clone(),
        Arc::new(BatchFileBuilder::default()),
    ));

    let qr_service = Arc::new(QRService::new(config.ksef_environment, qr_renderer.clone()));

    let offline_service = Arc::new(OfflineService::new(
        QRService::new(config.ksef_environment, qr_renderer),
        OfflineConfig::default(),
    ));

    // --- Job worker ---

    let job_worker = Arc::new(JobWorker::new(
        db.job_queue.clone(),
        invoice_service.clone(),
        session_service.clone(),
        ksef.clone(),
        encryptor,
        xml_converter,
        Duration::from_secs(2),
    ));

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let worker = job_worker.clone();
    let mut worker_handle = tokio::spawn(async move { worker.run(shutdown_rx).await });

    let app_state = AppState {
        ksef_environment: config.ksef_environment,
        user_repo: db.user_repo.clone(),
        nip_account_repo: db.nip_account_repo.clone(),
        export_keys: std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        fetch_jobs: std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        auth_rate_limiter: auth_rate_limit::AuthRateLimiter::default(),
        company_lookup_service,
        invoice_sequence: db.invoice_sequence.clone(),
        invoice_service,
        fetch_service,
        session_service,
        batch_service,
        permission_service,
        token_mgmt_service,
        local_token_repo: db.local_token_repo.clone(),
        export_service,
        offline_service,
        qr_service,
        audit_service,
    };

    // Session store — type differs per backend so we build the final app in each branch.
    // Axum 0.8's Router::layer erases the layer type, so both branches produce Router<()>.
    let base_router = routes::router()
        .nest_service(
            "/assets",
            ServeDir::new(
                std::env::var("ASSETS_DIR")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|_| {
                        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets")
                    }),
            ),
        )
        .with_state(app_state);

    let app: axum::Router = match db.kind {
        db_backend::DatabaseBackendKind::Sqlite => {
            let pool = sqlx::SqlitePool::connect(&config.database_url).await?;
            let session_store = tower_sessions_sqlx_store::SqliteStore::new(pool);
            session_store.migrate().await?;
            let session_layer = SessionManagerLayer::new(session_store)
                .with_expiry(Expiry::OnInactivity(CookieDuration::days(7)))
                .with_http_only(true)
                .with_secure(true)
                .with_same_site(SameSite::Strict)
                .with_path("/");
            base_router.layer(session_layer)
        }
        db_backend::DatabaseBackendKind::Postgres => {
            let pool = sqlx::PgPool::connect(&config.database_url).await?;
            let session_store = tower_sessions_sqlx_store::PostgresStore::new(pool);
            session_store.migrate().await?;
            let session_layer = SessionManagerLayer::new(session_store)
                .with_expiry(Expiry::OnInactivity(CookieDuration::days(7)))
                .with_http_only(true)
                .with_secure(true)
                .with_same_site(SameSite::Strict)
                .with_path("/");
            base_router.layer(session_layer)
        }
    };

    let addr = format!("{}:{}", config.server_host, config.server_port);
    let listener = TcpListener::bind(&addr).await?;
    tracing::info!("listening on {addr}");
    let mut shutdown_rx_for_server = shutdown_tx.subscribe();
    let shutdown_tx_for_signal = shutdown_tx.clone();
    let mut server = pin!(
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                tokio::select! {
                    signal = tokio::signal::ctrl_c() => {
                        if signal.is_ok() {
                            tracing::info!("shutdown signal received");
                        }
                    }
                    changed = shutdown_rx_for_server.changed() => {
                        if changed.is_ok() && *shutdown_rx_for_server.borrow() {
                            tracing::info!("application shutdown requested");
                        }
                    }
                }
                if let Err(err) = shutdown_tx_for_signal.send(true) {
                    tracing::debug!("shutdown signal already broadcast: {err}");
                }
            })
            .into_future()
    );

    let run_result: anyhow::Result<()> = tokio::select! {
        serve_result = &mut server => {
            if let Err(err) = shutdown_tx.send(true) {
                tracing::debug!("worker shutdown channel already closed: {err}");
            }

            let worker_result = worker_handle
                .await
                .map_err(|err| anyhow::anyhow!("worker task join failed: {err}"))?;
            worker_result.map_err(|err| anyhow::anyhow!("worker exited with error: {err}"))?;
            serve_result.map_err(|err| anyhow::anyhow!("server exited with error: {err}"))?;
            Ok(())
        }
        worker_join = &mut worker_handle => {
            // Worker finished first. This is normal during shutdown (worker reacts
            // to the watch channel faster than the server drains connections).
            // Only treat it as an error if shutdown was NOT requested.
            let shutdown_requested = *shutdown_tx.borrow();

            if let Err(err) = shutdown_tx.send(true) {
                tracing::debug!("worker shutdown channel already closed: {err}");
            }

            let worker_result = worker_join
                .map_err(|err| anyhow::anyhow!("worker task join failed: {err}"))?;
            worker_result.map_err(|err| anyhow::anyhow!("worker exited with error: {err}"))?;

            let serve_result = (&mut server).await;
            serve_result.map_err(|err| anyhow::anyhow!("server exited with error: {err}"))?;

            if shutdown_requested {
                tracing::info!("clean shutdown complete");
                Ok(())
            } else {
                Err(anyhow::anyhow!("worker exited unexpectedly before server shutdown"))
            }
        }
    };

    run_result?;

    Ok(())
}
