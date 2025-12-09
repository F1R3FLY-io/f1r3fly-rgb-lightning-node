//! F1r3fly RGB Wallet Adapter
//!
//! Adapter between rgb-lightning-node's API and F1r3fly-RGB components.
//! This replaces rgb-lib's Wallet with F1r3fly's WalletManager.

use lightning::rgb_utils::{
    wallet, AssetCFA, AssetNIA, AssetSchema, AssetUDA, Assets, Assignment, Balance, BitcoinNetwork,
    BtcBalance, Metadata, Online, OperationResult, ReceiveData, Recipient, RefreshResult,
    RgbLibError, RgbLibTransaction, RgbTransfer, SettlementExecutor, SignOptions, Transfer, UpdateRes,
    Unspent, WitnessOrd,
};

use bitcoin::Txid as RgbTxid;

use bitcoin::ScriptBuf;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use f1r3fly_rgb_wallet::config::GlobalConfig;
use f1r3fly_rgb_wallet::manager::WalletManager;

/// Adapter wrapper for F1r3fly RGB wallet
///
/// Provides rgb-lib-compatible API while using F1r3fly components underneath.
pub struct F1r3flyRgbWalletWrapper {
    /// Main wallet manager (handles BTC, RGB, F1r3fly coordination)
    wallet_manager: Arc<Mutex<WalletManager>>,

    /// Wallet name (for file paths and lookups)
    #[allow(dead_code)]
    wallet_name: String,

    /// Data directory path
    data_dir: PathBuf,

    /// Bitcoin network
    bitcoin_network: BitcoinNetwork,
}

impl F1r3flyRgbWalletWrapper {
    /// Create new F1r3fly RGB wallet wrapper
    ///
    /// Initializes WalletManager and loads/imports the wallet from mnemonic.
    /// Uses empty password since rgb-lib doesn't use password encryption.
    pub async fn new(wallet_data: wallet::WalletData) -> Result<Self, RgbLibError> {
        eprintln!("\n🚀 DEBUG: F1r3flyRgbWalletWrapper::new() called");
        eprintln!("   data_dir: {}", wallet_data.data_dir.display());
        
        // Convert BitcoinNetwork to f1r3fly NetworkType
        let network = match wallet_data.bitcoin_network {
            BitcoinNetwork::Mainnet => f1r3fly_rgb_wallet::config::NetworkType::Mainnet,
            BitcoinNetwork::Testnet => f1r3fly_rgb_wallet::config::NetworkType::Testnet,
            BitcoinNetwork::Testnet4 => f1r3fly_rgb_wallet::config::NetworkType::Testnet, // Map to Testnet
            BitcoinNetwork::Signet => f1r3fly_rgb_wallet::config::NetworkType::Signet,
            BitcoinNetwork::Regtest => f1r3fly_rgb_wallet::config::NetworkType::Regtest,
        };

        // Create global config
        let master_key = std::env::var("FIREFLY_PRIVATE_KEY")
            .unwrap_or_else(|_| "0000000000000000000000000000000000000000000000000000000000000000".to_string());

        let config = GlobalConfig {
            bitcoin: f1r3fly_rgb_wallet::config::BitcoinConfig {
                network,
                esplora_url: "http://localhost:3002".to_string(), // Electrs HTTP API (Esplora backend)
            },
            f1r3node: f1r3fly_rgb_wallet::config::F1r3nodeConfig {
                host: "localhost".to_string(),
                grpc_port: 40401,
                http_port: 40403,
                master_key,
            },
            wallets_dir: Some(wallet_data.data_dir.to_string_lossy().to_string()),
        };

        eprintln!("   Creating WalletManager...");
        // Create wallet manager
        let mut manager = WalletManager::new(config)
            .map_err(|e| RgbLibError::Other(format!("Failed to create WalletManager: {}", e)))?;

        // Wallet name derived from fingerprint or default
        let wallet_name = "rgb-lightning-wallet".to_string();
        eprintln!("   wallet_name: {}", wallet_name);

        // Try to load existing wallet, or import if it doesn't exist
        let password = ""; // rgb-lib doesn't use passwords

        eprintln!("   Attempting to load existing wallet...");
        let load_result = manager.load_wallet(&wallet_name, password);
        if load_result.is_err() {
            eprintln!("   ⚠️  Wallet doesn't exist, importing from mnemonic...");
            // Wallet doesn't exist, import it
            manager
                .import_wallet(&wallet_name, &wallet_data.mnemonic, password)
                .map_err(|e| RgbLibError::Other(format!("Failed to import wallet: {}", e)))?;

            manager
                .load_wallet(&wallet_name, password)
                .map_err(|e| RgbLibError::Other(format!("Failed to load wallet: {}", e)))?;
            eprintln!("   ✅ Wallet imported and loaded");
        } else {
            eprintln!("   ✅ Existing wallet loaded successfully");
        }

        // Check what contracts are loaded
        if let Some(contracts_mgr) = manager.f1r3fly_contracts() {
            let contract_list = contracts_mgr.contracts().list();
            eprintln!("   📋 Loaded contracts: {} contract(s)", contract_list.len());
            for contract_id in contract_list {
                eprintln!("      - {}", contract_id);
                
                // Show genesis UTXO for this contract
                if let Some(genesis) = contracts_mgr.get_genesis_utxo(&contract_id.to_string()) {
                    eprintln!("         Genesis UTXO: {}:{}", genesis.txid, genesis.vout);
                }
            }
        } else {
            eprintln!("   ⚠️  F1r3fly contracts manager not initialized");
        }

        eprintln!("✅ DEBUG: F1r3flyRgbWalletWrapper created successfully\n");

        Ok(Self {
            wallet_manager: Arc::new(Mutex::new(manager)),
            wallet_name,
            data_dir: wallet_data.data_dir,
            bitcoin_network: wallet_data.bitcoin_network,
        })
    }

    /// Check if a transaction exists in our Bitcoin wallet
    /// Returns true if we have this TX (i.e., we created it), false otherwise
    pub fn has_transaction(&self, txid: bitcoin::Txid) -> bool {
        let manager = self.wallet_manager.lock().unwrap();
        if let Some(bitcoin_wallet) = manager.bitcoin_wallet() {
            bitcoin_wallet.inner().get_tx(txid).is_some()
        } else {
            false
        }
    }

    /// Go online and sync with blockchain
    pub fn go_online(
        &self,
        _skip_consistency_check: bool,
        _indexer_url: String,
    ) -> Result<Online, RgbLibError> {
        let manager = self.wallet_manager.clone();

        eprintln!("🌐 DEBUG: go_online() called - syncing with blockchain");
        
        // Sync the wallet (async operation in blocking context)
        tokio::task::block_in_place(|| {
            futures::executor::block_on(async {
                let mut mgr = manager.lock().unwrap();
                mgr.sync_wallet()
                    .await
                    .map_err(|e| RgbLibError::Other(format!("Sync failed: {}", e)))?;
                
                // Show wallet state after going online
                if let Some(btc_wallet) = mgr.bitcoin_wallet() {
                    let utxos: Vec<_> = btc_wallet.inner().list_unspent().collect();
                    eprintln!("   Wallet has {} UTXO(s) after sync", utxos.len());
                    for (idx, utxo) in utxos.iter().enumerate() {
                        eprintln!("      UTXO {}: {}:{} ({}sats)", 
                            idx + 1,
                            utxo.outpoint.txid, 
                            utxo.outpoint.vout,
                            utxo.txout.value.to_sat());
                    }
                }
                
                if let Some(contracts_mgr) = mgr.f1r3fly_contracts() {
                    let contract_count = contracts_mgr.contracts().list().len();
                    eprintln!("   Wallet has {} RGB contract(s) loaded", contract_count);
                }
                
                eprintln!("✅ DEBUG: go_online() completed\n");
                Ok(Online) // Return Online token
            })
        })
    }

    // ========================================================================
    // Asset Operations
    // ========================================================================

    pub fn issue_asset_nia(
        &self,
        ticker: String,
        name: String,
        precision: u8,
        amounts: Vec<u64>,
    ) -> Result<AssetNIA, RgbLibError> {
        let manager = self.wallet_manager.clone();

        tokio::task::block_in_place(|| {
            futures::executor::block_on(async {
                let mut mgr = manager.lock().unwrap();

                // Sum amounts for total supply
                let total_supply: u64 = amounts.iter().sum();

                // CRITICAL: Sync wallet before UTXO selection
                // This ensures we have the latest UTXO state after any recent operations
                // (e.g., createutxos that may have just spent UTXOs)
                eprintln!("🔄 DEBUG: Syncing wallet before genesis UTXO selection...");
                mgr.sync_wallet()
                    .await
                    .map_err(|e| RgbLibError::Other(format!("Sync before issuance failed: {}", e)))?;

                // Get available UTXOs for genesis selection
                let filter = f1r3fly_rgb_wallet::types::UtxoFilter {
                    available_only: true, // Only non-RGB UTXOs
                    rgb_only: false,
                    confirmed_only: true, // Need confirmed UTXO
                    min_amount_sats: Some(1000), // Need at least dust amount
                };

                let utxos = mgr
                    .list_utxos(filter)
                    .await
                    .map_err(|e| RgbLibError::Other(format!("Failed to list UTXOs: {}", e)))?;

                // Genesis UTXO Selection Strategy:
                // We simply select the first available UTXO from the list. This is a basic
                // strategy that works for testing and initial implementation.
                //
                // IMPROVEMENT OPPORTUNITIES:
                // - Select UTXO with optimal amount (not too small, not too large)
                // - Prioritize older/more confirmed UTXOs for stability
                // - Allow caller to specify genesis UTXO for advanced use cases
                // - Select based on UTXO "quality" metrics (confirmations, amount, etc.)
                let genesis_utxo = utxos
                    .first()
                    .ok_or(RgbLibError::InsufficientBitcoins {
                        needed: 1000,
                        available: 0,
                    })?;

                eprintln!("🎯 DEBUG: Issuing RGB asset with {} available UTXOs", utxos.len());
                eprintln!("   Selected genesis UTXO: {}", genesis_utxo.outpoint);
                for (idx, utxo) in utxos.iter().enumerate() {
                    eprintln!("      UTXO {}: {}", idx, utxo.outpoint);
                }

                // Create issuance request
                let request = f1r3fly_rgb_wallet::f1r3fly::IssueAssetRequest {
                    ticker: ticker.clone(),
                    name: name.clone(),
                    supply: total_supply,
                    precision,
                    genesis_utxo: genesis_utxo.outpoint.clone(),
                };

                // Issue the asset
                let asset_info = mgr
                    .issue_asset(request)
                    .await
                    .map_err(|e| RgbLibError::FailedIssuance {
                        details: format!("F1r3fly issuance failed: {}", e),
                    })?;

                eprintln!("✅ DEBUG: Asset issued successfully");
                eprintln!("   Contract ID: {}", asset_info.contract_id);
                
                // Check what F1r3fly recorded as the genesis UTXO
                if let Some(contracts_mgr) = mgr.f1r3fly_contracts() {
                    if let Some(genesis) = contracts_mgr.get_genesis_utxo(&asset_info.contract_id) {
                        eprintln!("   F1r3fly stored genesis UTXO: {}:{}", genesis.txid, genesis.vout);
                    }
                }

                // Get current balance
                let balance_info = mgr
                    .get_asset_balance(&asset_info.contract_id)
                    .await
                    .map_err(|e| RgbLibError::Other(format!("Failed to get balance: {}", e)))?;

                // Convert to rgb-lib AssetNIA format
                Ok(AssetNIA {
                    asset_id: asset_info.contract_id,
                    added_at: chrono::Utc::now().timestamp(),
                    balance: Balance {
                        settled: balance_info.total,
                        future: balance_info.total,
                        spendable: balance_info.total,
                    },
                    details: None,
                    issued_supply: asset_info.supply,
                    ticker,
                    name,
                    precision,
                    timestamp: chrono::Utc::now().timestamp(),
                    media: None,
                })
            })
        })
    }

    pub fn issue_asset_cfa(
        &self,
        name: String,
        _details: Option<String>,
        precision: u8,
        amounts: Vec<u64>,
        _file_path: Option<String>,
    ) -> Result<AssetCFA, RgbLibError> {
        // CFA uses same flow as NIA for F1r3fly
        let asset_nia = self.issue_asset_nia(name.clone(), name, precision, amounts)?;

        // Convert to AssetCFA
        Ok(AssetCFA {
            asset_id: asset_nia.asset_id,
            added_at: asset_nia.added_at,
            balance: asset_nia.balance,
            details: None,
            issued_supply: asset_nia.issued_supply,
            ticker: asset_nia.ticker,
            name: asset_nia.name,
            precision: asset_nia.precision,
            timestamp: asset_nia.timestamp,
            media: None,
        })
    }

    pub fn issue_asset_uda(
        &self,
        _ticker: String,
        _name: String,
        _details: Option<String>,
        _precision: u8,
        _media_file_path: Option<String>,
        _attachments_file_paths: Vec<String>,
    ) -> Result<AssetUDA, RgbLibError> {
        // UDA is not directly supported in F1r3fly-RGB v1
        Err(RgbLibError::UnsupportedSchema {
            asset_schema: AssetSchema::Uda,
        })
    }

    pub fn get_asset_balance(&self, asset_id: String) -> Result<Balance, RgbLibError> {
        let manager = self.wallet_manager.clone();

        tokio::task::block_in_place(|| {
            futures::executor::block_on(async {
                let mut mgr = manager.lock().unwrap();

                eprintln!("🔍 DEBUG: get_asset_balance called for asset_id: {}", asset_id);
                
                // Show which UTXOs the BDK wallet has
                if let Some(btc_wallet) = mgr.bitcoin_wallet() {
                    let utxos: Vec<_> = btc_wallet.inner().list_unspent().collect();
                    eprintln!("   BDK wallet has {} UTXOs to check:", utxos.len());
                    for utxo in utxos.iter().take(5) {  // Show first 5
                        eprintln!("      - {}:{}", utxo.outpoint.txid, utxo.outpoint.vout);
                    }
                    if utxos.len() > 5 {
                        eprintln!("      ... and {} more", utxos.len() - 5);
                    }
                }
                
                let balance = mgr
                    .get_asset_balance(&asset_id)
                    .await
                    .map_err(|e| {
                        eprintln!("❌ DEBUG: get_asset_balance ERROR: {}", e);
                        RgbLibError::Other(format!("Failed to get balance: {}", e))
                    })?;

                eprintln!("✅ DEBUG: F1r3fly returned balance.total = {}", balance.total);
                eprintln!("   DEBUG: F1r3fly ticker: {}, name: {}", balance.ticker, balance.name);
                eprintln!("   DEBUG: F1r3fly UTXO balances count: {}", balance.utxo_balances.len());
                
                // Show which UTXOs have RGB allocations
                for utxo_bal in &balance.utxo_balances {
                    eprintln!("      RGB UTXO: {} = {} tokens", utxo_bal.outpoint, utxo_bal.amount);
                }

                // Convert f1r3fly AssetBalance to rgb-lib Balance
                Ok(Balance {
                    settled: balance.total,
                    future: balance.total,
                    spendable: balance.total,
                })
            })
        })
    }

