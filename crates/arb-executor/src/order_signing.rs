//! EIP-712 signing for Polymarket CTF Exchange orders.
//!
//! Reference: Polymarket/ctf-exchange `OrderStructs.sol`, `Hashing.sol`.
//! Cross-checked against Polymarket/python-order-utils test vectors.

use k256::ecdsa::{RecoveryId, Signature, SigningKey};
use primitive_types::{H160, U256};
use sha3::{Digest, Keccak256};

use crate::auth::AuthError;

/// Polymarket CTF Exchange contract addresses by chain and exchange flavor.
pub mod contracts {
    pub const POLYGON_CHAIN_ID: u64 = 137;
    pub const AMOY_CHAIN_ID: u64 = 80002;

    pub const POLYGON_CTF_EXCHANGE: &str = "0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E";
    pub const POLYGON_NEG_RISK_EXCHANGE: &str = "0xC5d563A36AE78145C45a50134d48A1215220f80a";

    pub const AMOY_CTF_EXCHANGE: &str = "0xdFE02Eb6733538f8Ea35D585af8DE5958AD99E40";
    pub const AMOY_NEG_RISK_EXCHANGE: &str = "0xd91E80cF2E7be2e162c6513ceD06f1dD0dA35296";

    /// Choose the verifying contract for an order based on chain and market flavor.
    pub fn verifying_contract(chain_id: u64, neg_risk: bool) -> Option<&'static str> {
        match (chain_id, neg_risk) {
            (POLYGON_CHAIN_ID, false) => Some(POLYGON_CTF_EXCHANGE),
            (POLYGON_CHAIN_ID, true) => Some(POLYGON_NEG_RISK_EXCHANGE),
            (AMOY_CHAIN_ID, false) => Some(AMOY_CTF_EXCHANGE),
            (AMOY_CHAIN_ID, true) => Some(AMOY_NEG_RISK_EXCHANGE),
            _ => None,
        }
    }
}

/// EIP-712 domain name — identical for CTF Exchange and Neg Risk CTF Exchange.
const DOMAIN_NAME: &str = "Polymarket CTF Exchange";
const DOMAIN_VERSION: &str = "1";

/// `Order(uint256 salt,address maker,address signer,address taker,uint256 tokenId,uint256 makerAmount,uint256 takerAmount,uint256 expiration,uint256 nonce,uint256 feeRateBps,uint8 side,uint8 signatureType)`
const ORDER_TYPE_STRING: &[u8] = b"Order(uint256 salt,address maker,address signer,address taker,uint256 tokenId,uint256 makerAmount,uint256 takerAmount,uint256 expiration,uint256 nonce,uint256 feeRateBps,uint8 side,uint8 signatureType)";

/// `EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)`
const DOMAIN_TYPE_STRING: &[u8] =
    b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)";

/// Side of an order as encoded in the EIP-712 struct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderSide {
    Buy = 0,
    Sell = 1,
}

/// Signature type — how the `maker` account relates to the signing key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureType {
    /// EOA signs; `maker` == `signer`.
    Eoa = 0,
    /// Magic/email user proxy; `maker` = CREATE2-derived proxy, `signer` = EOA.
    PolyProxy = 1,
    /// Browser wallet Gnosis Safe; `maker` = safe address, `signer` = EOA.
    PolyGnosisSafe = 2,
}

/// A CTF Exchange order in the exact shape hashed by EIP-712.
///
/// All `U256` fields correspond to Solidity `uint256`. `side` and `signatureType`
/// are `uint8` in the typehash, widened to `U256` for uniform encoding.
#[derive(Debug, Clone)]
pub struct Order {
    pub salt: U256,
    pub maker: H160,
    pub signer: H160,
    pub taker: H160,
    pub token_id: U256,
    pub maker_amount: U256,
    pub taker_amount: U256,
    pub expiration: U256,
    pub nonce: U256,
    pub fee_rate_bps: U256,
    pub side: OrderSide,
    pub signature_type: SignatureType,
}

fn keccak256(data: &[u8]) -> [u8; 32] {
    Keccak256::digest(data).into()
}

fn encode_u256(v: U256, out: &mut [u8]) {
    debug_assert_eq!(out.len(), 32);
    v.to_big_endian(out);
}

