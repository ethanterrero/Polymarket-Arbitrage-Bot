//! CREATE2 derivation of Polymarket proxy and Gnosis Safe addresses.
//!
//! For a Polymarket order, the `maker` field is the user's funded account, which
//! for most users is *not* their EOA. Two flavors are deployed via CREATE2 from
//! Polymarket-controlled factories on Polygon:
//!
//! - `PolyProxy` (Magic/email account)  — factory `0xaB45...4052`
//! - `PolyGnosisSafe` (browser wallet)  — factory `0xaacF...541b`
//!
//! Reference: `Polymarket/proxy-factories` (Solidity), `polymarket-client-sdk` v0.4.4.
//!
//! NOTE: For Magic-Link users the on-chain proxy can occasionally diverge from
//! the CREATE2 prediction (Polymarket's backend may map a user to a different
//! proxy). The robust live path fetches the funded address from `/profiles` and
//! treats this derivation as a verification/fallback, never as authoritative.
//! See <https://github.com/Polymarket/polymarket-cli/issues/14>.

use primitive_types::H160;
use sha3::{Digest, Keccak256};

use crate::order_signing::SignatureType;

/// Polymarket Proxy Wallet Factory on Polygon mainnet.
pub const POLYGON_PROXY_FACTORY: &str = "0xaB45c5A4B0c941a2F231C04C3f49182e1A254052";
/// Polymarket Safe Proxy Factory on Polygon mainnet.
pub const POLYGON_SAFE_FACTORY: &str = "0xaacFeEa03eb1561C4e67d661e40682Bd20E3541b";

/// keccak256 of the proxy clone init code (EIP-1167 minimal clone of the
/// implementation, with `cloneConstructor(bytes)` constructor data appended).
pub const PROXY_INIT_CODE_HASH: [u8; 32] = [
    0xd2, 0x1d, 0xf8, 0xdc, 0x65, 0x88, 0x0a, 0x86, 0x06, 0xf0, 0x9f, 0xe0, 0xce, 0x3d, 0xf9, 0xb8,
    0x86, 0x92, 0x87, 0xab, 0x0b, 0x05, 0x8b, 0xe0, 0x5a, 0xa9, 0xe8, 0xaf, 0x63, 0x30, 0xa0, 0x0b,
];

/// keccak256 of the Gnosis Safe proxy creation code with master copy appended.
pub const SAFE_INIT_CODE_HASH: [u8; 32] = [
    0x2b, 0xce, 0x21, 0x27, 0xff, 0x07, 0xfb, 0x63, 0x2d, 0x16, 0xc8, 0x34, 0x7c, 0x4e, 0xbf, 0x50,
    0x1f, 0x48, 0x41, 0x16, 0x8b, 0xed, 0x00, 0xd9, 0xe6, 0xef, 0x71, 0x5d, 0xdb, 0x6f, 0xce, 0xcf,
];

#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    #[error("invalid hex address: {0}")]
    InvalidHex(String),
    #[error("unsupported signature type for proxy derivation: {0:?}")]
    UnsupportedSignatureType(SignatureType),
}

fn keccak256(data: &[u8]) -> [u8; 32] {
    Keccak256::digest(data).into()
}

fn parse_address(hex_str: &str) -> Result<H160, ProxyError> {
    let s = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    let bytes = hex::decode(s).map_err(|_| ProxyError::InvalidHex(hex_str.to_string()))?;
    if bytes.len() != 20 {
        return Err(ProxyError::InvalidHex(hex_str.to_string()));
    }
    Ok(H160::from_slice(&bytes))
}

/// Standard CREATE2 address: `keccak256(0xff || factory || salt || initCodeHash)[12..32]`.
fn create2(factory: H160, salt: [u8; 32], init_code_hash: [u8; 32]) -> H160 {
    let mut buf = [0u8; 1 + 20 + 32 + 32];
    buf[0] = 0xff;
    buf[1..21].copy_from_slice(factory.as_bytes());
    buf[21..53].copy_from_slice(&salt);
    buf[53..85].copy_from_slice(&init_code_hash);
    let h = keccak256(&buf);
    H160::from_slice(&h[12..32])
}

