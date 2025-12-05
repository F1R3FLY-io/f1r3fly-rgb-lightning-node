# F1r3fly RGB Lightning Node Setup

## Getting the Baseline Running

### 1. Start Services

```bash
cd f1r3fly-rgb-lightning-node
./regtest.sh start
```

### 2. Verify Services

```bash
# Check all containers running
docker compose ps

# Test bitcoind
docker compose exec bitcoind bitcoin-cli -regtest -rpcuser=user -rpcpassword=password getblockchaininfo

# Test electrs
nc -zv localhost 50001

# Test RGB proxy
nc -zv localhost 3000
```

Expected:
- bitcoind: Up, 103 blocks
- electrs: Up, port 50001 open
- proxy: Up, responding on port 3000

### 3. Build Node Binary

```bash
cargo build --bin rgb-lightning-node
```

Build time: ~3 minutes

### 4. Verify Binary

```bash
./target/debug/rgb-lightning-node --help
```

## Services Ready

- **bitcoind**: `localhost:18443` (user/password)
- **electrs**: `localhost:50001`
- **RGB proxy**: `localhost:3000`
- **Binary**: `target/debug/rgb-lightning-node`

## Running the Nodes

### 5. Start Node 1 (Alice)

Open a new terminal:

```bash
cd f1r3fly-rgb-lightning-node
./target/debug/rgb-lightning-node dataldk0/ \
    --daemon-listening-port 3001 \
    --ldk-peer-listening-port 9735 \
    --network regtest \
    --disable-authentication
```

Keep this terminal open - the node runs in foreground.

### 6. Initialize Node 1

In a separate terminal:

```bash
curl -X POST http://localhost:3001/init \
  -H "Content-Type: application/json" \
  -d '{"password": "password123"}'
```

**IMPORTANT:** Save the mnemonic from the response!

### 7. Unlock Node 1

```bash
curl -X POST http://localhost:3001/unlock \
  -H "Content-Type: application/json" \
  -d '{
    "password": "password123",
    "bitcoind_rpc_username": "user",
    "bitcoind_rpc_password": "password",
    "bitcoind_rpc_host": "localhost",
    "bitcoind_rpc_port": 18443,
    "indexer_url": "127.0.0.1:50001",
    "proxy_endpoint": "rpc://127.0.0.1:3000/json-rpc",
    "announce_addresses": []
  }'
```

Expected: `{}` (empty response on success).

### 8. Get Bitcoin Address

```bash
curl -X POST http://localhost:3001/address \
  -H "Content-Type: application/json" \
  -d '{}'
```

Save the address from response.

### 9. Fund Node 1

```bash
# Send 1 BTC to Alice's address
./regtest.sh sendtoaddress <alice_address> 1

# Mine blocks to confirm
./regtest.sh mine 6

# Wait a moment, then sync
curl -X POST http://localhost:3001/sync \
  -H "Content-Type: application/json" \
  -d '{}'
```

Expected: `{}` (empty response on success).

### 10. Check Balance

```bash
curl -X POST http://localhost:3001/btcbalance \
  -H "Content-Type: application/json" \
  -d '{"skip_sync": false}'
```

Expected: `"settled": 100000000` (1 BTC in sats)

### 11. Create UTXOs for RGB

Before issuing RGB assets, we need to create "uncolored" UTXOs:

```bash
curl -X POST http://localhost:3001/createutxos \
  -H "Content-Type: application/json" \
  -d '{
    "up_to": false,
    "num": 10,
    "size": null,
    "fee_rate": 1,
    "skip_sync": false
  }'
```

Expected: `{}` (empty response on success).

This creates 10 UTXOs that can be used for RGB operations.

Wait for response, then mine blocks and sync:

```bash
# Mine blocks to confirm UTXO creation
./regtest.sh mine 1

# Sync the node
curl -X POST http://localhost:3001/sync \
  -H "Content-Type: application/json" \
  -d '{}'
```

### 12. Issue RGB Asset (NIA - Non-Inflatable Asset)

```bash
curl -X POST http://localhost:3001/issueassetnia \
  -H "Content-Type: application/json" \
  -d '{
    "ticker": "FIRE",
    "name": "FireToken",
    "amounts": [1000],
    "precision": 0
  }'
```

**IMPORTANT:** Save the `asset_id` from response! It will be in the format `contract:<id>` (e.g., `contract:xFS2uLdp-Kmlb4cS-IgMgte9-Va0JCwx-5huCTjs-N1VLCQw`).

