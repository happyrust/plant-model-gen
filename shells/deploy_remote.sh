#!/bin/bash

# ==============================================================================
# AIOS Web-Server Remote Deployment Script
# Targets: 123.57.182.243 (Ubuntu 22.04 x86_64)
# ==============================================================================

set -e

# --- Configuration ---
REMOTE_HOST="123.57.182.243"
REMOTE_USER="root"
REMOTE_PASS="Happytest123_"
REMOTE_PATH="/root"
SERVICE_NAME="web-server"
BINARY_NAME="web_server"
TARGET="x86_64-unknown-linux-gnu"
# Use glibc 2.31 for broad compatibility if needed
TARGET_SPEC="${TARGET}.2.31"

# Color output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${GREEN}🚀 Starting Remote Deployment to ${REMOTE_HOST}...${NC}"

# 1. Local Cross-Compilation
echo -e "${YELLOW}Step 1: Cross-compiling for ${TARGET}...${NC}"
cargo zigbuild --release --bin ${BINARY_NAME} --features="web_server" --target ${TARGET_SPEC}
echo -e "${GREEN}✅ Compilation successful.${NC}"

# 2. Preparation on Remote
echo -e "${YELLOW}Step 2: Stopping remote service (if exists)...${NC}"
sshpass -p "${REMOTE_PASS}" ssh -o StrictHostKeyChecking=no ${REMOTE_USER}@${REMOTE_HOST} "systemctl stop ${SERVICE_NAME} || true"
echo -e "${GREEN}✅ Service stopped.${NC}"

# 3. Transfer Binary
echo -e "${YELLOW}Step 3: Synchronizing binary to server...${NC}"
sshpass -p "${REMOTE_PASS}" rsync -avz -e "ssh -o StrictHostKeyChecking=no" target/${TARGET}/release/${BINARY_NAME} ${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_PATH}/${BINARY_NAME}_new
echo -e "${GREEN}✅ Binary synchronization completed.${NC}"

# 3.5. Transfer Assets Directory
echo -e "${YELLOW}Step 3.5: Synchronizing assets directory to server...${NC}"
sshpass -p "${REMOTE_PASS}" rsync -avz --delete -e "ssh -o StrictHostKeyChecking=no" assets/ ${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_PATH}/assets/
echo -e "${GREEN}✅ Assets synchronization completed.${NC}"

# 4. Update and Restart
echo -e "${YELLOW}Step 4: Updating binary and restarting service...${NC}"
# Note: This assumes the service unit file already exists on the new server.
# If not, it might fail. But the user asked to test run it.
sshpass -p "${REMOTE_PASS}" ssh -o StrictHostKeyChecking=no ${REMOTE_USER}@${REMOTE_HOST} "mv ${REMOTE_PATH}/${BINARY_NAME}_new ${REMOTE_PATH}/${BINARY_NAME} && chmod +x ${REMOTE_PATH}/${BINARY_NAME} && systemctl start ${SERVICE_NAME} || systemctl restart ${SERVICE_NAME}"
echo -e "${GREEN}✅ Service restarted.${NC}"

# 5. Verification
echo -e "${YELLOW}Step 5: Verifying deployment...${NC}"
sleep 2
sshpass -p "${REMOTE_PASS}" ssh -o StrictHostKeyChecking=no ${REMOTE_USER}@${REMOTE_HOST} "systemctl is-active ${SERVICE_NAME}" && \
echo -e "${GREEN}✅ Deployment verified! Service is active.${NC}" || \
echo -e "${RED}❌ Verification failed. Please check remote logs via: journalctl -u ${SERVICE_NAME}${NC}"

echo -e "${GREEN}🎉 Done!${NC}"
