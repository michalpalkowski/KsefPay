use chrono::{Duration, Utc};
use openssl::base64::encode_block;
use openssl::sha::sha256;
use uuid::Uuid;

const INVITE_TTL_DAYS: i64 = 7;

pub fn generate_invite_token() -> String {
    format!("{}.{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

pub fn hash_invite_token(raw_token: &str) -> String {
    encode_block(&sha256(raw_token.as_bytes()))
}

pub fn invite_expiration() -> chrono::DateTime<Utc> {
    Utc::now() + Duration::days(INVITE_TTL_DAYS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_tokens_are_url_safe() {
        let token = generate_invite_token();
        assert!(
            token
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '.')
        );
    }

    #[test]
    fn hash_is_deterministic() {
        let a = hash_invite_token("abc");
        let b = hash_invite_token("abc");
        assert_eq!(a, b);
    }
}
