#!/usr/bin/env bash
# 使用 release 模式测试生成 7999 的所有模型，并评估各批次耗时
#
# 用法：
#   ./scripts/perf_gen_model_7999.sh           # 默认 dbnum=7999
#   ./scripts/perf_gen_model_7999.sh 7997     # 指定 dbnum=7997（若 7999 无 tree 可改用）
#
# 前置：output/<project>/scene_tree/<dbnum>.tree 需已存在，否则先 --gen-indextree <dbnum>
#
# 输出：
#   - 控制台实时打印 [batch_perf] 每批次耗时
#   - perf_gen_model_<dbnum>_<timestamp>.log 完整日志
#   - 可用 grep/awk 分析最慢的批次

set -e
cd "$(dirname "$0")/.."

DBNUM="${1:-7999}"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
LOG_FILE="perf_gen_model_${DBNUM}_${TIMESTAMP}.log"

echo "=============================================="
echo "模型生成性能测试: dbnum=${DBNUM}"
echo "日志文件: ${LOG_FILE}"
echo "=============================================="

# 确保启用每批次耗时日志（默认已开启，可设 AIOS_LOG_BATCH_PERF=0 关闭）
export AIOS_LOG_BATCH_PERF=1

# release 构建
echo "[1/2] Release 构建..."
cargo build --release -p aios-database 2>&1 | tail -5

# 执行模型生成（--offline 使用嵌入式 SurrealDB 文件模式，避免 ws 端口冲突）
echo "[2/2] 执行模型生成 (--regen-model --dbnum ${DBNUM})..."
cargo run --release -p aios-database -- \
  --regen-model \
  --dbnum "${DBNUM}" \
  -v \
  --offline \
  2>&1 | tee "${LOG_FILE}"

echo ""
echo "=============================================="
echo "完成。分析最慢批次（按 total_ms 降序）:"
echo "  grep '\[batch_perf\]' ${LOG_FILE} | sed -E 's/.*batch=([0-9]+).*total_ms=([0-9]+).*sample=\[([^]]*)\].*/\\2 ms  batch=\\1  sample=[\\3]/' | sort -rn -k1 | head -20"
echo "=============================================="
