#!/bin/bash

# CI Test script for RGB Lightning Node
# Tests end-to-end flow: init, fund, channel open, payment, balance verification
#
# Usage: ./test-lightning-cli.sh [--clean]
#
# This script runs from within f1r3fly-rgb-lightning-node directory

set -e

# Parse flags
CLEAN_TMP=false
for arg in "$@"; do
    case $arg in
        --clean)
            CLEAN_TMP=true
            ;;
        -h|--help)
            echo "Usage: $0 [--clean]"
            exit 0
            ;;
    esac
done

# Configuration
NODE1_HTTP_PORT=3001
NODE1_PEER_PORT=9735
NODE1_DIR="tmp/cli_test/node1"

NODE2_HTTP_PORT=3003  # Avoid conflict with electrs-http on 3002
NODE2_PEER_PORT=9736
NODE2_DIR="tmp/cli_test/node2"

PASSWORD="password123"
CMD_LOG="tmp/cli_test/commands.log"
COMPOSE="docker compose"
BITCOIN_CLI="$COMPOSE exec -T -u blits bitcoind bitcoin-cli -regtest"

# Helper: Log command
log_cmd() {
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*" >> "$CMD_LOG"
}

# Helper: Mine blocks
mine_blocks() {
    local num_blocks="$1"
    log_cmd "mine $num_blocks blocks"
    $BITCOIN_CLI -rpcwallet=miner -generate "$num_blocks" > /dev/null
}

# Helper: Send to address
send_to_address() {
    local address="$1"
    local amount="$2"
    log_cmd "sendtoaddress $address $amount"
    $BITCOIN_CLI -rpcwallet=miner sendtoaddress "$address" "$amount"
}

# Helper: Call API
call_api() {
    local port="$1"
    local endpoint="$2"
    local data="$3"
    local result
    
    # GET endpoints (as per docs)
    if [ "$endpoint" = "/nodeinfo" ] || [ "$endpoint" = "/listpeers" ] || [ "$endpoint" = "/listchannels" ]; then
        log_cmd "curl -s \"http://localhost:${port}${endpoint}\""
        result=$(curl -s "http://localhost:${port}${endpoint}")
    elif [ -z "$data" ]; then
        log_cmd "curl -s \"http://localhost:${port}${endpoint}\""
        result=$(curl -s "http://localhost:${port}${endpoint}")
    else
        log_cmd "curl -s -X POST \"http://localhost:${port}${endpoint}\" -H \"Content-Type: application/json\" -d '$data'"
        result=$(curl -s -X POST "http://localhost:${port}${endpoint}" \
            -H "Content-Type: application/json" \
            -d "$data")
    fi
    
    # Log full response
    log_cmd "Response: $result"
    
    echo "$result"
}

# Helper: Wait for API
wait_for_api() {
    local port="$1"
    local max_attempts=30
    local attempt=0
    
    while [ $attempt -lt $max_attempts ]; do
        if curl -s "http://localhost:${port}/nodeinfo" > /dev/null 2>&1; then
            return 0
        fi
        sleep 1
        attempt=$((attempt + 1))
    done
    return 1
}

# Cleanup previous run
if [ "$CLEAN_TMP" = true ] && [ -d "tmp/cli_test" ]; then
    echo "Cleaning up previous test..."
    rm -rf tmp/cli_test
fi

# Kill any processes on our ports
for port in $NODE1_HTTP_PORT $NODE2_HTTP_PORT; do
    pid=$(lsof -ti ":$port" 2>/dev/null || true)
    if [ -n "$pid" ]; then
        echo "Killing process on port $port (PID: $pid)"
        kill -9 "$pid" 2>/dev/null || true
        sleep 1
    fi
done

mkdir -p "$NODE1_DIR" "$NODE2_DIR"

# Initialize command log
echo "# Command log - $(date)" > "$CMD_LOG"
echo "# RGB Lightning CLI Test" >> "$CMD_LOG"
echo "" >> "$CMD_LOG"

echo "=========================================="
echo "RGB Lightning CLI Test"
echo "=========================================="
echo ""

