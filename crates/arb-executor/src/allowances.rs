//! Startup approval checks against Polygon mainnet.
//!
//! Before live mode can place orders, the funded `maker` address must have
//! approved both Polymarket CTF Exchange contracts (standard + neg-risk) to
//! move its USDC.e (ERC-20 `allowance`) and its outcome tokens (CTF
//! `isApprovedForAll`). Without these approvals the on-chain settlement
//! reverts even when the CLOB accepts the signed order.
//!
//! All reads go through a Polygon JSON-RPC endpoint via `eth_call`; we don't
//! pull in `ethers-rs` since two known function selectors and a U256/bool
//! decode is all we need.
//!
//! Function selectors (keccak256 prefix):
//! - `allowance(address,address)`        = 0xdd62ed3e
//! - `isApprovedForAll(address,address)` = 0xe985e9c5

use crate::order_signing::{contracts, SignatureType};
use crate::proxy_address;
use primitive_types::{H160, U256};
use serde::{Deserialize, Serialize};
use tracing::{error, info};

#[derive(Debug, thiserror::Error)]
pub enum AllowanceError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("JSON-RPC error: {0}")]
    Rpc(String),
    #[error("Unsupported chain_id {0} for allowance check (only 137/Polygon supported)")]
    UnsupportedChain(u64),
    #[error("malformed eth_call result: {0}")]
    MalformedResult(String),
}

// Polygon mainnet contract addresses. USDC.e is the bridged-from-Ethereum
// USDC variant Polymarket settles in (NOT the native Circle USDC).
// CTF = Gnosis ConditionalTokens, the framework Polymarket markets settle on.
pub const USDC_E_POLYGON: &str = "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174";
pub const CTF_POLYGON: &str = "0x4D97DCd97eC945f40cF65F87097ACe5EA0476045";
pub const POLYGON_CHAIN_ID: u64 = 137;

const SELECTOR_ALLOWANCE: [u8; 4] = [0xdd, 0x62, 0xed, 0x3e];
const SELECTOR_IS_APPROVED_FOR_ALL: [u8; 4] = [0xe9, 0x85, 0xe9, 0xc5];

/// One row of the allowance check report.
#[derive(Debug, Clone)]
pub struct ExchangeAllowance {
    pub label: &'static str,
    pub exchange: H160,
    pub usdc_allowance: U256,
    pub ctf_approved: bool,
}

#[derive(Debug, Clone)]
pub struct AllowanceReport {
    pub maker: H160,
    pub min_usdc_allowance: U256,
    pub rows: Vec<ExchangeAllowance>,
}

impl AllowanceReport {
    /// Approvals that aren't sufficient for live trading. Each entry is a
    /// human-readable description suitable for an error log.
    pub fn missing(&self) -> Vec<String> {
        let mut out = Vec::new();
        for row in &self.rows {
            if row.usdc_allowance < self.min_usdc_allowance {
                out.push(format!(
                    "USDC allowance to {} ({:#x}) is {} (need ≥ {})",
                    row.label, row.exchange, row.usdc_allowance, self.min_usdc_allowance
                ));
            }
            if !row.ctf_approved {
                out.push(format!(
                    "CTF setApprovalForAll to {} ({:#x}) is false",
                    row.label, row.exchange
                ));
            }
        }
        out
    }

    pub fn all_present(&self) -> bool {
        self.missing().is_empty()
    }
}

