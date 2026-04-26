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

---

## 2026-04-25

### What we did
Implemented CREATE2 derivation of Polymarket proxy and Gnosis Safe addresses — the `maker` field on a signed CTF Exchange order. Pure cryptographic logic, no I/O. Pinned against Polymarket's own published address pairs so we know the bytes are correct without ever hitting Polygon.

### Why this was needed
A signed Polymarket order has a `maker` field that names the *funded account*. For the vast majority of users that's not the EOA — it's a CREATE2-deployed proxy contract owned by the EOA. Two flavors:

- `PolyProxy` (signature type 1): minimal-clone proxy used for Magic/email accounts.
- `PolyGnosisSafe` (signature type 2): Gnosis Safe deployed by Polymarket for browser-wallet users.

Without the right `maker` address, the digest changes and the on-chain settlement reverts (or the CLOB rejects the order before it ever lands). The EIP-712 signing layer from 2026-04-21 produces the bytes; this module produces the address those bytes have to commit to.

### What CREATE2 actually is
CREATE2 is the Ethereum opcode that lets a contract deploy another contract at a *predictable* address — predictable because the address is computed purely from inputs the deployer chooses, with no dependence on transaction order or nonce. The formula is:

```
address = keccak256(0xff || factoryAddress || salt || keccak256(initCode))[12..32]
```

The four inputs:
- `0xff` — a fixed prefix that domain-separates CREATE2 from regular CREATE.
- `factoryAddress` — the contract that will deploy the new code (20 bytes).
- `salt` — a 32-byte value the factory chooses; here, a hash of the user's EOA.
- `keccak256(initCode)` — hash of the bytecode + constructor args of the deployed contract.

Take keccak256 of those 85 bytes, drop the first 12, and that's the deployed address. The neat property: anyone who knows all four inputs can predict the address *before* deployment. So if Polymarket has registered "user X's funded account is at address Y", we don't need to call any RPC — we can verify it locally.

### The two factories aren't symmetric
This was the easy thing to get wrong. Both factories live on Polygon mainnet, both are CREATE2-deployed contracts owned by Polymarket, but the salt encoding differs:

```
PolyProxy:      salt = keccak256( abi.encodePacked(eoa) )    // 20 raw bytes
PolyGnosisSafe: salt = keccak256( abi.encode(eoa) )          // 32 bytes, address left-padded
```

Solidity's `abi.encodePacked` concatenates without padding (so a 20-byte address stays 20 bytes); `abi.encode` left-pads each value to a 32-byte word. Two different inputs to keccak256 → two different salts → two different addresses. If we used the same salt formula for both, our Safe predictions would be wrong for every user.

The init code hashes are also distinct because the contracts are different shapes — the proxy is an EIP-1167 minimal clone; the Safe is the full Gnosis Safe proxy with the master copy address baked into the constructor data.

### New code
- **`crates/arb-executor/src/proxy_address.rs`** — new module, ~140 lines:
  - `POLYGON_PROXY_FACTORY` / `POLYGON_SAFE_FACTORY` — factory addresses.
  - `PROXY_INIT_CODE_HASH` / `SAFE_INIT_CODE_HASH` — 32-byte init code digests as `[u8; 32]` constants (no string parsing at runtime).
  - `derive_poly_proxy(eoa)`, `derive_poly_safe(eoa)` — the two specific derivations.
  - `derive_maker_address(eoa, signature_type)` — single dispatch entry point that returns `eoa` for `Eoa` and routes the other two to the right derivation.
- **`crates/arb-executor/src/lib.rs`** — added `pub mod proxy_address;`. No new dependencies; `primitive_types` and `sha3` were already pulled in by the order-signing module.

### Tests
Four unit tests, two of them deterministic reference-vector pins from `polymarket-client-sdk` v0.4.4 using the Anvil/Foundry test account #0 (`0xf39F...2266`):

1. `poly_proxy_reference_vector` — expects `0x365f0cA36ae1F641E02Fe3b7743673DA42A13a70`.
2. `poly_safe_reference_vector` — expects `0xd93b25Cb943D14d0d34FBAf01fc93a0F8b5f6e47`.
3. `derive_maker_address_dispatches_by_type` — exercises the dispatch wrapper for all three signature types.
4. `proxy_and_safe_differ` — sanity check that the two derivations don't accidentally produce the same address (they would if the salt encoding got copy-pasted between functions).

All four pass. As with the order-signing tests, passing the reference vectors means the constants and the encoding are byte-identical to Polymarket's official client.

### Caveat we wrote into the module docs
For Magic-Link users, the on-chain proxy can occasionally diverge from the CREATE2 prediction — Polymarket's backend may have mapped a user to a different proxy than `keccak256(eoa)` predicts (open issue: `Polymarket/polymarket-cli#14`). The robust live path is to fetch the funded address from `/profiles` and treat this derivation as a verification/fallback. This isn't a correctness bug in the math; it's a backend mapping that doesn't always match the deterministic derivation. Documented at the top of the module so future-us doesn't get confused if a manual address check disagrees.

### Concept learned: counterfactual addresses
Because CREATE2 makes the address a pure function of inputs known *before* deployment, you can sign messages addressed to a contract that doesn't exist on-chain yet. This is what Polymarket does for new users: the funded proxy may not actually be deployed until the user's first trade, but the address is known and orders can be signed against it. The chain doesn't care — once the proxy is deployed (by anyone), the historical signatures referencing that address become valid. That property is what makes "1-click signup → trade" possible without paying a deployment gas fee upfront.

### State after today
- `cargo build --workspace` — clean.
- `cargo test --workspace` — 30 tests pass (was 26; +4 new).
- Branch: `feat/proxy-address-derivation`.
- Order signing layer + funded-address derivation are both complete and verified offline. We can now produce a (signature, maker) pair that matches what Polymarket's official clients produce, given a private key and a market.

### What's next (in order)
1. **BinaryMarket metadata plumbing** — add `neg_risk`, `fee_rate_bps`, `min_tick_size`, `condition_id` fields to the scanner's market type; parse them from the Gamma API response. Without these the order signer can't pick the right verifying contract or fee rate.
2. **Price/size → makerAmount/takerAmount math** — port `rs-clob-client`'s quantization (truncate to 6 decimals USDC scale, side-aware mapping). Reference vectors live in `python-order-utils/tests/test_order_builder.py`.
3. **Rewrite the `/order` request body** — wrap signed Order + orderType + owner (API key UUID) into the JSON shape the CLOB expects; salt as a JSON number that fits in u53; amounts as decimal strings; side as `"BUY"`/`"SELL"`.
4. **Wiremock integration test** — assert the full POSTed request body matches a real Polymarket client byte-for-byte on the order fields.
5. **Startup allowance check** — verify USDC and CTF approvals to the exchange before enabling live mode; fail loudly if missing.


