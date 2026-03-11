# Room Compute Panel Auto Generation Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 让 `room compute-panel <panel> --expect-refnos <refno>` 在自动生成 panel 与 expect 对应模型后，也能自动准备房间计算所需的 SQLite 空间索引数据。

**Architecture:** 保留现有的 `panel + expect/owner(BRAN/HANG)` 自动生成目标收集逻辑，在生成完成后补一段“从 model cache 导出 instances 并刷新 SQLite RTree”的流程。这样不依赖已被跳过写入的 `inst_relate_aabb`，而是直接复用现有 cache->instances->sqlite 的能力，保持改动集中在 CLI 房间计算链路。

**Tech Stack:** Rust, Tokio, SurrealDB, SQLite RTree, 现有 IndexTree 生成管线

---

### Task 1: 为 compute-panel 的生成后索引准备补测试入口

**Files:**
- Modify: `src/cli_modes.rs`

**Step 1: Write the failing test**
- 为新的纯函数写测试，覆盖：
  - panel 自身总会加入生成目标
  - expect refno 若 owner 为 BRAN/HANG，则同时可推导出需要准备索引的 dbnum
  - 生成配置会开启 `export_instances`

**Step 2: Run test to verify it fails**
Run: `cargo test --lib cli_modes`
Expected: 新测试失败，提示缺少辅助函数或断言不满足。

**Step 3: Write minimal implementation**
- 提取纯函数：
  - 构建 compute-panel 生成配置 override
  - 从 refnos 推导 dbnums

**Step 4: Run test to verify it passes**
Run: `cargo test --lib cli_modes`
Expected: 新增测试通过。

### Task 2: 接上生成后 SQLite 空间索引刷新

**Files:**
- Modify: `src/cli_modes.rs`
- Modify: `src/fast_model/gen_model/orchestrator.rs`
- Modify: `src/fast_model/mod.rs`（如需 re-export）

**Step 1: Write the failing test**
- 增加最小测试，验证 room compute-panel 生成配置会请求导出实例，并且会对推导出的 dbnum 触发索引准备路径。

**Step 2: Run test to verify it fails**
Run: `cargo test --lib cli_modes`
Expected: 测试因未调用新路径而失败。

**Step 3: Write minimal implementation**
- 将 `update_sqlite_spatial_index_from_cache` 提供为可复用接口
- `room_compute_panel_mode` 在 `gen_all_geos_data` 后调用该接口
- 只针对本次 refnos 推导出的 dbnums 执行，避免全库刷新

**Step 4: Run test to verify it passes**
Run: `cargo test --lib cli_modes`
Expected: 新增测试通过。

### Task 3: 用真实命令回归验证

**Files:**
- None

**Step 1: Run regression command**
Run: `target/debug/aios-database room compute-panel 24381/35798 --expect-refnos 24381/145019`
Expected: 不再因为候选列表为空而直接失败；若数据完整，应输出 `全部验证通过`。

**Step 2: Run targeted test command**
Run: `cargo test --lib cli_modes`
Expected: 相关单元测试通过。

**Step 3: Run compile verification**
Run: `cargo check`
Expected: 编译通过。