# Step 1: Verify Services
echo "Step 1: Verify Services"
echo "========================"
echo ""

# Check bitcoind
echo -n "Checking bitcoind... "
if $BITCOIN_CLI getblockchaininfo > /dev/null 2>&1; then
    echo "✓"
else
    echo "✗"
    echo "Error: bitcoind not running"
    exit 1
fi

# Check electrs
echo -n "Checking electrs... "
if nc -z localhost 50001 2>/dev/null; then
    echo "✓"
else
    echo "✗"
    echo "Error: electrs not running on port 50001"
    exit 1
fi

# Check RGB proxy
echo -n "Checking RGB proxy... "
if nc -z localhost 3000 2>/dev/null; then
    echo "✓"
else
    echo "✗"
    echo "Error: RGB proxy not running on port 3000"
    exit 1
fi
echo ""

# Step 2: Verify Binary
echo "Step 2: Verify Binary"
echo "====================="
echo ""

BINARY_PATH="./target/debug/rgb-lightning-node"
if [ ! -f "$BINARY_PATH" ]; then
    echo "Error: Binary not found at $BINARY_PATH"
    echo "Build with: cargo build --bin rgb-lightning-node"
    exit 1
fi
echo "✓ Binary exists: $BINARY_PATH"
$BINARY_PATH --version 2>/dev/null || $BINARY_PATH --help | head -3
echo ""

# Step 3: Start Node 1 (Alice)
echo "Step 3: Starting Node 1..."
$BINARY_PATH "$NODE1_DIR" \
    --daemon-listening-port "$NODE1_HTTP_PORT" \
    --ldk-peer-listening-port "$NODE1_PEER_PORT" \
    --network regtest \
    --disable-authentication > tmp/cli_test/node1.log 2>&1 &
NODE1_PID=$!

wait_for_api "$NODE1_HTTP_PORT"
echo "✓ Node 1 ready (PID: $NODE1_PID)"
echo ""

# Step 4: Initialize Node 1
echo "Step 4: Initialize Node 1..."
INIT_RESP=$(call_api "$NODE1_HTTP_PORT" "/init" "{\"password\": \"$PASSWORD\"}")
echo "$INIT_RESP" | jq '.'
sleep 3  # Wait for wallet persistence
echo ""

# Step 5: Unlock Node 1
echo "Step 5: Unlock Node 1..."
call_api "$NODE1_HTTP_PORT" "/unlock" "{
  \"password\": \"$PASSWORD\",
  \"bitcoind_rpc_username\": \"user\",
  \"bitcoind_rpc_password\": \"password\",
  \"bitcoind_rpc_host\": \"localhost\",
  \"bitcoind_rpc_port\": 18443,
  \"indexer_url\": \"127.0.0.1:50001\",
  \"proxy_endpoint\": \"rpc://127.0.0.1:3000/json-rpc\",
  \"announce_addresses\": []
}" > /dev/null
sleep 5  # Wait for unlock to complete
echo "✓ Unlocked"
echo ""

# Step 6: Get Node 1 address and fund
echo "Step 6: Get Bitcoin address..."
ADDR_RESP=$(call_api "$NODE1_HTTP_PORT" "/address" "{}")
NODE1_ADDR=$(echo "$ADDR_RESP" | jq -r '.address')
echo "Address: $NODE1_ADDR"
echo ""

echo "Step 7: Fund Node 1 with 1 BTC..."
send_to_address "$NODE1_ADDR" 1 > /dev/null
echo ""

echo "Step 8: Mine 6 blocks..."
mine_blocks 6
sleep 5
echo ""

# Step 9: Sync and check balance
echo "Step 9: Sync Node 1 and verify balance..."
MAX_BALANCE_RETRIES=10
BALANCE_RETRY=0
SETTLED=0

