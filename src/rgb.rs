use bitcoin::Txid as RgbTxid;
use hex_conservative::DisplayHex;
use hypersonic::ContractId;
use lightning::ln::types::ChannelId;
use lightning::rgb_utils::{
    check_proxy_url, wallet::WalletData, AssetCFA, AssetNIA, AssetSchema, AssetUDA, Assets,
    Assignment, Balance, BtcBalance, Metadata, Online, OperationResult, ReceiveData, Recipient,
    RefreshResult, RgbLibError, RgbLibTransaction, RgbTransfer, RgbTransport, Transfer,
    TransportEndpoint, Unspent, WitnessOrd,
};
use lightning::rgb_utils::{
    get_rgb_channel_info_path, is_channel_rgb, parse_rgb_channel_info, RgbInfo,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::{error::APIError, utils::UnlockedAppState};

impl UnlockedAppState {
    pub(crate) fn rgb_blind_receive(
        &self,
        asset_id: Option<String>,
        assignment: Assignment,
        duration_seconds: Option<u32>,
        transport_endpoints: Vec<String>,
        min_confirmations: u8,
    ) -> Result<ReceiveData, RgbLibError> {
        self.rgb_wallet_wrapper.blind_receive(
            asset_id,
            assignment,
            duration_seconds,
            transport_endpoints,
            min_confirmations,
        )
    }

    pub(crate) fn rgb_create_utxos(
        &self,
        up_to: bool,
        num: u8,
        size: u32,
        fee_rate: u64,
        skip_sync: bool,
    ) -> Result<u8, RgbLibError> {
        self.rgb_wallet_wrapper.create_utxos(
            Online,
            up_to,
            Some(num),
            Some(size),
            fee_rate,
            skip_sync,
        )
    }

    pub(crate) fn rgb_fail_transfers(
        &self,
        batch_transfer_idx: Option<i32>,
        no_asset_only: bool,
        skip_sync: bool,
    ) -> Result<bool, RgbLibError> {
        self.rgb_wallet_wrapper
            .fail_transfers(Online, batch_transfer_idx, no_asset_only, skip_sync)
    }

    pub(crate) fn rgb_get_address(&self) -> Result<String, RgbLibError> {
        self.rgb_wallet_wrapper.get_address()
    }

    pub(crate) fn rgb_get_asset_balance(
        &self,
        contract_id: ContractId,
    ) -> Result<Balance, RgbLibError> {
        self.rgb_wallet_wrapper
            .get_asset_balance(contract_id.to_string())
    }

    pub(crate) fn rgb_get_asset_metadata(
        &self,
        contract_id: ContractId,
    ) -> Result<Metadata, RgbLibError> {
        self.rgb_wallet_wrapper
            .get_asset_metadata(contract_id.to_string())
    }

    pub(crate) fn rgb_get_btc_balance(&self, skip_sync: bool) -> Result<BtcBalance, RgbLibError> {
        let online = if skip_sync { None } else { Some(Online) };
        self.rgb_wallet_wrapper.get_btc_balance(online, skip_sync)
    }

    pub(crate) fn rgb_get_fee_estimation(&self, blocks: u16) -> Result<f64, RgbLibError> {
        self.rgb_wallet_wrapper.get_fee_estimation(Online, blocks)
    }

    pub(crate) fn rgb_get_media_dir(&self) -> PathBuf {
        self.rgb_wallet_wrapper.get_media_dir()
    }

    pub(crate) fn rgb_get_send_consignment_path(
        &self,
        asset_id: &str,
        transfer_id: &str,
    ) -> PathBuf {
        self.rgb_wallet_wrapper
            .get_send_consignment_path(asset_id, transfer_id)
    }

    pub(crate) fn rgb_get_wallet_data(&self) -> WalletData {
        self.rgb_wallet_wrapper.get_wallet_data()
    }

    pub(crate) fn rgb_issue_asset_cfa(
        &self,
        name: String,
        details: Option<String>,
        precision: u8,
        amounts: Vec<u64>,
        file_path: Option<String>,
    ) -> Result<AssetCFA, RgbLibError> {
        self.rgb_wallet_wrapper
            .issue_asset_cfa(name, details, precision, amounts, file_path)
    }

    pub(crate) fn rgb_issue_asset_nia(
        &self,
        ticker: String,
        name: String,
        precision: u8,
        amounts: Vec<u64>,
    ) -> Result<AssetNIA, RgbLibError> {
        self.rgb_wallet_wrapper
            .issue_asset_nia(ticker, name, precision, amounts)
    }

    pub(crate) fn rgb_issue_asset_uda(
        &self,
        ticker: String,
        name: String,
        details: Option<String>,
        precision: u8,
        media_file_path: Option<String>,
        attachments_file_paths: Vec<String>,
    ) -> Result<AssetUDA, RgbLibError> {
        self.rgb_wallet_wrapper.issue_asset_uda(
            ticker,
            name,
            details,
            precision,
            media_file_path,
            attachments_file_paths,
        )
    }

    pub(crate) fn rgb_list_assets(
        &self,
        filter_asset_schemas: Vec<AssetSchema>,
    ) -> Result<Assets, RgbLibError> {
        self.rgb_wallet_wrapper.list_assets(filter_asset_schemas)
    }

    pub(crate) fn rgb_list_transactions(
        &self,
        skip_sync: bool,
    ) -> Result<Vec<RgbLibTransaction>, RgbLibError> {
        let online = if skip_sync { None } else { Some(Online) };
        self.rgb_wallet_wrapper.list_transactions(online, skip_sync)
    }

    pub(crate) fn rgb_list_transfers(
        &self,
        asset_id: String,
    ) -> Result<Vec<Transfer>, RgbLibError> {
        self.rgb_wallet_wrapper.list_transfers(Some(asset_id))
    }

    pub(crate) fn rgb_list_unspents(&self, skip_sync: bool) -> Result<Vec<Unspent>, RgbLibError> {
        let online = if skip_sync { None } else { Some(Online) };
        self.rgb_wallet_wrapper
            .list_unspents(online, false, skip_sync)
    }

    pub(crate) fn rgb_post_consignment<P: AsRef<Path>>(
        &self,
        proxy_url: &str,
        recipient_id: String,
        consignment_path: P,
        txid: String,
        vout: Option<u32>,
    ) -> Result<(), RgbLibError> {
        self.rgb_wallet_wrapper.post_consignment(
            proxy_url.to_string(),
            recipient_id,
            consignment_path,
            txid,
            vout,
        )
    }

    pub(crate) fn rgb_refresh(&self, skip_sync: bool) -> Result<RefreshResult, RgbLibError> {
        self.rgb_wallet_wrapper
            .refresh(Online, None, vec![], skip_sync)
    }

    pub(crate) fn rgb_save_new_asset(
        &self,
        consignment: RgbTransfer,
        _offchain_txid: String,
    ) -> Result<(), RgbLibError> {
        self.rgb_wallet_wrapper.save_new_asset(consignment)
    }

    pub(crate) fn rgb_send(
        &self,
        recipient_map: HashMap<String, Vec<Recipient>>,
        donation: bool,
        fee_rate: u64,
        min_confirmations: u8,
        skip_sync: bool,
    ) -> Result<OperationResult, RgbLibError> {
        self.rgb_wallet_wrapper.send(
            Online,
            recipient_map,
            donation,
            fee_rate,
            min_confirmations,
            skip_sync,
        )
    }

    pub(crate) fn rgb_send_begin(
        &self,
        recipient_map: HashMap<String, Vec<Recipient>>,
        donation: bool,
        fee_rate: u64,
        min_confirmations: u8,
    ) -> Result<String, RgbLibError> {
        self.rgb_wallet_wrapper.send_begin(
            Online,
            recipient_map,
            donation,
            fee_rate,
            min_confirmations,
        )
    }

    pub(crate) fn rgb_send_btc(
        &self,
        address: String,
        amount: u64,
        fee_rate: u64,
        skip_sync: bool,
    ) -> Result<String, RgbLibError> {
        self.rgb_wallet_wrapper
            .send_btc(Online, address, amount, fee_rate, skip_sync)
    }

    pub(crate) fn rgb_send_btc_begin(
        &self,
        address: String,
        amount: u64,
        fee_rate: u64,
    ) -> Result<String, RgbLibError> {
        self.rgb_wallet_wrapper
            .send_btc_begin(Online, address, amount, fee_rate, false)
    }

    pub(crate) fn rgb_send_btc_end(&self, signed_psbt: String) -> Result<String, RgbLibError> {
        self.rgb_wallet_wrapper
            .send_btc_end(Online, signed_psbt, false)
    }

    pub(crate) fn rgb_send_end(&self, signed_psbt: String) -> Result<OperationResult, RgbLibError> {
        self.rgb_wallet_wrapper.send_end(Online, signed_psbt, false)
    }

    pub(crate) fn rgb_sign_psbt(&self, unsigned_psbt: String) -> Result<String, RgbLibError> {
        self.rgb_wallet_wrapper.sign_psbt(unsigned_psbt, None)
    }

    pub(crate) fn rgb_sync(&self) -> Result<(), RgbLibError> {
        self.rgb_wallet_wrapper.sync(Online)
    }

    pub(crate) fn rgb_upsert_witness(
        &self,
        witness_id: RgbTxid,
        witness_ord: WitnessOrd,
    ) -> Result<(), RgbLibError> {
        self.rgb_wallet_wrapper
            .upsert_witness(witness_id, witness_ord)
    }

    pub(crate) fn rgb_witness_receive(
        &self,
        asset_id: Option<String>,
        assignment: Assignment,
        duration_seconds: Option<u32>,
        transport_endpoints: Vec<String>,
        min_confirmations: u8,
    ) -> Result<ReceiveData, RgbLibError> {
        self.rgb_wallet_wrapper.witness_receive(
            asset_id,
            assignment,
            duration_seconds,
            transport_endpoints,
            min_confirmations,
        )
    }
}

// RgbLibWalletWrapper has been replaced with F1r3flyRgbWalletWrapper
// All functionality is now in src/f1r3fly_rgb_adapter.rs
// The trait implementations below delegate to F1r3flyRgbWalletWrapper

// WalletSource and ChangeDestinationSource traits are now implemented
// on F1r3flyRgbWalletWrapper in src/f1r3fly_rgb_adapter.rs

pub(crate) async fn check_rgb_proxy_endpoint(proxy_endpoint: &str) -> Result<(), APIError> {
    let rgb_transport =
        RgbTransport::from_str(proxy_endpoint).map_err(|_| APIError::InvalidProxyEndpoint)?;
    let proxy_url = TransportEndpoint::try_from(rgb_transport).unwrap().endpoint;
    tokio::task::spawn_blocking(move || check_proxy_url(&proxy_url))
        .await
        .unwrap()?;
    Ok(())
}

pub(crate) fn get_rgb_channel_info_optional(
    channel_id: &ChannelId,
    ldk_data_dir: &Path,
    pending: bool,
) -> Option<(RgbInfo, PathBuf)> {
    if !is_channel_rgb(channel_id, ldk_data_dir) {
        return None;
    }
    let info_file_path =
        get_rgb_channel_info_path(&channel_id.0.as_hex().to_string(), ldk_data_dir, pending);
    let rgb_info = parse_rgb_channel_info(&info_file_path);
    Some((rgb_info, info_file_path))
}
