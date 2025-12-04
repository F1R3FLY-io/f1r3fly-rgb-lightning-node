#!/bin/bash
set -e

# Script to collect logs and artifacts for CI debugging
# Usage: ./collect-logs.sh <output-dir>

OUTPUT_DIR="${1:-ci-logs}"

echo "📦 Collecting logs and artifacts to: $OUTPUT_DIR"

# Create output directory structure
mkdir -p "$OUTPUT_DIR/docker"
mkdir -p "$OUTPUT_DIR/services"
mkdir -p "$OUTPUT_DIR/nodes"

# Collect docker compose logs
echo "  → Docker compose logs..."
docker compose logs > "$OUTPUT_DIR/docker/compose-all.log" 2>&1 || echo "No compose logs available"
docker compose logs bitcoind > "$OUTPUT_DIR/services/bitcoind.log" 2>&1 || echo "No bitcoind logs"
docker compose logs electrs > "$OUTPUT_DIR/services/electrs.log" 2>&1 || echo "No electrs logs"
docker compose logs electrs-http > "$OUTPUT_DIR/services/electrs-http.log" 2>&1 || echo "No electrs-http logs"
docker compose logs proxy > "$OUTPUT_DIR/services/proxy.log" 2>&1 || echo "No proxy logs"

# Collect container stats
echo "  → Container stats..."
docker compose ps --format json > "$OUTPUT_DIR/docker/containers.json" 2>&1 || echo "[]" > "$OUTPUT_DIR/docker/containers.json"
docker compose ps > "$OUTPUT_DIR/docker/containers.txt" 2>&1 || echo "No containers"

# Collect service health status
echo "  → Service health..."
{
    echo "=== Service Health Check ==="
    echo "Timestamp: $(date -u +"%Y-%m-%d %H:%M:%S UTC")"
    echo ""
    
    echo "=== Bitcoin Core Status ==="
    docker compose exec -T bitcoind bitcoin-cli -regtest -rpcuser=user -rpcpassword=password getblockchaininfo 2>&1 || echo "Bitcoin RPC not available"
    
    echo ""
    echo "=== Electrs TCP Status ==="
    nc -zv localhost 50001 2>&1 || echo "Electrs TCP not available"
    
    echo ""
    echo "=== Electrs HTTP Status ==="
    curl -s http://localhost:3002/blocks/tip/height 2>&1 || echo "Electrs HTTP not available"
    
    echo ""
    echo "=== RGB Proxy Status ==="
    nc -zv localhost 3000 2>&1 || echo "RGB Proxy not available"
} > "$OUTPUT_DIR/services/health-check.txt"

# Collect node test data (if exists)
echo "  → Node test data..."
if [ -d "tmp" ]; then
    echo "    Found tmp/ directory, copying..."
    cp -r tmp "$OUTPUT_DIR/nodes/" 2>/dev/null || echo "    Could not copy tmp/"
fi

# Also check parent directory for CLI test data
if [ -d "../tmp/cli_test" ]; then
    echo "    Found ../tmp/cli_test directory, copying..."
    cp -r ../tmp/cli_test "$OUTPUT_DIR/nodes/cli_test" 2>/dev/null || echo "    Could not copy ../tmp/cli_test"
fi

# Collect data directories (ldk data)
for dir in dataldk0 dataldk1 dataldk2; do
    if [ -d "$dir" ]; then
        echo "    Found $dir directory..."
        mkdir -p "$OUTPUT_DIR/nodes/$dir"
        # Copy logs subdirectory if it exists
        if [ -d "$dir/logs" ]; then
            cp -r "$dir/logs" "$OUTPUT_DIR/nodes/$dir/" 2>/dev/null || true
        fi
        # Copy any .json files for debugging
        find "$dir" -name "*.json" -exec cp {} "$OUTPUT_DIR/nodes/$dir/" \; 2>/dev/null || true
    fi
done

# Create summary
echo "  → Creating summary..."
{
    echo "=== Log Collection Summary ==="
    echo "Timestamp: $(date -u +"%Y-%m-%d %H:%M:%S UTC")"
    echo ""
    echo "=== Files Collected ==="
    find "$OUTPUT_DIR" -type f -exec echo "  {}" \; | sort
    echo ""
    echo "=== Total Size ==="
    du -sh "$OUTPUT_DIR"
} > "$OUTPUT_DIR/summary.txt"

echo "✅ Log collection complete: $OUTPUT_DIR"
echo "📊 Summary:"
cat "$OUTPUT_DIR/summary.txt"

