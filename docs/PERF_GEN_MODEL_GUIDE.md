# 模型生成性能评估与优化指南

## 一、使用方式

### 1. 运行 release 模型生成并捕获耗时

```bash
# 使用已添加的每批次耗时日志
cd plant-model-gen

# 方式 A：使用脚本（推荐）
./scripts/perf_gen_model_7999.sh 7999   # 生成 dbnum=7999
./scripts/perf_gen_model_7999.sh 7997   # 或 dbnum=7997（若 7999 无数据）

# 方式 B：直接命令
cargo run --release -p aios-database -- --regen-model --dbnum 7999 -v --offline 2>&1 | tee perf_7999.log
```

### 2. 分析最慢批次

日志中每行格式为：
```
[batch_perf] batch=N refnos=M inst_geos=K mesh_ms=X save_ms=Y total_ms=Z sample=[refno1, refno2, ...]
```

**按 total_ms 找最慢批次：**
```bash
grep '\[batch_perf\]' perf_7999.log | \
  sed -E 's/.*batch=([0-9]+).*refnos=([0-9]+).*inst_geos=([0-9]+).*mesh_ms=([0-9]+).*save_ms=([0-9]+).*total_ms=([0-9]+).*sample=\[([^]]*)\].*/\6 \1 \2 \3 \4 \5 \7/' | \
  sort -rn -k1 -k2 | head -20
```

输出示例（第一列为 total_ms）：
```
5234 42 15 28 4800 120 24381_145018, ...
```

---

## 二、主要耗时阶段（按典型占比）

根据 `orchestrator.rs` 与 `mesh_generate.rs` 的计时点：

| 阶段 | 位置 | 典型占比 | 说明 |
|------|------|----------|------|
| **几何体生成** | gen_index_tree_geos_optimized | 20-30% | BRAN/HANG/LOOP/CATE/PRIM 遍历与解析 |
| **Mesh 生成** | generate_meshes_for_batch | 40-60% | Manifold/Truck 三角化、LOD 计算 |
| **布尔运算** | run_bool_worker_from_tasks | 10-25% | 负实体差集、CatePos 计算 |
| **入库写入** | save_instance_data_optimize | 10-20% | SurrealDB inst_relate/inst_geo 写入 |
| **AABB 计算** | inst_relate_aabb | 5-15% | pe_transform 查询与边界框计算 |

---

## 三、优化方案（按耗时类型）

### 1. mesh_ms 占比高（Mesh 生成慢）

**现象**：`[batch_perf]` 中 `mesh_ms` 接近或超过 `total_ms` 的 50%。

**优化方向**：

- **调低 LOD 精度**：在 DbOption 中设置 `default_lod = L2` 或 `L3`，减少三角面数
- **缩小 batch 几何复杂度**：调大 `index_tree_batch_size`（如 300），让单批内几何更均衡
- **预计算 mesh 缓存**：首次全量生成后，增量模式会利用 `RecentGeoDeduper` 跳过未变更 geo
- **降低 mesh_tol_ratio**：如从 3.0 调到 2.0，可减少网格密度

### 2. save_ms 占比高（入库慢）

**现象**：`save_ms` 明显大于 `mesh_ms`。

**优化方向**：

- **使用 defer_db_write**：`--defer-db-write` 先写 SQL 到文件，再单独 `--import-sql` 批量导入
- **提高写库并发**：修改 `orchestrator.rs` 中 `db_write_semaphore` 的 `Semaphore::new(8)` 为 16
- **RocksDB 与连接**：确认 SurrealDB 使用 RocksDB 且无网络延迟

### 3. total_ms 高但 mesh_ms/save_ms 都不突出

**现象**：`total_ms` 大，但 mesh/save 比例正常，可能是几何阶段或等待 I/O。

**优化方向**：

- **几何生成并行**：检查 `index_tree_max_concurrent_targets`（默认 6），可尝试 8
- **跳过 AABB**：设置 `AIOS_SKIP_INST_RELATE_AABB=1` 做实验，排除 AABB 阶段影响
- **缩小目标类型**：`--gen-nouns BRAN,PIPE` 只生成部分类型，用于对比耗时

### 4. 某几个 batch 特别慢（outlier）

**现象**：少数 batch 的 `total_ms` 远高于其余。

**优化方向**：

- 用 `sample=[...]` 中的 refno 定位具体构件
- 检查是否为复杂负实体、大 CATE、高面数 PRIM
- 对这类 refno 使用 `index_tree_excluded_target_types` 或单独优化几何解析

---

## 四、环境变量速查

| 变量 | 作用 |
|------|------|
| `AIOS_LOG_BATCH_PERF=1` | 开启每批次耗时日志（默认开启） |
| `AIOS_SKIP_INST_RELATE_AABB=1` | 跳过 inst_relate_aabb 写入，用于定位耗时 |
| `AIOS_ENABLE_PARQUET_STREAM_WRITER=1` | 启用 Parquet 流式写入 |

---

## 五、代码改动说明

在 `orchestrator.rs` 的 insert_handle 循环中增加了：

1. 每批次开始时的 `batch_start = Instant::now()`
2. 本批次的 `batch_mesh_ms` 与 `batch_save_ms` 统计
3. 满足 `AIOS_LOG_BATCH_PERF` 时打印 `[batch_perf]` 日志

关闭每批次日志可设置：
```bash
export AIOS_LOG_BATCH_PERF=0
```
