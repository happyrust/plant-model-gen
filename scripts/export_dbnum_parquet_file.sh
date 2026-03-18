#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

BASE_CONFIG="${1:-db_options/DbOption-mac}"
DBNUM="${2:-7997}"
BIN_PATH="${BIN_PATH:-$ROOT_DIR/target/debug/aios-database}"
TMP_DIR="$ROOT_DIR/.tmp"
TMP_CFG_DIR="$ROOT_DIR/db_options/_tmp"
STAMP="$(date +%Y%m%d_%H%M%S)"

if [[ ! -f "${BASE_CONFIG}.toml" ]]; then
  echo "config not found: ${BASE_CONFIG}.toml" >&2
  exit 1
fi

if [[ ! -x "$BIN_PATH" ]]; then
  echo "binary not found: $BIN_PATH" >&2
  echo "请先执行: cargo build --bin aios-database" >&2
  exit 1
fi

mkdir -p "$TMP_DIR" "$TMP_CFG_DIR"

CLONE_DB_PATH="$TMP_DIR/ams-file-export-${DBNUM}-${STAMP}.db"
TEMP_CONFIG_NO_EXT="$TMP_CFG_DIR/DbOption-file-export-${DBNUM}-${STAMP}"
TEMP_CONFIG_PATH="${TEMP_CONFIG_NO_EXT}.toml"
RUN_LOG="$TMP_DIR/export_dbnum_parquet_file_${DBNUM}_${STAMP}.log"
TIME_LOG="$TMP_DIR/export_dbnum_parquet_file_${DBNUM}_${STAMP}.time"

python3 - "$ROOT_DIR" "${BASE_CONFIG}.toml" "$CLONE_DB_PATH" "$TEMP_CONFIG_PATH" <<'PY'
import pathlib
import re
import sys

root_dir = pathlib.Path(sys.argv[1])
base_config = pathlib.Path(sys.argv[2])
clone_db_path = pathlib.Path(sys.argv[3])
temp_config = pathlib.Path(sys.argv[4])
text = base_config.read_text()

def replace_table_block(name: str, new_block: str, raw: str) -> str:
    pattern = re.compile(rf'(?ms)^(\[{re.escape(name)}\]\n.*?)(?=^\[|\Z)')
    match = pattern.search(raw)
    if not match:
        return raw.rstrip() + "\n\n" + new_block.rstrip() + "\n"
    return raw[:match.start()] + new_block.rstrip() + "\n\n" + raw[match.end():]

surreal_path = None
surreal_match = re.search(r'(?ms)^\[surrealdb\]\n(.*?)(?=^\[|\Z)', text)
if surreal_match:
    body = surreal_match.group(1)
    path_match = re.search(r'(?m)^\s*path\s*=\s*"([^"]+)"\s*$', body)
    if path_match:
        surreal_path = path_match.group(1)

if surreal_path is None:
    web_match = re.search(r'(?ms)^\[web_server\]\n(.*?)(?=^\[|\Z)', text)
    if web_match:
        body = web_match.group(1)
        path_match = re.search(r'(?m)^\s*surreal_data_path\s*=\s*"([^"]+)"\s*$', body)
        if path_match:
            surreal_path = path_match.group(1)

if surreal_path is None:
    raise SystemExit("无法从配置中解析 SurrealDB 数据路径")

surreal_block = f"""[surrealdb]
mode = "file"
path = "{clone_db_path.as_posix()}"
"""
text = replace_table_block("surrealdb", surreal_block, text)
temp_config.write_text(text)
print(pathlib.Path(surreal_path).as_posix())
PY

SOURCE_DB_PATH="$(python3 - "$ROOT_DIR" "${BASE_CONFIG}.toml" <<'PY'
import pathlib
import re
import sys

base_config = pathlib.Path(sys.argv[2])
text = base_config.read_text()
surreal_match = re.search(r'(?ms)^\[surrealdb\]\n(.*?)(?=^\[|\Z)', text)
if surreal_match:
    body = surreal_match.group(1)
    path_match = re.search(r'(?m)^\s*path\s*=\s*"([^"]+)"\s*$', body)
    if path_match:
        print(path_match.group(1))
        raise SystemExit
web_match = re.search(r'(?ms)^\[web_server\]\n(.*?)(?=^\[|\Z)', text)
if web_match:
    body = web_match.group(1)
    path_match = re.search(r'(?m)^\s*surreal_data_path\s*=\s*"([^"]+)"\s*$', body)
    if path_match:
        print(path_match.group(1))
        raise SystemExit
raise SystemExit("无法从配置中解析 SurrealDB 数据路径")
PY
)"

echo "== file 模式导出准备 =="
echo "base_config : ${BASE_CONFIG}.toml"
echo "source_db   : $SOURCE_DB_PATH"
echo "clone_db    : $CLONE_DB_PATH"
echo "temp_config : $TEMP_CONFIG_PATH"
echo "dbnum       : $DBNUM"

/usr/bin/time -l cp -cR "$SOURCE_DB_PATH" "$CLONE_DB_PATH" 2>&1 | tee "$TIME_LOG"
rm -f "$CLONE_DB_PATH/LOCK"

echo
echo "== 开始导出 parquet =="
/usr/bin/time -l "$BIN_PATH" -c "$TEMP_CONFIG_NO_EXT" --export-parquet --dbnum "$DBNUM" --verbose \
  2>&1 | tee "$RUN_LOG"

echo
echo "== 完成 =="
echo "run_log  : $RUN_LOG"
echo "time_log : $TIME_LOG"
echo "config   : $TEMP_CONFIG_PATH"
echo "clone_db : $CLONE_DB_PATH"
