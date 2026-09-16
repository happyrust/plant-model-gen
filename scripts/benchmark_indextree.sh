#!/usr/bin/env bash
set -euo pipefail

# 一键基线/优化对比 + 稳定性回归脚本（debug 构建）
# 默认行为：
# 1) 跑 single + all 两个场景
# 2) baseline(profile=legacy) 与 optimized(profile=tuned) 各跑 3 次
# 3) 输出 P50/P90/平均值 + 提升比
# 4) 跑并发 1/2/4/8 稳定性检查（默认仅 single）

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_PATH="$ROOT_DIR/target/debug/aios-database"

RUNS=3
DBNUM=""
SKIP_BUILD=0
SKIP_SINGLE=0
SKIP_ALL=0
ALL_MODE="default"             # default -> --gen-indextree, force -> --gen-all-desi-indextree
BASELINE_MODE="legacy"         # legacy -> FORCE_CURRENT_THREAD=1 + CHUNK_CONCURRENCY=1, off -> 不跑基线
ENABLE_STABILITY=1
STABILITY_TARGET="single"      # single|all|both
STABILITY_CONCURRENCY_LIST="1,2,4,8"
REPORT_PATH=""

print_help() {
  cat <<'USAGE'
用法:
  scripts/benchmark_indextree.sh --dbnum <DBNUM> [选项]

必选参数:
  --dbnum <DBNUM>                    单库测试目标 dbnum

可选参数:
  --runs <N>                         每个场景每个 profile 的运行次数，默认 3
  --all-mode <default|force>         全量命令模式，默认 default
                                     default: --gen-indextree
                                     force  : --gen-all-desi-indextree
  --baseline-mode <legacy|off>       是否跑基线，默认 legacy
                                     legacy: AIOS_INDEXTREE_FORCE_CURRENT_THREAD=1 + AIOS_INDEXTREE_CHUNK_CONCURRENCY=1
                                     off   : 不跑基线，仅跑优化组
  --skip-single                      跳过单库场景
  --skip-all                         跳过全量场景
  --skip-build                       跳过 cargo build
  --disable-stability                跳过稳定性回归
  --stability-target <single|all|both>
                                     稳定性检查目标，默认 single
  --stability-concurrency <list>     逗号分隔，例如 1,2,4,8（默认 1,2,4,8）
  --report <PATH>                    报告输出路径（默认 logs/indextree_benchmark_时间戳.md）
  -h, --help                         显示帮助

环境变量:
  DB_OPTION_FILE                     若需要指定配置文件，可预先导出
  AIOS_INDEXTREE_RT_THREADS          可选；优化组会遵循该设置
  AIOS_INDEXTREE_SINGLE_CHUNK_SIZE   可选；单库 chunk size 覆盖
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dbnum)
      DBNUM="$2"
      shift 2
      ;;
    --runs)
      RUNS="$2"
      shift 2
      ;;
    --all-mode)
      ALL_MODE="$2"
      shift 2
      ;;
    --baseline-mode)
      BASELINE_MODE="$2"
      shift 2
      ;;
    --skip-single)
      SKIP_SINGLE=1
      shift
      ;;
    --skip-all)
      SKIP_ALL=1
      shift
      ;;
    --skip-build)
      SKIP_BUILD=1
      shift
      ;;
    --disable-stability)
      ENABLE_STABILITY=0
      shift
      ;;
    --stability-target)
      STABILITY_TARGET="$2"
      shift 2
      ;;
    --stability-concurrency)
      STABILITY_CONCURRENCY_LIST="$2"
      shift 2
      ;;
    --report)
      REPORT_PATH="$2"
      shift 2
      ;;
    -h|--help)
      print_help
      exit 0
      ;;
    *)
      echo "未知参数: $1" >&2
      print_help
      exit 1
      ;;
  esac
done

if [[ -z "$DBNUM" ]]; then
  echo "错误: 必须提供 --dbnum <DBNUM>" >&2
  print_help
  exit 1
fi

