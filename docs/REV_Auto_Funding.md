# REV Auto-Funding for RGB Operations

## Overview

RGB operations on F1r3node require REV tokens to pay for Rholang execution (phlo). Each wallet derives a unique F1r3fly key from its mnemonic, and this key needs REV before issuing assets, transferring tokens, or claiming consignments.

## How It Works

The F1r3fly RGB adapter automatically funds wallets with REV during initialization when running on **regtest**.

### Flow

1. Wallet is created/imported via `WalletManager`
2. F1r3fly master public key is derived from mnemonic
3. Pubkey is written to `.ldk/my_wallet_pubkey`
4. If `REV_FAUCET_PRIVATE_KEY` is configured, the adapter:
   - Creates a `RevFaucet` using the faucet private key
   - Transfers REV to the wallet's F1r3fly address
   - Logs the funding result

### Code Location

```
src/f1r3fly_rgb_adapter.rs (lines ~150-175)
```

## Configuration

Set these environment variables (or in `.env`):

```bash
# F1r3node connection
FIREFLY_HOST=127.0.0.1
FIREFLY_GRPC_PORT=40401
FIREFLY_HTTP_PORT=40403

# REV faucet (regtest only)
REV_FAUCET_PRIVATE_KEY=5f668a7ee96d944a4494cc947e4005e172d7ab3461ee5538f1f2a45a835e9657
REV_FAUCET_AMOUNT=100000000  # 1 REV in dust
```

The faucet key must have sufficient REV balance. This is configured in `ci/genesis/wallets.txt`:

```
1111AtahZeefej4tvVR6ti9TJtv8yxLebT31SCEVDCKMNikBk5r3g,500000000000000
```

## Networks

| Network | Auto-Funding                   |
| ------- | ------------------------------ |
| Regtest | Enabled (if faucet configured) |
| Signet  | Disabled                       |
| Testnet | Disabled                       |
| Mainnet | Disabled                       |

## Troubleshooting

**"REV faucet not configured"**: Set `REV_FAUCET_PRIVATE_KEY` in environment or `.env`

**"Insufficient funds"**: Increase allocation in `ci/genesis/wallets.txt` and restart F1r3node

**Asset issuance fails with "No data returned"**: Wallet has no REV. Check adapter logs for funding status.