while [ "$BALANCE_RETRY" -lt "$MAX_BALANCE_RETRIES" ] && [ "$SETTLED" != "100000000" ]; do
    call_api "$NODE1_HTTP_PORT" "/sync" "{}" > /dev/null
    sleep 2
    BALANCE=$(call_api "$NODE1_HTTP_PORT" "/btcbalance" "{\"skip_sync\": false}")
    SETTLED=$(echo "$BALANCE" | jq -r '.vanilla.settled')
    
    if [ "$SETTLED" = "100000000" ]; then
        echo "✓ Balance confirmed: $SETTLED sats"
        break
    fi
    
    BALANCE_RETRY=$((BALANCE_RETRY + 1))
    echo "  Balance: $SETTLED sats (attempt $BALANCE_RETRY/$MAX_BALANCE_RETRIES)"
    sleep 3
done

if [ "$SETTLED" != "100000000" ]; then
    echo "✗ Node 1 balance not settled after $MAX_BALANCE_RETRIES attempts"
    echo "  Expected: 100000000, Got: $SETTLED"
    exit 1
fi
echo ""

# Step 10: Create UTXOs
echo "Step 10: Create UTXOs..."
UTXO_RESP=$(call_api "$NODE1_HTTP_PORT" "/createutxos" "{
  \"up_to\": false,
  \"num\": 10,
  \"size\": null,
  \"fee_rate\": 1,
  \"skip_sync\": false
}")

if echo "$UTXO_RESP" | jq -e '.code' > /dev/null 2>&1; then
    echo "✗ createutxos failed: $UTXO_RESP"
    exit 1
fi
echo "✓ UTXOs created"

mine_blocks 1
sleep 5
call_api "$NODE1_HTTP_PORT" "/sync" "{}" > /dev/null
echo ""

# Note: REV funding is now handled automatically by the F1r3fly adapter
# during wallet initialization (see f1r3fly_rgb_adapter.rs)

# Step 11: Issue RGB Asset
echo "Step 11: Issue RGB asset..."
ISSUE_RESP=$(call_api "$NODE1_HTTP_PORT" "/issueassetnia" "{
  \"ticker\": \"FIRE\",
  \"name\": \"FireToken\",
  \"amounts\": [1000],
  \"precision\": 0
}")
ASSET_ID=$(echo "$ISSUE_RESP" | jq -r '.asset.asset_id')
echo "Asset ID: $ASSET_ID"

# Verify RGB balance
RGB_BAL_RESP=$(call_api "$NODE1_HTTP_PORT" "/assetbalance" "{\"asset_id\": \"$ASSET_ID\"}")
RGB_BALANCE=$(echo "$RGB_BAL_RESP" | jq -r '.settled')
echo "Balance: $RGB_BALANCE FIRE"
if [ "$RGB_BALANCE" != "1000" ]; then
    echo "✗ Expected 1000 FIRE, got $RGB_BALANCE"
    exit 1
fi
echo "✓ RGB balance confirmed"
echo ""

# Step 12: Start Node 2 (Bob)
echo "Step 12: Starting Node 2..."
$BINARY_PATH "$NODE2_DIR" \
    --daemon-listening-port "$NODE2_HTTP_PORT" \
    --ldk-peer-listening-port "$NODE2_PEER_PORT" \
    --network regtest \
    --disable-authentication > tmp/cli_test/node2.log 2>&1 &
NODE2_PID=$!

wait_for_api "$NODE2_HTTP_PORT"
echo "✓ Node 2 ready (PID: $NODE2_PID)"
echo ""

# Step 13: Initialize & Unlock Node 2
echo "Step 13: Initialize Node 2..."
INIT_RESP=$(call_api "$NODE2_HTTP_PORT" "/init" "{\"password\": \"$PASSWORD\"}")
echo "$INIT_RESP" | jq '.'
sleep 3
echo ""

echo "Step 14: Unlock Node 2..."
call_api "$NODE2_HTTP_PORT" "/unlock" "{
  \"password\": \"$PASSWORD\",
  \"bitcoind_rpc_username\": \"user\",
  \"bitcoind_rpc_password\": \"password\",
  \"bitcoind_rpc_host\": \"localhost\",
  \"bitcoind_rpc_port\": 18443,
  \"indexer_url\": \"127.0.0.1:50001\",
  \"proxy_endpoint\": \"rpc://127.0.0.1:3000/json-rpc\",
  \"announce_addresses\": []
}" > /dev/null
sleep 5
echo "✓ Unlocked"
echo ""