    pub fn get_asset_metadata(&self, asset_id: String) -> Result<Metadata, RgbLibError> {
        let manager = self.wallet_manager.clone();

        tokio::task::block_in_place(|| {
            futures::executor::block_on(async {
                let mgr = manager.lock().unwrap();

                let asset_info = mgr
                    .get_asset_info(&asset_id)
                    .map_err(|e| RgbLibError::Other(format!("Failed to get metadata: {}", e)))?;

                // Convert f1r3fly AssetInfo to rgb-lib Metadata
                Ok(Metadata {
                    asset_iface: "RGB20".to_string(),
                    asset_schema: AssetSchema::Nia,
                    issued_supply: asset_info.supply,
                    timestamp: 0,
                    name: asset_info.name,
                    precision: asset_info.precision,
                    ticker: Some(asset_info.ticker),
                    details: None,
                    initial_supply: asset_info.supply,
                    max_supply: asset_info.supply,
                    known_circulating_supply: asset_info.supply,
                    token: None,
                })
            })
        })
    }

    pub fn list_assets(&self, _filter_asset_schemas: Vec<AssetSchema>) -> Result<Assets, RgbLibError> {
        let manager = self.wallet_manager.clone();

        tokio::task::block_in_place(|| {
            futures::executor::block_on(async {
                let mut mgr = manager.lock().unwrap();

                eprintln!("📋 DEBUG: list_assets() called");
                
                let assets = mgr
                    .list_assets()
                    .map_err(|e| RgbLibError::Other(format!("Failed to list assets: {}", e)))?;

                eprintln!("   Found {} asset(s) from F1r3fly", assets.len());

                // Convert each asset to NIA format
                // Note: F1r3fly only supports NIA (fungible tokens) for now
                let mut nia_assets = vec![];

                for asset_item in assets {
                    // Scope to get asset info and balance, then release lock
                    let (asset_info, balance_info) = {
                        // Get detailed info for this asset
                        let asset_info = mgr
                            .get_asset_info(&asset_item.contract_id)
                            .map_err(|e| RgbLibError::Other(format!("Failed to get asset info: {}", e)))?;

                        // Get balance from F1r3node
                        let balance_info = mgr
                            .get_asset_balance(&asset_item.contract_id)
                            .await
                            .map_err(|e| RgbLibError::Other(format!("Failed to get balance: {}", e)))?;
                        
                        (asset_info, balance_info)
                    };

                    // Get locked amount in channels (Phase 3)
                    // The mgr lock is still held, but get_locked_amount only reads filesystem
                    let locked_amount = self.get_locked_amount(&asset_item.contract_id)
                        .unwrap_or(0); // If error, assume 0 locked
                    
                    eprintln!("   Contract {}: total={}, locked={}, spendable={}",
                        asset_item.contract_id, balance_info.total, locked_amount, balance_info.total.saturating_sub(locked_amount));

                    // Convert to AssetNIA with locked amounts subtracted from spendable
                    let asset_nia = AssetNIA {
                        asset_id: asset_info.contract_id.clone(),
                        added_at: chrono::Utc::now().timestamp(), // F1r3fly doesn't track this
                        balance: Balance {
                            settled: balance_info.total,
                            future: balance_info.total,
                            spendable: balance_info.total.saturating_sub(locked_amount), // Phase 3: subtract locked
                        },
                        details: None,
                        issued_supply: asset_info.supply,
                        ticker: asset_info.ticker.clone(),
                        name: asset_info.name.clone(),
                        precision: asset_info.precision,
                        timestamp: chrono::Utc::now().timestamp(),
                        media: None,
                    };

                    nia_assets.push(asset_nia);
                }

                Ok(Assets {
                    nia: Some(nia_assets),
                    cfa: Some(vec![]), // Not supported yet
                    uda: Some(vec![]), // Not supported yet
                })
            })
        })
    }

    // ========================================================================
    // Bitcoin/UTXO Operations
    // ========================================================================

    pub fn get_address(&self) -> Result<String, RgbLibError> {
        let manager = self.wallet_manager.clone();

        tokio::task::block_in_place(|| {
            let mut mgr = manager.lock().unwrap();
            mgr.get_new_address()
                .map_err(|e| RgbLibError::Other(format!("Failed to get address: {}", e)))
        })
    }

    pub fn get_btc_balance(&self, _online: Option<Online>, _skip_sync: bool) -> Result<BtcBalance, RgbLibError> {
        let manager = self.wallet_manager.clone();

        tokio::task::block_in_place(|| {
            let mgr = manager.lock().unwrap();

            let balance = mgr
                .get_balance()
                .map_err(|e| RgbLibError::Other(format!("Failed to get BTC balance: {}", e)))?;

            // Convert to rgb-lib BtcBalance format
            Ok(BtcBalance {
                vanilla: lightning::rgb_utils::Balance {
                    settled: balance.confirmed,
                    future: balance.confirmed + balance.unconfirmed,
                    spendable: balance.confirmed,
                },
                colored: lightning::rgb_utils::Balance {
                    settled: 0,
                    future: 0,
                    spendable: 0,
                },
            })
        })
    }

    pub fn create_utxos(
        &self,
        _online: Online,
        _up_to: bool,
        num: Option<u8>,
        size: Option<u32>,
        fee_rate: u64,
        skip_sync: bool,
    ) -> Result<u8, RgbLibError> {
        let manager = self.wallet_manager.clone();
        let count = num.unwrap_or(1) as usize;
        let amount_per_utxo = size.unwrap_or(1000) as u64;

        tokio::task::block_in_place(|| {
            futures::executor::block_on(async {
                // Sync wallet first unless skip_sync is true
                if !skip_sync {
                    let mut mgr = manager.lock().unwrap();
                    mgr.sync_wallet()
                        .await
                        .map_err(|e| RgbLibError::Other(format!("Sync failed: {}", e)))?;
                    drop(mgr);
                }

                let mut mgr = manager.lock().unwrap();

                // Get list of RGB-occupied UTXOs to exclude from coin selection
                // OPTIMIZATION: Cache this list if called multiple times in quick succession
                let occupied_utxos = mgr
                    .get_occupied_utxos()
                    .await
                    .map_err(|e| RgbLibError::Other(format!("Failed to get occupied UTXOs: {}", e)))?;

                eprintln!("   🔒 Excluding {} RGB-occupied UTXO(s) from coin selection", occupied_utxos.len());

                // Get Bitcoin wallet
                let bitcoin_wallet = mgr
                    .bitcoin_wallet_mut()
                    .ok_or_else(|| RgbLibError::Other("Bitcoin wallet not loaded".to_string()))?;

                // Generate all addresses first (before building tx)
                let mut addresses = Vec::new();
                for _ in 0..count {
                    let address_info = bitcoin_wallet.inner_mut().reveal_next_address(bdk_wallet::KeychainKind::External);
                    addresses.push(address_info.address.script_pubkey());
                }

                // Build a single transaction with multiple outputs
                let mut tx_builder = bitcoin_wallet.inner_mut().build_tx();

                // Exclude RGB-occupied UTXOs from being spent (prevents losing RGB tokens)
                for rgb_utxo in &occupied_utxos {
                    // Parse outpoint string (format: "txid:vout")
                    let parts: Vec<&str> = rgb_utxo.outpoint.split(':').collect();
                    if parts.len() != 2 {
                        eprintln!("⚠️  Invalid outpoint format: {}", rgb_utxo.outpoint);
                        continue;
                    }
                    
                    let txid = bdk_wallet::bitcoin::Txid::from_str(parts[0])
                        .map_err(|e| RgbLibError::Other(format!("Invalid txid in {}: {}", rgb_utxo.outpoint, e)))?;
                    let vout: u32 = parts[1]
                        .parse()
                        .map_err(|e| RgbLibError::Other(format!("Invalid vout in {}: {}", rgb_utxo.outpoint, e)))?;
                    
                    let outpoint = bdk_wallet::bitcoin::OutPoint { txid, vout };
                    tx_builder.add_unspendable(outpoint);
                }

                // Add all recipients (one for each UTXO to create)
                for address_script in addresses {
                    tx_builder.add_recipient(
                        address_script,
                        bdk_wallet::bitcoin::Amount::from_sat(amount_per_utxo),
                    );
                }

                // Set fee rate
                let fee_rate_config = f1r3fly_rgb_wallet::bitcoin::FeeRateConfig {
                    sat_per_vb: fee_rate as f64,
                };
                tx_builder.fee_rate(fee_rate_config.to_bdk_fee_rate());

                // Build and sign the PSBT
                let mut psbt = tx_builder
                    .finish()
                    .map_err(|e| RgbLibError::Other(format!("Failed to build split transaction: {}", e)))?;

                #[allow(deprecated)]
                bitcoin_wallet
                    .inner_mut()
                    .sign(&mut psbt, bdk_wallet::SignOptions::default())
                    .map_err(|e| RgbLibError::Other(format!("Failed to sign transaction: {}", e)))?;

                // Extract and broadcast the transaction
                let tx = psbt
                    .extract_tx()
                    .map_err(|e| RgbLibError::Other(format!("Failed to extract transaction: {}", e)))?;

                // Create EsploraClient from config
                let esplora_url = format!("http://localhost:3002");
                let network = match self.bitcoin_network {
                    BitcoinNetwork::Mainnet => f1r3fly_rgb_wallet::config::NetworkType::Mainnet,
                    BitcoinNetwork::Testnet => f1r3fly_rgb_wallet::config::NetworkType::Testnet,
                    BitcoinNetwork::Testnet4 => f1r3fly_rgb_wallet::config::NetworkType::Testnet,
                    BitcoinNetwork::Signet => f1r3fly_rgb_wallet::config::NetworkType::Signet,
                    BitcoinNetwork::Regtest => f1r3fly_rgb_wallet::config::NetworkType::Regtest,
                };
                let esplora_client = f1r3fly_rgb_wallet::bitcoin::network::EsploraClient::new(&esplora_url, network)
                    .map_err(|e| RgbLibError::Other(format!("Failed to create Esplora client: {}", e)))?;
                
                esplora_client
                    .inner()
                    .broadcast(&tx)
                    .map_err(|e| RgbLibError::Other(format!("Failed to broadcast transaction: {}", e)))?;

                // Persist wallet changes
                bitcoin_wallet.persist()
                    .map_err(|e| RgbLibError::Other(format!("Failed to persist wallet: {}", e)))?;

                drop(mgr);

                // CRITICAL: Sync wallet after creating UTXOs to update BDK state
                // This ensures subsequent operations see the new UTXOs and the spent inputs
                eprintln!("🔄 DEBUG: Syncing wallet after creating {} UTXOs...", count);
                let mut mgr = manager.lock().unwrap();
                mgr.sync_wallet()
                    .await
                    .map_err(|e| RgbLibError::Other(format!("Sync after createutxos failed: {}", e)))?;
                
                eprintln!("✅ DEBUG: Wallet synced - {} new UTXO(s) should now be visible", count);

                Ok(count as u8)
            })
        })
    }

    pub fn sync(&self, _online: Online) -> Result<(), RgbLibError> {
        let manager = self.wallet_manager.clone();

        tokio::task::block_in_place(|| {
            futures::executor::block_on(async {
                eprintln!("🔄 DEBUG: sync() called - syncing Bitcoin wallet with blockchain");
                let mut mgr = manager.lock().unwrap();
                mgr.sync_wallet()
                    .await
                    .map_err(|e| RgbLibError::Other(format!("Sync failed: {}", e)))?;
                
                // Show wallet UTXOs after sync
                if let Some(btc_wallet) = mgr.bitcoin_wallet() {
                    let utxo_count = btc_wallet.inner().list_unspent().count();
                    eprintln!("   BDK wallet has {} UTXO(s) after sync", utxo_count);
                }
                
                eprintln!("✅ DEBUG: sync() completed\n");
                Ok(())
            })
        })
    }

    pub fn get_wallet_data(&self) -> wallet::WalletData {
        wallet::WalletData {
            data_dir: self.data_dir.clone(),
            bitcoin_network: self.bitcoin_network,
            database_type: lightning::rgb_utils::wallet::DatabaseType::Sqlite,
            max_allocations_per_utxo: 1,
            account_xpub_vanilla: String::new(),
            account_xpub_colored: String::new(),
            master_fingerprint: String::new(),
            mnemonic: String::new(), // Don't expose mnemonic
            vanilla_keychain: None,
            supported_schemas: vec![AssetSchema::Nia, AssetSchema::Cfa, AssetSchema::Uda],
        }
    }

    // ========================================================================
    // RGB Transfer Operations
    // ========================================================================

    pub fn blind_receive(
        &self,
        _asset_id: Option<String>,
        _assignment: Assignment,
        _duration_seconds: Option<u32>,
        _transport_endpoints: Vec<String>,
        _min_confirmations: u8,
    ) -> Result<ReceiveData, RgbLibError> {
        let manager = self.wallet_manager.clone();

        tokio::task::block_in_place(|| {
            futures::executor::block_on(async {
                let _mgr = manager.lock().unwrap();

                // TODO: Implement using generate_invoice_with_pubkey
                Err(RgbLibError::Other("blind_receive: Not fully implemented".to_string()))
            })
        })
    }

    pub fn witness_receive(
        &self,
        asset_id: Option<String>,
        assignment: Assignment,
        duration_seconds: Option<u32>,
        transport_endpoints: Vec<String>,
        min_confirmations: u8,
    ) -> Result<ReceiveData, RgbLibError> {
        // Same as blind_receive for F1r3fly
        self.blind_receive(asset_id, assignment, duration_seconds, transport_endpoints, min_confirmations)
    }

