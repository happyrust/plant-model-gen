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

# 编译选项 (设为 true 强制重新编译)
BUILD_BINARY=false

# 项目根目录
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

# Color output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${GREEN}🚀 Starting Remote Deployment to ${REMOTE_HOST}...${NC}"

# 1. Local Cross-Compilation
if [ "$BUILD_BINARY" = "true" ]; then
    echo -e "${YELLOW}Step 1: Cross-compiling for ${TARGET}...${NC}"
    cd "$PROJECT_DIR"
    cargo zigbuild --release --bin ${BINARY_NAME} --features="web_server" --target ${TARGET}
    echo -e "${GREEN}✅ Compilation successful.${NC}"
else
    echo -e "${YELLOW}Step 1: Skipping compilation (BUILD_BINARY=false)${NC}"
fi

# 2. Preparation on Remote
echo -e "${YELLOW}Step 2: Stopping remote service (if exists)...${NC}"
sshpass -p "${REMOTE_PASS}" ssh -o StrictHostKeyChecking=no -o PreferredAuthentications=password ${REMOTE_USER}@${REMOTE_HOST} "systemctl stop ${SERVICE_NAME} || true"
echo -e "${GREEN}✅ Service stopped.${NC}"

# 3. Transfer Binary
echo -e "${YELLOW}Step 3: Synchronizing binary to server...${NC}"
sshpass -p "${REMOTE_PASS}" rsync -avz -e "ssh -o StrictHostKeyChecking=no -o PreferredAuthentications=password" $PROJECT_DIR/target/${TARGET}/release/${BINARY_NAME} ${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_PATH}/${BINARY_NAME}_new
echo -e "${GREEN}✅ Binary synchronization completed.${NC}"

# 3.5. Transfer Assets Directory
echo -e "${YELLOW}Step 3.5: Synchronizing assets directory to server...${NC}"
sshpass -p "${REMOTE_PASS}" rsync -avz --delete -e "ssh -o StrictHostKeyChecking=no -o PreferredAuthentications=password" $PROJECT_DIR/assets/ ${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_PATH}/assets/
echo -e "${GREEN}✅ Assets synchronization completed.${NC}"

# 3.6. Transfer Output Directory
if [ -d "$PROJECT_DIR/output" ]; then
    echo -e "${YELLOW}Step 3.6: Synchronizing output directory to server...${NC}"
    sshpass -p "${REMOTE_PASS}" rsync -avz --delete -e "ssh -o StrictHostKeyChecking=no -o PreferredAuthentications=password" $PROJECT_DIR/output/ ${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_PATH}/output/
    echo -e "${GREEN}✅ Output synchronization completed.${NC}"
else
    echo -e "${YELLOW}Step 3.6: Skipped (output directory not found)${NC}"
fi

# 4. Update and Restart
echo -e "${YELLOW}Step 4: Updating binary and restarting service...${NC}"
sshpass -p "${REMOTE_PASS}" ssh -o StrictHostKeyChecking=no -o PreferredAuthentications=password ${REMOTE_USER}@${REMOTE_HOST} "mv ${REMOTE_PATH}/${BINARY_NAME}_new ${REMOTE_PATH}/${BINARY_NAME} && chmod +x ${REMOTE_PATH}/${BINARY_NAME} && systemctl start ${SERVICE_NAME} || systemctl restart ${SERVICE_NAME}"
echo -e "${GREEN}✅ Service restarted.${NC}"

# 5. Verification
echo -e "${YELLOW}Step 5: Verifying deployment...${NC}"
sleep 2
sshpass -p "${REMOTE_PASS}" ssh -o StrictHostKeyChecking=no -o PreferredAuthentications=password ${REMOTE_USER}@${REMOTE_HOST} "systemctl is-active ${SERVICE_NAME}" && \
echo -e "${GREEN}✅ Deployment verified! Service is active.${NC}" || \
echo -e "${RED}❌ Verification failed. Please check remote logs via: journalctl -u ${SERVICE_NAME}${NC}"

echo -e "${GREEN}🎉 Done!${NC}"
