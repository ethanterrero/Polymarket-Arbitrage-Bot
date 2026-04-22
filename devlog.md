# Dev Log

---

## 2026-04-14

### What we did
Fixed a Rust borrow checker error in `crates/arb-inventory/src/lib.rs` that was preventing the entire workspace from compiling.

### The bug
Inside `InventoryManager::record_leg_fill`, we held a mutable reference to `state.open_legs` (via `legs`) and then tried to also mutably push into `state.paired_positions` on line 99 — all while the first borrow was still alive. Rust doesn't allow two mutable borrows of the same struct at the same time, even if they point to different fields.

### The fix
Wrapped all code that touches `legs` (a mutable reference into `state.open_legs`) inside a block scope `{ ... }`. In Rust, a variable is dropped when it goes out of scope — so when the block ends, `legs` is dropped and the borrow on `state.open_legs` is released. After the block:

```rust
let (new_pairs, remaining) = {
    let legs = state.open_legs.entry(...).or_default();
    // ... all legs work ...
    (new_pairs, remaining)
    // legs dropped here
};

// Now safe — open_legs borrow is gone
state.paired_positions.extend(new_pairs.clone());

// Re-borrow open_legs fresh for the remaining fill
state.open_legs.entry(...).or_default().push(OpenLeg { ... });
```

Also fixed a missing `rust_decimal` dependency in `arb-bot/Cargo.toml` that was revealed once the inventory error was resolved.

### Concept learned: Rust borrow checker
Rust enforces at compile time that you can't have two mutable references to the same thing simultaneously. This prevents data races and memory corruption without needing a garbage collector. When you mutably borrow a field on a struct, the whole struct is considered borrowed — so you have to finish with it before borrowing another field mutably.

### State after today
- `arb-inventory` compiles cleanly
- All 9 crates build successfully (`cargo build` passes)
- Bot runs in dry-run mode end-to-end
- Next: add real USDC balance feed from CLOB API → `RiskManager::update_balance`

---

## 2026-04-20

### What we did
Two things in sequence: fixed broken workspace tests, then implemented the full Polymarket CLOB authentication path to unblock live trading.

### Part 1 — Test config drift fix (PR #1, merged)
`cargo test --workspace` was failing because `PolymarketConfig` and `ScannerConfig` gained new fields (`wallet_address`, `market_keywords`) after the test helper in `arb-executor` was written. Struct literal initializers in Rust require every field to be named — there's no default fill-in at the call site. Added `wallet_address: String::new()` and `market_keywords: Vec::new()` to `make_test_config()`. Tests went from 2 compile errors → all green.

### Part 2 — Live trading auth (this PR)
The bot had a private key loader but was permanently stuck in dry-run because auth was a stub. We implemented the full two-part Polymarket CLOB auth flow.

**How Polymarket CLOB auth works:**

Step 1 — API key derivation. You don't manually create an API key. Instead, you call `POST /auth/derive-api-key` with a JSON body containing an EIP-712 signature that proves you own the wallet. The server returns `{ apiKey, secret, passphrase }`. EIP-712 is Ethereum's standard for signing structured data (not raw bytes) — it hashes a typed message in a specific way so the signer knows exactly what they're authorizing.

The signing chain:
```
domainSeparator = keccak256(abi.encode(domainTypeHash, hash("ClobAuthDomain"), hash("1"), chainId))
structHash      = keccak256(abi.encode(clobAuthTypeHash, address, hash(timestamp), nonce, hash(message)))
digest          = keccak256("\x19\x01" || domainSeparator || structHash)
signature       = secp256k1_sign(privateKey, digest)   → r || s || v  (v = 27 or 28)
```

Step 2 — Per-request HMAC signing. Every order request includes six `POLY_*` headers. The signature header is:
```
POLY_SIGNATURE = base64url(HMAC-SHA256(base64url_decode(secret), timestamp + "POST" + "/order" + body))
```
The body string is serialized once and used for both the HMAC computation and the HTTP request body — important that they're identical bytes.

