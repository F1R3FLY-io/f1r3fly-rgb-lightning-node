# Trait-Based RGB Key Provider Architecture

## Overview

This document outlines the long-term architectural plan to improve the RGB Lightning key management. The immediate fix (file-based pubkey passing) has been implemented, removing the need for the `FIREFLY_PRIVATE_KEY` environment variable. This document describes the next evolution: a trait-based callback system for cleaner architecture.

## Current State (Implemented)

### File-Based Solution (Option 1) - COMPLETED

The `FIREFLY_PRIVATE_KEY` environment variable has been **removed**. The current implementation uses file-based pubkey passing:

1. **Adapter writes pubkey at init** (`f1r3fly_rgb_adapter.rs`):
   ```rust
   // Write wallet's F1r3fly master public key to file for rust-lightning to access.
   let pubkey_file = ldk_data_dir.join("my_wallet_pubkey");
   std::fs::write(&pubkey_file, &pubkey_hex)?;
   ```

2. **rust-lightning reads from file** (`rgb_utils/mod.rs`):
   ```rust
   let pubkey_file = ldk_data_dir.join("my_wallet_pubkey");
   let wallet_pubkey = fs::read_to_string(&pubkey_file)?;
   ```

This works and removes the environment variable dependency, but has limitations.

### Limitations of File-Based Approach

1. **File I/O overhead** - Reading file on every operation
2. **No private key access** - Only public key is shared; signing operations can't use this pattern
3. **Single wallet assumption** - File path is fixed, doesn't support multi-wallet scenarios
4. **Initialization order dependency** - Adapter must write before rust-lightning reads

## Problem Statement (Historical)

Previously, `rust-lightning/lightning/src/rgb_utils/mod.rs` used this workaround:

```rust
// WORKAROUND: Read the FIREFLY_PRIVATE_KEY from environment and derive the pubkey
let master_key_hex = env::var("FIREFLY_PRIVATE_KEY")
    .map_err(|_| RgbLibError::Other("FIREFLY_PRIVATE_KEY not set".to_string()))?;
```

### Issues with Environment Variable Approach (Now Fixed)

1. **Shared key across all wallets** - Violated per-wallet isolation
2. **Environment variable dependency** - Required static configuration
3. **Security risk** - Private key in environment could be leaked
4. **Testing complexity** - Hard to test with different keys

## Proposed Solution: Trait-Based Callback

Define a trait in `rust-lightning` that the hosting application (`f1r3fly-rgb-lightning-node`) implements to provide wallet keys on demand.

### Architecture Diagram

```
┌─────────────────────────────────────────────────────────────────┐
│                  f1r3fly-rgb-lightning-node                     │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │              F1r3flyKeyProvider (implements trait)        │  │
│  │  ┌─────────────────────────────────────────────────────┐  │  │
│  │  │  WalletManager → F1r3flyContracts → Executor        │  │  │
│  │  │                  (per-wallet keys)                   │  │  │
│  │  └─────────────────────────────────────────────────────┘  │  │
│  └───────────────────────────────────────────────────────────┘  │
│                              │                                   │
│                              │ Arc<dyn RgbWalletKeyProvider>     │
│                              ▼                                   │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │                      rust-lightning                        │  │
│  │  ┌─────────────────────────────────────────────────────┐  │  │
│  │  │  Wallet { key_provider: Arc<dyn RgbWalletKeyProvider>│  │  │
│  │  │          ldk_data_dir: PathBuf }                     │  │  │
│  │  └─────────────────────────────────────────────────────┘  │  │
│  │                              │                             │  │
│  │                              ▼                             │  │
│  │  ┌─────────────────────────────────────────────────────┐  │  │
│  │  │  post_acceptor_pubkey_after_accept()                │  │  │
│  │  │  → key_provider.get_master_public_key_hex()         │  │  │
│  │  └─────────────────────────────────────────────────────┘  │  │
│  └───────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
```

## Implementation Plan

### Phase 1: Define Trait in rust-lightning

**File:** `rust-lightning/lightning/src/rgb_utils/mod.rs`

