# dbnum 7997 模型生成性能分析报告

## 一、测试环境与命令

- **构建**: `cargo run --release -p aios-database`
- **命令**: `--regen-model --dbnum 7997 -v`
- **目标类型**: PANE, BRAN, HANG（来自 DbOption-mac `index_tree_enabled_target_types`）
- **数据规模**: 11 个 SITE, 895 个 BRAN/HANG, 432 个 LOOP, 3574 个 mesh 产出

## 二、总耗时

| 指标 | 数值 |
|------|------|
| **总耗时** | **39.9 秒** |
| gen_all_geos_data | 39924 ms |
| index_tree_generation | 39869 ms |

## 三、阶段耗时占比

| 阶段 | 耗时 (ms) | 占比 |
|------|-----------|------|
| **categorize_and_inst_relate** | 38542 | **96.7%** |
| boolean_operation | 1326 | 3.3% |
| precheck | 51 | 0.1% |
| mesh_generation (perf 维度) | 0.3 | ~0% |
| aabb_write | 0.4 | ~0% |

> 注：perf 维度的 mesh_generation 仅统计独立 mesh 阶段；实际 mesh 已内联到 insert_handle 批次的 `t_mesh=1572ms` 中，被计入 categorize_and_inst_relate。

## 四、主要瓶颈分析

### 4.1 categorize_and_inst_relate 内部构成（≈38542 ms）

该阶段包含：几何体生成管线 → insert_handle 批次（mesh + 入库）。

#### BRAN/HANG 核心流水线：35272 ms（占总量约 88%）

| 子阶段 | 耗时 (ms) | 说明 |
|--------|-----------|------|
| **gen_cata_instances** | **24382** | 元件几何生成 worker 管线（45 个 worker） |
| **gen_branch_tubi** | **10667** | 管件 TUBI 生成 |
| save_tubi_info | 103 | 保存 tubi 信息 |
| collect_children | 30 | 收集 BRAN 子节点 |
| build_cata_hash_map | 29 | 构建元件哈希表 |

#### gen_cata_instances 细分（24382 ms）

| 子项 | 耗时 (ms) | 说明 |
|------|-----------|------|
| worker 流水线主体 | ~24381 | 45 个 gen_cata_geos worker 并发执行 |
| query_single | 1815 | SurrealDB 单条查询 |
| query_refnos | 1321 | 批量 refno 查询 |
| get_named_attmap | 321 | 获取命名属性 |

#### gen_branch_tubi 细分（10667 ms）

| 子项 | 耗时 (ms) | 说明 |
|------|-----------|------|
| **p4_global_prepare** | **6653** | 全局预准备（transform 等） |
| **process_branch** | **7441** | 管件几何处理（与 p4 有重叠） |
| **tubi_query** | **2579** | TUBI 相关数据库查询 |
| p4_local_prepare | 175 | 局部预准备 |
| p4_axis_db_fallback | 152 | 轴线数据库回退 |
| p4_spkbrk | 141 | 规格解析 |
| p4_transform_prefetch | 20 | 变换预取 |

#### insert_handle 批次汇总（94 批）

| 指标 | 数值 |
|------|------|
| batch_cnt | 94 |
| t_mesh | 1572 ms |
| t_save_db | 226 ms |
| mesh 产出数 | 3574 |

### 4.2 布尔运算：1326 ms（3.3%）

- 处理 105 个实例
- 单批约 534–657 ms
- 存在若干“未找到负实体 manifold”的失败，对整体影响有限

### 4.3 最慢批次（按 total_ms）

| total_ms | batch | refnos | inst_geos | mesh_ms | save_ms | sample |
|----------|-------|--------|-----------|---------|---------|--------|
| 141 | 1 | 196 | 26 | 139 | 0 | 24381_148225, ... |
| 120 | 3 | 179 | 65 | 119 | 0 | 24381_91410, ... |
| 83 | 2 | 199 | 65 | 82 | 0 | 24381_103824, ... |
| 81 | 63 | 5 | 5 | 2 | 78 | 24381_40946, ...（save 占主导） |

## 五、优化建议（按优先级）

### 1. 几何体生成管线（约 35s）

**gen_cata_instances（≈24s）**
- 增加 `gen_cata_geos` worker 并发（当前 6）
- 优化 `query_single` / `query_refnos`：批量查询、缓存、索引
- 检查 `get_named_attmap` 调用频率与缓存策略

**gen_branch_tubi（≈10.6s）**
- 优化 `p4_global_prepare`：预计算、缓存、减少重复计算
- 减少 `tubi_query` 调用：批量查询、本地缓存
- 评估 `process_branch` 与 `p4_global_prepare` 的重叠，避免重复计算

### 2. Mesh 与入库

- 当前 t_mesh=1572ms、t_save_db=226ms，占比较小
- 若后续数据量增大，可考虑：
  - `--defer-db-write` 将 SQL 落盘，再批量 `--import-sql`
  - 调大 `db_write_semaphore` 提升写入并发

### 3. 布尔运算

- 占比 3.3%，可暂不优先
- 可适当提高布尔 worker 并发，或在无负实体场景跳过

### 4. 其他

- 调低 LOD（如 L2/L3）可减少 mesh 计算量
- 使用 `--gen-nouns BRAN,PIPE` 等限定类型做对比测试

## 六、复现与分析方法

```bash
# 1. 运行 release 测试
./scripts/perf_gen_model_7999.sh 7997

# 或直接命令（需 SurrealDB 在 8020 运行）
cargo run --release -p aios-database -- --regen-model --dbnum 7997 -v 2>&1 | tee perf_7997.log

# 2. 分析最慢批次
grep '\[batch_perf\]' perf_7997.log | sed -E 's/.*total_ms=([0-9]+).*/\1/' | sort -rn | head -20

# 3. 查看性能报告
output/AvevaMarineSample/profile/perf_gen_model_index_tree_dbnum_7997_*.csv
```

---

*报告生成时间: 2026-03-10*