fn encode_address(a: H160, out: &mut [u8]) {
    debug_assert_eq!(out.len(), 32);
    out[..12].fill(0);
    out[12..].copy_from_slice(a.as_bytes());
}

/// Compute the EIP-712 struct hash: `keccak256(abi.encode(ORDER_TYPEHASH, ...fields))`.
pub fn order_struct_hash(order: &Order) -> [u8; 32] {
    let order_typehash = keccak256(ORDER_TYPE_STRING);

    // 13 fields * 32 bytes = 416 bytes.
    let mut buf = [0u8; 32 * 13];
    buf[0..32].copy_from_slice(&order_typehash);
    encode_u256(order.salt, &mut buf[32..64]);
    encode_address(order.maker, &mut buf[64..96]);
    encode_address(order.signer, &mut buf[96..128]);
    encode_address(order.taker, &mut buf[128..160]);
    encode_u256(order.token_id, &mut buf[160..192]);
    encode_u256(order.maker_amount, &mut buf[192..224]);
    encode_u256(order.taker_amount, &mut buf[224..256]);
    encode_u256(order.expiration, &mut buf[256..288]);
    encode_u256(order.nonce, &mut buf[288..320]);
    encode_u256(order.fee_rate_bps, &mut buf[320..352]);
    encode_u256(U256::from(order.side as u8), &mut buf[352..384]);
    encode_u256(U256::from(order.signature_type as u8), &mut buf[384..416]);

    keccak256(&buf)
}

/// Compute the EIP-712 domain separator for the given chain and verifying contract.
pub fn domain_separator(chain_id: u64, verifying_contract: H160) -> [u8; 32] {
    let domain_typehash = keccak256(DOMAIN_TYPE_STRING);
    let name_hash = keccak256(DOMAIN_NAME.as_bytes());
    let version_hash = keccak256(DOMAIN_VERSION.as_bytes());

    let mut buf = [0u8; 32 * 5];
    buf[0..32].copy_from_slice(&domain_typehash);
    buf[32..64].copy_from_slice(&name_hash);
    buf[64..96].copy_from_slice(&version_hash);
    encode_u256(U256::from(chain_id), &mut buf[96..128]);
    encode_address(verifying_contract, &mut buf[128..160]);

    keccak256(&buf)
}

/// Full EIP-712 digest: `keccak256("\x19\x01" || domainSeparator || structHash)`.
/// This is the 32-byte value the ECDSA signing operation consumes.
pub fn order_digest(order: &Order, chain_id: u64, verifying_contract: H160) -> [u8; 32] {
    let sep = domain_separator(chain_id, verifying_contract);
    let sh = order_struct_hash(order);

    let mut buf = [0u8; 66];
    buf[0] = 0x19;
    buf[1] = 0x01;
    buf[2..34].copy_from_slice(&sep);
    buf[34..66].copy_from_slice(&sh);
    keccak256(&buf)
}

/// Sign an order. Returns the 65-byte `r || s || v` signature, where `v ∈ {27, 28}`.
pub fn sign_order(
    key: &SigningKey,
    order: &Order,
    chain_id: u64,
    verifying_contract: H160,
) -> Result<[u8; 65], AuthError> {
    let digest = order_digest(order, chain_id, verifying_contract);
    let (sig, recid): (Signature, RecoveryId) = key.sign_prehash_recoverable(&digest)?;
    let sig_bytes: [u8; 64] = sig.to_bytes().into();
    let v = 27u8 + recid.to_byte();

    let mut out = [0u8; 65];
    out[0..64].copy_from_slice(&sig_bytes);
    out[64] = v;
    Ok(out)
}