if [[ "$ALL_MODE" != "default" && "$ALL_MODE" != "force" ]]; then
  echo "错误: --all-mode 仅支持 default|force" >&2
  exit 1
fi

if [[ "$BASELINE_MODE" != "legacy" && "$BASELINE_MODE" != "off" ]]; then
  echo "错误: --baseline-mode 仅支持 legacy|off" >&2
  exit 1
fi

if [[ "$STABILITY_TARGET" != "single" && "$STABILITY_TARGET" != "all" && "$STABILITY_TARGET" != "both" ]]; then
  echo "错误: --stability-target 仅支持 single|all|both" >&2
  exit 1
fi

if ! [[ "$RUNS" =~ ^[0-9]+$ ]] || [[ "$RUNS" -lt 1 ]]; then
  echo "错误: --runs 必须是 >=1 的整数" >&2
  exit 1
fi

mkdir -p "$ROOT_DIR/logs"
TS="$(date +%Y%m%d_%H%M%S)"
WORK_DIR="$ROOT_DIR/logs/indextree_benchmark_${TS}"
mkdir -p "$WORK_DIR"

if [[ -z "$REPORT_PATH" ]]; then
  REPORT_PATH="$ROOT_DIR/logs/indextree_benchmark_${TS}.md"
fi

if [[ "$SKIP_BUILD" -eq 0 ]]; then
  echo "[build] cargo build --bin aios-database"
  (cd "$ROOT_DIR" && cargo build --bin aios-database)
fi

if [[ ! -x "$BIN_PATH" ]]; then
  echo "错误: 未找到可执行文件 $BIN_PATH" >&2
  exit 1
fi

declare -a SCENARIOS=()
if [[ "$SKIP_SINGLE" -eq 0 ]]; then
  SCENARIOS+=("single")
fi
if [[ "$SKIP_ALL" -eq 0 ]]; then
  SCENARIOS+=("all")
fi

if [[ "${#SCENARIOS[@]}" -eq 0 ]]; then
  echo "错误: single/all 都被跳过，没有可执行场景" >&2
  exit 1
fi

cmd_for_scenario() {
  local scenario="$1"
  if [[ "$scenario" == "single" ]]; then
    echo "$BIN_PATH --gen-indextree $DBNUM"
    return 0
  fi

  if [[ "$ALL_MODE" == "force" ]]; then
    echo "$BIN_PATH --gen-all-desi-indextree"
  else
    echo "$BIN_PATH --gen-indextree"
  fi
}

# 计算统计值：输出 count p50 p90 avg
calc_stats() {
  local file="$1"
  local count
  count="$(wc -l < "$file" | tr -d ' ')"
  if [[ "$count" -eq 0 ]]; then
    echo "0 0 0 0"
    return 0
  fi

  local p50_rank p90_rank
  p50_rank=$(( (count * 50 + 99) / 100 ))
  p90_rank=$(( (count * 90 + 99) / 100 ))

  local sorted
  sorted="$(sort -n "$file")"
  local p50 p90 avg
  p50="$(printf '%s\n' "$sorted" | sed -n "${p50_rank}p")"
  p90="$(printf '%s\n' "$sorted" | sed -n "${p90_rank}p")"
  avg="$(awk '{sum+=$1} END {if (NR==0) print 0; else printf "%.3f", sum/NR}' "$file")"

  echo "$count $p50 $p90 $avg"
}

fmt3() {
  awk -v v="$1" 'BEGIN {printf "%.3f", v + 0}'
}

calc_improvement_percent() {
  local base="$1"
  local now="$2"
  awk -v b="$base" -v n="$now" 'BEGIN {
    if (b <= 0) { print "N/A"; exit }
    printf "%.2f%%", ((b - n) / b) * 100
  }'
}