    pub fn send(
        &self,
        _online: Online,
        recipient_map: HashMap<String, Vec<Recipient>>,
        _donation: bool,
        fee_rate: u64,
        _min_confirmations: u8,
        _skip_sync: bool,
    ) -> Result<OperationResult, RgbLibError> {
        // On-chain RGB asset transfer (combines send_begin + sign + send_end)
        // This is used for sending RGB assets to recipients outside of Lightning channels
        
        eprintln!("📤 send: Starting on-chain RGB transfer");
        
        // Step 1: Extract recipient information
        let (asset_id, recipients) = recipient_map
            .iter()
            .next()
            .ok_or_else(|| RgbLibError::Other("No recipient in map".to_string()))?;
        
        let recipient = recipients
            .first()
            .ok_or_else(|| RgbLibError::Other("No recipient for asset".to_string()))?;
        
        // Extract RGB amount from assignment
        let rgb_amount = match &recipient.assignment {
            Assignment::Fungible(amount) => *amount,
            _ => return Err(RgbLibError::Other("Non-fungible assignments not supported".to_string())),
        };
        
        eprintln!("  Asset: {}", asset_id);
        eprintln!("  Recipient: {}", recipient.recipient_id);
        eprintln!("  RGB Amount: {}", rgb_amount);
        
        let manager = self.wallet_manager.clone();
        let data_dir = self.data_dir.clone();
        let recipient_id_clone = recipient.recipient_id.clone();
        let asset_id_clone = asset_id.clone();

        tokio::task::block_in_place(|| {
            futures::executor::block_on(async {
                let mut mgr = manager.lock().unwrap();
                
                // Step 2: Parse recipient address
                let network = mgr.bitcoin_wallet()
                    .ok_or_else(|| RgbLibError::Other("Wallet not loaded".to_string()))?
                    .network();
                
                let bdk_network = match network {
                    f1r3fly_rgb_wallet::config::NetworkType::Mainnet => bitcoin::Network::Bitcoin,
                    f1r3fly_rgb_wallet::config::NetworkType::Testnet => bitcoin::Network::Testnet,
                    f1r3fly_rgb_wallet::config::NetworkType::Regtest => bitcoin::Network::Regtest,
                    f1r3fly_rgb_wallet::config::NetworkType::Signet => bitcoin::Network::Signet,
                };
                
                let addr = bitcoin::Address::from_str(&recipient_id_clone)
                    .map_err(|e| RgbLibError::Other(format!("Invalid address: {}", e)))?
                    .require_network(bdk_network)
                    .map_err(|e| RgbLibError::Other(format!("Address network mismatch: {}", e)))?;
                
                // Step 3: Get RGB-occupied UTXOs and state hash
                let rgb_occupied_vec = mgr.get_occupied_utxos().await
                    .map_err(|e| RgbLibError::Other(format!("Failed to get occupied UTXOs: {}", e)))?;
                
                let state_hash = mgr.get_genesis_state_hash(&asset_id_clone)
                    .map_err(|e| RgbLibError::Other(format!("Failed to get state hash: {}", e)))?;
                
                // Step 4: Build PSBT with minimal BTC output + OP_RETURN
                let bitcoin_wallet = mgr.bitcoin_wallet_mut()
                    .ok_or_else(|| RgbLibError::Other("Wallet not loaded".to_string()))?;
                
                let mut tx_builder = bitcoin_wallet.inner_mut().build_tx();
                // Send minimal BTC (546 sats = dust limit) to recipient
                tx_builder.add_recipient(addr.script_pubkey(), bitcoin::Amount::from_sat(546));
                tx_builder.fee_rate(bdk_wallet::bitcoin::FeeRate::from_sat_per_vb_unchecked(fee_rate));
                
                // Exclude RGB-occupied UTXOs
                for occupied_utxo in rgb_occupied_vec {
                    let outpoint = bitcoin::OutPoint::from_str(&occupied_utxo.outpoint)
                        .map_err(|e| RgbLibError::Other(format!("Invalid outpoint {}: {}", occupied_utxo.outpoint, e)))?;
                    tx_builder.add_unspendable(outpoint);
                }
                
                let mut psbt = tx_builder
                    .finish()
                    .map_err(|e| RgbLibError::Other(format!("Failed to build PSBT: {}", e)))?;

                // Add OP_RETURN with state hash BEFORE signing
                let opreturn_output = bitcoin::TxOut {
                    value: bitcoin::Amount::ZERO,
                    script_pubkey: bitcoin::ScriptBuf::new_op_return(&state_hash),
                };
                psbt.unsigned_tx.output.push(opreturn_output);
                psbt.outputs.push(Default::default());

                eprintln!("  Built PSBT with {} outputs (including OP_RETURN)", psbt.unsigned_tx.output.len());

                // Step 5: Sign PSBT
                #[allow(deprecated)]
                let sign_options = bdk_wallet::SignOptions {
                    trust_witness_utxo: true,
                    ..Default::default()
                };
                let finalized = bitcoin_wallet.inner_mut()
                    .sign(&mut psbt, sign_options)
                    .map_err(|e| RgbLibError::Other(format!("Failed to sign PSBT: {}", e)))?;
                
                if !finalized {
                    return Err(RgbLibError::Other("PSBT signing did not finalize".to_string()));
                }
                
                // Step 6: Extract transaction
                let tx = psbt.extract_tx()
                    .map_err(|e| RgbLibError::Other(format!("Failed to extract TX: {}", e)))?;
                
                let txid = tx.compute_txid().to_string();
                
                // Find the recipient output (first non-OP_RETURN output)
                let recipient_vout = tx.output.iter()
                    .position(|out| !out.script_pubkey.is_op_return())
                    .ok_or(RgbLibError::Other("No recipient output found".to_string()))?;
                
                let recipient_witness_id = format!("witness:{}:{}", txid, recipient_vout);
                
                eprintln!("  TX ID: {}", txid);
                eprintln!("  Recipient witness: {}", recipient_witness_id);
                
                // Step 7: Determine source UTXO for RGB transfer
                let state_file_path = data_dir.join("rgb-lightning-wallet").join("f1r3fly_state.json");
                let state_json = std::fs::read_to_string(&state_file_path)
                    .map_err(|e| RgbLibError::Other(format!("Failed to read f1r3fly_state.json: {}", e)))?;
                let state: serde_json::Value = serde_json::from_str(&state_json)
                    .map_err(|e| RgbLibError::Other(format!("Failed to parse f1r3fly_state.json: {}", e)))?;
                
                let genesis_utxo_data = state["genesis_utxos"][&asset_id_clone].as_object()
                    .ok_or(RgbLibError::Other(format!("Contract {} not found in genesis_utxos", asset_id_clone)))?;
                
                let genesis_txid = genesis_utxo_data["txid"].as_str()
                    .ok_or(RgbLibError::Other("Missing txid in genesis UTXO".to_string()))?;
                let genesis_vout = genesis_utxo_data["vout"].as_u64()
                    .ok_or(RgbLibError::Other("Missing vout in genesis UTXO".to_string()))?;
                
                let source_utxo = format!("{}:{}", genesis_txid, genesis_vout);
                
                eprintln!("  Source UTXO: {}", source_utxo);
                
                // Step 8: Execute F1r3fly transfer
                {
                    let contracts_mgr = mgr.f1r3fly_contracts_mut()
                        .ok_or(RgbLibError::Other("Contracts manager not initialized".to_string()))?;
                    
                    use f1r3fly_rgb::ContractId;
                    let contract_id_typed = ContractId::from_str(&asset_id_clone)
                        .map_err(|e| RgbLibError::Other(format!("Invalid contract ID: {}", e)))?;
                    
                    let derivation_index = contracts_mgr.get_contract_derivation_index(&asset_id_clone)
                        .map_err(|e| RgbLibError::Other(format!("Failed to get derivation index: {}", e)))?;
                    let signing_key = contracts_mgr.contracts().executor().get_child_key_at_index(derivation_index)
                        .map_err(|e| RgbLibError::Other(format!("Failed to get signing key: {}", e)))?;
                    
                    use secp256k1::{Secp256k1, PublicKey};
                    let secp = Secp256k1::new();
                    let public_key = PublicKey::from_secret_key(&secp, &signing_key);
                    let recipient_pubkey = hex::encode(public_key.serialize());
                    
                    use f1r3fly_rgb::{generate_nonce, generate_transfer_signature};
                    let transfer_nonce = generate_nonce();
                    let transfer_signature = generate_transfer_signature(
                        &source_utxo,
                        &recipient_witness_id,
                        rgb_amount,
                        transfer_nonce,
                        &signing_key,
                    ).map_err(|e| RgbLibError::Other(format!("Failed to generate transfer signature: {}", e)))?;
                    
                    let contract = contracts_mgr.contracts_mut()
                        .get_mut(&contract_id_typed)
                        .ok_or(RgbLibError::Other(format!("Contract {} not found", asset_id_clone)))?;
                    
                    use f1r3fly_rgb::StrictVal;
                    use amplify::confinement::SmallOrdMap;
                    let result = contract.call_method(
                        "transfer",
                        &[
                            ("from", StrictVal::from(source_utxo.as_str())),
                            ("to", StrictVal::from(recipient_witness_id.as_str())),
                            ("amount", StrictVal::from(rgb_amount)),
                            ("toPubKey", StrictVal::from(recipient_pubkey.as_str())),
                            ("nonce", StrictVal::from(transfer_nonce)),
                            ("fromSignatureHex", StrictVal::from(transfer_signature.as_str())),
                        ],
                        SmallOrdMap::new(),
                    ).await
                        .map_err(|e| RgbLibError::Other(format!("F1r3node transfer failed: {}", e)))?;
                    
                    eprintln!("✅ F1r3node transfer completed");
                    eprintln!("  New state hash: {}", hex::encode(result.state_hash));
                    
                    contracts_mgr.save_state()
                        .map_err(|e| RgbLibError::Other(format!("Failed to save state: {}", e)))?;
                }
                
                // Step 9: Broadcast transaction
                let broadcast_txid = mgr.broadcast_transaction(&tx)
                    .map_err(|e| RgbLibError::Other(format!("Failed to broadcast: {}", e)))?;
                
                eprintln!("✅ Transaction broadcast: {}", broadcast_txid);
                
                // Step 10: Post transfer metadata to proxy (if transport endpoints provided)
                // This allows the recipient to discover the transfer
                let transport_endpoints = &recipient.transport_endpoints;
                if !transport_endpoints.is_empty() {
                    eprintln!("📤 Posting transfer metadata to proxy...");
                    
                    // Prepare JSON payload
                    // F1r3fly only supports NIA (Non-Inflatable Assets) for now
                    let payload = serde_json::json!({
                        "contract_id": asset_id_clone,
                        "genesis_state_hash": state_hash.to_vec(),
                        "asset_amount": rgb_amount,
                        "schema": "Nia",
                    });
                        
                    // Post to proxy using txid as recipient_id
                    drop(mgr); // Release lock before network call
                    
                    for endpoint_str in transport_endpoints {
                        let base_url = if endpoint_str.starts_with("rpc://") {
                            endpoint_str.replacen("rpc://", "http://", 1)
                        } else if endpoint_str.starts_with("rpcs://") {
                            endpoint_str.replacen("rpcs://", "https://", 1)
                        } else {
                            endpoint_str.clone()
                        };
                        
                        let json_bytes = serde_json::to_vec(&payload)
                            .map_err(|e| RgbLibError::Other(format!("Failed to serialize payload: {}", e)))?;
                        
                        let file_part = reqwest::blocking::multipart::Part::bytes(json_bytes)
                            .file_name("f1r3fly_state.json")
                            .mime_str("application/json")
                            .map_err(|e| RgbLibError::Other(format!("Failed to create file part: {}", e)))?;
                        
                        let form = reqwest::blocking::multipart::Form::new()
                            .text("method", "consignment.post")
                            .text("jsonrpc", "2.0")
                            .text("id", "1")
                            .text("params", serde_json::to_string(&serde_json::json!({
                                "recipient_id": txid.clone()
                            })).unwrap())
                            .part("file", file_part);
                        
                        let client = reqwest::blocking::Client::new();
                        let response = client.post(&base_url)
                            .multipart(form)
                            .send()
                            .map_err(|e| RgbLibError::Other(format!("HTTP POST failed: {}", e)))?;
                        
                        if !response.status().is_success() {
                            eprintln!("⚠️  Proxy POST failed: {}", response.status());
                        } else {
                            eprintln!("✅ Metadata posted to proxy");
                        }
                    }
                }
                
                Ok(OperationResult {
                    txid: broadcast_txid,
                })
            })
        })
    }

    pub fn send_begin(
        &self,
        _online: Online,
        recipient_map: HashMap<String, Vec<Recipient>>,
        _donation: bool,
        fee_rate: u64,
        _min_confirmations: u8,
    ) -> Result<String, RgbLibError> {
        // For F1r3fly RGB channel funding:
        // - Create a Bitcoin funding transaction PSBT
        // - RGB assets stay in genesis UTXO (managed on F1r3node, no on-chain transfer)
        // - Add OP_RETURN with F1r3fly state hash BEFORE signing
        // - Return unsigned PSBT (LDK will sign it later)
        
        // Extract recipient information from the recipient_map
        // For channel funding, there should be exactly one asset and one recipient
        let (asset_id, recipients) = recipient_map
            .iter()
            .next()
            .ok_or_else(|| RgbLibError::Other("No recipient in map".to_string()))?;
        
        let recipient = recipients
            .first()
            .ok_or_else(|| RgbLibError::Other("No recipient for asset".to_string()))?;
        
        let amount_sat = recipient
            .witness_data
            .as_ref()
            .ok_or_else(|| RgbLibError::Other("No witness data in recipient".to_string()))?
            .amount_sat;
        
        // Parse the recipient_id to get the funding address
        let address = recipient.recipient_id.clone();
        
        let manager = self.wallet_manager.clone();

        tokio::task::block_in_place(|| {
            futures::executor::block_on(async {
                let mut mgr = manager.lock().unwrap();
                
                // Get network type
                let network = mgr.bitcoin_wallet()
                    .ok_or_else(|| RgbLibError::Other("Wallet not loaded".to_string()))?
                    .network();
                
                let bdk_network = match network {
                    f1r3fly_rgb_wallet::config::NetworkType::Mainnet => bitcoin::Network::Bitcoin,
                    f1r3fly_rgb_wallet::config::NetworkType::Testnet => bitcoin::Network::Testnet,
                    f1r3fly_rgb_wallet::config::NetworkType::Regtest => bitcoin::Network::Regtest,
                    f1r3fly_rgb_wallet::config::NetworkType::Signet => bitcoin::Network::Signet,
                };
                
                // Parse the address
                let addr = bitcoin::Address::from_str(&address)
                    .map_err(|e| RgbLibError::Other(format!("Invalid address: {}", e)))?
                    .require_network(bdk_network)
                    .map_err(|e| RgbLibError::Other(format!("Address network mismatch: {}", e)))?;
                
                // Get RGB-occupied UTXOs to exclude from coin selection
                let rgb_occupied_vec = mgr.get_occupied_utxos().await
                    .map_err(|e| RgbLibError::Other(format!("Failed to get occupied UTXOs: {}", e)))?;
                
                // Get F1r3fly state hash for this contract
                let state_hash = mgr.get_genesis_state_hash(asset_id)
                    .map_err(|e| RgbLibError::Other(format!("Failed to get state hash: {}", e)))?;
                
                // Get mutable access to bitcoin wallet
                let bitcoin_wallet = mgr.bitcoin_wallet_mut()
                    .ok_or_else(|| RgbLibError::Other("Wallet not loaded".to_string()))?;
                
                // Build the transaction
                let mut tx_builder = bitcoin_wallet.inner_mut().build_tx();
                tx_builder.add_recipient(addr.script_pubkey(), bitcoin::Amount::from_sat(amount_sat));
                tx_builder.fee_rate(bdk_wallet::bitcoin::FeeRate::from_sat_per_vb_unchecked(fee_rate));
                
                // Exclude RGB-occupied UTXOs from being spent
                for occupied_utxo in rgb_occupied_vec {
                    let outpoint = bitcoin::OutPoint::from_str(&occupied_utxo.outpoint)
                        .map_err(|e| RgbLibError::Other(format!("Invalid outpoint {}: {}", occupied_utxo.outpoint, e)))?;
                    tx_builder.add_unspendable(outpoint);
                }
                
                // Build the PSBT
                let mut psbt = tx_builder
                    .finish()
                    .map_err(|e| RgbLibError::Other(format!("Failed to build PSBT: {}", e)))?;

                // CRITICAL: Add OP_RETURN with ACTUAL F1r3fly state hash BEFORE signing
                // This is the key difference from the placeholder approach
                let opreturn_output = bitcoin::TxOut {
                    value: bitcoin::Amount::ZERO,
                    script_pubkey: bitcoin::ScriptBuf::new_op_return(&state_hash),
                };
                psbt.unsigned_tx.output.push(opreturn_output);
                psbt.outputs.push(Default::default());

                eprintln!("📤 send_begin (RGB): Created PSBT with ACTUAL state hash:");
                eprintln!("  unsigned_tx.output.len() = {}", psbt.unsigned_tx.output.len());
                eprintln!("  psbt.outputs.len() = {}", psbt.outputs.len());
                eprintln!("  State hash: {:02x?}", &state_hash[..]);
                for (i, out) in psbt.unsigned_tx.output.iter().enumerate() {
                    if out.script_pubkey.is_op_return() {
                        eprintln!("  Output {}: OP_RETURN ({} bytes)", i, out.script_pubkey.len());
                    } else {
                        eprintln!("  Output {}: {} sats", i, out.value.to_sat());
                    }
                }
                
                // Store RGB transfer details for send_end() to execute F1r3node transfer
                // Extract RGB amount from assignment
                let rgb_amount = match &recipient.assignment {
                    lightning::rgb_utils::Assignment::Fungible(amt) => *amt,
                    lightning::rgb_utils::Assignment::NonFungible => 1,
                    _ => 0,
                };
                
                let transfer_details = serde_json::json!({
                    "contract_id": asset_id,
                    "rgb_amount": rgb_amount,
                    "btc_amount_sat": amount_sat,
                });
                
                let transfer_details_path = self.data_dir.join(".ldk").join("pending_rgb_transfer.json");
                std::fs::write(&transfer_details_path, serde_json::to_string(&transfer_details).unwrap())
                    .map_err(|e| RgbLibError::Other(format!("Failed to store transfer details: {}", e)))?;
                
                eprintln!("📝 send_begin: Stored transfer details: contract_id={}, rgb_amount={}", asset_id, rgb_amount);
                
                Ok(psbt.to_string())
            })
        })
    }

