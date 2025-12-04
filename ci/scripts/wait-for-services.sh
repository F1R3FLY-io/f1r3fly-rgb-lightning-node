#!/bin/bash
set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

TIMEOUT=180

echo -e "${BLUE}===========================================${NC}"
echo -e "${BLUE}Waiting for Test Services to be Ready${NC}"
echo -e "${BLUE}===========================================${NC}"
echo ""

# Function to wait for a port to be available
wait_for_port() {
    local port=$1
    local description=$2
    local start_time=$(date +%s)
    
    echo -e "${YELLOW}⏳ Waiting for $description (port $port)...${NC}"
    
    while true; do
        current_time=$(date +%s)
        elapsed=$((current_time - start_time))
        
        if [ $elapsed -gt $TIMEOUT ]; then
            echo -e "${RED}❌ Timeout waiting for $description (${TIMEOUT}s)${NC}"
            exit 1
        fi
        
        if nc -z localhost $port 2>/dev/null; then
            echo -e "${GREEN}✅ $description is ready (${elapsed}s)${NC}"
            return 0
        fi
        
        echo -ne "${YELLOW}⏱️  Waiting for $description... ${elapsed}s / ${TIMEOUT}s\r${NC}"
        sleep 2
    done
}

# Function to wait for specific log message in Docker Compose
wait_for_log() {
    local service=$1
    local pattern=$2
    local description=$3
    local start_time=$(date +%s)
    
    echo -e "${YELLOW}⏳ Waiting for $description...${NC}"
    
    while true; do
        current_time=$(date +%s)
        elapsed=$((current_time - start_time))
        
        if [ $elapsed -gt $TIMEOUT ]; then
            echo -e "${RED}❌ Timeout waiting for $description (${TIMEOUT}s)${NC}"
            echo -e "${YELLOW}📋 Container logs:${NC}"
            docker compose logs --tail=100 $service
            exit 1
        fi
        
        if docker compose logs $service 2>&1 | grep -q "$pattern"; then
            echo -e "${GREEN}✅ $description detected (${elapsed}s)${NC}"
            return 0
        fi
        
        echo -ne "${YELLOW}⏱️  Waiting for $description... ${elapsed}s / ${TIMEOUT}s\r${NC}"
        sleep 2
    done
}

# Function to check Bitcoin RPC
check_bitcoin_rpc() {
    docker compose exec -T bitcoind bitcoin-cli -regtest -rpcuser=user -rpcpassword=password getblockchaininfo > /dev/null 2>&1
}

# Main wait sequence
echo -e "${BLUE}Step 1: Bitcoin Core${NC}"
wait_for_port 18443 "Bitcoin RPC"

# Additional check that RPC is responding
echo -e "${YELLOW}⏳ Verifying Bitcoin RPC responds...${NC}"
start_time=$(date +%s)
while true; do
    current_time=$(date +%s)
    elapsed=$((current_time - start_time))
    
    if [ $elapsed -gt 60 ]; then
        echo -e "${RED}❌ Bitcoin RPC not responding${NC}"
        docker compose logs bitcoind
        exit 1
    fi
    
    if check_bitcoin_rpc; then
        echo -e "${GREEN}✅ Bitcoin RPC responding (${elapsed}s)${NC}"
        break
    fi
    
    sleep 2
done

echo ""
echo -e "${BLUE}Step 2: Electrs (Electrum protocol)${NC}"
wait_for_port 50001 "Electrs TCP"
wait_for_log "electrs" "finished full compaction" "Electrs indexing complete"

echo ""
echo -e "${BLUE}Step 3: Electrs HTTP API${NC}"
wait_for_port 3002 "Electrs HTTP"

# Verify HTTP API responds
echo -e "${YELLOW}⏳ Verifying Electrs HTTP API...${NC}"
start_time=$(date +%s)
while true; do
    current_time=$(date +%s)
    elapsed=$((current_time - start_time))
    
    if [ $elapsed -gt 30 ]; then
        echo -e "${YELLOW}⚠️  Electrs HTTP API slow, proceeding anyway${NC}"
        break
    fi
    
    if curl -s http://localhost:3002/blocks/tip/height > /dev/null 2>&1; then
        height=$(curl -s http://localhost:3002/blocks/tip/height)
        echo -e "${GREEN}✅ Electrs HTTP API ready, height: $height (${elapsed}s)${NC}"
        break
    fi
    
    sleep 1
done

echo ""
echo -e "${BLUE}Step 4: RGB Proxy${NC}"
wait_for_port 3000 "RGB Proxy"

echo ""
echo -e "${GREEN}===========================================${NC}"
echo -e "${GREEN}✅ All Services Ready for Testing${NC}"
echo -e "${GREEN}===========================================${NC}"
echo ""
echo -e "${BLUE}Service Endpoints:${NC}"
echo -e "  Bitcoin RPC:     http://localhost:18443"
echo -e "  Electrs TCP:     localhost:50001"
echo -e "  Electrs HTTP:    http://localhost:3002"
echo -e "  RGB Proxy:       http://localhost:3000"
echo ""