run_timed_case() {
  local profile="$1"       # legacy|tuned
  local scenario="$2"      # single|all
  local run_idx="$3"

  local cmd
  cmd="$(cmd_for_scenario "$scenario")"

  local case_dir="$WORK_DIR/${profile}_${scenario}"
  mkdir -p "$case_dir"

  local run_log="$case_dir/run_${run_idx}.log"
  local run_time="$case_dir/run_${run_idx}.time"

  local -a env_args=()
  if [[ "$profile" == "legacy" ]]; then
    env_args+=("AIOS_INDEXTREE_FORCE_CURRENT_THREAD=1")
    env_args+=("AIOS_INDEXTREE_CHUNK_CONCURRENCY=1")
  else
    # tuned: 显式关闭 force_current_thread，避免继承外部环境
    env_args+=("AIOS_INDEXTREE_FORCE_CURRENT_THREAD=0")
  fi

  echo "[run] profile=$profile scenario=$scenario run=$run_idx"
  set +e
  /usr/bin/time -p env "${env_args[@]}" bash -lc "$cmd" >"$run_log" 2>"$run_time"
  local code=$?
  set -e

  local real_time
  real_time="$(awk '/^real / {print $2}' "$run_time" | tail -n 1)"
  if [[ -z "$real_time" ]]; then
    real_time="0"
  fi

  echo "$real_time" >> "$case_dir/times.txt"

  if [[ "$code" -ne 0 ]]; then
    echo "FAIL" > "$case_dir/run_${run_idx}.status"
    echo "[error] profile=$profile scenario=$scenario run=$run_idx exit_code=$code"
    return "$code"
  fi

  if grep -qi "panic" "$run_log"; then
    echo "PANIC" > "$case_dir/run_${run_idx}.status"
    echo "[error] profile=$profile scenario=$scenario run=$run_idx 检测到 panic"
    return 2
  fi

  echo "OK" > "$case_dir/run_${run_idx}.status"
  return 0
}

run_performance_suite() {
  echo "[stage] 性能对比开始"

  local -a profiles=("tuned")
  if [[ "$BASELINE_MODE" == "legacy" ]]; then
    profiles=("legacy" "tuned")
  fi

  for profile in "${profiles[@]}"; do
    for scenario in "${SCENARIOS[@]}"; do
      local case_dir="$WORK_DIR/${profile}_${scenario}"
      rm -f "$case_dir/times.txt"
      mkdir -p "$case_dir"

      local i
      for ((i=1; i<=RUNS; i++)); do
        run_timed_case "$profile" "$scenario" "$i"
      done
    done
  done
}

should_run_stability_for() {
  local scenario="$1"
  case "$STABILITY_TARGET" in
    single)
      [[ "$scenario" == "single" ]]
      ;;
    all)
      [[ "$scenario" == "all" ]]
      ;;
    both)
      return 0
      ;;
  esac
}

run_stability_suite() {
  echo "[stage] 稳定性回归开始"
  local csv="$STABILITY_CONCURRENCY_LIST"
  IFS=',' read -r -a conc_list <<< "$csv"

  for scenario in "${SCENARIOS[@]}"; do
    if ! should_run_stability_for "$scenario"; then
      continue
    fi

    local stab_dir="$WORK_DIR/stability_${scenario}"
    mkdir -p "$stab_dir"

    local c
    for c in "${conc_list[@]}"; do
      c="$(echo "$c" | xargs)"
      if ! [[ "$c" =~ ^[0-9]+$ ]] || [[ "$c" -lt 1 ]]; then
        echo "[warn] 跳过非法并发值: $c"
        continue
      fi

      local run_log="$stab_dir/conc_${c}.log"
      local run_time="$stab_dir/conc_${c}.time"
      local cmd
      cmd="$(cmd_for_scenario "$scenario")"

      echo "[stability] scenario=$scenario conc=$c"
      set +e
      /usr/bin/time -p env \
        AIOS_INDEXTREE_FORCE_CURRENT_THREAD=0 \
        AIOS_INDEXTREE_CHUNK_CONCURRENCY="$c" \
        bash -lc "$cmd" >"$run_log" 2>"$run_time"
      local code=$?
      set -e

      local real_time
      real_time="$(awk '/^real / {print $2}' "$run_time" | tail -n 1)"
      [[ -z "$real_time" ]] && real_time="0"

      local status="OK"
      if [[ "$code" -ne 0 ]]; then
        status="FAIL(exit=$code)"
      elif grep -qi "panic" "$run_log"; then
        status="PANIC"
      fi

      printf "%s\t%s\t%s\n" "$c" "$(fmt3 "$real_time")" "$status" >> "$stab_dir/summary.tsv"
    done
  done
}