    pub fn send_end(&self, _online: Online, signed_psbt: String, _skip_sync: bool) -> Result<OperationResult, RgbLibError> {
        // For F1r3fly RGB channel funding:
        // 1. Read stored RGB transfer details
        // 2. Execute F1r3node transfer (genesis UTXO → funding witness ID)
        // 3. Extract and broadcast Bitcoin transaction
        // 4. Clean up transfer details file
        
        let manager = self.wallet_manager.clone();
        let data_dir = self.data_dir.clone();

        tokio::task::block_in_place(|| {
            futures::executor::block_on(async {
                // Step 1: Read RGB transfer details
                let transfer_details_path = data_dir.join(".ldk").join("pending_rgb_transfer.json");
                let transfer_details_str = std::fs::read_to_string(&transfer_details_path)
                    .map_err(|e| RgbLibError::Other(format!("Failed to read transfer details: {}", e)))?;
                let transfer_details: serde_json::Value = serde_json::from_str(&transfer_details_str)
                    .map_err(|e| RgbLibError::Other(format!("Failed to parse transfer details: {}", e)))?;
                
                let contract_id = transfer_details["contract_id"].as_str()
                    .ok_or(RgbLibError::Other("Missing contract_id in transfer details".to_string()))?;
                let rgb_amount = transfer_details["rgb_amount"].as_u64()
                    .ok_or(RgbLibError::Other("Missing rgb_amount in transfer details".to_string()))?;
                
                eprintln!("📤 send_end: Retrieved transfer details: contract_id={}, rgb_amount={}", contract_id, rgb_amount);
                
                // Step 2: Parse signed PSBT and extract transaction
                let psbt = bitcoin::psbt::Psbt::from_str(&signed_psbt)
                    .map_err(|e| RgbLibError::Other(format!("Invalid PSBT: {}", e)))?;
                
                let tx = psbt.extract_tx()
                    .map_err(|e| RgbLibError::Other(format!("Failed to extract TX from PSBT: {}", e)))?;
                
                let funding_txid = tx.compute_txid().to_string();
                
                // Find the funding output (the largest non-OP_RETURN output, which is the funding output)
                // CRITICAL: Use enumerate() to get the ACTUAL vout index, not the iterator position!
                // If TX has [change_out, funding_out, op_return], funding is at vout 1, not position 0
                let (funding_vout, _) = tx.output.iter()
                    .enumerate()
                    .filter(|(_, out)| !out.script_pubkey.is_op_return())
                    .max_by_key(|(_, out)| out.value)
                    .ok_or(RgbLibError::Other("No funding output found in TX".to_string()))?;
                
                let funding_witness_id = format!("witness:{}:{}", funding_txid, funding_vout);
                
                eprintln!("📤 send_end: Funding UTXO: {}:{}", funding_txid, funding_vout);
                eprintln!("📤 send_end: Funding witness ID: {}", funding_witness_id);
                eprintln!("   ✅ Found funding output at vout {} (largest non-OP_RETURN)", funding_vout);
                
                // Step 3: Lock manager and check if TX was already broadcast
                let mut mgr = manager.lock().unwrap();
                
                // Check if this funding transaction was already broadcast
                // by attempting to broadcast first. If it succeeds, we need to execute the F1r3node transfer.
                // If it fails with "already in utxo set", the transfer was already done - skip it.
                let already_broadcast = match mgr.broadcast_transaction(&tx) {
                    Ok(txid) => {
                        eprintln!("✅ send_end: Bitcoin TX broadcast: {}", txid);
                        false  // First time broadcasting - need to do F1r3node transfer
                    }
                    Err(e) => {
                        let err_msg = format!("{}", e);
                        if err_msg.contains("already in utxo set") || err_msg.contains("txn-already-in-mempool") {
                            eprintln!("ℹ️  send_end: TX already broadcast ({}), skipping F1r3node transfer", funding_txid);
                            true  // Already broadcast - skip F1r3node transfer to avoid double-spend
                        } else {
                            return Err(RgbLibError::Other(format!("Failed to broadcast TX: {}", e)));
                        }
                    }
                };
                
                // Only execute F1r3node transfer if this is the first broadcast attempt
                if !already_broadcast {
                
                // Step 4: Get genesis UTXO from f1r3fly_state.json
                let state_file_path = data_dir.join("rgb-lightning-wallet").join("f1r3fly_state.json");
                let state_json = std::fs::read_to_string(&state_file_path)
                    .map_err(|e| RgbLibError::Other(format!("Failed to read f1r3fly_state.json: {}", e)))?;
                let state: serde_json::Value = serde_json::from_str(&state_json)
                    .map_err(|e| RgbLibError::Other(format!("Failed to parse f1r3fly_state.json: {}", e)))?;
                
                let genesis_utxo_data = state["genesis_utxos"][contract_id].as_object()
                    .ok_or(RgbLibError::Other(format!("Contract {} not found in genesis_utxos", contract_id)))?;
                
                let genesis_txid = genesis_utxo_data["txid"].as_str()
                    .ok_or(RgbLibError::Other("Missing txid in genesis UTXO".to_string()))?;
                let genesis_vout = genesis_utxo_data["vout"].as_u64()
                    .ok_or(RgbLibError::Other("Missing vout in genesis UTXO".to_string()))?;
                
                let genesis_utxo = format!("{}:{}", genesis_txid, genesis_vout);
                
                eprintln!("📤 send_end: Genesis UTXO: {}", genesis_utxo);
                
                // Step 5: Execute F1r3node transfer
                eprintln!("🔄 send_end: Calling F1r3node transfer()...");
                eprintln!("  From: {}", genesis_utxo);
                eprintln!("  To: {}", funding_witness_id);
                eprintln!("  Amount: {}", rgb_amount);
                
                // Scope block to execute F1r3node transfer and save state
                // The contracts_mgr reference will be dropped at the end of this block
                {
                    // Get contracts manager and execute transfer
                    let contracts_mgr = mgr.f1r3fly_contracts_mut()
                        .ok_or(RgbLibError::Other("Contracts manager not initialized".to_string()))?;
                    
                    // Get contract ID as proper type
                    use f1r3fly_rgb::ContractId;
                    let contract_id_typed = ContractId::from_str(contract_id)
                        .map_err(|e| RgbLibError::Other(format!("Invalid contract ID: {}", e)))?;
                    
                    // Get derivation index and signing key BEFORE getting mutable contract reference
                    // (to avoid borrow checker issues)
                    let derivation_index = contracts_mgr.get_contract_derivation_index(contract_id)
                        .map_err(|e| RgbLibError::Other(format!("Failed to get derivation index: {}", e)))?;
                    let signing_key = contracts_mgr.contracts().executor().get_child_key_at_index(derivation_index)
                        .map_err(|e| RgbLibError::Other(format!("Failed to get signing key: {}", e)))?;
                    
                    // Derive public key from secret key
                    use secp256k1::{Secp256k1, PublicKey};
                    let secp = Secp256k1::new();
                    let public_key = PublicKey::from_secret_key(&secp, &signing_key);
                    let recipient_pubkey = hex::encode(public_key.serialize());
                    
                    // Generate nonce and signature for transfer authorization
                    use f1r3fly_rgb::{generate_nonce, generate_transfer_signature};
                    let transfer_nonce = generate_nonce();
                    let transfer_signature = generate_transfer_signature(
                        &genesis_utxo,
                        &funding_witness_id,
                        rgb_amount,
                        transfer_nonce,
                        &signing_key,
                    ).map_err(|e| RgbLibError::Other(format!("Failed to generate transfer signature: {}", e)))?;
                    
                    // Now get mutable contract reference
                    let contract = contracts_mgr.contracts_mut()
                        .get_mut(&contract_id_typed)
                        .ok_or(RgbLibError::Other(format!("Contract {} not found", contract_id)))?;
                    
                    // Call transfer method on contract
                    use f1r3fly_rgb::StrictVal;
                    use amplify::confinement::SmallOrdMap;
                    let result = contract.call_method(
                        "transfer",
                        &[
                            ("from", StrictVal::from(genesis_utxo.as_str())),
                            ("to", StrictVal::from(funding_witness_id.as_str())),
                            ("amount", StrictVal::from(rgb_amount)),
                            ("toPubKey", StrictVal::from(recipient_pubkey.as_str())),
                            ("nonce", StrictVal::from(transfer_nonce)),
                            ("fromSignatureHex", StrictVal::from(transfer_signature.as_str())),
                        ],
                        SmallOrdMap::new(), // Empty seals map for now
                    ).await
                        .map_err(|e| RgbLibError::Other(format!("F1r3node transfer failed: {}", e)))?;
                    
                    eprintln!("✅ send_end: F1r3node transfer completed successfully");
                    eprintln!("  State hash: {}", hex::encode(result.state_hash));
                    
                    // Save contracts manager state after transfer
                    contracts_mgr.save_state()
                        .map_err(|e| RgbLibError::Other(format!("Failed to save state after transfer: {}", e)))?;
                    eprintln!("💾 send_end: Contracts manager state saved");
                } // contracts_mgr reference is dropped here, releasing the mutable borrow
                
                // Track this UTXO as locked in a channel (Phase 2)
                // Now we can safely call track_locked_utxo since contracts_mgr is out of scope
                let funding_vout_u32 = funding_vout.try_into()
                    .map_err(|_| RgbLibError::Other("Funding vout overflow".to_string()))?;
                self.track_locked_utxo(contract_id, &funding_txid, funding_vout_u32, rgb_amount)
                    .map_err(|e| RgbLibError::Other(format!("Failed to track locked UTXO: {}", e)))?;
                eprintln!("🔒 send_end: Tracked locked channel UTXO: {}:{} ({})", funding_txid, funding_vout, rgb_amount);
                }
                
                // Step 6: Clean up transfer details file
                let _ = std::fs::remove_file(&transfer_details_path);
                
                // Return operation result
                Ok(OperationResult {
                    txid: funding_txid,
                })
            })
        })
    }

    pub fn send_btc(
        &self,
        _online: Online,
        address: String,
        amount: u64,
        fee_rate: u64,
        _skip_sync: bool,
    ) -> Result<String, RgbLibError> {
        let manager = self.wallet_manager.clone();

        tokio::task::block_in_place(|| {
            let mut mgr = manager.lock().unwrap();
            let fee_config = f1r3fly_rgb_wallet::bitcoin::FeeRateConfig {
                sat_per_vb: fee_rate as f64,
            };

            mgr.send_bitcoin(&address, amount, &fee_config)
                .map_err(|e| RgbLibError::Other(format!("Failed to send BTC: {}", e)))
        })
    }

    pub fn send_btc_begin(
        &self,
        _online: Online,
        address: String,
        amount: u64,
        fee_rate: u64,
        _skip_sync: bool,
    ) -> Result<String, RgbLibError> {
        // Build a Bitcoin PSBT for channel funding (or other BTC sends)
        // Returns unsigned PSBT - caller will sign and broadcast
        
        let manager = self.wallet_manager.clone();

        tokio::task::block_in_place(|| {
            let mut mgr = manager.lock().unwrap();
            
            // Get network type first (before any mutable borrows)
            let network = mgr.bitcoin_wallet()
                .ok_or_else(|| RgbLibError::Other("Wallet not loaded".to_string()))?
                .network();
            
            // Convert to BDK network type
            let bdk_network = match network {
                f1r3fly_rgb_wallet::config::NetworkType::Mainnet => bitcoin::Network::Bitcoin,
                f1r3fly_rgb_wallet::config::NetworkType::Testnet => bitcoin::Network::Testnet,
                f1r3fly_rgb_wallet::config::NetworkType::Regtest => bitcoin::Network::Regtest,
                f1r3fly_rgb_wallet::config::NetworkType::Signet => bitcoin::Network::Signet,
            };
            
            // Parse the address
            let addr = bitcoin::Address::from_str(&address)
                .map_err(|e| RgbLibError::Other(format!("Invalid address: {}", e)))?
                .require_network(bdk_network)
                .map_err(|e| RgbLibError::Other(format!("Address network mismatch: {}", e)))?;
            
            // Get RGB-occupied UTXOs to exclude from coin selection (async call)
            // Must be done before taking mutable reference to bitcoin_wallet
            let rgb_occupied_vec = futures::executor::block_on(async {
                mgr.get_occupied_utxos().await
                    .map_err(|e| RgbLibError::Other(format!("Failed to get occupied UTXOs: {}", e)))
            })?;
            
            // Now get mutable access to bitcoin wallet for building transaction
            let bitcoin_wallet = mgr.bitcoin_wallet_mut()
                .ok_or_else(|| RgbLibError::Other("Wallet not loaded".to_string()))?;
            
            // Build the transaction
            let mut tx_builder = bitcoin_wallet.inner_mut().build_tx();
            tx_builder.add_recipient(addr.script_pubkey(), bitcoin::Amount::from_sat(amount));
            tx_builder.fee_rate(bdk_wallet::bitcoin::FeeRate::from_sat_per_vb_unchecked(fee_rate));
            
            // Exclude RGB-occupied UTXOs from being spent
            for occupied_utxo in rgb_occupied_vec {
                // Parse the outpoint string (format: "txid:vout")
                let outpoint = bitcoin::OutPoint::from_str(&occupied_utxo.outpoint)
                    .map_err(|e| RgbLibError::Other(format!("Invalid outpoint {}: {}", occupied_utxo.outpoint, e)))?;
                tx_builder.add_unspendable(outpoint);
            }
            
            // Finish building the PSBT (unsigned)
            let psbt = tx_builder
                .finish()
                .map_err(|e| RgbLibError::Other(format!("Failed to build PSBT: {}", e)))?;
            
            // NOTE: send_btc_begin is for NON-RGB channels (vanilla Bitcoin)
            // RGB channels use send_begin() which adds OP_RETURN with state hash
            
            eprintln!("📤 send_btc_begin (non-RGB): Created PSBT:");
            eprintln!("  unsigned_tx.output.len() = {}", psbt.unsigned_tx.output.len());
            eprintln!("  psbt.outputs.len() = {}", psbt.outputs.len());
            for (i, out) in psbt.unsigned_tx.output.iter().enumerate() {
                eprintln!("  Output {}: {} sats", i, out.value.to_sat());
            }
            
            // Return the unsigned PSBT as a string
            Ok(psbt.to_string())
        })
    }

