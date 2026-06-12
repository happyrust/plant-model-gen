# Plan: aabb/vec3 增量持久化

## 现状 vs 目标态

```
现状（O(N²)，每批 ~5.5s 固定成本 × 2415 批 ÷ 2 workers ≈ 126 min）：

  batch.mesh_results ──┐
                       ├─→ persist_mesh_results
  全局 mesh_aabb_map ──┤      ├─ save_pts_to_surreal(整个 pts_map)   ← 全量！100行/chunk
  （单调增长到 37k+）   │      ├─ save_aabb_to_surreal(整个 aabb_map) ← 全量！300行/chunk
  全局 mesh_pts_map ───┘      └─ inst_geo mesh 回写（本批，OK）

目标态（每批只写 delta，收尾补写一次）：

  batch.mesh_results ──→ persist_mesh_results
       │                    ├─ delta_aabb = mesh_results[*].aabb_hash → 查全局 map 取值
       │                    ├─ delta_pts  = mesh_results[*].pts_hashes → 查全局 map 取值
       │                    ├─ save_pts_to_surreal(delta_pts)    ← 仅本批，~几十行
       │                    ├─ save_aabb_to_surreal(delta_aabb)  ← 仅本批，~几十行
       │                    └─ inst_geo mesh 回写（不变）
       │
  sink 全部完成（orchestrator.rs L1322 "ModelWriter finish"）
       └─→ finalize_mesh_entities(全局 aabb_map, 全局 pts_map)   ← 一次性 ~5s 安全网
            （INSERT IGNORE 全量补写；AIOS_SKIP_FINAL_AABB_SWEEP=1 跳过）
```

delta 可完全由 `MeshResult` 还原（无需 dirty-set 共享状态）的依据：
- CSG 新生成：`handle_csg_mesh`（mesh_generate.rs L2142-2187）回填
  `aabb_hash: Some(h)` + `pts_hashes`，且同步 insert 进全局 map。
- 缓存命中（本地 AABB 可得）：`generate_meshes_for_batch`（L378-389 / L408-421）
  回填 `aabb_hash: Some(h)` 并 `or_insert` 进全局 map → delta 会包含它（IGNORE 幂等，无害）。
- 缓存命中（仅 mesh 存在）：`aabb_hash=None`、`pts_hashes=[]` → 本批不写，
  行应已在历史运行写入；漂移由收尾补写兜底。

## 改动清单

| # | 位置 | 改动 |
|---|---|---|
| 1 | `src/fast_model/gen_model/model_writer.rs::persist_mesh_results`（L281-337） | 删除 L305-306 的全量 `save_pts_to_surreal(mesh_pts_map)` / `save_aabb_to_surreal(mesh_aabb_map)`；改为从 `mesh_results` 收集 `aabb_hash`/`pts_hashes`，在全局 map 中取值构造两个局部 `DashMap`，传给原有 save_* 函数（函数本身不动） |
| 2 | `src/fast_model/gen_model/model_writer.rs`（trait `ModelWriterBackend`） | 新增 `finalize_mesh_entities(&self, aabb_map, pts_map) -> Result<ModelWriterStageReport>`，默认实现 no-op skipped；`SurrealModelWriterBackend` 实现为全量 `INSERT IGNORE` 补写（复用 save_*），返回行数；`DrainOnlyModelWriterBackend` 用默认 no-op |
| 3 | `src/fast_model/gen_model/orchestrator.rs`（L1322 finish 处） | sink 完成后调用 `model_writer.finalize_mesh_entities(...)`；打印 `[gen_model] final_sweep aabb=N pts=M elapsed_ms=T`；读取 env `AIOS_SKIP_FINAL_AABB_SWEEP` 决定跳过（跳过也打印一行原因） |
| 4 | `src/fast_model/gen_model/precheck_coordinator.rs::check_pe_transform`（L202-221） | 增加覆盖探测：对每个目标 dbnum 发一次轻量 count/LIMIT 1 查询；表缺失或未覆盖 → 调 `refresh_pe_transform_for_dbnums`（`pe_transform_refresh.rs` 既有函数），刷新条数写入 `stats.pe_transform_refreshed`；摘要打印覆盖/刷新结果 |
| 5 | `src/fast_model/gen_model/transform_rkyv_cache.rs`（构建失败路径） | 失败负缓存：同 dbnum 构建失败后本次运行内直接走 DB 回退，不再重复尝试构建 + 刷错误日志（基线 1 秒内同库 6+ 次） |
| 6 | `specs/004-model-generation-write-pipeline-performance/spec.md` | Decisions 表加注：实测剩余瓶颈（每批全量重写 aabb/vec3）归属 spec 006 |
| 7 | `CHANGELOG.md` | 记录增量化 + 收尾补写 + precheck pe_transform 真实化 |

## 关键决策依据

- **为什么 delta 从 `mesh_results` 还原而不是 dirty-set**（grill 自答）：
  `MeshResult.aabb_hash`/`pts_hashes` 已携带本批全部新增键，零新增共享状态、
  零锁竞争；dirty-set 需要 producer/consumer 双端配合，复杂度高且容易在
  异常路径漏 drain。
- **为什么保留收尾全量补写（Q2=A）**：增量化后"缓存命中但 DB 缺行"
  （换库/手动清表/上轮写失败）会变成永久缺行且难排查；补写幂等、一次性
  ~5s（对照基线每批 5s），是用确定性换可忽略成本。spec 005 拒绝的是
  DELETE/预查，补写不属其列。
- **为什么 pe_transform 刷新放 precheck 而非生成期按需**：rkyv 构建以
  dbnum 为粒度，生成期按需触发会在并发 worker 间放大（基线即如此）；
  precheck 串行刷一次（仅未覆盖时，冷启动 ~4min，热路径 ≤1 查询/dbnum）
  换全程缓存命中。

## 风险与对策

- **R1 收尾补写被误关导致漂移缺行**：跳过时打印显式日志；验收第 4 条强制
  确认补写恰好一次。
- **R2 delta 构造引用全局 map 的读放大**：本批键数 ≈ mesh_tasks（几十），
  DashMap 点查 O(1)，可忽略。
- **R3 precheck 探测误判（表在但部分 dbnum 未覆盖）**：探测按 dbnum 粒度
  count，不做表级存在性短路。
- **R4 finalize 时机与 boolean 阶段交叠**：finalize 在 sink join 之后、
  boolean 启动之前同步执行，无并发写者（mesh worker 已全部退出）。

## 验证方式

仓库规则不跑 Rust test：`cargo build --release --bin aios-database` +
`quicktest-7997-8080` 站点同配置重跑 + 日志 grep（`[batch_perf]` 分布、
final_sweep 行、pe_transform 报错为零）+ SurrealDB count 查询（CLI+json）
比对 aabb/vec3 行数与 Parquet 产物（详见 spec Acceptance Criteria 六条）。