# Step 15: Fund Node 2
echo "Step 15: Get Node 2 address..."
ADDR_RESP=$(call_api "$NODE2_HTTP_PORT" "/address" "{}")
NODE2_ADDR=$(echo "$ADDR_RESP" | jq -r '.address')
echo "Address: $NODE2_ADDR"

echo "Funding Node 2 with 1 BTC..."
send_to_address "$NODE2_ADDR" 1 > /dev/null
mine_blocks 6
sleep 5
echo ""

echo "Step 16: Sync Node 2 and verify balance..."
MAX_BALANCE_RETRIES=10
BALANCE_RETRY=0
SETTLED=0

while [ "$BALANCE_RETRY" -lt "$MAX_BALANCE_RETRIES" ] && [ "$SETTLED" != "100000000" ]; do
    call_api "$NODE2_HTTP_PORT" "/sync" "{}" > /dev/null
    sleep 2
    BALANCE=$(call_api "$NODE2_HTTP_PORT" "/btcbalance" "{\"skip_sync\": false}")
    SETTLED=$(echo "$BALANCE" | jq -r '.vanilla.settled')
    
    if [ "$SETTLED" = "100000000" ]; then
        echo "✓ Balance confirmed: $SETTLED sats"
        break
    fi
    
    BALANCE_RETRY=$((BALANCE_RETRY + 1))
    echo "  Balance: $SETTLED sats (attempt $BALANCE_RETRY/$MAX_BALANCE_RETRIES)"
    sleep 3
done

if [ "$SETTLED" != "100000000" ]; then
    echo "✗ Node 2 balance not settled"
    exit 1
fi
echo ""

# Step 17: Create UTXOs for Node 2
echo "Step 17: Create UTXOs for Node 2..."
UTXO_RESP=$(call_api "$NODE2_HTTP_PORT" "/createutxos" "{
  \"up_to\": false,
  \"num\": 10,
  \"size\": null,
  \"fee_rate\": 1,
  \"skip_sync\": false
}")

if echo "$UTXO_RESP" | jq -e '.code' > /dev/null 2>&1; then
    echo "✗ createutxos failed: $UTXO_RESP"
    exit 1
fi
echo "✓ UTXOs created"

mine_blocks 1
sleep 5
call_api "$NODE2_HTTP_PORT" "/sync" "{}" > /dev/null
echo ""

# Step 18: Get node pubkeys
echo "Step 18: Get node public keys..."
NODE1_INFO=$(call_api "$NODE1_HTTP_PORT" "/nodeinfo")
NODE1_PUBKEY=$(echo "$NODE1_INFO" | jq -r '.pubkey // empty')
echo "Alice: $NODE1_PUBKEY"

NODE2_INFO=$(call_api "$NODE2_HTTP_PORT" "/nodeinfo")
NODE2_PUBKEY=$(echo "$NODE2_INFO" | jq -r '.pubkey // empty')
echo "Bob: $NODE2_PUBKEY"

if [ -z "$NODE1_PUBKEY" ] || [ -z "$NODE2_PUBKEY" ]; then
    echo "Error: Could not get node pubkeys"
    exit 1
fi
echo ""

# Step 19: Connect peers
echo "Step 19: Connect Alice to Bob..."
call_api "$NODE1_HTTP_PORT" "/connectpeer" "{
  \"peer_pubkey_and_addr\": \"${NODE2_PUBKEY}@127.0.0.1:${NODE2_PEER_PORT}\"
}" > /dev/null
sleep 2

PEERS=$(call_api "$NODE1_HTTP_PORT" "/listpeers")
echo "✓ Connected ($(echo "$PEERS" | jq -r '.peers | length') peers)"
echo ""