    pub fn send_btc_end(&self, _online: Online, signed_psbt: String, _skip_sync: bool) -> Result<String, RgbLibError> {
        // Broadcast signed Bitcoin PSBT (for non-RGB channel funding)
        
        let manager = self.wallet_manager.clone();

        tokio::task::block_in_place(|| {
            let mgr = manager.lock().unwrap();
            
            // Parse the signed PSBT
            let psbt = bitcoin::psbt::Psbt::from_str(&signed_psbt)
                .map_err(|e| RgbLibError::Other(format!("Invalid PSBT: {}", e)))?;
            
            // Extract the final transaction
            let tx = psbt.extract_tx()
                .map_err(|e| RgbLibError::Other(format!("Failed to extract TX from PSBT: {}", e)))?;
            
            // Broadcast the transaction
            let txid = mgr.broadcast_transaction(&tx)
                .map_err(|e| RgbLibError::Other(format!("Failed to broadcast TX: {}", e)))?;
            
            Ok(txid)
        })
    }

    pub fn sign_psbt(&self, unsigned_psbt: String, _sign_options: Option<SignOptions>) -> Result<String, RgbLibError> {
        let manager = self.wallet_manager.clone();

        tokio::task::block_in_place(|| {
            // Parse PSBT from string
            use bitcoin::psbt::Psbt;
            use std::str::FromStr;

            let mut psbt = Psbt::from_str(&unsigned_psbt)
                .map_err(|e| RgbLibError::Other(format!("Failed to parse PSBT: {}", e)))?;

            eprintln!("✍️  sign_psbt: Before signing:");
            eprintln!("  unsigned_tx.output.len() = {}", psbt.unsigned_tx.output.len());
            eprintln!("  psbt.outputs.len() = {}", psbt.outputs.len());

            // Sign using BDK
            #[allow(deprecated)]
            let sign_options = bdk_wallet::SignOptions::default();

            // Need mutable access to wallet
            let mut mgr_mut = manager.lock().unwrap();
            let bitcoin_wallet_mut = mgr_mut
                .bitcoin_wallet_mut()
                .ok_or(RgbLibError::Other("Bitcoin wallet not loaded".to_string()))?;

            bitcoin_wallet_mut
                .inner_mut()
                .sign(&mut psbt, sign_options)
                .map_err(|e| RgbLibError::Other(format!("Failed to sign PSBT: {}", e)))?;

            eprintln!("✍️  sign_psbt: After signing:");
            eprintln!("  unsigned_tx.output.len() = {}", psbt.unsigned_tx.output.len());
            eprintln!("  psbt.outputs.len() = {}", psbt.outputs.len());
            eprintln!("  psbt.inputs[0].final_script_witness.is_some() = {}", 
                psbt.inputs.first().map(|i| i.final_script_witness.is_some()).unwrap_or(false));

            Ok(psbt.to_string())
        })
    }

    // ========================================================================
    // Listing/Query Operations
    // ========================================================================

    pub fn list_unspents(
        &self,
        _online: Option<Online>,
        settled_only: bool,
        _skip_sync: bool,
    ) -> Result<Vec<Unspent>, RgbLibError> {
        let manager = self.wallet_manager.clone();

        tokio::task::block_in_place(|| {
            futures::executor::block_on(async {
                let mut mgr = manager.lock().unwrap();

                let filter = f1r3fly_rgb_wallet::types::UtxoFilter {
                    available_only: false,
                    rgb_only: false,
                    confirmed_only: settled_only,
                    min_amount_sats: None,
                };

                let utxos = mgr
                    .list_utxos(filter)
                    .await
                    .map_err(|e| RgbLibError::Other(format!("Failed to list UTXOs: {}", e)))?;

                // Convert to rgb-lib Unspent format
                let unspents = utxos
                    .into_iter()
                    .map(|utxo| {
                        // Parse outpoint
                        let outpoint_inner = lightning::rgb_utils::wallet::OutpointInner {
                            txid: utxo.txid.clone(),
                            vout: utxo.vout,
                        };

                        // Create Outpoint
                        let outpoint = lightning::rgb_utils::wallet::Outpoint {
                            outpoint: outpoint_inner,
                            txid: utxo.txid.clone(),
                            vout: utxo.vout,
                            btc_amount: utxo.amount_sats,
                            colorable: utxo.rgb_assets.is_empty(), // Colorable if no RGB allocations
                        };

                        // Convert RGB assets to allocations
                        let rgb_allocations: Vec<lightning::rgb_utils::RgbAllocation> = utxo
                            .rgb_assets
                            .into_iter()
                            .map(|seal_info| {
                                let amount = seal_info.amount.unwrap_or(0);
                                lightning::rgb_utils::RgbAllocation {
                                    asset_id: Some(seal_info.contract_id),
                                    amount,
                                    assignment: lightning::rgb_utils::Assignment::Fungible(amount),
                                    settled: true,
                                }
                            })
                            .collect();

                        // Create TxOut with empty script_pubkey
                        //
                        // LIMITATION: F1r3fly's UtxoInfo doesn't expose the script_pubkey
                        // (the Bitcoin locking script for this UTXO). This is because BDK
                        // (the underlying Bitcoin wallet) already knows the scripts internally
                        // and F1r3fly doesn't need to expose them for RGB operations.
                        //
                        // IMPACT: Code that inspects the script_pubkey from list_unspents()
                        // will see an empty script. However, this shouldn't affect:
                        // - PSBT signing (BDK has access to real scripts internally)
                        // - RGB operations (they only need the outpoint, not the script)
                        // - Channel operations (LDK uses its own UTXO tracking)
                        //
                        // If script_pubkey access becomes necessary, we could:
                        // - Extend F1r3fly's UtxoInfo to include it
                        // - Query BDK wallet directly for the script
                        // - Derive script from address (if we track derivation paths)
                        let txout = bitcoin::TxOut {
                            value: bitcoin::Amount::from_sat(utxo.amount_sats),
                            script_pubkey: bitcoin::ScriptBuf::new(), // Empty - see comment above
                        };

                        Unspent {
                            outpoint: outpoint.clone(),
                            txout,
                            rgb_allocations,
                            utxo: outpoint,
                        }
                    })
                    .collect();

                Ok(unspents)
            })
        })
    }

    #[allow(dead_code)]
    pub fn list_unspents_vanilla(
        &self,
        _online: Online,
        _min_confirmations: u8,
        _skip_sync: bool,
    ) -> Result<Vec<Unspent>, RgbLibError> {
        // Same as list_unspents but filter vanilla only
        self.list_unspents(None, false, true)
    }

    pub fn list_transactions(
        &self,
        _online: Option<Online>,
        _skip_sync: bool,
    ) -> Result<Vec<RgbLibTransaction>, RgbLibError> {
        // TODO: Implement transaction listing
        Ok(vec![])
    }

    pub fn list_transfers(&self, _asset_id: Option<String>) -> Result<Vec<Transfer>, RgbLibError> {
        // TODO: Implement transfer listing
        Ok(vec![])
    }

    pub fn refresh(
        &self,
        _online: Online,
        _asset_id: Option<String>,
        _filter: Vec<String>,
        _skip_sync: bool,
    ) -> Result<RefreshResult, RgbLibError> {
        // Sync wallet
        self.sync(Online)?;
        Ok(RefreshResult { new: vec![], updated: vec![] })
    }

    // ========================================================================
    // Transaction Coloring & Consignment Operations
    // ========================================================================

    pub fn color_psbt_and_consume(
        &self,
        _psbt: &mut bitcoin::psbt::Psbt,
        _coloring_info: lightning::rgb_utils::ColoringInfo,
    ) -> Result<Vec<RgbTransfer>, RgbLibError> {
        // TODO: Implement OP_RETURN coloring
        Err(RgbLibError::Other("color_psbt_and_consume: Not fully implemented".to_string()))
    }

    pub fn post_consignment<P: AsRef<Path>>(
        &self,
        proxy_url: String,
        recipient_id: String,
        consignment_path: P,
        txid: String,
        _vout: Option<u32>,
    ) -> Result<(), RgbLibError> {
        // For F1r3fly: Instead of posting a binary consignment,
        // we post JSON with contract_id + state_hash + amount + schema
        
        // 1. Parse asset_id (contract_id) from consignment_path
        // Path format: "{data_dir}/consignments/{asset_id}_{txid}.consignment"
        let _path_str = consignment_path.as_ref().to_str()
            .ok_or(RgbLibError::Other("Invalid consignment path".to_string()))?;

        let filename = consignment_path.as_ref()
            .file_name()
            .and_then(|f| f.to_str())
            .ok_or(RgbLibError::Other("Cannot extract filename".to_string()))?;
        
        // Extract asset_id from filename: "{asset_id}_{txid}.consignment"
        let asset_id = filename
            .split('_')
            .next()
            .ok_or(RgbLibError::Other("Cannot parse asset_id from path".to_string()))?
            .to_string();
        
        // 2. Get genesis state hash, contract metadata, and wallet public key from wallet manager
        let manager = self.wallet_manager.clone();
        let (state_hash, asset_info, contract_metadata, wallet_pubkey) = tokio::task::block_in_place(|| {
            futures::executor::block_on(async {
                let mgr = manager.lock().unwrap();

                // Get genesis state hash
                let state_hash = mgr.get_genesis_state_hash(&asset_id)
                    .map_err(|e| RgbLibError::Other(format!("Failed to get state hash: {}", e)))?;
                
                // Get asset info (ticker, name, precision, supply)
                let asset_info = mgr.get_asset_info(&asset_id)
                    .map_err(|e| RgbLibError::Other(format!("Failed to get asset info: {}", e)))?;
                
                // Get contract metadata (registry_uri, rholang_source)
                let contract_metadata = mgr.get_contract_metadata(&asset_id)
                    .map_err(|e| RgbLibError::Other(format!("Failed to get contract metadata: {}", e)))?;
                
                // Get this node's wallet master public key for witness ownership
                // This will be sent to the counterparty so they can register witness ownership correctly
                let wallet_pubkey = mgr.f1r3fly_contracts()
                    .ok_or_else(|| RgbLibError::Other("F1r3fly contracts not initialized".to_string()))?
                    .contracts()
                    .executor()
                    .get_master_public_key_hex()
                    .map_err(|e| RgbLibError::Other(format!("Failed to get wallet public key: {}", e)))?;
                
                eprintln!("📤 post_consignment: Wallet public key: {}", wallet_pubkey);
                
                Ok::<_, RgbLibError>((state_hash, asset_info, contract_metadata, wallet_pubkey))
            })
        })?;
        
        // 3. Read RGB channel info from temporary file written by ldk.rs
        let ldk_data_dir = self.data_dir.join(".ldk");
        let temp_info_path = ldk_data_dir.join(format!("temp_rgb_info_{}", txid));
        
        let rgb_info: lightning::rgb_utils::RgbInfo = if temp_info_path.exists() {
            let content = std::fs::read_to_string(&temp_info_path)
                .map_err(|e| RgbLibError::Other(format!("Cannot read temp RGB info: {}", e)))?;
            let parsed: lightning::rgb_utils::RgbInfo = serde_json::from_str(&content)
                .map_err(|e| RgbLibError::Other(format!("Cannot parse RGB info: {}", e)))?;
            // Clean up temp file
            let _ = std::fs::remove_file(&temp_info_path);
            parsed
        } else {
            return Err(RgbLibError::Other(format!(
                "Cannot find temporary RGB info file for txid: {}", 
                txid
            )));
        };
        
        // 4. Create JSON payload with full contract metadata
        // This enables Node2 to register the contract in its F1r3flyContractsManager
        let payload = serde_json::json!({
            "contract_id": asset_id,
            "genesis_state_hash": hex::encode(&state_hash),
            "asset_amount": rgb_info.local_rgb_amount,
            "schema": match rgb_info.schema {
                lightning::rgb_utils::AssetSchema::Nia => "Nia",
                lightning::rgb_utils::AssetSchema::Uda => "Uda",
                lightning::rgb_utils::AssetSchema::Cfa => "Cfa",
                lightning::rgb_utils::AssetSchema::Ifa => "Ifa",
            },
            // Contract metadata for Node2 registration
            "ticker": asset_info.ticker,
            "name": asset_info.name,
            "precision": asset_info.precision,
            "supply": asset_info.supply,
            "registry_uri": contract_metadata.registry_uri,
            "rholang_source": contract_metadata.rholang_source,
            "methods": contract_metadata.methods,
            // Wallet public key for witness ownership registration (Phase 2)
            "wallet_pubkey": wallet_pubkey,
        });
        
        let json_body = serde_json::to_string(&payload)
            .map_err(|e| RgbLibError::Other(format!("JSON serialization failed: {}", e)))?;
        
        // 5. Convert proxy_url from rpc:// to http:// scheme
        let http_proxy_url = if proxy_url.starts_with("rpc://") {
            proxy_url.replacen("rpc://", "http://", 1)
        } else if proxy_url.starts_with("rpcs://") {
            proxy_url.replacen("rpcs://", "https://", 1)
        } else {
            proxy_url.clone()
        };
        
        // 6. POST to proxy using JSON-RPC multipart format (same as rgb-lib)
        // Create JSON-RPC params
        let params_json = serde_json::json!({
            "recipient_id": recipient_id,
            "txid": txid
        });
        let params_str = serde_json::to_string(&params_json)
            .map_err(|e| RgbLibError::Other(format!("Failed to serialize params: {}", e)))?;
        
        // Write JSON to a temporary file (proxy expects file upload, not text)
        let temp_file_path = ldk_data_dir.join(format!("temp_consignment_{}.json", txid));
        std::fs::write(&temp_file_path, json_body.clone())
            .map_err(|e| RgbLibError::Other(format!("Failed to write temp file: {}", e)))?;
        
        // Create multipart form with JSON-RPC fields and file
        let form = reqwest::blocking::multipart::Form::new()
            .text("method", "consignment.post")
            .text("jsonrpc", "2.0")
            .text("id", "1")
            .text("params", params_str)
            .file("file", &temp_file_path)
            .map_err(|e| RgbLibError::Other(format!("Failed to attach file: {}", e)))?;
        
        let client = reqwest::blocking::Client::new();
        let response = client.post(&http_proxy_url)
            .multipart(form)
            .send()
            .map_err(|e| RgbLibError::Other(format!("Failed to post to proxy: {}", e)))?;
        
        // Clean up temp file
        let _ = std::fs::remove_file(&temp_file_path);
        
        if !response.status().is_success() {
            return Err(RgbLibError::Other(format!(
                "Proxy returned error status: {}",
                response.status()
            )));
        }
        
        // Parse JSON-RPC response to check for errors
        #[derive(serde::Deserialize)]
        struct JsonRpcResponse {
            result: Option<bool>,
            error: Option<JsonRpcError>,
        }
        
        #[derive(serde::Deserialize)]
        struct JsonRpcError {
            code: i64,
            message: String,
        }
        
        let json_resp = response.json::<JsonRpcResponse>()
            .map_err(|e| RgbLibError::Other(format!("Failed to parse proxy response: {}", e)))?;
        
        if let Some(error) = json_resp.error {
            if error.code == -101 {
                return Err(RgbLibError::RecipientIDAlreadyUsed);
            }
            return Err(RgbLibError::Other(format!(
                "Proxy error: {}",
                error.message
            )));
        }
        
        if json_resp.result.is_none() {
            return Err(RgbLibError::Other("Invalid proxy response".to_string()));
        }
        
        Ok(())
    }