write_report() {
  {
    echo "# Indextree 基线与回归报告"
    echo
    echo "- 时间: $(date '+%Y-%m-%d %H:%M:%S')"
    echo "- 工作目录: \`$WORK_DIR\`"
    echo "- 二进制: \`$BIN_PATH\`"
    echo "- runs: $RUNS"
    echo "- baseline_mode: $BASELINE_MODE"
    echo "- all_mode: $ALL_MODE"
    echo "- stability: $ENABLE_STABILITY (target=$STABILITY_TARGET, concurrency=$STABILITY_CONCURRENCY_LIST)"
    echo

    echo "## 性能对比"
    echo
    echo "| 场景 | profile | 样本数 | P50(s) | P90(s) | Avg(s) |"
    echo "|---|---:|---:|---:|---:|---:|"

    local scenario
    for scenario in "${SCENARIOS[@]}"; do
      local profile
      for profile in legacy tuned; do
        if [[ "$profile" == "legacy" && "$BASELINE_MODE" != "legacy" ]]; then
          continue
        fi

        local tf="$WORK_DIR/${profile}_${scenario}/times.txt"
        if [[ ! -f "$tf" ]]; then
          continue
        fi

        read -r count p50 p90 avg < <(calc_stats "$tf")
        echo "| $scenario | $profile | $count | $(fmt3 "$p50") | $(fmt3 "$p90") | $(fmt3 "$avg") |"
      done
    done

    if [[ "$BASELINE_MODE" == "legacy" ]]; then
      echo
      echo "## 提升幅度（legacy -> tuned）"
      echo
      echo "| 场景 | P50 提升 | P90 提升 | Avg 提升 |"
      echo "|---|---:|---:|---:|"

      for scenario in "${SCENARIOS[@]}"; do
        local f_legacy="$WORK_DIR/legacy_${scenario}/times.txt"
        local f_tuned="$WORK_DIR/tuned_${scenario}/times.txt"
        if [[ ! -f "$f_legacy" || ! -f "$f_tuned" ]]; then
          continue
        fi

        read -r _ lp50 lp90 lavg < <(calc_stats "$f_legacy")
        read -r _ tp50 tp90 tavg < <(calc_stats "$f_tuned")

        local ip50 ip90 iavg
        ip50="$(calc_improvement_percent "$lp50" "$tp50")"
        ip90="$(calc_improvement_percent "$lp90" "$tp90")"
        iavg="$(calc_improvement_percent "$lavg" "$tavg")"

        echo "| $scenario | $ip50 | $ip90 | $iavg |"
      done
    fi

    if [[ "$ENABLE_STABILITY" -eq 1 ]]; then
      echo
      echo "## 稳定性回归"
      echo
      local scenario
      for scenario in "${SCENARIOS[@]}"; do
        local stab="$WORK_DIR/stability_${scenario}/summary.tsv"
        if [[ ! -f "$stab" ]]; then
          continue
        fi

        echo "### $scenario"
        echo
        echo "| chunk_concurrency | 耗时(s) | 状态 |"
        echo "|---:|---:|---|"
        awk -F '\t' '{printf "| %s | %s | %s |\n", $1, $2, $3}' "$stab"
        echo
      done
    fi

    echo "## 日志目录"
    echo
    echo "- 详细运行日志: \`$WORK_DIR\`"
  } > "$REPORT_PATH"
}

run_performance_suite
if [[ "$ENABLE_STABILITY" -eq 1 ]]; then
  run_stability_suite
fi
write_report

echo
echo "完成。"
echo "报告: $REPORT_PATH"
echo "日志目录: $WORK_DIR"
