# 房间计算性能优化记录

## 优化目标
将全量房间计算性能从当前耗时优化到 5 分钟内完成。

## 已实施的优化

### 1. room->panel 查询改用 TreeIndex（高优先级）

**问题**：原实现使用嵌套 SurrealQL 查询 `FRMW -> SBFR -> PANE`，对几百个房间会产生大量递归子查询。

**优化**：
- 新增 `query_room_panels_with_tree_index()` 函数
- 新增 `query_candidate_rooms()` 函数
- 使用 `TreeIndexManager` 和 `aios_core::collect_descendant_filter_ids()` 查询面板
- 按 dbnum 分组复用 TreeIndexManager，避免重复加载

**预期收益**：对于几百个房间的场景，查询时间从分钟级降至秒级。

### 2. 写回改为分块提交（中优先级）

**问题**：原实现将所有关系拼接成一个超大 SQL 字符串，导致解析慢、网络压力大。

**优化**：
- 新增 `create_room_panel_relations_batch_chunked()` - 每批 50 个房间
- 新增 `save_room_relate_batch_chunked()` - 每批 100 个面板关系
- 所有调用点已更新使用分块版本

**预期收益**：降低单次 query 压力，提升写入稳定性。

## 验证方法

```bash
# 运行房间计算
cargo run --release --bin aios-database -- room compute

# 或指定关键词
cargo run --release --bin aios-database -- room compute --room-keyword "-RM"
```

## 运维排障

若房间计算在 TreeIndex 路径失败，可先看统一手册：

- `docs/房间计算-TreeIndex排障与日志检索手册.md`

重点日志标签：

- `[ROOM_TREE_INDEX_DBNUM_RESOLVE_FAILED]`
- `[ROOM_TREE_INDEX_LOAD_FAILED]`
- `[ROOM_TREE_INDEX_ROOM_MISSING]`
- `[ROOM_TREE_INDEX_QUERY_FAILED]`

## 后续优化方向（如仍未达标）

1. **空间索引范围化刷新**：避免全量 clear+reload
2. **候选构件二级过滤**：减少进入 27 点投票的候选数
3. **并发粒度调整**：从 room 级改为 panel 级并发

## 修改的文件

- `src/fast_model/room_model.rs`
  - 修改 `build_room_panels_relate_common_with_persist()`
  - 新增 `query_room_panels_with_tree_index()`
  - 新增 `query_candidate_rooms()`
  - 新增 `create_room_panel_relations_batch_chunked()`
  - 新增 `save_room_relate_batch_chunked()`
  - 更新所有调用点
