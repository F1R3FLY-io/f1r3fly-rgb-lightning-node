use super::*;

const TEST_DIR_BASE: &str = "tmp/issue_nia/";

#[serial_test::serial]
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
#[traced_test]
async fn issue_nia() {
    initialize();

    let amt = 5000;

    let test_dir_node1 = format!("{TEST_DIR_BASE}node1");
    let (node1_addr, _) = start_node(&test_dir_node1, NODE1_PEER_PORT, false).await;

    fund_and_create_utxos(node1_addr, None).await;

    // check /createutxos size parameter
    let unspents_1 = list_unspents(node1_addr).await;
    create_utxos(node1_addr, false, Some(1), Some(amt)).await;
    let unspents_2 = list_unspents(node1_addr).await;
    assert_eq!(unspents_1.len(), unspents_2.len() - 1);
    assert!(!unspents_1.iter().any(|u| u.utxo.btc_amount == amt as u64));
    assert!(unspents_2.iter().any(|u| u.utxo.btc_amount == amt as u64));

    // issue assets
    let asset_nia = issue_asset_nia(node1_addr).await;

    // check /listassets
    let assets = list_assets(node1_addr).await;
    let assets_nia = assets.nia.unwrap();
    assert_eq!(assets_nia.len(), 1);
    let nia_asset = assets_nia.first().unwrap();
    assert_eq!(nia_asset.asset_id, asset_nia.asset_id);
}
