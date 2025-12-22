# F1r3fly RGB Lightning Node - Open Issues

---

## Issue 1: Document REV Auto-Funding Mechanism

**Priority:** Low  
**Type:** Documentation

### Description
The REV auto-funding mechanism has been implemented in `f1r3fly_rgb_adapter.rs`. This automatically funds wallets with REV tokens during initialization on regtest, eliminating manual faucet calls.

### Action Required
- Review and finalize `docs/REV_Auto_Funding.md`
- Add link to DEVELOPER.md
- Ensure `.env.example` documents all required variables

### Reference
See: [docs/REV_Auto_Funding.md](REV_Auto_Funding.md)

---

## Issue 2: Implement Trait-Based Key Provider

**Priority:** Medium  
**Type:** Enhancement

### Description
The current file-based pubkey passing (`my_wallet_pubkey` file) works but has limitations. A trait-based callback system would provide cleaner architecture, better testability, and support for multi-wallet scenarios.

### Action Required
- Implement `RgbWalletKeyProvider` trait in rust-lightning
- Add `F1r3flyKeyProvider` implementation in adapter
- Thread provider through channel layer
- Remove file-based fallback once stable

### Estimate
~1 week (185 lines across both repos)

### Reference
See: [docs/Trait_Based_Key_Provider_Plan.md](Trait_Based_Key_Provider_Plan.md)

---

## Issue 3: Fix Channel Closing Settlement

**Priority:** High  
**Type:** Bug

### Description
During cooperative channel close, the closing transaction fails signature validation:
```
EVENT: Channel closed due to: ProcessingError { err: "Invalid closing tx signature from peer" }
```

The `execute_settlement()` function is called but the resulting PSBT fails extraction with `MissingInputValue`. The channel then force-closes instead of cooperating.

### Action Required
- Debug `color_psbt` for closing transactions
- Ensure witness UTXOs are properly populated in PSBT
- Verify settlement execution returns valid signed transaction
- Test cooperative close completes without force-close fallback

---

## Issue 4: Enable On-Chain RGB Transfer Tests

**Priority:** Medium  
**Type:** Enhancement

### Description
The `payment::success` test has commented-out on-chain RGB transfer tests (lines 139-216) that require `blind_receive()` / `rgb_invoice()` implementation.

### Blocked By
- `rgb_invoice()` endpoint implementation
- `send_asset()` for on-chain transfers
- `refresh_transfers()` for consignment processing

### Action Required
- Implement `blind_receive()` in F1r3fly adapter
- Wire up `rgb_invoice` route
- Uncomment and verify on-chain transfer tests
- Ensure node3 receives assets after settlement

### Reference
See: `src/test/payment.rs` lines 134-216