    pub fn save_new_asset(&self, _consignment: RgbTransfer) -> Result<(), RgbLibError> {
        // TODO: Implement asset saving from consignment
        Ok(())
    }

    /// Settle channel closing - atomic distribution to both parties
    ///
    /// Executes F1r3node contract.settle_channel() to atomically split funding UTXO
    /// balance into witness IDs for holder and counterparty.
    ///
    /// Returns the state hash for embedding in closing TX's OP_RETURN.
    ///
    /// # Arguments
    ///
    /// * `funding_utxo` - Funding UTXO being settled (e.g. "tx555:0")
    /// * `holder_amount` - Amount for holder (e.g. 700)
    /// * `counterparty_amount` - Amount for counterparty (e.g. 300)
    /// * `contract_id` - RGB contract ID
    ///
    /// # Returns
    ///
    /// 32-byte state hash to embed in closing transaction's OP_RETURN
    pub fn settle_channel_close(
        &self,
        funding_utxo: &str,
        holder_amount: u64,
        counterparty_amount: u64,
        contract_id: &str,
    ) -> Result<[u8; 32], RgbLibError> {
        // Production approach: Hold wallet_manager lock during entire F1r3node operation
        // This matches rgb-lib's synchronous pattern and our working send_end() implementation
        let manager = self.wallet_manager.clone();

        // This method is called from within a futures::executor::block_on() context.
        // We CANNOT nest block_on() calls as it causes deadlock.
        // Solution: Spawn a new thread with its own tokio runtime to execute the async call.
        
        eprintln!("🔐 settle_channel_close: Preparing to execute in separate thread...");
        
        // Clone all needed data before spawning thread
        let funding_utxo = funding_utxo.to_string();
        let contract_id = contract_id.to_string();
        
        let result = std::thread::spawn(move || {
            eprintln!("🧵 settle_channel_close: Thread started, acquiring wallet_manager lock...");
            let mut mgr = manager.lock().unwrap();
            eprintln!("✅ settle_channel_close: Got wallet_manager lock in thread");

            // Get F1r3flyContractsManager (hold lock during entire operation)
            let contracts_mgr = mgr
                .f1r3fly_contracts_mut()
                .ok_or(RgbLibError::Other("F1r3fly contracts not initialized".to_string()))?;

            // Parse contract ID
            let contract_id_parsed = hypersonic::ContractId::from_str(&contract_id)
                    .map_err(|e| RgbLibError::Other(format!("Invalid contract ID: {}", e)))?;

            eprintln!("📞 settle_channel_close: Preparing F1r3node call...");
            eprintln!("   Contract ID: {}", contract_id_parsed);
            eprintln!("   Funding UTXO (input): {}", funding_utxo);
            eprintln!("   Holder amount: {}, Counterparty amount: {}", holder_amount, counterparty_amount);

            // CRITICAL: During channel opening (send_end), we transferred assets to "witness:txid:vout"
            // During channel closing, we must settle FROM the same "witness:txid:vout" key
            // This is the F1r3fly-specific flow - assets are at witness IDs, not plain UTXOs
            let funding_witness_id = format!("witness:{}", funding_utxo);
            eprintln!("   Funding witness ID (for F1r3node): {}", funding_witness_id);

            // Generate deterministic witness IDs from funding witness ID (not plain UTXO)
            use bitcoin::hashes::{Hash, sha256};
            let holder_hash = sha256::Hash::hash(
                format!("{}holder", funding_witness_id).as_bytes()
            );
            let holder_hash_hex = format!("{}", holder_hash);
            let holder_witness = format!("witness:{}:0", &holder_hash_hex[0..32]);

            let counterparty_hash = sha256::Hash::hash(
                format!("{}counterparty", funding_witness_id).as_bytes()
            );
            let counterparty_hash_hex = format!("{}", counterparty_hash);
            let counterparty_witness = format!("witness:{}:0", &counterparty_hash_hex[0..32]);

            eprintln!("   Holder witness: {}", holder_witness);
            eprintln!("   Counterparty witness: {}", counterparty_witness);

            // Get derivation index and signing key
            let derivation_index = contracts_mgr.get_contract_derivation_index(&contract_id)
                .map_err(|e| RgbLibError::Other(format!("Failed to get derivation index: {}", e)))?;
            let signing_key = contracts_mgr.contracts().executor().get_child_key_at_index(derivation_index)
                .map_err(|e| RgbLibError::Other(format!("Failed to get signing key: {}", e)))?;

            eprintln!("✅ settle_channel_close: Got signing key");

            // Phase 5: Get this node's (holder's) wallet master public key
            // This is the key that will own the holder_witness after settlement
            let holder_pubkey = contracts_mgr
                .contracts()
                .executor()
                .get_master_public_key_hex()
                .map_err(|e| RgbLibError::Other(format!("Failed to get holder public key: {}", e)))?;

            eprintln!("   Holder pubkey (this node): {}", holder_pubkey);

            // Phase 5 & 7: Get counterparty's wallet public key with cache + proxy fallback
            // First tries local cache (channel_counterparties), then fetches from proxy if needed
            let counterparty_pubkey = {
                let state_file_path = contracts_mgr.state_path().to_path_buf();
                
                // Try cache first
                let cached_pubkey = if state_file_path.exists() {
                    let state_json = std::fs::read_to_string(&state_file_path)
                        .map_err(|e| RgbLibError::Other(format!("Failed to read state file: {}", e)))?;
                    
                    let state: serde_json::Value = serde_json::from_str(&state_json)
                        .map_err(|e| RgbLibError::Other(format!("Failed to parse state JSON: {}", e)))?;
                    
                    state
                        .get("channel_counterparties")
                        .and_then(|cp| cp.get(&contract_id))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                } else {
                    None
                };
                
                if let Some(pubkey) = cached_pubkey {
                    eprintln!("   Counterparty pubkey (from cache): {}", pubkey);
                    pubkey
                } else {
                    // Phase 7: Proxy fallback - fetch from proxy if not in cache
                    eprintln!("   Counterparty pubkey not in cache, attempting proxy fallback...");
                    
										// TODO:
                    // Get proxy URL from environment variable
                    // In production, this should come from node config
                    let proxy_url = std::env::var("RGB_PROXY_URL")
                        .unwrap_or_else(|_| "rpc://localhost:3000/json-rpc".to_string());
                    
                    let recipient_id = format!("{}_acceptor_pubkey", contract_id);
                    
                    // Convert proxy_url from rpc:// to http:// scheme
                    let http_proxy_url = if proxy_url.starts_with("rpc://") {
                        proxy_url.replacen("rpc://", "http://", 1)
                    } else if proxy_url.starts_with("rpcs://") {
                        proxy_url.replacen("rpcs://", "https://", 1)
                    } else {
                        proxy_url.clone()
                    };
                    
                    // Create JSON-RPC GET request
                    let rpc_request = serde_json::json!({
                        "method": "consignment.get",
                        "jsonrpc": "2.0",
                        "id": "1",
                        "params": {
                            "recipient_id": recipient_id
                        }
                    });
                    
                    eprintln!("   Fetching from proxy: {}", http_proxy_url);
                    
                    let client = reqwest::blocking::Client::new();
                    let response = client.post(&http_proxy_url)
                        .header("Content-Type", "application/json")
                        .json(&rpc_request)
                        .send()
                        .map_err(|e| RgbLibError::Other(format!("Failed to fetch counterparty pubkey from proxy: {}", e)))?;
                    
                    if !response.status().is_success() {
                        return Err(RgbLibError::Other(format!(
                            "Proxy returned error status {}: Counterparty pubkey not found for contract {}",
                            response.status(),
                            contract_id
                        )));
                    }
                    
                    // Parse JSON-RPC response
                    #[derive(serde::Deserialize)]
                    struct JsonRpcResponse {
                        result: Option<JsonRpcResult>,
                        error: Option<serde_json::Value>,
                    }
                    
                    #[derive(serde::Deserialize)]
                    struct JsonRpcResult {
                        consignment: String,
                    }
                    
                    let rpc_response: JsonRpcResponse = response.json()
                        .map_err(|e| RgbLibError::Other(format!("Failed to parse proxy response: {}", e)))?;
                    
                    if let Some(error) = rpc_response.error {
                        return Err(RgbLibError::Other(format!("Proxy error: {}", error)));
                    }
                    
                    let result = rpc_response.result
                        .ok_or_else(|| RgbLibError::Other("No result in proxy response".to_string()))?;
                    
                    // Decode base64 consignment
                    use base64::Engine;
                    let json_bytes = base64::engine::general_purpose::STANDARD.decode(&result.consignment)
                        .map_err(|e| RgbLibError::Other(format!("Failed to decode base64: {}", e)))?;
                    
                    let json_text = String::from_utf8(json_bytes)
                        .map_err(|e| RgbLibError::Other(format!("Failed to convert to UTF-8: {}", e)))?;
                    
                    // Parse pubkey payload
                    #[derive(serde::Deserialize)]
                    struct PubkeyPayload {
                        wallet_pubkey: String,
                    }
                    
                    let payload: PubkeyPayload = serde_json::from_str(&json_text)
                        .map_err(|e| RgbLibError::Other(format!("Failed to parse pubkey payload: {}", e)))?;
                    
                    let fetched_pubkey = payload.wallet_pubkey;
                    eprintln!("   ✅ Counterparty pubkey (from proxy): {}", fetched_pubkey);
                    
                    // Cache it for future use
                    if state_file_path.exists() {
                        let mut state: serde_json::Value = serde_json::from_str(
                            &std::fs::read_to_string(&state_file_path)
                                .unwrap_or_else(|_| "{}".to_string())
                        ).unwrap_or(serde_json::json!({}));
                        
                        if state.get("channel_counterparties").is_none() {
                            state["channel_counterparties"] = serde_json::json!({});
                        }
                        state["channel_counterparties"][&contract_id] = serde_json::json!(&fetched_pubkey);
                        
                        if let Ok(json_str) = serde_json::to_string_pretty(&state) {
                            let _ = std::fs::write(&state_file_path, &json_str);
                            eprintln!("   💾 Cached counterparty pubkey for future use");
                        }
                    }
                    
                    fetched_pubkey
                }
            };

            eprintln!("   Counterparty pubkey: {}", counterparty_pubkey);
            
            eprintln!("✅ settle_channel_close: Retrieved public keys");

            // Generate nonce (timestamp-based for uniqueness)
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();

            // Generate signature using f1r3fly-rgb's signature generation
            // NOTE: Signature is over the witness ID (not plain UTXO) because that's what F1r3node stores
            use f1r3fly_rgb::generate_settle_channel_signature;
            let signature = generate_settle_channel_signature(
                &funding_witness_id,
                holder_amount,
                counterparty_amount,
                nonce,
                &signing_key,
            )
            .map_err(|e| RgbLibError::Other(format!("Signature generation failed: {}", e)))?;
            
            eprintln!("✅ settle_channel_close: Generated signature");

            // Call F1r3node contract's settle_channel method (while holding lock)
            use f1r3fly_rgb::StrictVal;

            eprintln!("🚀 settle_channel_close: Calling F1r3node settle_channel method...");
            eprintln!("   Parameters:");
            eprintln!("     funding_witness_id: {}", funding_witness_id);
            eprintln!("     holder_witness: {}", holder_witness);
            eprintln!("     holder_amount: {}", holder_amount);
            eprintln!("     counterparty_witness: {}", counterparty_witness);
            eprintln!("     counterparty_amount: {}", counterparty_amount);
            eprintln!("     nonce: {}", nonce);
            
            // Execute F1r3node call using individual contract (same pattern as send_end)
            // Get the specific contract (has its own executor clone)
            eprintln!("🔍 settle_channel_close: About to call contracts_mgr.contracts_mut()...");
            let contract = contracts_mgr.contracts_mut()
                .get_mut(&contract_id_parsed)
                .ok_or(RgbLibError::Other(format!("Contract {} not found", contract_id)))?;
            eprintln!("✅ settle_channel_close: Got contract reference");
            
            // Call settle_channel on the contract
            use amplify::confinement::SmallOrdMap;
            eprintln!("🔍 settle_channel_close: Creating call_method future...");
            
            // Create parameters array with proper lifetimes
            // CRITICAL: Pass funding_witness_id (not funding_utxo) to match send_end() transfer destination
            let params = [
                ("funding_utxo", StrictVal::from(funding_witness_id.as_str())),
                ("holder_witness", StrictVal::from(holder_witness.as_str())),
                ("holder_amount", StrictVal::from(holder_amount)),
                ("counterparty_witness", StrictVal::from(counterparty_witness.as_str())),
                ("counterparty_amount", StrictVal::from(counterparty_amount)),
                ("holderPubKey", StrictVal::from(holder_pubkey.as_str())),
                ("counterpartyPubKey", StrictVal::from(counterparty_pubkey.as_str())),
                ("nonce", StrictVal::from(nonce)),
                ("signatureHex", StrictVal::from(signature.as_str())),
            ];
            
            let call_future = contract.call_method(
                "settle_channel",
                &params,
                SmallOrdMap::new(), // Empty seals map for settlement
            );
            
            // Create a new tokio runtime for this thread to execute the async call
            eprintln!("🔍 settle_channel_close: Creating tokio runtime in thread...");
            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| RgbLibError::Other(format!("Failed to create tokio runtime: {}", e)))?;
            
            eprintln!("🔍 settle_channel_close: Blocking on future with rt.block_on...");
            let result = rt.block_on(call_future);
            eprintln!("✅ settle_channel_close: block_on completed!");
            eprintln!("🔍 settle_channel_close: Result status: {}", if result.is_ok() { "Ok" } else { "Err" });
            
            let result = result.map_err(|e| {
                eprintln!("❌ settle_channel_close: F1r3node error: {}", e);
                RgbLibError::Other(format!("F1r3node settle_channel call failed: {}", e))
            })?;

            eprintln!("✅ settle_channel_close: F1r3node call successful");
            eprintln!("   State hash: {}", hex::encode(result.state_hash));
            eprintln!("   ⚠️  NOTE: settle_channel() distributed {} tokens to witness {}", holder_amount, holder_witness);
            eprintln!("   ⚠️  NOTE: The Rholang contract should have moved {} from {} to witness IDs", holder_amount + counterparty_amount, funding_witness_id);
            
            Ok(result.state_hash)
        }).join().map_err(|e| RgbLibError::Other(format!("Thread join failed: {:?}", e)))??;
        
