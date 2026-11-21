use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

pub struct TokenData {
    pub token: String,
    pub session_id: String,
    pub created_at: u64,
}

pub struct TokenClaims {
    pub user_id: String,
    pub session_id: String,
}

pub fn sign(user_id: &str, secret: &str) -> TokenData {
    let payload64 = URL_SAFE_NO_PAD.encode(user_id);
    let session_id = URL_SAFE_NO_PAD.encode(Uuid::new_v4().as_bytes());

    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("System clock is before UNIX_EPOCH")
        .as_millis() as u64;

    let msg = format!("{payload64}.{session_id}");

    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts keys of any size");
    mac.update(msg.as_bytes());
    let signature64 = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());

    TokenData {
        token: format!("{msg}.{signature64}"),
        session_id,
        created_at,
    }
}

pub fn strip_auth_prefix(raw: &str) -> &str {
    let trimmed = raw.trim();
    trimmed
        .strip_prefix("Bearer ")
        .or_else(|| trimmed.strip_prefix("Bot "))
        .unwrap_or(trimmed)
        .trim()
}

pub fn verify(token: &str, secret: &str) -> Result<TokenClaims, String> {
    let parts: Vec<&str> = token.trim().split('.').collect();
    if parts.len() != 3 {
        return Err("Malformed token".to_string());
    }

    let [payload64, session_id, signature64] = [parts[0], parts[1], parts[2]];

    if session_id.is_empty() || session_id.len() > 36 {
        return Err("Invalid session segment".to_string());
    }

    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts keys of any size");
    mac.update(format!("{payload64}.{session_id}").as_bytes());

    let sig_bytes = URL_SAFE_NO_PAD
        .decode(signature64)
        .map_err(|_| "Invalid signature encoding".to_string())?;

    mac.verify_slice(&sig_bytes)
        .map_err(|_| "Invalid token signature".to_string())?;

    let user_id_bytes = URL_SAFE_NO_PAD
        .decode(payload64)
        .map_err(|_| "Invalid token payload encoding".to_string())?;
    let user_id =
        String::from_utf8(user_id_bytes).map_err(|_| "Token payload is not valid UTF-8".to_string())?;

    Ok(TokenClaims {
        user_id,
        session_id: session_id.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify_roundtrip() {
        let secret = "super_secret_test_key";
        let data = sign("1234567890", secret);
        let claims = verify(&data.token, secret).expect("token should verify");
        assert_eq!(claims.user_id, "1234567890");
        assert_eq!(claims.session_id, data.session_id);
    }

    #[test]
    fn reject_tampered_or_wrong_secret() {
        let data = sign("42", "secret_a");
        assert!(verify(&data.token, "secret_b").is_err());
        assert!(verify(&format!("{}x", data.token), "secret_a").is_err());
        assert!(verify("not.enough", "secret_a").is_err());
    }

    #[test]
    fn strip_auth_prefix_handles_bearer_bot_and_whitespace() {
        assert_eq!(strip_auth_prefix("  Bearer abc.def.ghi  "), "abc.def.ghi");
        assert_eq!(strip_auth_prefix("Bot  tok123 "), "tok123");
        assert_eq!(strip_auth_prefix("raw_token"), "raw_token");
    }
}

