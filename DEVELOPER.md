# F1r3fly RGB Lightning Node - Developer Guide

## Quick Start

### Option 1: Docker Compose (Recommended)

Start all infrastructure services and lightning nodes:

```bash
docker compose -f compose.yaml -f compose-nodes.yaml up -d
```

Check status:

```bash
docker compose -f compose.yaml -f compose-nodes.yaml ps
```

View logs:

```bash
# All services
docker compose -f compose.yaml -f compose-nodes.yaml logs -f

# Specific service
docker compose -f compose.yaml -f compose-nodes.yaml logs -f alice
docker compose -f compose.yaml -f compose-nodes.yaml logs -f bob
```

Stop all services:

```bash
docker compose -f compose.yaml -f compose-nodes.yaml down -v
```

### Option 2: regtest.sh + Manual Nodes

Start infrastructure only:

```bash
./regtest.sh start
```

Then run nodes manually (see [SETUP.md](SETUP.md) for details):

```bash
# Terminal 1 - Alice
./target/debug/rgb-lightning-node dataldk0/ \
    --daemon-listening-port 3001 \
    --ldk-peer-listening-port 9735 \
    --network regtest \
    --disable-authentication

# Terminal 2 - Bob
./target/debug/rgb-lightning-node dataldk1/ \
    --daemon-listening-port 3003 \
    --ldk-peer-listening-port 9736 \
    --network regtest \
    --disable-authentication
```

Stop infrastructure:

```bash
./regtest.sh stop
```

## Services

| Service   | Port  | Description                  |
|-----------|-------|------------------------------|
| bitcoind  | 18443 | Bitcoin Core regtest RPC     |
| electrs   | 50001 | Electrum server              |
| proxy     | 3000  | RGB proxy server             |
| f1r3node  | 40403 | F1r3fly node HTTP API        |
| alice     | 3001  | Lightning node 1 (Alice) API |
| bob       | 3003  | Lightning node 2 (Bob) API   |

## Node Initialization

After starting, nodes require initialization. This is the same for both Docker and manual modes:

```bash
# Initialize Alice
curl -X POST http://localhost:3001/init \
  -H "Content-Type: application/json" \
  -d '{"password": "password123"}'

# Unlock Alice (Docker mode - use container hostnames)
curl -X POST http://localhost:3001/unlock \
  -H "Content-Type: application/json" \
  -d '{
    "password": "password123",
    "bitcoind_rpc_username": "user",
    "bitcoind_rpc_password": "password",
    "bitcoind_rpc_host": "bitcoind",
    "bitcoind_rpc_port": 18443,
    "indexer_url": "electrs:50001",
    "proxy_endpoint": "rpc://proxy:3000/json-rpc",
    "announce_addresses": []
  }'
```

> **Note:** When using Docker Compose, use container hostnames (`bitcoind`, `electrs`, `proxy`) instead of `localhost`.

For manual mode, use `localhost` and `127.0.0.1`:

```bash
# Unlock Alice (Manual mode - use localhost)
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

## Example Workflow

See [SETUP.md](SETUP.md) for a complete step-by-step guide covering:

- Funding nodes with Bitcoin
- Issuing RGB assets
- Opening Lightning channels (BTC and RGB)
- Sending RGB payments over Lightning
- Verifying balances

## Useful Commands

### Mining Blocks

```bash
./regtest.sh mine 6
```

### Send Bitcoin

```bash
./regtest.sh sendtoaddress <address> <amount>
```

### Check Node Status

```bash
curl http://localhost:3001/nodeinfo
curl http://localhost:3003/nodeinfo
```

### List Channels

```bash
curl http://localhost:3001/listchannels
curl http://localhost:3003/listchannels
```

## Cleanup

Remove all data and start fresh:

```bash
# Stop services
docker compose -f compose.yaml -f compose-nodes.yaml down -v

# Remove node data
rm -rf dataldk0/ dataldk1/ datacore/ dataindex/

# Remove logs
rm -rf logs/
```

