use base64ct::{Base64UrlUnpadded, Encoding as _};
use rand::RngCore;
use sha2::{Digest, Sha256};

#[must_use]
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    format!("bbln_{}", Base64UrlUnpadded::encode_string(&bytes))
}

#[must_use]
pub fn hash_token(token: &str) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    h.finalize().to_vec()
}

#[must_use]
pub fn verify(token: &str, expected_hash: &[u8]) -> bool {
    let got = hash_token(token);
    got.len() == expected_hash.len()
        && got
            .iter()
            .zip(expected_hash)
            .fold(0u8, |a, (x, y)| a | (x ^ y))
            == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_token_has_prefix_and_stable_hash() {
        let t = generate_token();
        assert!(t.starts_with("bbln_"));
        assert!(t.len() > 40);
        assert_eq!(hash_token(&t), hash_token(&t));
        assert_ne!(hash_token(&t), hash_token(&generate_token()));
        assert!(verify(&t, &hash_token(&t)));
        assert!(!verify("bbln_wrong", &hash_token(&t)));
    }
}