        Ok(result)
    }

    /// Claim RGB assets from a witness ID to a Bitcoin UTXO
    ///
    /// Called after channel close to convert settled witness IDs back to spendable Bitcoin UTXOs.
    /// This is part of the F1r3fly settlement flow where `settle_channel()` creates witness IDs,
    /// and `claim()` moves assets from witness to actual UTXOs.
    ///
    /// # Arguments
    /// * `witness_id` - The witness ID to claim from (e.g., "witness:hash:0")
    /// * `claim_utxo` - The Bitcoin UTXO to claim to (format: "txid:vout")
    /// * `amount` - Amount of RGB assets to claim
    /// * `contract_id` - The RGB contract ID
    ///
    /// # Returns
    /// * `Ok(())` on success
    /// * `Err(RgbLibError)` if claim fails
    pub fn claim(
        &self,
        witness_id: String,
        claim_utxo: String,
        contract_id: &str,
    ) -> Result<(), RgbLibError> {
        eprintln!("🎁 claim: Starting claim operation");
        eprintln!("   Witness ID: {}", witness_id);
        eprintln!("   Claim UTXO: {}", claim_utxo);
        eprintln!("   Contract ID: {}", contract_id);

        let manager = self.wallet_manager.clone();
        let contract_id = contract_id.to_string();
        
        let result = std::thread::spawn(move || {
            eprintln!("🧵 claim: Thread started, acquiring wallet_manager lock...");
            let mut mgr = manager.lock().unwrap();
            eprintln!("✅ claim: Got wallet_manager lock in thread");

            // Get F1r3flyContractsManager
            let contracts_mgr = mgr
                .f1r3fly_contracts_mut()
                .ok_or(RgbLibError::Other("F1r3fly contracts not initialized".to_string()))?;

            // Parse contract ID
            let contract_id_parsed = hypersonic::ContractId::from_str(&contract_id)
                .map_err(|e| RgbLibError::Other(format!("Invalid contract ID: {}", e)))?;

            eprintln!("📞 claim: Preparing F1r3node call...");
            eprintln!("   Contract ID: {}", contract_id_parsed);

            // Phase 6: For claims, use the wallet's MASTER key (not contract deployment key)
            //
            // This aligns with settle_channel() which registers witness ownership using
            // wallet master public keys (holderPubKey, counterpartyPubKey), not contract keys.
            //
            // Why master key?
            // - settle_channel() stores: witness_id → wallet_master_pubkey (in Rholang contract state)
            // - claim() must verify: signature matches witness_id's registered owner
            // - Therefore: claim signature must be generated with wallet_master_privkey
            //
            // This works for both Node1 (issuer) and Node2 (acceptor) because:
            // - Each node uses its OWN wallet master key for its OWN witness
            // - No dependency on contract deployment index
            let signing_key = contracts_mgr
                .contracts()
                .executor()
                .get_master_key()
                .map_err(|e| RgbLibError::Other(format!("Failed to get wallet master key: {}", e)))?;

            eprintln!("✅ claim: Using wallet master key for signing (not contract deployment key)");

            // Generate signature for claim authorization
            // Note: Rholang claim() only requires (witness_id, real_utxo) signature
            use f1r3fly_rgb::generate_claim_signature;
            let signature = generate_claim_signature(
                &witness_id,
                &claim_utxo,
                &signing_key,
            )
            .map_err(|e| RgbLibError::Other(format!("Signature generation failed: {}", e)))?;
            
            eprintln!("✅ claim: Generated signature");

            // Call F1r3node contract's claim method
            use f1r3fly_rgb::StrictVal;
            use amplify::confinement::SmallOrdMap;

            eprintln!("🚀 claim: Calling F1r3node claim method...");
            eprintln!("   Parameters:");
            eprintln!("     witness_id: {}", witness_id);
            eprintln!("     real_utxo: {}", claim_utxo);
            
            // Get the specific contract
            let contract = contracts_mgr.contracts_mut()
                .get_mut(&contract_id_parsed)
                .ok_or(RgbLibError::Other(format!("Contract {} not found", contract_id)))?;
            eprintln!("✅ claim: Got contract reference");
            
            // Create parameters array (matches Rholang contract signature)
            let params = [
                ("witness_id", StrictVal::from(witness_id.as_str())),
                ("real_utxo", StrictVal::from(claim_utxo.as_str())),
                ("claimantSignatureHex", StrictVal::from(signature.as_str())),
            ];
            
            let call_future = contract.call_method(
                "claim",
                &params,
                SmallOrdMap::new(),
            );
            
            // Create a new tokio runtime for this thread to execute the async call
            eprintln!("🔍 claim: Creating tokio runtime in thread...");
            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| RgbLibError::Other(format!("Failed to create tokio runtime: {}", e)))?;
            
            eprintln!("🔍 claim: Blocking on future with rt.block_on...");
            let result = rt.block_on(call_future);
            eprintln!("✅ claim: block_on completed!");
            eprintln!("🔍 claim: Result status: {}", if result.is_ok() { "Ok" } else { "Err" });
            
            let result = result.map_err(|e| {
                eprintln!("❌ claim: F1r3node error: {}", e);
                RgbLibError::Other(format!("F1r3node claim call failed: {}", e))
            })?;

            eprintln!("✅ claim: F1r3node call successful");
            eprintln!("   State hash: {}", hex::encode(result.state_hash));
            eprintln!("   ⚠️  NOTE: claim() should have moved tokens from {} to {}", witness_id, claim_utxo);

            // Phase AUTO-CLAIM: Register the claimed UTXO in storage
            // This enables balance calculations to find the claimed UTXO
            // Aligns with f1r3fly-rgb-wallet's claim tracking mechanism
            {
                eprintln!("💾 claim: Registering claimed UTXO in storage...");
                
                // Parse claim_utxo to extract txid and vout
                let parts: Vec<&str> = claim_utxo.split(':').collect();
                if parts.len() != 2 {
                    return Err(RgbLibError::Other(format!(
                        "Invalid claim UTXO format: {}. Expected 'txid:vout'",
                        claim_utxo
                    )));
                }
                
                let destination_txid = parts[0].to_string();
                let destination_vout: u32 = parts[1].parse()
                    .map_err(|e| RgbLibError::Other(format!("Invalid vout: {}", e)))?;
                
                // Check if this claim already exists (idempotency)
                // The database has a UNIQUE constraint on (witness_id, contract_id)
                // If the claim already exists, we skip insertion (this can happen if color_psbt is called multiple times)
                let existing_claims = contracts_mgr
                    .claim_storage()
                    .get_all_claims(&contract_id)
                    .unwrap_or_default();
                
                let already_claimed = existing_claims.iter().any(|c| c.witness_id == witness_id);
                
                if already_claimed {
                    eprintln!("   ⚠️  Claim already registered for witness_id={}, skipping duplicate", witness_id);
                } else {
                    // Get current timestamp
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs();
                    
                    // Create PendingClaim record
                    use f1r3fly_rgb_wallet::storage::{PendingClaim, ClaimStatus};
                    use std::path::PathBuf;
                    
                    let pending_claim = PendingClaim {
                        id: None,
                        witness_id: witness_id.clone(),
                        recipient_address: "".to_string(), // Not applicable for channel close claims
                        expected_vout: destination_vout,
                        contract_id: contract_id.clone(),
                        consignment_file: PathBuf::new(), // Not applicable for channel close
                        status: ClaimStatus::Pending,
                        error: None,
                        created_at: now,
                        claimed_at: None,
                        actual_txid: Some(destination_txid),
                        actual_vout: Some(destination_vout),
                    };
                    
                    // Insert claim record
                    let claim_id = contracts_mgr
                        .claim_storage_mut()
                        .insert_pending_claim(&pending_claim)
                        .map_err(|e| RgbLibError::Other(format!("Failed to insert claim: {}", e)))?;
                    
                    eprintln!("   Claim record inserted (id={})", claim_id);
                    
                    // Mark as claimed (successful)
                    contracts_mgr
                        .claim_storage_mut()
                        .mark_claim_completed(claim_id)
                        .map_err(|e| RgbLibError::Other(format!("Failed to mark claim completed: {}", e)))?;
                    
                    eprintln!("✅ claim: Claimed UTXO registered in storage");
                    eprintln!("   Destination: {}", claim_utxo);
                }
            }

            // Save contracts manager state after claim
            contracts_mgr.save_state()
                .map_err(|e| RgbLibError::Other(format!("Failed to save state after claim: {}", e)))?;
            eprintln!("💾 claim: Contracts manager state saved");

            Ok(())
        }).join().map_err(|e| RgbLibError::Other(format!("Thread join failed: {:?}", e)))??;
        
        Ok(result)
    }

    // ========================================================================
    // Public Key Exchange (Phase 7)
    // ========================================================================

    /// Get counterparty's wallet public key with cache + proxy fallback (Phase 7)
    ///
    /// This method:
    /// 1. Checks local cache (channel_counterparties in f1r3fly_state.json)
    /// 2. If not found, fetches from proxy (recipient_id: "{contract_id}_acceptor_pubkey")
    /// 3. Caches the result for future use
    ///
    /// Used by settle_channel_close() to get the counterparty's pubkey for witness registration.
    ///
    /// # Production Enhancement
    /// TODO: Add retry logic and timeout for proxy fetches in production
    #[allow(dead_code)]
    pub fn get_counterparty_wallet_pubkey(
        &self,
        contract_id: &str,
        proxy_url: &str,
    ) -> Result<String, RgbLibError> {
        eprintln!("🔍 get_counterparty_wallet_pubkey: Looking up for contract {}", contract_id);
        
        // Check cache first  
        let state_file_path = self.data_dir.join("rgb-lightning-wallet").join("f1r3fly_state.json");
        
        if state_file_path.exists() {
            let state_json = std::fs::read_to_string(&state_file_path)
                .map_err(|e| RgbLibError::Other(format!("Failed to read state file: {}", e)))?;
            
            let state: serde_json::Value = serde_json::from_str(&state_json)
                .map_err(|e| RgbLibError::Other(format!("Failed to parse state JSON: {}", e)))?;
            
            if let Some(counterparty_pubkey) = state
                .get("channel_counterparties")
                .and_then(|cp| cp.get(contract_id))
                .and_then(|v| v.as_str())
            {
                eprintln!("✅ get_counterparty_wallet_pubkey: Found in cache: {}", counterparty_pubkey);
                return Ok(counterparty_pubkey.to_string());
            }
        }
        
        // Not in cache - fetch from proxy
        eprintln!("📥 get_counterparty_wallet_pubkey: Not in cache, fetching from proxy...");
        
        let recipient_id = format!("{}_acceptor_pubkey", contract_id);
        
        // Convert proxy_url from rpc:// to http:// scheme
        let http_proxy_url = if proxy_url.starts_with("rpc://") {
            proxy_url.replacen("rpc://", "http://", 1)
        } else if proxy_url.starts_with("rpcs://") {
            proxy_url.replacen("rpcs://", "https://", 1)
        } else {
            proxy_url.to_string()
        };
        
        // Create JSON-RPC GET request
        let rpc_request = serde_json::json!({
            "method": "consignment.get",
            "jsonrpc": "2.0",
            "id": "1",
            "params": {
                "recipient_id": recipient_id
            }
        });
        
        let client = reqwest::blocking::Client::new();
        let response = client.post(&http_proxy_url)
            .header("Content-Type", "application/json")
            .json(&rpc_request)
            .send()
            .map_err(|e| RgbLibError::Other(format!("Failed to get counterparty pubkey: {}", e)))?;
        
        if !response.status().is_success() {
            return Err(RgbLibError::Other(format!(
                "Proxy GET failed for acceptor pubkey: {}",
                response.status()
            )));
        }
        
        // Parse JSON-RPC response
        #[derive(serde::Deserialize)]
        #[allow(dead_code)]
        struct JsonRpcResponse {
            result: Option<JsonRpcResult>,
            error: Option<serde_json::Value>,
        }

        #[derive(serde::Deserialize)]
        #[allow(dead_code)]
        struct JsonRpcResult {
            consignment: String,
        }
        
        let rpc_response: JsonRpcResponse = response.json()
            .map_err(|e| RgbLibError::Other(format!("Failed to parse response: {}", e)))?;
        
        if let Some(error) = rpc_response.error {
            return Err(RgbLibError::Other(format!("Proxy error: {}", error)));
        }
        
        let result = rpc_response.result
            .ok_or_else(|| RgbLibError::Other("No result in response".to_string()))?;
        
        // Decode base64 consignment
        use base64::Engine;
        let json_bytes = base64::engine::general_purpose::STANDARD.decode(&result.consignment)
            .map_err(|e| RgbLibError::Other(format!("Failed to decode base64: {}", e)))?;
        
        let json_text = String::from_utf8(json_bytes)
            .map_err(|e| RgbLibError::Other(format!("Failed to convert to UTF-8: {}", e)))?;
        
        // Parse pubkey payload
        #[derive(serde::Deserialize)]
        #[allow(dead_code)]
        struct PubkeyPayload {
            wallet_pubkey: String,
        }
        
        let payload: PubkeyPayload = serde_json::from_str(&json_text)
            .map_err(|e| RgbLibError::Other(format!("Failed to parse pubkey payload: {}", e)))?;
        
        let counterparty_pubkey = payload.wallet_pubkey;
        eprintln!("✅ get_counterparty_wallet_pubkey: Retrieved from proxy: {}", counterparty_pubkey);
        
        // Cache it for future use
        if state_file_path.exists() {
            let mut state: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(&state_file_path)
                    .map_err(|e| RgbLibError::Other(format!("Failed to read state: {}", e)))?
            ).unwrap_or(serde_json::json!({}));
            
            if state.get("channel_counterparties").is_none() {
                state["channel_counterparties"] = serde_json::json!({});
            }
            state["channel_counterparties"][contract_id] = serde_json::json!(&counterparty_pubkey);
            
            let json_str = serde_json::to_string_pretty(&state)
                .map_err(|e| RgbLibError::Other(format!("Failed to serialize: {}", e)))?;
            
            std::fs::write(&state_file_path, &json_str)
                .map_err(|e| RgbLibError::Other(format!("Failed to write state: {}", e)))?;
            
            eprintln!("💾 get_counterparty_wallet_pubkey: Cached for future use");
        }
        
        Ok(counterparty_pubkey)
    }

    // ========================================================================
    // Claim UTXO Management (Phase 4)
    // ========================================================================

    /// Ensure claim UTXOs are available for future channel closes
    ///
    /// Called during wallet unlock to pre-create small Bitcoin UTXOs that can be used
    /// for claiming RGB assets after channel settlement. This avoids delays when channels close.
    ///
    /// # Strategy
    /// - Maintains 2 small UTXOs (~1000 sats each) suitable for RGB claims
    /// - Only creates if count falls below 2
    /// - Aligns with rgb-lib philosophy: "assets available immediately after channel close"
    ///
    /// # Production Enhancement
    /// TODO: In production (non-regtest), add confirmation checking before considering
    /// UTXOs as "available". For now, regtest mining is instant so confirmations are immediate.
    pub fn ensure_claim_utxos_available(&self) -> Result<(), RgbLibError> {
        eprintln!("🔧 ensure_claim_utxos_available: Checking claim UTXO availability...");
        
        let unspents = self.list_unspents(None, false, false)?;
        
        // Count small, non-RGB UTXOs suitable for claims
        let available_count = unspents
            .iter()
            .filter(|u| u.rgb_allocations.is_empty())  // Not RGB-occupied
            .filter(|u| u.utxo.btc_amount <= 2000)     // Small (claim-sized)
            .count();
        
        eprintln!("   Found {} available claim UTXOs", available_count);
        
        // Ensure we have at least 2 UTXOs ready
        const MIN_CLAIM_UTXOS: usize = 2;
        if available_count < MIN_CLAIM_UTXOS {
            let needed = MIN_CLAIM_UTXOS - available_count;
            eprintln!("   Creating {} additional claim UTXO(s)...", needed);
            
            self.create_utxos(
                Online,
                false,           // up_to
                Some(needed as u8),
                Some(1000),      // 1000 sats each
                1,               // 1 sat/vB (regtest)
                false,           // don't skip sync
            )?;
            
            eprintln!("   ✅ Created {} claim UTXO(s)", needed);
        } else {
            eprintln!("   ✅ Sufficient claim UTXOs available");
        }
        
        Ok(())
    }

    /// Get a Bitcoin UTXO suitable for claiming RGB assets
    ///
    /// Finds the smallest available non-RGB UTXO, or creates one on-demand if none exist.
    /// This is called during channel close to get a UTXO for the claim() operation.
    ///
    /// # Returns
    /// UTXO string in format "txid:vout"
    ///
    /// # Production Enhancement
    /// TODO: In production (non-regtest), this should:
    /// 1. Filter UTXOs by min_confirmations (e.g., 6 blocks)
    /// 2. After on-demand creation, wait for confirmation before returning
    /// 3. Add timeout and retry logic for confirmation waits
    /// For now, regtest blocks mine instantly so confirmations are immediate.
    pub fn get_claim_utxo(&self) -> Result<String, RgbLibError> {
        eprintln!("🔍 get_claim_utxo: Finding available UTXO for claim...");
        
        let unspents = self.list_unspents(None, false, false)?;
        
        // Try to find a suitable UTXO (smallest non-RGB UTXO)
        if let Some(utxo) = unspents
            .iter()
            .filter(|u| u.rgb_allocations.is_empty())  // Not RGB-occupied
            .filter(|u| u.utxo.btc_amount <= 2000)     // Small (claim-sized)
            .min_by_key(|u| u.utxo.btc_amount)         // Smallest first
        {
            let claim_utxo = format!("{}:{}", utxo.utxo.txid, utxo.utxo.vout);
            eprintln!("   ✅ Found available UTXO: {} ({} sats)", claim_utxo, utxo.utxo.btc_amount);
            return Ok(claim_utxo);
        }
        
        // Fallback: Create on-demand
        eprintln!("   ⚠️  No suitable UTXO found, creating on-demand...");
        self.create_utxos(
            Online,
            false,       // up_to
            Some(1),     // create 1
            Some(1000),  // 1000 sats
            1,           // 1 sat/vB
            false,       // don't skip sync
        )?;
        
        // Get the newly created UTXO
        // In regtest: instant confirmation, immediately available
        // In production: would need to wait_for_utxo_confirmation() here
        let unspents = self.list_unspents(None, false, false)?;
        let utxo = unspents
            .last()
            .ok_or(RgbLibError::Other("Failed to create claim UTXO".to_string()))?;
        
        let claim_utxo = format!("{}:{}", utxo.utxo.txid, utxo.utxo.vout);
        eprintln!("   ✅ Created claim UTXO: {} ({} sats)", claim_utxo, utxo.utxo.btc_amount);
        
        Ok(claim_utxo)
    }

    // ========================================================================
    // Locked Channel UTXO Tracking (Phase 2)
    // ========================================================================

    /// Track a UTXO as locked in a Lightning channel
    ///
    /// Updates the locked_channel_utxos.json file to mark this UTXO as unspendable.
    /// Called after successfully executing F1r3node transfer during channel funding.
    fn track_locked_utxo(&self, contract_id: &str, txid: &str, vout: u32, amount: u64) -> Result<(), String> {
        let locked_file_path = self.data_dir.join("rgb-lightning-wallet").join("locked_channel_utxos.json");
        
        // Read existing locked UTXOs or create new structure
        let mut locked_data: serde_json::Value = if locked_file_path.exists() {
            let content = std::fs::read_to_string(&locked_file_path)
                .map_err(|e| format!("Failed to read locked UTXOs file: {}", e))?;
            serde_json::from_str(&content)
                .unwrap_or_else(|_| serde_json::json!({}))
        } else {
            // Ensure directory exists
            if let Some(parent) = locked_file_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create directory: {}", e))?;
            }
            serde_json::json!({})
        };
        
        // Add this UTXO
        let utxo_key = format!("{}:{}", txid, vout);
        locked_data[utxo_key] = serde_json::json!({
            "contract_id": contract_id,
            "amount": amount,
            "locked_at": chrono::Utc::now().timestamp(),
        });
        
        // Write back
        let json_str = serde_json::to_string_pretty(&locked_data)
            .map_err(|e| format!("Failed to serialize locked UTXOs: {}", e))?;
        std::fs::write(&locked_file_path, json_str)
            .map_err(|e| format!("Failed to write locked UTXOs file: {}", e))?;
        
        Ok(())
    }
    
    /// Release a UTXO from locked status (called on channel close)
    pub fn release_locked_utxo(&self, txid: &str, vout: u32) -> Result<(), String> {
        let locked_file_path = self.data_dir.join("rgb-lightning-wallet").join("locked_channel_utxos.json");
        
        if !locked_file_path.exists() {
            return Ok(()); // No locked UTXOs, nothing to release
        }
        
        let content = std::fs::read_to_string(&locked_file_path)
            .map_err(|e| format!("Failed to read locked UTXOs file: {}", e))?;
        let mut locked_data: serde_json::Value = serde_json::from_str(&content)
            .unwrap_or_else(|_| serde_json::json!({}));
        
        // Remove this UTXO
        let utxo_key = format!("{}:{}", txid, vout);
        if let Some(obj) = locked_data.as_object_mut() {
            obj.remove(&utxo_key);
        }
        
        // Write back
        let json_str = serde_json::to_string_pretty(&locked_data)
            .map_err(|e| format!("Failed to serialize locked UTXOs: {}", e))?;
        std::fs::write(&locked_file_path, json_str)
            .map_err(|e| format!("Failed to write locked UTXOs file: {}", e))?;
        
        Ok(())
    }
    
    /// Get total amount locked in channels for a specific contract
    fn get_locked_amount(&self, contract_id: &str) -> Result<u64, String> {
        let locked_file_path = self.data_dir.join("rgb-lightning-wallet").join("locked_channel_utxos.json");
        
        if !locked_file_path.exists() {
            return Ok(0); // No locked UTXOs
        }
        
        let content = std::fs::read_to_string(&locked_file_path)
            .map_err(|e| format!("Failed to read locked UTXOs file: {}", e))?;
        let locked_data: serde_json::Value = serde_json::from_str(&content)
            .unwrap_or_else(|_| serde_json::json!({}));
        
        // Sum all amounts for this contract
        let mut total = 0u64;
        if let Some(obj) = locked_data.as_object() {
            for (_, utxo_data) in obj {
                if let Some(utxo_contract_id) = utxo_data.get("contract_id").and_then(|c| c.as_str()) {
                    if utxo_contract_id == contract_id {
                        if let Some(amount) = utxo_data.get("amount").and_then(|a| a.as_u64()) {
                            total += amount;
                        }
                    }
                }
            }
        }
        
        Ok(total)
    }

    // ========================================================================
    // Utility Methods
    // ========================================================================

    pub fn get_media_dir(&self) -> PathBuf {
        self.data_dir.join("media")
    }

    pub fn get_send_consignment_path(&self, asset_id: &str, txid: &str) -> PathBuf {
        self.data_dir
            .join("consignments")
            .join(format!("{}_{}.consignment", asset_id, txid))
    }
    
    #[allow(dead_code)]
    fn read_rgb_channel_info_from_txid(&self, ldk_data_dir: &Path, _txid: &str) -> Result<lightning::rgb_utils::RgbInfo, RgbLibError> {
        // Find the RGB channel info file
        // The file is stored as: {ldk_data_dir}/{temporary_channel_id}_pending
        // At this point in the flow, there should be exactly one pending RGB channel
        
        use std::fs;
        
        let entries = fs::read_dir(ldk_data_dir)
            .map_err(|e| RgbLibError::Other(format!("Cannot read ldk_data_dir: {}", e)))?;
        
        for entry in entries {
            let entry = entry.map_err(|e| RgbLibError::Other(format!("Cannot read entry: {}", e)))?;
            let path = entry.path();
            
            if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
                if filename.ends_with("_pending") && !filename.starts_with('.') && !filename.contains("consignment") {
                    // Try to read this file
                    if let Ok(content) = fs::read_to_string(&path) {
                        if let Ok(rgb_info) = serde_json::from_str::<lightning::rgb_utils::RgbInfo>(&content) {
                            // Return the first valid RgbInfo we find
                            // In the actual flow, there should only be one pending RGB channel at this point
                            return Ok(rgb_info);
                        }
                    }
                }
            }
        }
        
        // If no pending file found, return dummy values
        // This can happen if the channel info hasn't been written yet
        // In that case, we'll use reasonable defaults
        Err(RgbLibError::Other("Cannot find RGB channel info - channel may not be RGB-enabled".to_string()))
    }

    pub fn get_tx_height(&self, _txid: String) -> Result<Option<u32>, RgbLibError> {
        // TODO: Query blockchain for tx height
        Ok(None)
    }

    pub fn get_fee_estimation(&self, _online: Online, _blocks: u16) -> Result<f64, RgbLibError> {
        // TODO: Get fee estimation from esplora
        Ok(1.0) // Default 1 sat/vbyte
    }

    pub fn fail_transfers(
        &self,
        _online: Online,
        _batch_transfer_idx: Option<i32>,
        _no_asset_only: bool,
        _skip_sync: bool,
    ) -> Result<bool, RgbLibError> {
        // TODO: Implement transfer failure
        Ok(false)
    }

    pub fn update_witnesses(
        &self,
        _after_height: u32,
        _force_witnesses: Vec<RgbTxid>,
    ) -> Result<UpdateRes, RgbLibError> {
        // TODO: Implement witness updates
        Ok(UpdateRes {
            failed: vec![],
        })
    }

    pub fn upsert_witness(
        &self,
        _witness_id: RgbTxid,
        _witness_ord: WitnessOrd,
    ) -> Result<(), RgbLibError> {
        // TODO: Implement witness upsert
        Ok(())
    }

    #[allow(dead_code)]
    pub fn bitcoin_network(&self) -> BitcoinNetwork {
        self.bitcoin_network
    }
}