/// Check USDC.e + CTF approvals to both CTF Exchange contracts for a given
/// EOA + signature type. Only Polygon mainnet (chain_id 137) is supported;
/// returns `UnsupportedChain` otherwise.
pub async fn check_startup_allowances(
    client: &reqwest::Client,
    rpc_url: &str,
    chain_id: u64,
    eoa: H160,
    signature_type: SignatureType,
    min_usdc_allowance: U256,
) -> Result<AllowanceReport, AllowanceError> {
    if chain_id != POLYGON_CHAIN_ID {
        return Err(AllowanceError::UnsupportedChain(chain_id));
    }
    let maker = proxy_address::derive_maker_address(eoa, signature_type);
    let usdc = parse_h160(USDC_E_POLYGON);
    let ctf = parse_h160(CTF_POLYGON);

    let standard_addr = parse_h160(
        contracts::verifying_contract(chain_id, false)
            .expect("standard exchange address known for Polygon"),
    );
    let neg_risk_addr = parse_h160(
        contracts::verifying_contract(chain_id, true)
            .expect("neg-risk exchange address known for Polygon"),
    );

    let mut rows = Vec::with_capacity(2);
    for (label, exchange) in [
        ("standard CTF Exchange", standard_addr),
        ("neg-risk CTF Exchange", neg_risk_addr),
    ] {
        let usdc_allowance = read_allowance(client, rpc_url, usdc, maker, exchange).await?;
        let ctf_approved =
            read_is_approved_for_all(client, rpc_url, ctf, maker, exchange).await?;
        rows.push(ExchangeAllowance {
            label,
            exchange,
            usdc_allowance,
            ctf_approved,
        });
    }

    Ok(AllowanceReport {
        maker,
        min_usdc_allowance,
        rows,
    })
}

/// Log the report, return Ok if all present, Err with the missing summary
/// otherwise. Designed for use at bot startup.
pub fn enforce(report: &AllowanceReport) -> Result<(), String> {
    info!(
        maker = %format!("{:#x}", report.maker),
        min_usdc_allowance = %report.min_usdc_allowance,
        "Checking on-chain approvals"
    );
    for row in &report.rows {
        info!(
            label = row.label,
            exchange = %format!("{:#x}", row.exchange),
            usdc_allowance = %row.usdc_allowance,
            ctf_approved = row.ctf_approved,
            "approval row"
        );
    }
    let missing = report.missing();
    if missing.is_empty() {
        info!("All on-chain approvals satisfied");
        Ok(())
    } else {
        for m in &missing {
            error!(detail = %m, "missing on-chain approval");
        }
        Err(format!(
            "{} on-chain approval(s) missing — see error logs",
            missing.len()
        ))
    }
}

async fn read_allowance(
    client: &reqwest::Client,
    rpc_url: &str,
    usdc: H160,
    owner: H160,
    spender: H160,
) -> Result<U256, AllowanceError> {
    let mut data = Vec::with_capacity(4 + 32 + 32);
    data.extend_from_slice(&SELECTOR_ALLOWANCE);
    data.extend_from_slice(&pad_address(owner));
    data.extend_from_slice(&pad_address(spender));
    let result = eth_call(client, rpc_url, usdc, &data).await?;
    decode_uint256(&result)
}

async fn read_is_approved_for_all(
    client: &reqwest::Client,
    rpc_url: &str,
    ctf: H160,
    owner: H160,
    operator: H160,
) -> Result<bool, AllowanceError> {
    let mut data = Vec::with_capacity(4 + 32 + 32);
    data.extend_from_slice(&SELECTOR_IS_APPROVED_FOR_ALL);
    data.extend_from_slice(&pad_address(owner));
    data.extend_from_slice(&pad_address(operator));
    let result = eth_call(client, rpc_url, ctf, &data).await?;
    decode_bool(&result)
}

#[derive(Serialize)]
struct EthCallParams<'a> {
    to: &'a str,
    data: &'a str,
}

#[derive(Serialize)]
struct JsonRpcRequest<'a> {
    jsonrpc: &'a str,
    method: &'a str,
    params: serde_json::Value,
    id: u64,
}

#[derive(Deserialize)]
struct JsonRpcResponse {
    #[serde(default)]
    result: Option<String>,
    #[serde(default)]
    error: Option<JsonRpcError>,
}

#[derive(Deserialize)]
struct JsonRpcError {
    #[allow(dead_code)]
    code: i64,
    message: String,
}

