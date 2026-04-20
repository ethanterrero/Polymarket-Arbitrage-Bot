use base64::{engine::general_purpose::URL_SAFE, Engine};
use chrono::Utc;
use hmac::{Hmac, Mac};
use k256::ecdsa::{RecoveryId, Signature, SigningKey};
use reqwest::header::{HeaderMap, HeaderValue};
use serde::Deserialize;
use sha2::Sha256;
use sha3::{Digest, Keccak256};

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("base64 decode error: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("HMAC key error")]
    HmacKey,
    #[error("hex decode error: {0}")]
    Hex(#[from] hex::FromHexError),
    #[error("signing key error: {0}")]
    Key(#[from] k256::ecdsa::Error),
    #[error("invalid header value")]
    Header,
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("API key derivation failed ({status}): {body}")]
    DeriveFailed { status: u16, body: String },
}

impl From<hmac::digest::InvalidLength> for AuthError {
    fn from(_: hmac::digest::InvalidLength) -> Self {
        AuthError::HmacKey
    }
}

impl From<reqwest::header::InvalidHeaderValue> for AuthError {
    fn from(_: reqwest::header::InvalidHeaderValue) -> Self {
        AuthError::Header
    }
}

/// Polymarket CLOB API credentials, derived from a private key.
#[derive(Debug, Clone)]
pub struct ApiCredentials {
    pub api_key: String,
    pub api_secret: String,
    pub api_passphrase: String,
    pub wallet_address: String,
}

#[derive(Debug, Deserialize)]
struct DeriveKeyResponse {
    #[serde(rename = "apiKey")]
    api_key: String,
    secret: String,
    passphrase: String,
}

/// Compute an HMAC-SHA256 request signature per the Polymarket CLOB L2 auth spec.
///
/// message  = timestamp + method + request_path + body
/// signature = base64url(HMAC-SHA256(base64url_decode(secret), message))
pub fn build_hmac_signature(
    secret: &str,
    timestamp: &str,
    method: &str,
    path: &str,
    body: &str,
) -> Result<String, AuthError> {
    let key_bytes = URL_SAFE.decode(secret)?;
    let mut mac = HmacSha256::new_from_slice(&key_bytes)?;
    mac.update(format!("{}{}{}{}", timestamp, method, path, body).as_bytes());
    Ok(URL_SAFE.encode(mac.finalize().into_bytes()))
}

/// Build the six `POLY_*` headers required by every authenticated CLOB request.
pub fn build_auth_headers(
    creds: &ApiCredentials,
    method: &str,
    path: &str,
    body: &str,
) -> Result<HeaderMap, AuthError> {
    let timestamp = Utc::now().timestamp().to_string();
    let sig = build_hmac_signature(&creds.api_secret, &timestamp, method, path, body)?;

    let mut h = HeaderMap::new();
    h.insert("POLY_ADDRESS", HeaderValue::from_str(&creds.wallet_address)?);
    h.insert("POLY_SIGNATURE", HeaderValue::from_str(&sig)?);
    h.insert("POLY_TIMESTAMP", HeaderValue::from_str(&timestamp)?);
    h.insert("POLY_NONCE", HeaderValue::from_static("0"));
    h.insert("POLY_API_KEY", HeaderValue::from_str(&creds.api_key)?);
    h.insert("POLY_PASSPHRASE", HeaderValue::from_str(&creds.api_passphrase)?);
    Ok(h)
}

/// Derive API credentials from a private key by calling `POST /auth/derive-api-key`
/// with an EIP-712 signed proof of wallet ownership.
pub async fn derive_api_key(
    client: &reqwest::Client,
    clob_url: &str,
    private_key_hex: &str,
    chain_id: u64,
) -> Result<ApiCredentials, AuthError> {
    let key_bytes = hex::decode(private_key_hex.trim_start_matches("0x"))?;
    let signing_key = SigningKey::from_bytes(key_bytes.as_slice().into())?;
    let wallet_address = address_from_signing_key(&signing_key);

    let timestamp = Utc::now().timestamp();
    let nonce: u64 = 0;
    let message = "This message attests that I control the given wallet";

    let sig_hex =
        eip712_sign_clob_auth(&signing_key, &wallet_address, &timestamp.to_string(), nonce, chain_id, message)?;

    let body = serde_json::json!({
        "address": wallet_address,
        "timestamp": timestamp,
        "nonce": nonce,
        "signature": sig_hex,
    });

    let resp = client
        .post(format!("{}/auth/derive-api-key", clob_url))
        .json(&body)
        .send()
        .await?;

    let status = resp.status().as_u16();
    if status >= 400 {
        let text = resp.text().await.unwrap_or_default();
        return Err(AuthError::DeriveFailed { status, body: text });
    }

    let data: DeriveKeyResponse = resp.json().await?;

    Ok(ApiCredentials {
        api_key: data.api_key,
        api_secret: data.secret,
        api_passphrase: data.passphrase,
        wallet_address,
    })
}

/// Derive an EIP-55 checksummed Ethereum address from a secp256k1 signing key.
fn address_from_signing_key(key: &SigningKey) -> String {
    let point = key.verifying_key().to_encoded_point(false);
    let pubkey_bytes = &point.as_bytes()[1..]; // drop 0x04 prefix
    let hash = Keccak256::digest(pubkey_bytes);
    eip55_checksum(&hash[12..]) // last 20 bytes
}

fn eip55_checksum(addr_bytes: &[u8]) -> String {
    let hex_addr = hex::encode(addr_bytes);
    let hash = hex::encode(Keccak256::digest(hex_addr.as_bytes()));
    let checksummed: String = hex_addr
        .chars()
        .zip(hash.chars())
        .map(|(c, h)| {
            if c.is_ascii_alphabetic() && h.to_digit(16).unwrap_or(0) >= 8 {
                c.to_ascii_uppercase()
            } else {
                c
            }
        })
        .collect();
    format!("0x{}", checksummed)
}

/// EIP-712 sign a `ClobAuth` structured message.
///
/// Domain:  `{ name: "ClobAuthDomain", version: "1", chainId }`
/// Type:    `ClobAuth(address address,string timestamp,uint256 nonce,string message)`
fn eip712_sign_clob_auth(
    key: &SigningKey,
    address: &str,
    timestamp: &str,
    nonce: u64,
    chain_id: u64,
    message: &str,
) -> Result<String, AuthError> {
    // Type hashes.
    let domain_typehash =
        keccak256(b"EIP712Domain(string name,string version,uint256 chainId)");
    let clob_auth_typehash = keccak256(
        b"ClobAuth(address address,string timestamp,uint256 nonce,string message)",
    );

    // Domain separator: abi.encode(domainTypeHash, hash("ClobAuthDomain"), hash("1"), chainId)
    let mut domain_enc = [0u8; 128];
    domain_enc[0..32].copy_from_slice(&domain_typehash);
    domain_enc[32..64].copy_from_slice(&keccak256(b"ClobAuthDomain"));
    domain_enc[64..96].copy_from_slice(&keccak256(b"1"));
    domain_enc[96..128].copy_from_slice(&u256_be(chain_id));
    let domain_separator = keccak256(&domain_enc);

    // Address: 20 bytes left-padded to 32.
    let addr_bytes = hex::decode(address.trim_start_matches("0x"))?;
    let mut addr_padded = [0u8; 32];
    addr_padded[12..32].copy_from_slice(&addr_bytes);

    // Struct hash: abi.encode(typeHash, address, hash(timestamp), nonce, hash(message))
    let mut struct_enc = [0u8; 160];
    struct_enc[0..32].copy_from_slice(&clob_auth_typehash);
    struct_enc[32..64].copy_from_slice(&addr_padded);
    struct_enc[64..96].copy_from_slice(&keccak256(timestamp.as_bytes()));
    struct_enc[96..128].copy_from_slice(&u256_be(nonce));
    struct_enc[128..160].copy_from_slice(&keccak256(message.as_bytes()));
    let struct_hash = keccak256(&struct_enc);

    // Final EIP-712 digest: keccak256("\x19\x01" || domainSeparator || structHash)
    let mut to_sign = [0u8; 66];
    to_sign[0] = 0x19;
    to_sign[1] = 0x01;
    to_sign[2..34].copy_from_slice(&domain_separator);
    to_sign[34..66].copy_from_slice(&struct_hash);
    let digest = keccak256(&to_sign);

    // Sign and encode as 0x-prefixed hex: r (32) || s (32) || v (1), v = 27 or 28.
    let (sig, recid): (Signature, RecoveryId) = key.sign_prehash_recoverable(&digest)?;
    let sig_bytes: [u8; 64] = sig.to_bytes().into();
    let v = 27u8 + recid.to_byte();

    let mut full_sig = [0u8; 65];
    full_sig[0..64].copy_from_slice(&sig_bytes);
    full_sig[64] = v;

    Ok(format!("0x{}", hex::encode(full_sig)))
}

fn keccak256(data: &[u8]) -> [u8; 32] {
    Keccak256::digest(data).into()
}

fn u256_be(n: u64) -> [u8; 32] {
    let mut buf = [0u8; 32];
    buf[24..32].copy_from_slice(&n.to_be_bytes());
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hmac_signature_deterministic() {
        // Known-good: same inputs always produce same output.
        let secret = URL_SAFE.encode(b"test-secret-key-32-bytes-padding");
        let sig1 = build_hmac_signature(&secret, "1700000000", "POST", "/order", "{}").unwrap();
        let sig2 = build_hmac_signature(&secret, "1700000000", "POST", "/order", "{}").unwrap();
        assert_eq!(sig1, sig2);
        assert!(!sig1.is_empty());
    }

    #[test]
    fn test_address_derivation_known_key() {
        // Private key 0xac0974...  is the well-known Hardhat test key #0.
        // Expected address: 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
        let key_bytes =
            hex::decode("ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80")
                .unwrap();
        let signing_key = SigningKey::from_bytes(key_bytes.as_slice().into()).unwrap();
        let addr = address_from_signing_key(&signing_key);
        assert_eq!(addr, "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
    }

    #[test]
    fn test_eip712_sign_produces_65_byte_hex_sig() {
        let key_bytes =
            hex::decode("ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80")
                .unwrap();
        let signing_key = SigningKey::from_bytes(key_bytes.as_slice().into()).unwrap();
        let addr = address_from_signing_key(&signing_key);
        let sig = eip712_sign_clob_auth(
            &signing_key,
            &addr,
            "1700000000",
            0,
            137,
            "This message attests that I control the given wallet",
        )
        .unwrap();
        // 0x + 65 bytes = "0x" + 130 hex chars
        assert_eq!(sig.len(), 132);
        assert!(sig.starts_with("0x"));
        // v must be 27 or 28
        let v = u8::from_str_radix(&sig[sig.len() - 2..], 16).unwrap();
        assert!(v == 27 || v == 28);
    }
}