// ========================================================================
// LDK Trait Implementations
// ========================================================================

impl lightning::sign::ChangeDestinationSource for F1r3flyRgbWalletWrapper {
    fn get_change_destination_script(&self) -> Result<ScriptBuf, ()> {
        let address = self.get_address().map_err(|_| ())?;
        
        use std::str::FromStr;
        let addr = bitcoin::Address::from_str(&address)
            .map_err(|_| ())?
            .assume_checked();
        
        Ok(addr.script_pubkey())
    }
}

impl lightning::events::bump_transaction::WalletSource for F1r3flyRgbWalletWrapper {
    fn list_confirmed_utxos(&self) -> Result<Vec<lightning::events::bump_transaction::Utxo>, ()> {
        // TODO: Implement UTXO listing for LDK
        Ok(vec![])
    }

    fn get_change_script(&self) -> Result<ScriptBuf, ()> {
        let address = self.get_address().map_err(|_| ())?;
        
        use std::str::FromStr;
        let addr = bitcoin::Address::from_str(&address)
            .map_err(|_| ())?
            .assume_checked();
        
        Ok(addr.script_pubkey())
    }

    fn sign_psbt(&self, psbt: bitcoin::psbt::Psbt) -> Result<bitcoin::Transaction, ()> {
        let signed_psbt_str = F1r3flyRgbWalletWrapper::sign_psbt(self, psbt.to_string(), None)
            .map_err(|_| ())?;

        use std::str::FromStr;
        let signed_psbt = bitcoin::psbt::Psbt::from_str(&signed_psbt_str).map_err(|_| ())?;

        signed_psbt.extract_tx().map_err(|_| ())
    }
}

/// Implement SettlementExecutor trait for F1r3flyRgbWalletWrapper
///
/// This allows the rgb_utils module to call settle_channel_close() without
/// creating circular dependencies.
impl SettlementExecutor for F1r3flyRgbWalletWrapper {
    fn settle_channel_close(
        &self,
        funding_utxo: &str,
        holder_amount: u64,
        counterparty_amount: u64,
        contract_id: &str,
    ) -> Result<[u8; 32], RgbLibError> {
        // Delegate to the existing settle_channel_close() method
        F1r3flyRgbWalletWrapper::settle_channel_close(
            self,
            funding_utxo,
            holder_amount,
            counterparty_amount,
            contract_id,
        )
    }

    fn claim_from_witness(
        &self,
        witness_id: &str,
        destination_utxo: &str,
        contract_id: &str,
    ) -> Result<(), RgbLibError> {
        eprintln!("🎯 claim_from_witness: START");
        eprintln!("   Witness ID: {}", witness_id);
        eprintln!("   Destination UTXO: {}", destination_utxo);
        eprintln!("   Contract ID: {}", contract_id);

        // Call the existing claim() method
        // The amount is determined by F1r3node from the witness_id's balance
        self.claim(
            witness_id.to_string(),
            destination_utxo.to_string(),
            contract_id,
        )?;

        eprintln!("✅ claim_from_witness: Claim successful");
        Ok(())
    }

    fn is_witness_claimed(
        &self,
        witness_id: &str,
        contract_id: &str,
    ) -> Result<bool, RgbLibError> {
        let manager = self.wallet_manager.clone();
        
        tokio::task::block_in_place(|| {
            futures::executor::block_on(async {
                let mgr = manager.lock().unwrap();
                
                let contracts_manager = mgr
                    .f1r3fly_contracts()
                    .ok_or(RgbLibError::Other("F1r3fly not initialized".to_string()))?;
                
                // Query claim_storage for any claims with this witness_id and contract_id
                let all_claims = contracts_manager
                    .claim_storage()
                    .get_all_claims(contract_id)
                    .unwrap_or_default();
                
                let is_claimed = all_claims.iter().any(|claim| {
                    claim.witness_id == witness_id
                });
                
                Ok(is_claimed)
            })
        })
    }
}

impl lightning::rgb_utils::ContractReloader for F1r3flyRgbWalletWrapper {
    fn reload_contracts(&self) -> Result<(), RgbLibError> {
        eprintln!("🔄 ContractReloader::reload_contracts: Called");
        
        let manager = self.wallet_manager.clone();
        
        tokio::task::block_in_place(|| {
            futures::executor::block_on(async {
                let mut mgr = manager.lock().unwrap();
                
                mgr.reload_contracts()
                    .map_err(|e| RgbLibError::Other(format!("Failed to reload contracts: {}", e)))?;
                
                eprintln!("✅ ContractReloader::reload_contracts: Success");
                Ok(())
            })
        })
    }
}

