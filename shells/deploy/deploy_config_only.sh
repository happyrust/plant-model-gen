#!/usr/bin/env bash
# 仅将本地 DbOption（默认 db_options/DbOption-mac.toml）同步到远端 /root/DbOption.toml。
# 默认会执行 systemctl restart web-server，使 web_server 重新加载配置。不上传二进制、assets、output。
#
#   REMOTE_PASS='...' ./shells/deploy/deploy_config_only.sh
#
# 可选：DB_OPTION_FILE、REMOTE_HOST、REMOTE_* 路径、DEPLOY_APPLY_DB_PATH_OVERRIDES。
# 仅写配置不重启：RESTART_AFTER_CONFIG_ONLY=false ./shells/deploy/deploy_config_only.sh

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export CONFIG_ONLY=true
: "${RESTART_AFTER_CONFIG_ONLY:=true}"
export RESTART_AFTER_CONFIG_ONLY
exec "$SCRIPT_DIR/deploy_web_server_bundle.sh"