/// Parse a `0x…` hex-encoded Ethereum address into `H160`.
pub fn parse_address(s: &str) -> Result<H160, AuthError> {
    let bytes = hex::decode(s.trim_start_matches("0x"))?;
    if bytes.len() != 20 {
        return Err(AuthError::Hex(hex::FromHexError::InvalidStringLength));
    }
    Ok(H160::from_slice(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference vector from Polymarket/python-order-utils
    /// `tests/test_order_builder.py::test_build_order_signature`.
    ///
    /// If this passes, the domain, typehash, field order, and encoding are all correct.
    #[test]
    fn amoy_reference_vector_digest() {
        let order = Order {
            salt: U256::from(479249096354u64),
            maker: parse_address("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266").unwrap(),
            signer: parse_address("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266").unwrap(),
            taker: H160::zero(),
            token_id: U256::from(1234u64),
            maker_amount: U256::from(100_000_000u64),
            taker_amount: U256::from(50_000_000u64),
            expiration: U256::zero(),
            nonce: U256::zero(),
            fee_rate_bps: U256::from(100u64),
            side: OrderSide::Buy,
            signature_type: SignatureType::Eoa,
        };

        let verifying =
            parse_address(contracts::AMOY_CTF_EXCHANGE).unwrap();
        let digest = order_digest(&order, contracts::AMOY_CHAIN_ID, verifying);

        let expected =
            hex::decode("02ca1d1aa31103804173ad1acd70066cb6c1258a4be6dada055111f9a7ea4e55")
                .unwrap();
        assert_eq!(
            digest.to_vec(),
            expected,
            "EIP-712 digest mismatch — domain/typehash/encoding is wrong"
        );
    }

    /// Reference vector for the Neg Risk exchange variant (same inputs, different verifyingContract).
    #[test]
    fn amoy_neg_risk_reference_vector_digest() {
        let order = Order {
            salt: U256::from(479249096354u64),
            maker: parse_address("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266").unwrap(),
            signer: parse_address("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266").unwrap(),
            taker: H160::zero(),
            token_id: U256::from(1234u64),
            maker_amount: U256::from(100_000_000u64),
            taker_amount: U256::from(50_000_000u64),
            expiration: U256::zero(),
            nonce: U256::zero(),
            fee_rate_bps: U256::from(100u64),
            side: OrderSide::Buy,
            signature_type: SignatureType::Eoa,
        };

        // Neg-risk variant uses the Polygon address here per the research notes;
        // Amoy neg-risk verifier is `0xd91E80cF2E7be2e162c6513ceD06f1dD0dA35296`, but the
        // reference vector was generated against `0xC5d563A36AE78145C45a50134d48A1215220f80a`
        // on chainId 80002. We reproduce that exact pairing.
        let verifying =
            parse_address("0xC5d563A36AE78145C45a50134d48A1215220f80a").unwrap();
        let digest = order_digest(&order, contracts::AMOY_CHAIN_ID, verifying);

        let expected =
            hex::decode("f15790d3edc4b5aed427b0b543a9206fcf4b1a13dfed016d33bfb313076263b8")
                .unwrap();
        assert_eq!(digest.to_vec(), expected);
    }

    /// Reference signature for the Amoy vector. If the digest test passes but
    /// this fails, the issue is in ECDSA signing or v-byte encoding.
    #[test]
    fn amoy_reference_vector_signature() {
        let key_bytes =
            hex::decode("ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80")
                .unwrap();
        let signing_key = SigningKey::from_bytes(key_bytes.as_slice().into()).unwrap();

        let order = Order {
            salt: U256::from(479249096354u64),
            maker: parse_address("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266").unwrap(),
            signer: parse_address("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266").unwrap(),
            taker: H160::zero(),
            token_id: U256::from(1234u64),
            maker_amount: U256::from(100_000_000u64),
            taker_amount: U256::from(50_000_000u64),
            expiration: U256::zero(),
            nonce: U256::zero(),
            fee_rate_bps: U256::from(100u64),
            side: OrderSide::Buy,
            signature_type: SignatureType::Eoa,
        };

        let verifying = parse_address(contracts::AMOY_CTF_EXCHANGE).unwrap();
        let sig = sign_order(&signing_key, &order, contracts::AMOY_CHAIN_ID, verifying).unwrap();

        let expected = hex::decode(
            "302cd9abd0b5fcaa202a344437ec0b6660da984e24ae9ad915a592a90facf5a5\
             1bb8a873cd8d270f070217fea1986531d5eec66f1162a81f66e026db653bf7ce1c",
        )
        .unwrap();
        assert_eq!(sig.to_vec(), expected);
    }
}
