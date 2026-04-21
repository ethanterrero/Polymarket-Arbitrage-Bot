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