/// Derive the Polymarket Proxy Wallet (Magic/email account) for an EOA.
///
/// Salt = `keccak256(packed_20_byte_address)` (Solidity `abi.encodePacked`).
pub fn derive_poly_proxy(eoa: H160) -> H160 {
    let factory = parse_address(POLYGON_PROXY_FACTORY).expect("static factory address");
    let salt = keccak256(eoa.as_bytes());
    create2(factory, salt, PROXY_INIT_CODE_HASH)
}

/// Derive the Polymarket Gnosis Safe (browser wallet) for an EOA.
///
/// Salt = `keccak256(abi.encode(eoa))` — the address left-padded to 32 bytes.
pub fn derive_poly_safe(eoa: H160) -> H160 {
    let factory = parse_address(POLYGON_SAFE_FACTORY).expect("static factory address");
    let mut padded = [0u8; 32];
    padded[12..32].copy_from_slice(eoa.as_bytes());
    let salt = keccak256(&padded);
    create2(factory, salt, SAFE_INIT_CODE_HASH)
}

/// Derive the funded `maker` address for an EOA given a signature type.
///
/// Returns `eoa` for `SignatureType::Eoa`. For the proxy variants this is a
/// pure CREATE2 prediction — see the module-level note about Magic-Link
/// divergence before treating the result as authoritative.
pub fn derive_maker_address(eoa: H160, signature_type: SignatureType) -> H160 {
    match signature_type {
        SignatureType::Eoa => eoa,
        SignatureType::PolyProxy => derive_poly_proxy(eoa),
        SignatureType::PolyGnosisSafe => derive_poly_safe(eoa),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Anvil/Foundry test account #0 — the canonical reference EOA used by
    /// `polymarket-client-sdk` tests.
    const REFERENCE_EOA: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";

    /// Expected PolyProxy derivation for the reference EOA on Polygon mainnet.
    /// Pinned from `polymarket-client-sdk` v0.4.4.
    const REFERENCE_POLY_PROXY: &str = "0x365f0cA36ae1F641E02Fe3b7743673DA42A13a70";

    /// Expected PolyGnosisSafe derivation for the reference EOA on Polygon mainnet.
    /// Pinned from `polymarket-client-sdk` v0.4.4.
    const REFERENCE_POLY_SAFE: &str = "0xd93b25Cb943D14d0d34FBAf01fc93a0F8b5f6e47";

    fn addr(s: &str) -> H160 {
        parse_address(s).unwrap()
    }

    #[test]
    fn poly_proxy_reference_vector() {
        let derived = derive_poly_proxy(addr(REFERENCE_EOA));
        assert_eq!(derived, addr(REFERENCE_POLY_PROXY));
    }

    #[test]
    fn poly_safe_reference_vector() {
        let derived = derive_poly_safe(addr(REFERENCE_EOA));
        assert_eq!(derived, addr(REFERENCE_POLY_SAFE));
    }

    #[test]
    fn derive_maker_address_dispatches_by_type() {
        let eoa = addr(REFERENCE_EOA);
        assert_eq!(derive_maker_address(eoa, SignatureType::Eoa), eoa);
        assert_eq!(
            derive_maker_address(eoa, SignatureType::PolyProxy),
            addr(REFERENCE_POLY_PROXY),
        );
        assert_eq!(
            derive_maker_address(eoa, SignatureType::PolyGnosisSafe),
            addr(REFERENCE_POLY_SAFE),
        );
    }

    #[test]
    fn proxy_and_safe_differ() {
        // Sanity: the two derivations must not collide for the same EOA — the
        // salt encoding (packed vs padded) and the init code hashes differ.
        let eoa = addr(REFERENCE_EOA);
        assert_ne!(derive_poly_proxy(eoa), derive_poly_safe(eoa));
    }
}