# Step 20: Open vanilla BTC channel
echo "Step 20: Open vanilla BTC channel..."
VANILLA_RESP=$(call_api "$NODE1_HTTP_PORT" "/openchannel" "{
  \"peer_pubkey_and_opt_addr\": \"$NODE2_PUBKEY\",
  \"capacity_sat\": 100000,
  \"push_msat\": 50000000,
  \"public\": false,
  \"with_anchors\": false
}")
TEMP_ID=$(echo "$VANILLA_RESP" | jq -r '.temporary_channel_id // empty')

if [ -z "$TEMP_ID" ] || [ "$TEMP_ID" = "null" ]; then
    echo "Error: Failed to open vanilla channel"
    echo "Response: $VANILLA_RESP"
    exit 1
fi
echo "Temporary ID: $TEMP_ID"

mine_blocks 6
sleep 5
echo ""

echo "Step 20b: Wait for vanilla BTC channel to be ready..."
MAX_ATTEMPTS=15
ATTEMPT=0
VANILLA_READY="false"

while [ "$ATTEMPT" -lt "$MAX_ATTEMPTS" ] && [ "$VANILLA_READY" != "true" ]; do
    mine_blocks 1
    sleep 2
    
    call_api "$NODE1_HTTP_PORT" "/sync" "{}" > /dev/null
    call_api "$NODE2_HTTP_PORT" "/sync" "{}" > /dev/null
    
    CHANNELS=$(call_api "$NODE1_HTTP_PORT" "/listchannels")
    VANILLA_READY=$(echo "$CHANNELS" | jq -r '.channels[] | select(.asset_id == null) | .ready // false')
    
    if [ "$VANILLA_READY" = "true" ]; then
        echo "✓ Vanilla BTC channel is ready"
        break
    fi
    
    ATTEMPT=$((ATTEMPT + 1))
    if [ $((ATTEMPT % 5)) -eq 0 ]; then
        echo "  ... waiting for vanilla channel (attempt $ATTEMPT/$MAX_ATTEMPTS)"
    fi
done

if [ "$VANILLA_READY" != "true" ]; then
    echo "✗ Vanilla channel not ready after 30 seconds"
    echo "Current channels:"
    echo "$CHANNELS" | jq '.'
    exit 1
fi
echo ""

# Step 21: Open RGB channel
echo "Step 21: Open RGB channel..."
RGB_RESP=$(call_api "$NODE1_HTTP_PORT" "/openchannel" "{
  \"peer_pubkey_and_opt_addr\": \"$NODE2_PUBKEY\",
  \"capacity_sat\": 100000,
  \"push_msat\": 0,
  \"asset_amount\": 500,
  \"asset_id\": \"$ASSET_ID\",
  \"public\": false,
  \"with_anchors\": true
}")
RGB_TEMP_ID=$(echo "$RGB_RESP" | jq -r '.temporary_channel_id // empty')

if [ -z "$RGB_TEMP_ID" ] || [ "$RGB_TEMP_ID" = "null" ]; then
    echo "Error: Failed to open RGB channel"
    echo "Response: $RGB_RESP"
    exit 1
fi
echo "Temporary ID: $RGB_TEMP_ID"

mine_blocks 6
sleep 10
echo ""

echo "Step 21b: Wait for RGB channel to be ready..."
MAX_ATTEMPTS=15
ATTEMPT=0
RGB_READY="false"

while [ "$ATTEMPT" -lt "$MAX_ATTEMPTS" ] && [ "$RGB_READY" != "true" ]; do
    mine_blocks 1
    sleep 2
    
    call_api "$NODE1_HTTP_PORT" "/sync" "{}" > /dev/null
    call_api "$NODE2_HTTP_PORT" "/sync" "{}" > /dev/null
    
    CHANNELS=$(call_api "$NODE1_HTTP_PORT" "/listchannels")
    RGB_READY=$(echo "$CHANNELS" | jq -r ".channels[] | select(.asset_id == \"$ASSET_ID\") | .ready // false")
    
    if [ "$RGB_READY" = "true" ]; then
        echo "✓ RGB channel is ready"
        break
    fi
    
    ATTEMPT=$((ATTEMPT + 1))
    if [ $((ATTEMPT % 5)) -eq 0 ]; then
        echo "  ... waiting for RGB channel (attempt $ATTEMPT/$MAX_ATTEMPTS)"
    fi
