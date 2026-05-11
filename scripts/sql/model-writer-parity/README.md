# Model Writer Parity SQL Scripts (v3 Phase E)

Phase 1 canonical raw 表的 SQL 平行校验脚本，配合 `aios-database model-writer` 子命令族做 backend 双写的事后比对。脚本依据 `docs/development/model-writer-storage/07-validation-plan.md` §3 编写，使用 DuckDB 直读 `model_writer_storage/raw/` 下的 JSONL fallback 输出。

## 适用范围

- **Phase 1 表**（13 张）：`raw_inst_info` / `raw_inst_relate` / `raw_inst_geo` / `raw_geo_relate` / `raw_tubi_info` / `raw_tubi_relate` / `raw_neg_relate` / `raw_ngmr_relate` / `raw_aabb` / `raw_trans` / `raw_vec3` / `raw_inst_relate_aabb` / `raw_refno_assoc_index`
- **Phase 2 不在本目录范围**：`inst_relate_bool` / `inst_relate_cata_bool`（mission 00 §Non-goals + 08 §Phase 5）

## 文件命名约定

```
<table>_count.sql          # 单边 row count
<table>_diff.sql           # 双边 key-set diff（LEFT EXCEPT RIGHT + RIGHT EXCEPT LEFT）
```

13 张表 × 2 个 SQL = **26 个脚本**。

## 输入约定

每个 SQL 脚本通过 DuckDB 的 `read_json_auto()` 读两个目录：

```sql
-- 在脚本顶部按需替换占位符
SET VARIABLE LEFT_ROOT = 'output/run-baseline/model_writer_storage/raw';
SET VARIABLE RIGHT_ROOT = 'output/run-candidate/model_writer_storage/raw';
SET VARIABLE PROJECT = 'project-x';
SET VARIABLE DBNUM = 1;
```

每个表的 JSONL 真实路径：
```
${LEFT_ROOT}/<table>/project_name=${PROJECT}/dbnum=${DBNUM}/batch_*.jsonl
```

## 推荐工作流

1. **运行两边 backend**（v3 Phase B + C 已支持）：
   ```bash
   # 基线
   aios-database --model-writer parquet --parquet-output-root output/run-baseline --parquet-dbnum 1 [...]
   # 候选
   aios-database --model-writer parquet --parquet-output-root output/run-candidate --parquet-dbnum 1 [...]
   # 或一次 compare：
   aios-database --model-writer surreal --model-writer-compare-with parquet \
                 --parquet-output-root output/run-candidate --parquet-dbnum 1 [...]
   ```

2. **快速 row-count 比对**（CLI，无需 DuckDB）：
   ```bash
   aios-database model-writer diff-summary \
     --left  output/run-baseline/model_writer_storage/summary/project_name=project-x/dbnum=1/batch_1.json \
     --right output/run-candidate/model_writer_storage/summary/project_name=project-x/dbnum=1/batch_1.json \
     --fail-on-diff
   ```

3. **细粒度 SQL 比对**（本目录脚本）：
   ```bash
   duckdb -c ".read scripts/sql/model-writer-parity/raw_inst_relate_diff.sql"
   ```

## DuckDB 安装

```powershell
# 任选其一：
choco install duckdb           # Windows + chocolatey
scoop install duckdb           # Windows + scoop
brew install duckdb            # macOS

# 或直接下载 https://duckdb.org/docs/installation/
```

## 限制

- 当前 JSONL fallback 落盘的是 Surreal-json payload 字段，**不是 typed Parquet**。SQL 脚本能跑 row count + key 比对，但 schema 类型断言要等 v4 typed `.parquet` 落地（mission 05 §Phase boundary）。
- `raw_refno_assoc_index` 标 phase1_limitation：本 PR 仅检查行数存在，不验证 index 字段内容（mission 02 §refno_assoc_index）。
- mission 03 §Boolean boundary 明确 boolean 表在 Phase 2；本目录不含相关脚本。

## v4 升级路径

当 typed `.parquet` 物化落地后：

1. SQL 脚本里 `read_json_auto()` 换成 `read_parquet()`
2. 在每个 `_count.sql` 后附加一个 `_schema.sql` 校验列类型
3. `raw_inst_relate_bool` / `raw_inst_relate_cata_bool` 进入 Phase 2 拓展目录 `scripts/sql/model-writer-parity-phase2/`
