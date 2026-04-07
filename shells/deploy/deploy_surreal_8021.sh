#!/bin/bash

# ==============================================================================
# SurrealDB ams-8021.db Deployment Script
# Targets: 123.57.182.243
# ==============================================================================

set -e

# --- Configuration ---
REMOTE_HOST="123.57.182.243"
REMOTE_USER="root"
REMOTE_PASS="Happytest123_"
REMOTE_DATA_PATH="/root/surreal_data"
DB_NAME="ams-8021.db"
LOCAL_DB_PATH="/Volumes/DPC/work/plant-code/gen-model-fork/ams-8021.db"
LOCAL_RUN_SCRIPT="/Volumes/DPC/work/plant-code/gen-model-fork/shells/run_surreal_8021.sh"
REMOTE_SHELL_PATH="/root/shells"

# Color output
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${GREEN}🚀 Starting Deployment of ${DB_NAME} to ${REMOTE_HOST}...${NC}"

# 1. Ensure remote directories exist
echo -e "${YELLOW}Step 1: Preparing remote directories...${NC}"
sshpass -p "${REMOTE_PASS}" ssh -o StrictHostKeyChecking=no ${REMOTE_USER}@${REMOTE_HOST} "mkdir -p ${REMOTE_DATA_PATH} ${REMOTE_SHELL_PATH}"

# 2. Synchronize DB file/directory
echo -e "${YELLOW}Step 2: Synchronizing ${DB_NAME} to server...${NC}"
if [ -d "${LOCAL_DB_PATH}" ]; then
    sshpass -p "${REMOTE_PASS}" rsync -avz -e "ssh -o StrictHostKeyChecking=no" "${LOCAL_DB_PATH}/" "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_DATA_PATH}/${DB_NAME}/"
else
    sshpass -p "${REMOTE_PASS}" rsync -avz -e "ssh -o StrictHostKeyChecking=no" "${LOCAL_DB_PATH}" "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_DATA_PATH}/${DB_NAME}"
fi
echo -e "${GREEN}✅ Database synchronization completed.${NC}"

# 3. Synchronize Run Script
echo -e "${YELLOW}Step 3: Synchronizing run script...${NC}"
sshpass -p "${REMOTE_PASS}" rsync -avz -e "ssh -o StrictHostKeyChecking=no" "${LOCAL_RUN_SCRIPT}" "${REMOTE_USER}@${REMOTE_HOST}:${REMOTE_SHELL_PATH}/run_surreal_8021.sh"
sshpass -p "${REMOTE_PASS}" ssh -o StrictHostKeyChecking=no ${REMOTE_USER}@${REMOTE_HOST} "chmod +x ${REMOTE_SHELL_PATH}/run_surreal_8021.sh"
echo -e "${GREEN}✅ Run script synchronization completed.${NC}"

# 4. Start SurrealDB on Remote
echo -e "${YELLOW}Step 4: Starting SurrealDB on remote...${NC}"
# We need to run it in the background or as a service. For now, we'll try to run the script.
# Note: The run script uses rocksdb://ams-8021.db, we might need to adjust the path in the script or run it from the right directory.
sshpass -p "${REMOTE_PASS}" ssh -o StrictHostKeyChecking=no ${REMOTE_USER}@${REMOTE_HOST} "cd ${REMOTE_DATA_PATH} && nohup ${REMOTE_SHELL_PATH}/run_surreal_8021.sh > surreal_8021.log 2>&1 &"
echo -e "${GREEN}✅ SurrealDB start command issued.${NC}"

# 5. Verification
echo -e "${YELLOW}Step 5: Verifying...${NC}"
sleep 2
sshpass -p "${REMOTE_PASS}" ssh -o StrictHostKeyChecking=no ${REMOTE_USER}@${REMOTE_HOST} "netstat -tuln | grep 8021" && \
echo -e "${GREEN}✅ Port 8021 is listening.${NC}" || \
echo -e "${YELLOW}⚠️ Port 8021 is not yet listening, check /root/surreal_data/surreal_8021.log on remote.${NC}"

echo -e "${GREEN}🎉 Done!${NC}"