**Note:** There are 3 asset types available:
- `/issueassetnia` - Non-Inflatable Assets (like this example)
- `/issueassetcfa` - Collectible Fungible Assets
- `/issueassetuda` - Unique Digital Assets

### 13. Check RGB Balance

```bash
# Use the asset_id from the previous response
curl -X POST http://localhost:3001/assetbalance \
  -H "Content-Type: application/json" \
  -d '{
    "asset_id": "<asset_id_from_step_12>"
  }'
```

Expected response includes: `"settled": 1000`

## Baseline Testing Complete

At this point you have:
- ✅ RGB Lightning Node running
- ✅ Node funded with Bitcoin
- ✅ Uncolored UTXOs created for RGB operations
- ✅ RGB asset issued (1000 FIRE tokens)
- ✅ Asset balance confirmed

## Multi-Node Testing: Lightning Channels & RGB Payments

### 14. Start Node 2 (Bob)

In a **new terminal**:

```bash
cd f1r3fly-rgb-lightning-node
./target/debug/rgb-lightning-node dataldk1/ \
    --daemon-listening-port 3003 \
    --ldk-peer-listening-port 9736 \
    --network regtest \
    --disable-authentication
```

Keep this terminal open.

### 15. Initialize & Unlock Bob

In another terminal:

```bash
# Initialize
curl -X POST http://localhost:3003/init \
  -H "Content-Type: application/json" \
  -d '{"password": "password123"}'
```

Save Bob's mnemonic!

```bash
# Unlock
curl -X POST http://localhost:3003/unlock \
  -H "Content-Type: application/json" \
  -d '{
    "password": "password123",
    "bitcoind_rpc_username": "user",
    "bitcoind_rpc_password": "password",
    "bitcoind_rpc_host": "localhost",
    "bitcoind_rpc_port": 18443,
    "indexer_url": "127.0.0.1:50001",
    "proxy_endpoint": "rpc://127.0.0.1:3000/json-rpc",
    "announce_addresses": []
  }'
```

### 16. Fund Bob

```bash
# Get Bob's address
curl -X POST http://localhost:3003/address \
  -H "Content-Type: application/json" \
  -d '{}'

# Send 1 BTC
./regtest.sh sendtoaddress <bob_address> 1

# Mine blocks
./regtest.sh mine 6

# Sync
curl -X POST http://localhost:3003/sync \
  -H "Content-Type: application/json" \
  -d '{}'
```

### 17. Create UTXOs for Bob

```bash
curl -X POST http://localhost:3003/createutxos \
  -H "Content-Type: application/json" \
  -d '{
    "up_to": false,
    "num": 10,
    "size": null,
    "fee_rate": 1,
    "skip_sync": false
  }'

# Mine and sync
./regtest.sh mine 1
curl -X POST http://localhost:3003/sync -H "Content-Type: application/json" -d '{}'
```

### 18. Get Node Public Keys

```bash
# Alice's node info
curl http://localhost:3001/nodeinfo

# Bob's node info
curl http://localhost:3003/nodeinfo
```

**IMPORTANT:** Save both `pubkey` values!

### 19. Connect Peers

Connect Alice to Bob:

```bash
curl -X POST http://localhost:3001/connectpeer \
  -H "Content-Type: application/json" \
  -d '{
    "peer_pubkey_and_addr": "<bob_node_pubkey>@127.0.0.1:9736"
  }'
```

Verify:

```bash
curl http://localhost:3001/listpeers
```

Should show Bob connected.

### 20. Open Lightning Channel (BTC only first)

Open a vanilla Bitcoin channel first to test:

```bash
curl -X POST http://localhost:3001/openchannel \
  -H "Content-Type: application/json" \
  -d '{
    "peer_pubkey_and_opt_addr": "<bob_node_pubkey>",
    "capacity_sat": 100000,
    "push_msat": 50000000,
    "public": false,
    "with_anchors": false
  }'
```

This opens a 100k sat channel with 50k pushed to Bob.

Mine blocks to confirm:

```bash
./regtest.sh mine 6
```

Wait ~10 seconds, then check:

```bash
curl http://localhost:3001/listchannels
curl http://localhost:3003/listchannels
```

Both should show the channel as active.

### 21. Open RGB Channel

Now open a channel with RGB assets:

```bash
curl -X POST http://localhost:3001/openchannel \
  -H "Content-Type: application/json" \
  -d '{
    "peer_pubkey_and_opt_addr": "<bob_node_pubkey>",
    "capacity_sat": 100000,
    "push_msat": 0,
    "asset_amount": 500,
    "asset_id": "<alice_fire_asset_id>",
    "public": false,
    "with_anchors": true
  }'
```