```rust
/// Trait for providing wallet keys to RGB operations.
/// 
/// Implemented by the hosting application to provide per-wallet key access
/// without requiring rust-lightning to depend on rgb-satchel directly.
pub trait RgbWalletKeyProvider: Send + Sync {
    /// Get the master F1r3fly public key (130 hex chars, uncompressed secp256k1).
    /// Used for registering witness ownership during RGB channel operations.
    fn get_master_public_key_hex(&self) -> Result<String, RgbLibError>;
    
    /// Get the master F1r3fly private key for claim signing (optional).
    /// Only needed for claim operations, not for channel opening.
    fn get_master_private_key(&self) -> Result<secp256k1::SecretKey, RgbLibError>;
    
    /// Get the REV address derived from the master public key.
    /// Used for phlo payments on F1r3node.
    fn get_rev_address(&self) -> Result<String, RgbLibError>;
}
```

**Estimated changes:** ~15 lines

### Phase 2: Modify Wallet Struct

**File:** `rust-lightning/lightning/src/rgb_utils/mod.rs`

```rust
pub struct Wallet {
    ldk_data_dir: PathBuf,
    /// Optional key provider - if None, falls back to file-based lookup
    key_provider: Option<Arc<dyn RgbWalletKeyProvider>>,
}

impl Wallet {
    /// Create wallet with key provider (preferred)
    pub fn with_key_provider(
        ldk_data_dir: PathBuf, 
        key_provider: Arc<dyn RgbWalletKeyProvider>
    ) -> Self {
        Self { ldk_data_dir, key_provider: Some(key_provider) }
    }
    
    /// Create wallet without key provider (falls back to file)
    pub fn new(ldk_data_dir: PathBuf) -> Self {
        Self { ldk_data_dir, key_provider: None }
    }
    
    /// Get master public key - tries provider first, then file fallback
    fn get_master_public_key_hex(&self) -> Result<String, RgbLibError> {
        if let Some(ref provider) = self.key_provider {
            provider.get_master_public_key_hex()
        } else {
            // Fallback: read from file (Option 1 implementation)
            let pubkey_file = self.ldk_data_dir.join("my_wallet_pubkey");
            fs::read_to_string(&pubkey_file)
                .map_err(|_| RgbLibError::Other("Wallet pubkey not found".into()))
        }
    }
}
```

**Estimated changes:** ~35 lines

### Phase 3: Update Internal Functions

**File:** `rust-lightning/lightning/src/rgb_utils/mod.rs`

Update `post_acceptor_pubkey_after_accept()` to use the Wallet method:

```rust
fn post_acceptor_pubkey_after_accept(
    wallet: &Wallet,  // Changed from ldk_data_dir: &Path
    contract_id: &str,
    proxy_url: &str,
) -> Result<(), RgbLibError> {
    // Use wallet's method instead of env::var
    let wallet_pubkey = wallet.get_master_public_key_hex()?;
    
    // ... rest of function unchanged
}
```

This requires updating the call chain:

| Function                            | File             | Change                              |
| ----------------------------------- | ---------------- | ----------------------------------- |
| `post_acceptor_pubkey_after_accept` | rgb_utils/mod.rs | Accept `&Wallet` instead of `&Path` |
| `accept_inbound_channel`            | rgb_utils/mod.rs | Pass wallet reference               |
| `handle_funding`                    | rgb_utils/mod.rs | Pass wallet reference               |

**Estimated changes:** ~30 lines

### Phase 4: Thread Provider Through Channel Layer

The most complex part - the key provider needs to be accessible from channel operations.

**Option A: Store in Wallet (simpler)**

Since `Wallet` is already created with `ldk_data_dir`, store the provider there and pass wallet reference through the call chain.

**Option B: Store in ChannelManager (more invasive)**

Add to `ChannelManager`:

```rust
pub struct ChannelManager<...> {
    // ... existing fields
    rgb_key_provider: Option<Arc<dyn RgbWalletKeyProvider>>,
}
```

**Recommendation:** Option A is simpler and contained within `rgb_utils`.

**Estimated changes:** ~40 lines

### Phase 5: Implement Trait in Adapter

**File:** `f1r3fly-rgb-lightning-node/src/f1r3fly_rgb_adapter.rs`

