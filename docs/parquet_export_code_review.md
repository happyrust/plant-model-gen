# Parquet 导出流程 Code Review 报告

> **审核范围**：plant-model-gen 中所有 Parquet 导出相关模块
> **审核日期**：2025-01
> **目标**：定位性能瓶颈，提出重构建议

---

## 一、架构概览

当前存在 **两套主 Parquet 写入路径**，职责仍有一定重叠：

| 模块 | 数据源 | 触发场景 | 输出格式 |
|------|--------|---------|---------|
| `export_dbnum_instances_parquet.rs` | SurrealDB 实时查询 | CLI `--export-parquet`、`--export-dbnum-instances`、MBD pipe 后台 | 5 表 + manifest |
| `parquet_writer.rs` (`ParquetManager`) | `ExportData` 内存结构 | `instance_export.rs` 增量写 | Polars DataFrame → incremental parquet |
| `parquet_stream_writer.rs` (`ParquetStreamWriter`) | `ShapeInstancesData` 流式 | 模型生成期间流式写 | 同上 + merge |

此外还有两个辅助导出模块：
- `export_pdms_tree_parquet.rs` — 导出 PDMS 树结构
- `scene_tree/parquet_export.rs` — 导出 scene_node 数据

---

## 二、性能瓶颈分析（按影响程度排序）

### 🔴 P0 - 严重瓶颈

#### 1. `fn::default_full_name(in)` 数据库函数调用（最大瓶颈）
**位置**：`export_dbnum_instances_parquet.rs:575`
```sql
fn::default_full_name(in) as name
```
**问题**：
- `fn::default_name` / `fn::default_full_name` 会触发 `fn::order` 和 `pe_owner` 图遍历
- 每条 `inst_relate` 记录都会执行一次，批量导出时开销巨大
- `export_pdms_tree_parquet.rs:136` 的注释已经明确指出此问题：
  > "default_name 会调用 fn::order / pe_owner 图遍历，批量导出时开销非常大（会导致导出耗时数十分钟甚至更久）"

**影响**：对于数万条记录，这一个函数可能占总耗时的 **50-80%**。

**建议**：
- 改用 `in.name as name`（直接取 pe.name）
- name 为空时，在 Rust 端用 TreeIndex 的 order_map 兜底生成 `"{NOUN} {order+1}"`
- 保持唯一 SurrealDB 导出路径内部策略一致，避免再引入平行导出实现

#### 2. `in->inst_relate_aabb[0].out` 图遍历语法
**位置**：`export_dbnum_instances_parquet.rs:576`
```sql
IF in->inst_relate_aabb[0].out != NONE THEN record::id(in->inst_relate_aabb[0].out) END as aabb_hash
```
**问题**：
- SurrealDB 3.x 中 `->` 图遍历语法在 SELECT 子查询中可能返回空结果（已知 bug，见 MEMORY）
- 即使能正常工作，每行都做图遍历性能很差
- 且 `aabb_hash` 在后续步骤 3 中通过 `query_insts_for_export` 又会被查询一次（冗余）

**建议**：
- 从 `query_inst_relate_rows` 中移除 `aabb_hash` 查询
- 统一从 `query_insts_for_export` 的结果中获取 `world_aabb_hash`

#### 3. 串行查询，无并发
**位置**：`export_dbnum_instances_parquet.rs:874-1143`

主导出函数中的查询完全串行执行：
```
步骤 1: TreeIndex 加载（同步）
步骤 2: query_inst_relate_rows（串行分批 await）
步骤 3: query_insts_for_export（串行 await）
步骤 4: query_tubi_relate（串行分批 await）
步骤 5: 构建行数据（CPU）
步骤 6: query_trans_rows + query_aabb_rows（串行 await）
步骤 7: 写 Parquet（同步 I/O）
```

**建议**：
- **步骤 2 + 3**：inst_relate 和 export_inst 查询可以流水线化
- **步骤 6**：`query_trans_rows` 和 `query_aabb_rows` 完全无依赖，应 `tokio::join!` 并行
- **步骤 7**：5 个 Parquet 文件的写入可以用 `rayon::join` 或 `tokio::task::spawn_blocking` 并行

### 🟡 P1 - 中等瓶颈

#### 4. 分批查询批大小偏小
**位置**：
- `query_inst_relate_rows` BATCH_SIZE = 500
- `query_tubi_relate` chunk size = 50
- `query_trans_rows` / `query_aabb_rows` chunk size = 500

**问题**：
- `tubi_relate` 的 chunk=50 过小，每个 chunk 生成一条 SQL + 一次网络往返
- 过多的小批次查询导致网络延迟累积

**建议**：
- `tubi_relate` 批大小提升到 200-500

#### 5. 大量 String clone 和中间分配
**位置**：`export_dbnum_instances_parquet.rs:895-1124`

构建行数据时存在大量 `.clone()`、`.to_string()`：
```rust
// 每个 child 都有：
refno_str: child.refno.to_string(),      // clone
name: child.name.clone(),                 // clone
owner_refno_str: Some(owner_refno.to_string()), // clone
trans_hash: trans_hash.clone(),           // clone
aabb_hash: child_aabb_hash,              // move（较好）
```