**Note:** RGB channels require `with_anchors: true` for the RGB commitment anchor.

This opens a channel with:
- 100k sats capacity
- 500 FIRE tokens from Alice

Mine and confirm:

```bash
./regtest.sh mine 6
sleep 10

curl http://localhost:3001/listchannels
```

### 22. Create Lightning RGB Invoice (Bob)

Bob creates a Lightning invoice to receive 100 FIRE tokens:

```bash
curl -X POST http://localhost:3003/lninvoice \
  -H "Content-Type: application/json" \
  -d '{
    "amt_msat": 3000000,
    "expiry_sec": 900,
    "asset_id": "<alice_fire_asset_id>",
    "asset_amount": 100
  }'
```

**Note:** 
- Use `/lninvoice` for Lightning channel payments
- Use `/rgbinvoice` for on-chain RGB transfers (different use case!)
- `amt_msat`: Bitcoin amount (millisatoshis) - can be 0 for RGB-only payments
- `asset_amount`: The amount of RGB assets to receive

**IMPORTANT:** Save the `invoice` string from the response!

### 23. Send RGB Lightning Payment (Alice)

Alice pays Bob's Lightning invoice:

```bash
curl -X POST http://localhost:3001/sendpayment \
  -H "Content-Type: application/json" \
  -d '{
    "invoice": "<bob_lightning_invoice>"
  }'
```

**Note:** The `sendpayment` endpoint only needs the invoice string. The asset details are encoded in the Lightning invoice itself.

Expected: Payment succeeds, you'll get a response with `payment_id` and `payment_hash`.

### 24. Verify RGB Balances

Check Alice's balance:

```bash
curl -X POST http://localhost:3001/assetbalance \
  -H "Content-Type: application/json" \
  -d '{
    "asset_id": "<alice_fire_asset_id>"
  }'
```

Check Bob's balance:

```bash
curl -X POST http://localhost:3003/assetbalance \
  -H "Content-Type: application/json" \
  -d '{
    "asset_id": "<alice_fire_asset_id>"
  }'
```

Expected output (Alice):
```json
{
  "settled": 500,
  "future": 500,
  "spendable": 500,
  "offchain_outbound": 400,
  "offchain_inbound": 100
}
```

Expected output (Bob):
```json
{
  "settled": 0,
  "future": 0,
  "spendable": 0,
  "offchain_outbound": 100,
  "offchain_inbound": 400
}
```

**Balance fields explained:**
- `settled` - Confirmed on-chain balance
- `spendable` - Available for on-chain transfers
- `offchain_outbound` - Can send this much via Lightning
- `offchain_inbound` - Can receive this much via Lightning

## Testing Complete! 🎉

You now have:
- ✅ Two RGB Lightning Nodes running
- ✅ Vanilla BTC channel working
- ✅ RGB asset channel open
- ✅ RGB payment sent via Lightning
- ✅ Balances verified

## Understanding the Flow

**What just happened:**

1. **Channel Creation**: Alice locked 500 FIRE tokens in a Lightning channel with Bob
2. **Invoice Generation**: Bob created a Lightning invoice for RGB assets using `/lninvoice` (encodes asset type and amount)
3. **RGB HTLC**: Alice sent 100 FIRE via Lightning (just like BTC, but for RGB) using `/sendpayment`
4. **State Update**: Both nodes updated their local channel state
5. **Client-Side Validation**: `rgb-lib` tracked the asset movements and validated state transitions

**Key Endpoints:**
- `/lninvoice` - Create Lightning invoice for RGB assets (used for channel payments)
- `/rgbinvoice` - Create RGB invoice for on-chain transfers (different from Lightning!)
- `/sendpayment` - Pay a Lightning invoice (both BTC and RGB)
- `/sendasset` - Send RGB assets on-chain (not through Lightning channels)

## Teardown

### Quick Teardown

```bash
cd f1r3fly-rgb-lightning-node

# 1. Stop the Lightning nodes (Ctrl+C in their terminals)
# Press Ctrl+C in the terminal running Alice (port 3001)
# Press Ctrl+C in the terminal running Bob (port 3003)

# 2. Stop Docker services
./regtest.sh stop

# 3. Verify shutdown
docker compose ps
```

### Complete Cleanup (Optional)

If you want to remove all data and start fresh:

```bash
cd f1r3fly-rgb-lightning-node

# Stop services
./regtest.sh stop

# Remove Docker volumes (blockchain data, etc.)
docker compose down -v

# Remove node data directories
rm -rf dataldk0/ dataldk1/

# Remove logs
rm -rf logs/
```

