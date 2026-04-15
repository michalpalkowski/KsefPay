use tower_sessions::Session;

/// Session key used to store the CSRF token.
pub const CSRF_SESSION_KEY: &str = "csrf_token";

/// Ensure a CSRF token exists in session and return it.
pub async fn ensure_csrf_token(session: &Session) -> Result<String, String> {
    if let Some(existing) = session
        .get::<String>(CSRF_SESSION_KEY)
        .await
        .map_err(|e| format!("session read error: {e}"))?
    {
        return Ok(existing);
    }

    let token = uuid::Uuid::new_v4().to_string();
    session
        .insert(CSRF_SESSION_KEY, token.clone())
        .await
        .map_err(|e| format!("session write error: {e}"))?;
    Ok(token)
}
