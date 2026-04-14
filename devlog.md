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