async fn eth_call(
    client: &reqwest::Client,
    rpc_url: &str,
    to: H160,
    data: &[u8],
) -> Result<Vec<u8>, AllowanceError> {
    let to_hex = format!("{:#x}", to);
    let data_hex = format!("0x{}", hex::encode(data));
    let req = JsonRpcRequest {
        jsonrpc: "2.0",
        method: "eth_call",
        params: serde_json::json!([
            EthCallParams {
                to: &to_hex,
                data: &data_hex,
            },
            "latest"
        ]),
        id: 1,
    };
    let resp = client.post(rpc_url).json(&req).send().await?;
    let status = resp.status();
    if !status.is_success() {
        return Err(AllowanceError::Rpc(format!(
            "HTTP {} from RPC",
            status.as_u16()
        )));
    }
    let body: JsonRpcResponse = resp.json().await?;
    if let Some(e) = body.error {
        return Err(AllowanceError::Rpc(e.message));
    }
    let raw = body
        .result
        .ok_or_else(|| AllowanceError::Rpc("missing result field".to_string()))?;
    let stripped = raw.strip_prefix("0x").unwrap_or(&raw);
    hex::decode(stripped).map_err(|e| AllowanceError::MalformedResult(e.to_string()))
}

fn decode_uint256(bytes: &[u8]) -> Result<U256, AllowanceError> {
    if bytes.len() != 32 {
        return Err(AllowanceError::MalformedResult(format!(
            "expected 32-byte uint256 result, got {} bytes",
            bytes.len()
        )));
    }
    Ok(U256::from_big_endian(bytes))
}

fn decode_bool(bytes: &[u8]) -> Result<bool, AllowanceError> {
    if bytes.len() != 32 {
        return Err(AllowanceError::MalformedResult(format!(
            "expected 32-byte bool result, got {} bytes",
            bytes.len()
        )));
    }
    Ok(bytes[31] != 0)
}

fn pad_address(addr: H160) -> [u8; 32] {
    let mut buf = [0u8; 32];
    buf[12..].copy_from_slice(addr.as_bytes());
    buf
}

