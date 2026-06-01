# Dev Log

---

## 2026-05-17 (Phase 2 — Maker pricing in `analyze_asymmetric`)

### What we did
Replaced the taker-disguised-as-async pricing in `analyze_asymmetric` with real maker pricing. Each leg is now quoted at `best_bid + min_tick_size` (inside the spread) capped at the per-side target — instead of the previous behavior of crossing at the best ask. Combined with Phase 1, these GTC posts now correctly rest on the CLOB.

### Why this was needed
Phase 1 added the resting-order tracking but didn't change *what price* `analyze_asymmetric` quotes. Without this change, all the resting-order machinery was being fed `best_ask`-priced orders that would never actually rest (they'd cross immediately). This is the change that turns the asymmetric mode from a fancy taker into a maker.

### Key changes
- **`arb-types`** — new `best_yes_bid` / `best_no_bid` helpers on `BinaryOrderBook` (mirrors the existing ask helpers).
- **`arb-strategy`** — `analyze_asymmetric` rewritten. Per side:
  - Compute `target_price` from `asymmetric_target_total_cost - opposite_side_best_ask - fee_overhead` (unchanged math; what changed is where the post lands).
  - Compute `target_floor = snap_to_tick(target_price, min_tick_size)` so the post price always falls on the CLOB's valid tick grid.
  - `improved_bid = best_bid + min_tick_size`.
  - Skip if `improved_bid > target_floor` (queue jump isn't worth the spread loss) OR `improved_bid >= best_ask` (would just become a taker).
  - Otherwise post at `improved_bid` with `use_fok = false` (must be GTC to rest).
  - Size is bounded by the resting bid's size — sane upper bound; risk layer clamps further.
- New helper: `snap_to_tick(price, tick)` floor-rounds to a tick multiple.

### What's intentionally deferred
- **Sizing strategy.** Current size = best-bid size. Crude but lets us land the pricing change without redesigning sizing. Phase 3's repost loop is the natural home for tier-1/tier-2/tier-3 sizing.
- **Multi-level posts.** Today posts only at the inside-the-spread tier. Phase 3 can add deeper resting orders if the strategy proves itself.

### Tests
- 6 new pinned unit tests in `arb-strategy`:
  - Profitable inside-the-spread post (asserts the exact post price).
  - Skip-when-improved-bid-exceeds-target (with NO-side qualifying as cross-check).
  - Skip-when-improved-bid-would-cross-ask.
  - Target-floor snaps to tick.
  - Skip side with no resting bid.
  - `snap_to_tick` direct unit tests.

`cargo test --workspace` — 59 pass (was 53 on main). Build clean, no new warnings.

### State after today
- Branch: `feat/asymmetric-phase-2-maker-pricing`.
- Asymmetric mode now produces real maker quotes that will actually rest on the CLOB. Combined with Phase 1, the system can post → poll → fill → pair.
- Next: Phase 3 — repost stale quotes when the market moves, enforce `max_unpaired_legs_per_market` against resting + filled-unpaired, send IOC closer when a leg fills.

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

---

## 2026-04-25 (afternoon)

### What we did
Plumbed three new market metadata fields end-to-end (`neg_risk`, `fee_rate_bps`, `min_tick_size`) so the order signer has the inputs it needs. Found and fixed a latent scanner bug along the way: the bot has been silently fetching zero markets in dry-run.

### The latent bug
`GammaMarketResponse::condition_id` was declared as bare `condition_id: Option<String>` with `#[serde(default)]`. Polymarket's Gamma API returns `conditionId` (camelCase). Serde matches on the literal field name unless `rename` is set, so the field defaulted to `None` for every response. Then in `parse_binary_market` the very first line is `let condition_id = raw.condition_id?;` — early-return on `None` — so every market was dropped silently.

The bot still ran end-to-end because dry-run mode logged "0 opportunities found" without distinguishing "no profitable spreads" from "no markets to scan." Caught it before adding new fields by curling Gamma and noticing the response uses `conditionId`, `negRisk`, `clobTokenIds` — camelCase across the board (`clobTokenIds` was already correctly renamed; `condition_id` had been overlooked).

Fix: `#[serde(rename = "conditionId")]` on the field. Same renames added preemptively for the new fields.

### Why metadata plumbing was needed
The order signer needs three pieces of per-market info that the scanner wasn't providing:

- **`neg_risk: bool`** — selects which CTF Exchange `verifyingContract` goes into the EIP-712 domain. Two contracts on Polygon: standard (`0x4bFb…8982E`) and Neg Risk (`0xC5d5…20f80a`). A signature produced for one is not valid at the other. ~Half of Polymarket's markets use Neg Risk; getting this wrong would make all of those silently fail.
- **`fee_rate_bps: u32`** — gets ABI-encoded into the order struct hash. Wrong value → wrong digest → CLOB rejection.
- **`min_tick_size: Decimal`** — needed for limit price quantization. The CLOB rejects prices that aren't a multiple of this.

### What changed

**`crates/arb-types/src/lib.rs`** — three new fields on `BinaryMarket`. Picked types deliberately:
- `neg_risk: bool` — Gamma returns a real boolean, no encoding gymnastics.
- `fee_rate_bps: u32` — Solidity `uint256`, but per-market fees are always tiny integers (Polymarket's max is ~500 bps); `u32` is enough headroom and avoids dragging `U256` into a domain type.
- `min_tick_size: Decimal` — Gamma returns `0.01` / `0.001` as JSON numbers; `Decimal` preserves that exactly without float imprecision.

**`crates/arb-scanner/src/lib.rs`**:
- Fixed the `conditionId` rename bug.
- Added `negRisk` and `orderPriceMinTickSize` to `GammaMarketResponse` with `#[serde(default)]` so missing fields don't break parsing.
- New `default_fee_rate_bps` field on `MarketScanner`, derived once at construction from `config.strategy.base_fee_rate` via a new `decimal_to_bps` helper that truncates toward zero (matches the CTF Exchange's integer-bps representation).
- New constant `DEFAULT_MIN_TICK_SIZE = 0.01` for markets where Gamma omits the tick size.

**Test fixtures** — `BinaryMarket` is constructed directly in three test sites (arb-risk, arb-executor, arb-strategy). All updated.

### Why fee_rate_bps comes from config, not the API
Gamma doesn't expose per-market fee rates. The CLOB API has a `/fee-rate-bps` endpoint that does, but wiring that adds a network dependency on every market refresh and another failure mode to handle. Scope for this PR is metadata *plumbing* — adding the field and a sane default gets the order signer unblocked. Replacing the default with real per-market rates from the CLOB belongs in the same task that wires the order request body, where we'll already be talking to the CLOB API.

This is captured as a TODO comment on the new `default_fee_rate_bps` field. Polymarket's standard fee is currently 0 bps for most markets, so this default isn't actively wrong — it's just not personalized per market.

### New tests
Three tests in `arb-scanner`, total +3:

1. **`deserializes_real_gamma_field_names`** — captured a trimmed real Gamma response (the GTA VI / Russia-Ukraine ceasefire market, `0x9c1a…5763`) and asserts the deserializer extracts all the camelCase fields correctly. This is the test that would have caught the `condition_id` bug from day one if it existed.
2. **`defaults_apply_when_neg_risk_and_tick_size_are_missing`** — defensive: if Gamma ever drops these fields, we fall through to `None` rather than panicking.
3. **`decimal_to_bps_basic_cases`** — the bps conversion truncates correctly (`0.0001` → 1 bp, `0.00019` → 1 bp, not 2).

### Concept learned: silent failures from default-deserialized fields
`#[serde(default)]` is a useful resilience pattern — it means a missing field doesn't fail the whole parse — but it's also a great way to hide bugs. If the field name doesn't match the wire format, the field always defaults, and downstream code never knows the difference between "API didn't send it" and "we asked for the wrong key." Two ways to defend:
1. Always assert against a real captured response in unit tests, not synthetic JSON written to match the struct (because both will pass even if the struct is wrong).
2. For load-bearing fields where missing-data should be a hard error, drop `#[serde(default)]` and let serde fail loudly. The `condition_id` field is load-bearing — we could plausibly upgrade that to non-default once we're confident in the shape.

### State after today
- `cargo build --workspace` — clean.
- `cargo test --workspace` — 33 tests pass (was 30; +3 new).
- Branch: `feat/market-metadata-plumbing`.
- Scanner now produces actual non-zero market lists with the metadata the order signer needs.

### What's next (in order)
1. **Price/size → makerAmount/takerAmount math** — port `rs-clob-client`'s quantization (truncate to 6 decimals USDC scale, side-aware mapping). Reference vectors live in `python-order-utils/tests/test_order_builder.py`.
2. **Rewrite the `/order` request body** — wrap signed Order + orderType + owner (API key UUID) into the JSON shape the CLOB expects; salt as a JSON number that fits in u53; amounts as decimal strings; side as `"BUY"`/`"SELL"`. This is also where we'd swap `default_fee_rate_bps` for a real per-market fetch from `/fee-rate-bps`.
3. **Wiremock integration test** — assert the full POSTed request body matches a real Polymarket client byte-for-byte on the order fields.
4. **Startup allowance check** — verify USDC and CTF approvals to the exchange before enabling live mode; fail loudly if missing.

---

## 2026-04-25 (evening)

### What we did
Started wiring the full signed-order `/order` payload so live trading can submit CTF Exchange orders the CLOB will accept.

### Key changes
- **Signed order request shape**: replaced the legacy `{token_id, price, size, ...}` body with a structured payload that includes a signed EIP-712 `Order` plus `orderType` and `owner` (API key id).
- **Quantization**: added `min_tick_size` price snapping and 6-decimal truncation for amounts before converting to `uint256` and signing.
- **Maker address**: `maker` is derived from the signer address + `SignatureType` (EOA by default; overrideable via `POLYMARKET_SIGNATURE_TYPE`).
- **Deterministic auth headers for tests**: added `build_auth_headers_at(...)` so HMAC signing can be tested with a fixed timestamp.
- **Integration-ish test**: added a mock-server test that asserts we can build and send a signed `/order` request with auth headers and get a success response.

### Caveats / still TODO
- **Amount math**: `makerAmount`/`takerAmount` scaling is implemented at 6 decimals for both, but we still need to cross-check against Polymarket’s `rs-clob-client` / official spec for token amount scaling and edge-case rounding.
- **Per-market fee rate**: `fee_rate_bps` is still a default derived from config; wiring the real `/fee-rate-bps` fetch belongs with the finalized order-wiring path.
- **Allowances**: live mode still needs explicit startup allowance checks before enabling live execution.

### State after today
- `cargo test --workspace` passes (now includes a mock `/order` test in `arb-executor`).

### What's next
1. Verify `makerAmount`/`takerAmount` quantization against a known-good client and add a pinned reference-vector test for the full request body.
2. Swap fee rate default for a real per-market `/fee-rate-bps` lookup (cached).
3. Add startup allowance checks for USDC + CTF approvals before live execution.

---

## 2026-04-26

### What we did
Closed the three remaining live-trading gates from the 2026-04-25 evening backlog: pinned the BUY amount quantization against `py-clob-client`, replaced the config-derived per-market fee rate with a real `/fee-rate` lookup, and added an on-chain allowance check that fails the bot at startup if approvals aren't in place.

### Quantization (commit 1)
The previous executor truncated `size` to 6 decimal places and floored `price` to a tick multiple. Polymarket's `py-clob-client` (`get_order_amounts` + `ROUNDING_CONFIG`) uses `round_down(size, 2)` and `round_normal(price, tick_decimals)` instead. For sizes with more than 2 decimals (e.g. `50.7891`) our code would emit a `taker_amount` of `50_789_100` USDC base units. The CLOB only accepts amounts at 0.01-token granularity (`50_780_000`), so any such order was a guaranteed live-mode rejection — fortunately not yet triggered because dry-run still dominates.

The fix:
- New `tick_decimals` helper mapping `{0.1, 0.01, 0.001, 0.0001} → {1, 2, 3, 4}` (rejects other ticks loud — `py-clob-client`'s `ROUNDING_CONFIG` only defines those four).
- Price → `MidpointNearestEven` to tick decimals (matches Python's `round()` / `round_normal`).
- Size → `ToZero` truncation to 2 decimals (matches `round_down(size, 2)`).
- `decimal_to_u256_scaled` now uses `MidpointNearestEven` at the integer step too (matches `to_token_decimals`); a no-op for these tick configs, kept for defense-in-depth.
- New `buy_order_amounts` wrapper used by the signed-order builder.

Seven pinned tests, lifted from the upstream algorithm and verified by hand: all four supported ticks, the size-truncation case (against the prior bug), the price-rounding-up case, and the rejections.

### Per-market fee rate fetch (commit 2)
`Order.feeRateBps` is part of the EIP-712 digest, so a wrong fee rate makes the CLOB silently reject the order. Previously sourced from `StrategyConfig.base_fee_rate` plumbed through `BinaryMarket` — fine for the ~0bps majority but wrong for any market with a non-zero fee.

Matched `py-clob-client`'s `get_fee_rate_bps`:
- New `fee_rates` module with a `FeeRateCache` keyed by token_id (not condition_id — each outcome token can in principle carry its own fee).
- Lazily fetches `GET /fee-rate?token_id=<id>` and reads the `base_fee` key. Missing key → 0 (matches `result.get("base_fee") or 0`).
- HTTP failure bubbles as `ExecutorError::FeeRate` rather than poisoning the cache with 0; subsequent calls retry the network until a real value lands.
- The executor resolves the fee at `place_order` time and feeds it into `build_signed_order_http_request`. `BinaryMarket.fee_rate_bps` stays on the type for now (used by non-live test paths and `LegOrder` plumbing); the executor is authoritative in live mode.

Five tests cover fetch+cache behavior, the missing-key fallback, the HTTP-error-doesn't-cache rule, distinct-tokens-distinct-rates, and an integration test asserting the executor first hits `/fee-rate`, then sends `/order` with the fetched bps in the signed payload, and that the second call only re-hits `/order`.

### Startup allowance check (commit 3)
The `maker` on a signed Polymarket order is the funded address (EOA / Magic-Link proxy / Gnosis Safe). It must have approved both CTF Exchange contracts to move its USDC.e (ERC-20 `allowance`) and outcome tokens (CTF `isApprovedForAll`). Without those approvals on-chain, settlement reverts even though the CLOB cheerfully accepts the POST. This adds a startup probe so live mode fails at process boot rather than at first attempted fill.

- New `allowances` module: four Polygon JSON-RPC `eth_call`s (USDC + CTF, against the standard and neg-risk exchanges). Function selectors are inline constants (`0xdd62ed3e` = `allowance(address,address)`, `0xe985e9c5` = `isApprovedForAll(address,address)`); ABI encoding is two left-padded 20-byte addresses. No `ethers-rs` dependency — the dependency cost would dwarf the ~30 lines of encode/decode.
- USDC.e (`0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174`) and CTF (`0x4D97DCd97eC945f40cF65F87097ACe5EA0476045`) addresses are pinned for Polygon mainnet only. Other `chain_id`s return `UnsupportedChain`; we don't have the testnet equivalents pinned and silently passing with the wrong addresses would be worse than failing.
- `OrderExecutor::enforce_startup_allowances` is the convenience entry point. Threshold is `risk.max_total_exposure_usdc * 10^6` — strict enough to catch revoked-or-tiny approvals, but the standard MAX_UINT256 approval most users have always satisfies it.
- `arb-bot/main.rs` calls it in the Live branch right after `new_live`. On failure it logs a structured error per missing approval and exits non-zero. Dry-run is unaffected.
- New `polymarket.polygon_rpc_url` config key, default `https://polygon-rpc.com`. Users hitting the public RPC's rate limits override with their own provider.

Six tests cover non-Polygon chain rejection, happy-path with mocked RPC, missing CTF approval (both exchange rows flagged), insufficient USDC allowance (both flagged), and `enforce`'s Ok/Err semantics.

### Concept learned: reference-vector tests catch silent-failure bugs
The size-truncation bug existed for as long as the signed-order path did, and the existing test suite passed because the integration test happened to use `size=10.0` — no extra decimals to truncate. The first new test in this PR (`buy_amounts_size_truncates_to_two_dp`) uses `size=10.789`, which is the bug's exact failure shape, and only exists because we cross-referenced `py-clob-client`'s implementation before writing the test. Without that pin, the bug would have surfaced as a CLOB rejection in production — the worst place to discover a wire-format mismatch. Lesson reinforces the 2026-04-25 (afternoon) note about silent-default deserialization: cryptographic and wire-format code is not the place to write your own test inputs.

### State after today
- `cargo test --workspace` — 47 tests pass (was 33; +14 across the three commits). Zero new warnings from this PR.
- Branch: `feat/finish-live-execution`.
- The three blockers from the 2026-04-25 evening "what's next" list are closed. Live trading is now production-capable subject to the user having actually approved the contracts on Polygon (which the new startup check verifies).

### Recommended discipline before first live session
- Cap `risk.max_order_size_usdc` to a few dollars and `risk.max_total_exposure_usdc` to ~$10–20 for the first run. The new allowance check verifies approvals exist; it doesn't verify the wire format is right end-to-end against a real CLOB. A small-size shakedown is the cheapest way to learn if anything we missed bites.
- If using a private Polygon RPC, set `polymarket.polygon_rpc_url` explicitly — public-RPC rate limits will surface as `RPC` errors at startup, not as silent failures.

---

## 2026-05-31

### What we did
Started a dashboard to visualize what the bot is doing (for a blockchain-club demo). This PR is **Phase 0 + Phase 1** of that effort: the data-capture pipeline. The bot can now record its activity to a Supabase (Postgres) database; a React frontend that reads/streams from it comes in a later PR.

Decisions made up front: **Supabase Postgres + Realtime** for the data layer (built-in streaming, less backend code), capture the **full activity feed** (opportunities, dry-runs, fills — not just real fills, which are rare), and **React + Vite + Tailwind + shadcn/ui** for the eventual frontend.

### Schema (Phase 0)
`supabase/migrations/20260531_init_dashboard_schema.sql` is the exact migration applied to the project (ref `kawgriwaxfgvgcvyepjj`). Two tables:
- `activity` — append-only feed, one row per event (`opportunity_detected | dry_run | full_fill | partial_fill | no_fill | error`), with prices/size/spread/profit columns plus a `jsonb` `detail` holding the full serialized result for audit.
- `snapshots` — periodic bot state (balance, exposure, open positions) for time-series charts.

RLS is read-only for the anon/publishable key the frontend will use; the bot inserts with the `service_role` key, which bypasses RLS. Realtime is enabled on both tables. Supabase security advisor: clean.

### `arb-recorder` crate (Phase 1)
New crate, deliberately **additive**: the trading core (`arb-risk`, `arb-executor`) does not depend on it, and no HTTP dependency leaks into those crates. `Recorder` exposes `record_opportunity`, `record_execution`, `record_leg_execution`, `record_snapshot`. Every write is **fire-and-forget** (`tokio::spawn` → PostgREST insert) and failure-tolerant — a Supabase outage logs a warning and never blocks or crashes the trading path. When telemetry is disabled, or `SUPABASE_URL` / `SUPABASE_SERVICE_KEY` are absent, the recorder is a silent no-op, so existing behavior is unchanged.

Decimals are carried to the wire as `f64` (JSON numbers PostgREST inserts cleanly into `numeric` columns); the lossless original is preserved in the `detail` jsonb. Credentials come from the environment, never the committed config — the `service_role` key is server-side only and must never reach the frontend.

### Wiring
- `arb-config`: new optional `[telemetry]` section (`enabled`, default false).
- `arb-bot/main.rs`: build the recorder once, thread it through `try_simultaneous` / `try_asymmetric` exactly like `risk_manager` / `executor`. Emit an opportunity at detection (before risk, so risk-rejected opportunities still show), an execution event in each spawned task, and a state snapshot every 30s from the balance loop.
- `arb-risk`: added a read-only `open_positions()` accessor (no logic change) for the snapshot.

### State after today
- `cargo test --workspace` — green; +4 tests in `arb-recorder` (httpmock-backed insert assertions, plus the disabled/no-op paths).
- Branch: `feat/dashboard-telemetry`.
- **Not yet done:** end-to-end smoke test against the live project (needs the `service_role` key in `.env`), and the React dashboard itself (Phase 2+).

### Gotcha: don't run two cargo processes on one target dir
While verifying, a `cargo build` and `cargo test --workspace` ran concurrently and produced a flurry of scary `failed to create dependency graph … (os error 2)` / `could not compile` errors across unrelated crates. Those were incremental-compilation corruption from two cargo invocations sharing `target/`, not real errors — a single clean run was green. Also: `cargo … | tail` masks cargo's exit code (you get `tail`'s 0), so check the captured output, not just the exit status.

### Next steps
1. Add the `service_role` key to `.env`, flip `telemetry.enabled = true`, run the bot in dry-run, and confirm rows land in `activity` / `snapshots` (snapshot loop fires within seconds, so this doesn't depend on a real arb appearing).
2. Phase 2: scaffold the React + Vite dashboard, subscribe to `activity` via Realtime, render the live feed + status header.
3. Phase 3: KPI cards and charts (cumulative expected profit, per-market activity, exposure over time). Phase 4: seed/replay script + demo config so the screen is never empty during a live demo.

