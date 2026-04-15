use axum::http::HeaderMap;

fn parse_first_ip(raw: &str) -> Option<String> {
    raw.split(',')
        .next()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned)
}

/// Best-effort client IP extraction for reverse-proxy deployments.
///
/// Priority:
/// 1. `x-forwarded-for` (first entry)
/// 2. `x-real-ip`
/// 3. `None`
#[must_use]
pub fn client_ip(headers: &HeaderMap) -> Option<String> {
    if let Some(ip) = headers
        .get("x-forwarded-for")
        .and_then(|h| h.to_str().ok())
        .and_then(parse_first_ip)
    {
        return Some(ip);
    }

    headers
        .get("x-real-ip")
        .and_then(|h| h.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn picks_first_x_forwarded_for_ip() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            HeaderValue::from_static("203.0.113.1, 10.0.0.1"),
        );

        assert_eq!(client_ip(&headers).as_deref(), Some("203.0.113.1"));
    }

    #[test]
    fn falls_back_to_x_real_ip() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", HeaderValue::from_static("198.51.100.2"));

        assert_eq!(client_ip(&headers).as_deref(), Some("198.51.100.2"));
    }
}