### New code
- `crates/arb-executor/src/auth.rs` — new module: `derive_api_key`, `build_auth_headers`, `build_hmac_signature`, EIP-712 signing helpers, EIP-55 address derivation. Three unit tests: HMAC determinism, address derivation against the known Hardhat test key, EIP-712 sig format validation.
- `crates/arb-executor/src/lib.rs` — replaced three separate `Option<String>` credential fields with `creds: Option<ApiCredentials>`, added `OrderExecutor::new_live(config, private_key_hex)` async constructor, `place_order` now computes real HMAC headers.
- `crates/arb-bot/src/main.rs` — bot now calls `new_live` when `POLYMARKET_PRIVATE_KEY` is set; falls back to dry-run with a warning if derivation fails.

### New dependencies added
`hmac`, `sha2`, `sha3` (keccak256), `k256` (secp256k1 signing), `base64`, `hex` — all small, well-audited crates from the RustCrypto project.

### Concept learned: EIP-712
EIP-712 is a standard for signing structured data in Ethereum wallets. Before it, dApps would ask users to sign raw hex hashes — you had no idea what you were signing. EIP-712 forces the signer to commit to a specific typed schema (domain + field names + types), so wallets can show a human-readable breakdown. Here we use it headlessly (no wallet UI) to prove to the Polymarket CLOB that we control the private key associated with a given address.