done

if [ "$RGB_READY" != "true" ]; then
    echo "✗ RGB channel not ready after 30 seconds"
    echo "Current channels:"
    echo "$CHANNELS" | jq '.'
    exit 1
fi
echo ""

# Step 22: Create RGB Lightning invoice
echo "Step 22: Bob creates RGB invoice..."
INVOICE_RESP=$(call_api "$NODE2_HTTP_PORT" "/lninvoice" "{
  \"amt_msat\": 3000000,
  \"expiry_sec\": 900,
  \"asset_id\": \"$ASSET_ID\",
  \"asset_amount\": 100
}")
INVOICE=$(echo "$INVOICE_RESP" | jq -r '.invoice')
echo "Invoice: ${INVOICE:0:50}..."
echo ""

# Step 23: Send RGB Lightning payment
echo "Step 23: Alice sends payment..."
PAYMENT_RESP=$(call_api "$NODE1_HTTP_PORT" "/sendpayment" "{
  \"invoice\": \"$INVOICE\"
}")
echo "$PAYMENT_RESP" | jq '.'
echo ""

sleep 5

# Step 24: Verify balances
echo "Step 24: Verify balances..."
echo ""

echo "Alice RGB balance:"
ALICE_BAL=$(call_api "$NODE1_HTTP_PORT" "/assetbalance" "{\"asset_id\": \"$ASSET_ID\"}")
echo "$ALICE_BAL" | jq '.'

ALICE_OUTBOUND=$(echo "$ALICE_BAL" | jq -r '.offchain_outbound')
ALICE_INBOUND=$(echo "$ALICE_BAL" | jq -r '.offchain_inbound')
echo ""

echo "Bob RGB balance:"
BOB_BAL=$(call_api "$NODE2_HTTP_PORT" "/assetbalance" "{\"asset_id\": \"$ASSET_ID\"}")
echo "$BOB_BAL" | jq '.'

BOB_OUTBOUND=$(echo "$BOB_BAL" | jq -r '.offchain_outbound')
BOB_INBOUND=$(echo "$BOB_BAL" | jq -r '.offchain_inbound')
echo ""

# Expected after payment:
# Alice: started with 500 in channel, sent 100 -> outbound=400, inbound=100
# Bob: started with 0 in channel, received 100 -> outbound=100, inbound=400
echo "Verifying balances..."
BALANCE_OK=true

if [ "$ALICE_OUTBOUND" != "400" ]; then
    echo "✗ Alice offchain_outbound: expected 400, got $ALICE_OUTBOUND"
    BALANCE_OK=false
else
    echo "✓ Alice offchain_outbound: 400"
fi

if [ "$ALICE_INBOUND" != "100" ]; then
    echo "✗ Alice offchain_inbound: expected 100, got $ALICE_INBOUND"
    BALANCE_OK=false
else
    echo "✓ Alice offchain_inbound: 100"
fi

if [ "$BOB_OUTBOUND" != "100" ]; then
    echo "✗ Bob offchain_outbound: expected 100, got $BOB_OUTBOUND"
    BALANCE_OK=false
else
    echo "✓ Bob offchain_outbound: 100"
fi

if [ "$BOB_INBOUND" != "400" ]; then
    echo "✗ Bob offchain_inbound: expected 400, got $BOB_INBOUND"
    BALANCE_OK=false
else
    echo "✓ Bob offchain_inbound: 400"
fi

echo ""

if [ "$BALANCE_OK" = "false" ]; then
    echo "✗ Balance verification FAILED"
    exit 1
fi

echo "✓ All balances verified correctly!"
echo ""

# Cleanup
echo "=========================================="
echo "Test PASSED"
echo "=========================================="
echo ""
kill $NODE1_PID $NODE2_PID 2>/dev/null || true
echo "Logs saved in tmp/cli_test/"
echo ""