```rust
use lightning::rgb_utils::RgbWalletKeyProvider;

/// Key provider implementation backed by WalletManager
pub struct F1r3flyKeyProvider {
    wallet_manager: Arc<Mutex<WalletManager>>,
}

impl F1r3flyKeyProvider {
    pub fn new(wallet_manager: Arc<Mutex<WalletManager>>) -> Self {
        Self { wallet_manager }
    }
}

impl RgbWalletKeyProvider for F1r3flyKeyProvider {
    fn get_master_public_key_hex(&self) -> Result<String, RgbLibError> {
        let mgr = self.wallet_manager.lock()
            .map_err(|_| RgbLibError::Other("Lock poisoned".into()))?;
        
        mgr.f1r3fly_contracts()
            .ok_or_else(|| RgbLibError::Other("F1r3fly not initialized".into()))?
            .contracts()
            .executor()
            .get_master_public_key_hex()
            .map_err(|e| RgbLibError::Other(e.to_string()))
    }
    
    fn get_master_private_key(&self) -> Result<secp256k1::SecretKey, RgbLibError> {
        let mgr = self.wallet_manager.lock()
            .map_err(|_| RgbLibError::Other("Lock poisoned".into()))?;
        
        mgr.f1r3fly_contracts()
            .ok_or_else(|| RgbLibError::Other("F1r3fly not initialized".into()))?
            .contracts()
            .executor()
            .get_master_key()
            .map_err(|e| RgbLibError::Other(e.to_string()))
    }
    
    fn get_rev_address(&self) -> Result<String, RgbLibError> {
        let mgr = self.wallet_manager.lock()
            .map_err(|_| RgbLibError::Other("Lock poisoned".into()))?;
        
        mgr.f1r3fly_contracts()
            .ok_or_else(|| RgbLibError::Other("F1r3fly not initialized".into()))?
            .contracts()
            .executor()
            .get_rev_address()
            .map_err(|e| RgbLibError::Other(e.to_string()))
    }
}
```

**Estimated changes:** ~50 lines

### Phase 6: Wire Up at Initialization

**File:** `f1r3fly-rgb-lightning-node/src/f1r3fly_rgb_adapter.rs`

When creating the RGB wallet wrapper:

```rust
impl F1r3flyRgbWalletWrapper {
    pub fn new(data_dir: PathBuf, wallet_manager: Arc<Mutex<WalletManager>>) -> Self {
        let key_provider = Arc::new(F1r3flyKeyProvider::new(wallet_manager.clone()));
        let ldk_data_dir = data_dir.join(".ldk");
        
        // Create wallet with key provider
        let rgb_wallet = Wallet::with_key_provider(ldk_data_dir, key_provider);
        
        // ... rest of initialization
    }
}
```

**Estimated changes:** ~15 lines

## Summary

| Phase     | Component                    | Lines    | Complexity |
| --------- | ---------------------------- | -------- | ---------- |
| 1         | Trait definition             | ~15      | Low        |
| 2         | Wallet struct changes        | ~35      | Low        |
| 3         | Internal function updates    | ~30      | Medium     |
| 4         | Call chain threading         | ~40      | Medium     |
| 5         | Adapter trait implementation | ~50      | Low        |
| 6         | Initialization wiring        | ~15      | Low        |
| **Total** |                              | **~185** | **Medium** |

## Migration Strategy

1. ~~**Implement Option 1 first** (file-based) as a quick fix~~ **COMPLETED**
2. **Phase 1-2** can be done without breaking existing code (add trait, make provider optional)
3. **Phase 3-4** update internals to use provider when available
4. **Phase 5-6** implement in adapter
5. **Remove file-based fallback** once trait implementation is stable

## Testing Strategy

1. **Unit tests for trait** - Mock implementation that returns fixed keys
2. **Integration tests** - Full channel open/close with real keys
3. **Backward compatibility** - Ensure file-based fallback still works during migration

## Dependencies

This plan requires:
- `rgbl1` crate exposes `get_master_public_key_hex()` on executor (already done)
- `rgb-satchel` exposes `WalletManager` with F1r3fly contracts access (already done)

## Timeline Estimate

- **Phase 1-2:** 1 day
- **Phase 3-4:** 2-3 days (call chain analysis and threading)
- **Phase 5-6:** 1 day
- **Testing:** 2 days

**Total: ~1 week**

## Future Enhancements

Once the trait is in place, it can be extended for:
- Multi-wallet support (different keys per channel)
- Key rotation
- Hardware wallet integration
- Remote signing service