fn parse_h160(s: &str) -> H160 {
    let stripped = s.strip_prefix("0x").unwrap_or(s);
    let bytes = hex::decode(stripped).expect("hard-coded address must be valid hex");
    let mut out = [0u8; 20];
    out.copy_from_slice(&bytes);
    H160::from(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::Method::POST;
    use httpmock::MockServer;

    fn eoa() -> H160 {
        // Anvil/Foundry test account #0.
        parse_h160("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266")
    }

    fn rpc_response_uint256(value: U256) -> String {
        let mut bytes = [0u8; 32];
        value.to_big_endian(&mut bytes);
        format!(
            r#"{{"jsonrpc":"2.0","id":1,"result":"0x{}"}}"#,
            hex::encode(bytes)
        )
    }

    fn rpc_response_bool(b: bool) -> String {
        let mut bytes = [0u8; 32];
        if b {
            bytes[31] = 1;
        }
        format!(
            r#"{{"jsonrpc":"2.0","id":1,"result":"0x{}"}}"#,
            hex::encode(bytes)
        )
    }

    #[tokio::test]
    async fn rejects_non_polygon_chain() {
        let client = reqwest::Client::new();
        let err = check_startup_allowances(
            &client,
            "http://unused",
            80002, // Amoy
            eoa(),
            SignatureType::Eoa,
            U256::from(1u64),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AllowanceError::UnsupportedChain(80002)));
    }

    #[tokio::test]
    async fn report_passes_when_all_approvals_present() {
        let server = MockServer::start();
        let max = U256::MAX;

        // Two exchanges × two reads = 4 RPC calls per startup. The mock server
        // responds in arrival order via separate mocks keyed on `data` prefix.
        // For simplicity we set up one mock per response value and rely on
        // ordering — the implementation issues the four calls sequentially
        // (standard USDC, standard CTF, neg-risk USDC, neg-risk CTF).
        // To avoid coupling to call order, every USDC call returns U256::MAX
        // and every CTF call returns true.
        server.mock(|when, then| {
            when.method(POST)
                .body_includes(&format!("0x{}", hex::encode(SELECTOR_ALLOWANCE)));
            then.status(200)
                .header("content-type", "application/json")
                .body(rpc_response_uint256(max));
        });
        server.mock(|when, then| {
            when.method(POST)
                .body_includes(&format!("0x{}", hex::encode(SELECTOR_IS_APPROVED_FOR_ALL)));
            then.status(200)
                .header("content-type", "application/json")
                .body(rpc_response_bool(true));
        });

        let client = reqwest::Client::new();
        let report = check_startup_allowances(
            &client,
            &server.url(""),
            POLYGON_CHAIN_ID,
            eoa(),
            SignatureType::Eoa,
            U256::from(1_000_000u64), // 1 USDC
        )
        .await
        .unwrap();

        assert_eq!(report.rows.len(), 2);
        assert!(report.all_present(), "report missing: {:?}", report.missing());
    }

    #[tokio::test]
    async fn report_flags_missing_ctf_approval() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(POST)
                .body_includes(&format!("0x{}", hex::encode(SELECTOR_ALLOWANCE)));
            then.status(200).body(rpc_response_uint256(U256::MAX));
        });
        server.mock(|when, then| {
            when.method(POST)
                .body_includes(&format!("0x{}", hex::encode(SELECTOR_IS_APPROVED_FOR_ALL)));
            then.status(200).body(rpc_response_bool(false));
        });

        let client = reqwest::Client::new();
        let report = check_startup_allowances(
            &client,
            &server.url(""),
            POLYGON_CHAIN_ID,
            eoa(),
            SignatureType::Eoa,
            U256::from(1_000_000u64),
        )
        .await
        .unwrap();

        assert!(!report.all_present());
        // Both exchange rows should report missing CTF approval.
        let missing = report.missing();
        assert_eq!(missing.iter().filter(|m| m.contains("CTF setApprovalForAll")).count(), 2);
    }

    #[tokio::test]
    async fn report_flags_insufficient_usdc_allowance() {
        let server = MockServer::start();
        // USDC allowance is exactly 0.5 USDC (500_000 base units), below the
        // 1 USDC threshold the test uses.
        server.mock(|when, then| {
            when.method(POST)
                .body_includes(&format!("0x{}", hex::encode(SELECTOR_ALLOWANCE)));
            then.status(200).body(rpc_response_uint256(U256::from(500_000u64)));
        });
        server.mock(|when, then| {
            when.method(POST)
                .body_includes(&format!("0x{}", hex::encode(SELECTOR_IS_APPROVED_FOR_ALL)));
            then.status(200).body(rpc_response_bool(true));
        });

        let client = reqwest::Client::new();
        let report = check_startup_allowances(
            &client,
            &server.url(""),
            POLYGON_CHAIN_ID,
            eoa(),
            SignatureType::Eoa,
            U256::from(1_000_000u64), // 1 USDC
        )
        .await
        .unwrap();

        assert!(!report.all_present());
        assert_eq!(
            report
                .missing()
                .iter()
                .filter(|m| m.contains("USDC allowance"))
                .count(),
            2
        );
    }

    #[test]
    fn enforce_returns_ok_when_all_present() {
        let report = AllowanceReport {
            maker: eoa(),
            min_usdc_allowance: U256::from(1u64),
            rows: vec![
                ExchangeAllowance {
                    label: "standard",
                    exchange: H160::zero(),
                    usdc_allowance: U256::MAX,
                    ctf_approved: true,
                },
                ExchangeAllowance {
                    label: "neg-risk",
                    exchange: H160::zero(),
                    usdc_allowance: U256::MAX,
                    ctf_approved: true,
                },
            ],
        };
        assert!(enforce(&report).is_ok());
    }

    #[test]
    fn enforce_returns_err_when_anything_missing() {
        let report = AllowanceReport {
            maker: eoa(),
            min_usdc_allowance: U256::from(1u64),
            rows: vec![ExchangeAllowance {
                label: "standard",
                exchange: H160::zero(),
                usdc_allowance: U256::from(0u64),
                ctf_approved: false,
            }],
        };
        assert!(enforce(&report).is_err());
    }
}