**建议**：
- 使用 `Arc<str>` 或 `Rc<str>` 替代频繁 clone 的 hash 字符串
- 或在构建 Arrow Array 时直接 collect，避免中间 `Vec<InstanceRow>` 结构

#### 6. `ParquetManager` / `ParquetStreamWriter` 的 compact 操作
**位置**：`parquet_writer.rs:compact_table`, `parquet_stream_writer.rs:compact_dbno`

**问题**：
- compact 需要读取所有增量文件 + 主文件 → vstack → unique → 重写
- `ensure_geo_items_format` 对每个 DataFrame 都做 schema 检查和类型转换
- unique 去重使用全 DataFrame 扫描

**建议**：
- 在增量文件数达到阈值时才 compact（如 ≥10 个文件）
- 或改用 append-only 写入 + 外部索引去重

### 🟢 P2 - 改善建议

#### 7. 多套导出/写入路径代码重复
**问题**：
- `export_dbnum_instances_parquet` 与增量写入链路（`parquet_writer.rs` / `parquet_stream_writer.rs`）仍存在 schema、批构建和文件写入层面的重复
- `parquet_writer.rs` 和 `parquet_stream_writer.rs` 的 DataFrame 创建、geo_items 格式处理、compact 逻辑高度相似

**建议**：
- 提取公共的 `ParquetTableBuilder` trait/struct：负责 schema 定义、batch 构建、文件写入
- `ParquetManager` 与 `ParquetStreamWriter` 合并为一个带模式标记的统一 writer


#### 9. scene_tree parquet 使用了 `fn::default_name`
**位置**：`scene_tree/parquet_export.rs:48`
```sql
fn::default_name(type::record('pe', record::id(id))) ?? '' as name
```
**问题**：与 P0-1 相同的性能问题，但 scene_tree 节点数通常较少，影响较轻。

**建议**：统一改用 pe.name + Rust 端兜底。

---

## 三、重构方案

### Phase 1：快速见效（预计提速 3-10x）

1. **移除 `fn::default_full_name` 调用**
   - `query_inst_relate_rows` 中改用 `in.name as name`
   - 在 Rust 端用 TreeIndex 的 order_map 生成 default_name 兜底
   - 同步修改 `scene_tree/parquet_export.rs`

2. **移除冗余的 `aabb_hash` 图遍历查询**
   - 从 `query_inst_relate_rows` SQL 中删除 `inst_relate_aabb` 相关子查询
   - `aabb_hash` 统一从 `export_inst_map` 获取

3. **并行化独立查询**
   ```rust
   // 步骤 6 并行
   let (transform_rows, aabb_row_data) = tokio::join!(
       query_trans_rows(&trans_hashes, &unit_converter, verbose),
       query_aabb_rows(&aabb_hashes, &unit_converter, verbose),
   );
   ```

4. **增大 tubi_relate 批大小**
   - 从 50 提升到 200

### Phase 2：结构性重构（中期）

5. **统一数据源抽象**
   - 提取 `trait ParquetDataSource`，将 SurrealDB 查询和 Cache 读取统一为相同接口
   - 复用行构建 + Parquet 写入逻辑

6. **合并 ParquetManager / ParquetStreamWriter**
   - 保留流式写入能力，但合并 compact 和 DataFrame 创建逻辑

7. **Parquet 文件写入并行化**
   - 5 个 Parquet 文件使用 `rayon` 或 `tokio::task::spawn_blocking` 并行写入

### Phase 3：深度优化（长期）

8. **查询流水线**
   - inst_relate 查询完成一批后，立即启动 geo_relate 查询，而非等待所有 inst_relate 完成

9. **Arrow 零拷贝构建**
   - 直接从查询结果构建 Arrow Array，跳过中间 `Vec<Row>` 结构

---

## 四、预期收益

| 优化项 | 预期提速 | 改动量 | 风险 |
|--------|---------|--------|------|
| 移除 fn::default_full_name | **3-10x** | 小 | 低 |
| 移除冗余 aabb 图遍历 | 10-30% | 小 | 低 |
| 并行化 trans/aabb 查询 | 20-40% | 小 | 低 |
| 增大 tubi 批大小 | 10-20% | 极小 | 低 |
| 统一数据源 + 代码去重 | 维护性提升 | 中 | 中 |
| Parquet 文件并行写入 | 10-20% | 小 | 低 |

**Phase 1 综合预期**：总导出耗时减少 **60-90%**（主要来自移除 fn::default_full_name）。

---

## 五、待确认事项

1. `fn::default_full_name` 与 `fn::default_name` 的区别——前端是否依赖 full_name 格式？如果是，需要在 Rust 端复现相同逻辑。
2. `parquet_writer.rs` 和 `parquet_stream_writer.rs` 是否仍在活跃使用？从 `instance_export.rs` 看 `ParquetManager` 有使用，但 `ParquetStreamWriter` 的调用方不明确。
3. 当前导出的 Parquet 文件大小和行数量级？（决定是否需要分片写入或 row group 调优）