### State after today
- `cargo build --workspace` — clean, zero errors
- `cargo test --workspace` — all 23 tests pass
- Live trading is now reachable: set `POLYMARKET_PRIVATE_KEY=0x...` and the bot derives an API key on startup and signs real orders
- Next: add `execution.mode = dry_run | live` config flag so live trading requires explicit opt-in (can't be triggered by env var presence alone)

---

## 2026-04-21

### What we did
Scoped the real next blocker for live trading, then landed the cryptographic core of it: EIP-712 signing for Polymarket CTF Exchange orders. Pinned it against Polymarket's own published test vectors so we know the bytes are correct before we ever touch the network.

### Why this was needed
The auth work on 2026-04-20 got us past the *request* authentication layer — the `POLY_*` HMAC headers that prove "this HTTP request is from an authenticated API client." But the Polymarket CLOB requires a second, independent signature on the *order itself*: a structured EIP-712 signature over the order fields (tokenId, amounts, side, expiration, etc.) using the wallet's private key. Without it, the CLOB rejects orders even with valid HMAC headers. Our old `ClobOrderRequest` was a 5-field JSON blob — nowhere near the full signed Order struct the exchange expects.

### Research first
Before writing any code, spent real effort finding exact references in Polymarket's open-source repos:
- `Polymarket/ctf-exchange` — the on-chain Solidity contract (source of truth for the struct layout and typehash)
- `Polymarket/python-order-utils` — signing library with **published test vectors**
- `Polymarket/rs-clob-client` — Polymarket's own Rust client (confirms wire format, contract addresses, price→amount math)

The test vectors were the key find. They let us verify a pure cryptographic implementation deterministically, offline, before touching anything networked.

### The Order struct
Polymarket orders hashed under EIP-712 have this exact layout (from `OrderStructs.sol`):

```
Order(
    uint256 salt,
    address maker,        // funded account — proxy wallet for most users
    address signer,       // the EOA that holds the key
    address taker,        // 0x000...000 = public order (normal case)
    uint256 tokenId,      // the CLOB outcome token (YES or NO side)
    uint256 makerAmount,  // what you pay (USDC-scaled integer, 6 decimals)
    uint256 takerAmount,  // what you receive (tokens, same scale)
    uint256 expiration,   // unix seconds, 0 = GTC
    uint256 nonce,        // on-chain cancel nonce, usually 0
    uint256 feeRateBps,   // per-market, fetched from /fee-rate
    uint8   side,         // 0=BUY, 1=SELL
    uint8   signatureType // 0=EOA, 1=POLY_PROXY, 2=POLY_GNOSIS_SAFE
)
```

The full signing chain:
```
orderTypeHash     = keccak256("Order(uint256 salt,address maker,...)")
structHash        = keccak256(abi.encode(orderTypeHash, ...fields))
domainSeparator   = keccak256(abi.encode(
                        domainTypeHash,
                        keccak256("Polymarket CTF Exchange"),
                        keccak256("1"),
                        chainId,
                        verifyingContract))
digest            = keccak256("\x19\x01" || domainSeparator || structHash)
signature         = secp256k1_sign(privateKey, digest) → r || s || v
```

### Two exchanges, not one
Polymarket actually runs two CTF Exchange contracts on Polygon: the standard one (`0x4bFb4…8982E`) and a **Neg Risk** variant (`0xC5d56…20f80a`) for markets with linked outcomes. Which one signs your order depends on a `neg_risk` flag that comes from the market metadata. The EIP-712 domain name and version are identical — only the `verifyingContract` differs — which means the signature produced for one contract is *not* valid at the other. Getting this wrong would cause all orders on ~half of all markets to silently fail.

### New code
- **`crates/arb-executor/src/order_signing.rs`** — new module, ~280 lines, pure cryptographic logic with zero network or filesystem side effects:
  - `Order` struct mirroring the Solidity layout exactly
  - `OrderSide` (Buy/Sell) and `SignatureType` (Eoa/PolyProxy/PolyGnosisSafe) enums
  - `order_struct_hash` — keccak256 over ABI-encoded fields
  - `domain_separator` — per-chain, per-exchange
  - `order_digest` — the final 32-byte ECDSA input
  - `sign_order` — returns `r || s || v` as 65 bytes with v ∈ {27, 28}
  - `contracts` submodule with canonical addresses for Polygon mainnet + Amoy testnet, for both exchange flavors
- **`crates/arb-executor/src/lib.rs`** — wired the new module with `pub mod order_signing;`
- **`Cargo.toml`** — added `primitive-types = "0.12"` for `U256` and `H160`. Our existing `auth.rs` hand-rolled its own 32-byte big-endian encoding; for orders we need real `uint256` math because token IDs can be ~77-digit numbers that won't fit in `u64`.

### Tests that actually prove it works
Three unit tests, all using the Anvil test key and inputs lifted verbatim from `python-order-utils/tests/test_order_builder.py`:

1. **`amoy_reference_vector_digest`** — feeds a known order into our digest function, expects the 32-byte value `0x02ca1d1aa31103804173ad1acd70066cb6c1258a4be6dada055111f9a7ea4e55`. This catches any bug in the domain hash, typehash, field ordering, or ABI encoding.
2. **`amoy_neg_risk_reference_vector_digest`** — same inputs, different `verifyingContract`, expects `0xf15790d3…`. Proves the neg-risk routing works.
3. **`amoy_reference_vector_signature`** — runs the full sign path end-to-end. Expects the exact 65-byte signature `0x302cd9ab…1c`. Catches any bug in ECDSA signing or v-byte encoding.

All three pass. Because the inputs and expected outputs were produced by Polymarket's own library, passing these tests means our signatures are byte-identical to what their official clients would produce.

### Concept learned: deterministic test vectors as a cross-check
A reference vector is a "known input → known output" pair published by a trusted source (here, the upstream project's own test suite). For cryptographic code, reference vectors are uniquely powerful: the output is sensitive to every single bit of the input, so if you produce the expected bytes, the odds of having a subtle bug are essentially zero. It's very different from "our test passed" for a regular unit test — it's "we computed the same thing the reference implementation did." Always look for these when implementing a spec that someone else has already implemented.

### State after today
- `cargo test --workspace` — all tests pass, including the 3 new cryptographic reference-vector tests
- Pure cryptographic layer for order signing is done and verified
- Branch: `feat/eip712-order-signing` (not yet pushed)
- **The bot still can't place live orders yet** — signing produces correct bytes, but those bytes aren't yet being attached to anything sent to the CLOB. That's the wiring task next.

### What's next (in order)
1. **CREATE2 proxy/safe derivation** — given an EOA, deterministically compute the Polymarket proxy address used as `maker`. No HTTP call needed — pure address arithmetic with known vectors.
2. **BinaryMarket metadata plumbing** — add `neg_risk`, `fee_rate_bps`, `min_tick_size` fields; update the scanner to parse them from the Gamma API.
3. **Price/size → makerAmount/takerAmount math** — matching `rs-clob-client`'s quantization (truncate to 6 decimals, USDC-scale).
4. **Rewrite the `/order` request body** — salt as a JSON number (must fit in u53), amounts as decimal strings, side as `"BUY"`/`"SELL"` at the wire layer, `owner` = API key UUID. Wrap the signed Order + orderType + owner into the shape the CLOB actually expects.
5. **Wiremock integration test** — assert the full POSTed request body matches what a real Polymarket client would send, byte-for-byte on the order fields.
6. **Startup allowance check** — verify USDC and CTF approvals to the exchange before enabling live mode; fail loudly if missing.

