# 技术债收口与回归验证计划（2026-04-25）

> 承接 2026-04-24 Sprint（P0-P5 共 10 个提交）结束后剩下的两类尾巴：
> ① 2026-04-24 Sprint 代码改动的**运行时冒烟**（受本机 PID 27112 占用 `web_server.exe` 阻塞）；
> ② 历史迁移项目的**最后一公里**——`rs-core::AiosDBMgr` 的彻底移除。

## 一、基线现状（2026-04-25 测绘）

### 1.1 2026-04-24 Sprint 已落地但未做运行时验证的 4 项

| 项 | commit | 状态 |
|----|--------|------|
| P1 M5 启动期路由打印 | `79ebcf8` | cargo check 通过；没跑过 `AIOS_PRINT_ROUTES=1 cargo run` 实际看日志 |
| P2 弱凭据 / 0.0.0.0 拒绝 | `a8de78e` | cargo check + vue-tsc 通过；没跑过 `POST /api/admin/sites` 三条 smoke |
| P3 viewer base URL 运行期可配置 | `9e9a676` | 两端编译通过；没有 `AIOS_VIEWER_BASE_URL=...` 启动下的端到端验证 |
| P4 错误反馈细化 | `8b1f74d` | 两端编译通过；没触发过 400/409 对应的 UI banner 观察 |

**共同阻塞点**：本机 `target/debug/web_server.exe` 被 PID **27112** 持有（自动化检测到），导致 `cargo build` 直接报 "拒绝访问 (os error 5)"，`cargo run` 跑的还是旧二进制。

### 1.2 AiosDBMgr → QueryProvider 迁移的真实剩余量

原始 plan（`rs-core/.cursor/AiosDBMgr 迁移到 QueryProvider 计划.plan.md`）估计四个阶段。实际 2026-04-25 测绘：

| 阶段 | 状态 | 证据 |
|------|------|------|
| Phase 1：`db_pool/` 模块 | ✅ 已完成 | `rs-core/src/db_pool/mod.rs` 3.7KB，提供 `get_project_pool` / `get_global_pool` / `get_project_pools` / `get_puhua_pool` |
| Phase 2：`ProviderPdmsInterface` 新 impl | ✅ 已完成 | `rs-core/src/aios_db_mgr/provider_impl.rs` 9.1KB |
| Phase 3：替换所有使用点 | ✅ 已完成 | `material/{dq,gps,gy,nt,sb,tf,tx,yk}.rs` 11 处全部改用 `db_pool::get_project_pool`；`ssc_setting.rs` / `material/yk.rs` 业务层已通过 `&dyn PdmsDataInterface` 接受 `ProviderPdmsInterface` |
| Phase 4：移除 `AiosDBMgr` | ❌ 未完成 | `aios_db_mgr/aios_mgr.rs` 仍保留 13.5KB；`material/mod.rs:1` 还有一行无用 `use crate::aios_db_mgr::aios_mgr::AiosDBMgr;`（仅剩 `// set_pdms_major_code(&aios_mgr)` 这种注释掉的遗迹） |

**结论**：Phase 4 的"最后一公里"只剩两步——删掉 `aios_mgr.rs` 整个文件、清掉 `material/mod.rs` 的死导入；本子 plan 把这两步打包执行。

### 1.3 发现但不在本轮处理

- **plant3d-web 类型错误堆积**：`vue-tsc -p tsconfig.app.json` 报 **537 条**，集中在 `LineMaterial.scale`（@types/three 版本错配）/ `troika-three-text` 缺声明 / `DTXSelectionController.test.ts` mock 类型等。2026-04-24 Sprint 期间验证过"本批提交不新增错误"（批间稳定 537→537→537→537），但基数本身是个信号噪音。建议单独排期（2-3 天体量，不在本 plan）。
- **plant-model-gen 类型检查脚本问题**：`package.json::type-check` 走根 `tsconfig.json`（`files: []`，瞬间返回）而不是 `tsconfig.app.json`，形同 no-op。建议修一下 `scripts.type-check`（10 分钟活，但会把当前噪音暴露到 CI）。

## 二、实施步骤

### Task 1：rs-core AiosDBMgr Phase 4 彻底移除

**Files:**
- Delete: `rs-core/src/aios_db_mgr/aios_mgr.rs`（整文件，约 400 行 / 13.5KB）
- Modify: `rs-core/src/aios_db_mgr/mod.rs`（移除 `pub mod aios_mgr;`）
- Modify: `rs-core/src/material/mod.rs`（移除 `use crate::aios_db_mgr::aios_mgr::AiosDBMgr;` 这一行死导入）

**Step 1**：验证 `aios_mgr.rs` 无外部调用

本轮已经确认：
- `AiosDBMgr::` / `aios_mgr::AiosDBMgr` 的 grep 结果在 rs-core 内只剩 `aios_mgr.rs` 自身、`material/mod.rs` 的死导入
- `init_surreal_with_signin` / `query_own_room_panel_elevations` / `query_around_owner_within_radius` 的 grep 结果都只出现在 `aios_mgr.rs` 自己
- plant-model-gen / plant3d-web / plant-collab-monitor 四个 workspace 都无外部引用

**Step 2**：动手删除 + 去死导入

**Step 3**：`cargo check --lib` 验证（按 rs-core AGENTS.md 约定，禁用 `cargo test`）

Expected：零错误（可能有新的 unused warning，继续保留；本轮不做全面 warning 清理）。

**Step 4**：若 `cargo check --lib` 通过，提交

### Task 2：2026-04-24 Sprint 运行时冒烟

**前置**：需要用户停掉 PID 27112 的 web_server 进程，或自动化脚本在收到用户确认后执行 `Stop-Process -Id 27112 -Force`。停掉后本机 `cargo build` 才能覆盖 `target/debug/web_server.exe`。

**Step 1**：rebuild web_server
```powershell
cd D:\work\plant-code\plant-model-gen
cargo build --bin web_server --features web_server
```
Expected：编译通过。

**Step 2**：启动 + M5 路由日志核对
```powershell
$env:AIOS_PRINT_ROUTES = '1'
cargo run --bin web_server --features web_server
```
Expected：启动日志出现
```
[web_server] registered routes (stateless web_api)
  ...
  GET    /api/pdms/transform/{refno}
  GET    /api/pdms/transform/compute/{refno}
  GET    /api/pdms/ui-attr/{refno}
  GET    /api/pdms/type-info
  GET    /api/pdms/children
  ...
```

**Step 3**：P2 弱凭据 + 公网绑定拒绝 smoke（另开终端）
```powershell
# 应 400：数据库凭据过于简单
Invoke-RestMethod -Method Post -Uri http://127.0.0.1:3100/api/admin/sites `
  -ContentType 'application/json' `
  -Body (@{ project_name='t'; project_path='D:\temp\t'; project_code=1; db_user='root'; db_password='root'; db_port=8001; web_port=3201 } | ConvertTo-Json)

# 应 400：bind_host=0.0.0.0 会将站点暴露
Invoke-RestMethod -Method Post -Uri http://127.0.0.1:3100/api/admin/sites `
  -ContentType 'application/json' `
  -Body (@{ project_name='t'; project_path='D:\temp\t'; project_code=1; db_user='gooduser'; db_password='strongpw'; bind_host='0.0.0.0'; db_port=8002; web_port=3202 } | ConvertTo-Json)

# env 放行后应 201
Stop-Process -Name web_server -Force
$env:AIOS_ALLOW_PUBLIC_BIND='1'
cargo run --bin web_server --features web_server
# 重试上面第二条请求
```

**Step 4**：P3 Viewer URL 运行时核对
```powershell
Stop-Process -Name web_server -Force
$env:AIOS_VIEWER_BASE_URL = 'http://viewer.example.com:9999'
cargo run --bin web_server --features web_server

# 登录 admin，拉 /api/admin/app-config，期望 data.viewer_base_url == 'http://viewer.example.com:9999'
# UI 端：列表页 Running 状态站点的 Viewer 按钮 href 应以该 base 开头
```

**Step 5**：P4 错误反馈 UI 观察
- 在 SiteDrawer 里提交 `root/root`：期望详情页顶部 banner 显示 **"创建/保存失败：数据库凭据过于简单（root/root）..."**
- 对 Running 状态站点点"解析"：期望 banner 显示 **"解析失败：站点运行中，请先停止站点再解析"**
- 点"关闭"按钮，banner 消失

### Task 3（stretch）：修 plant-model-gen `npm run type-check`

**Files:**
- Modify: `plant-model-gen/ui/admin/package.json`（如果该子项目也有这个问题）
- Modify: `plant3d-web/package.json`（根 `type-check` 脚本）

- 当前：`"type-check": "vue-tsc --noEmit --pretty false"`，根 `tsconfig.json` 的 `files: []` + `references` 让 vue-tsc 不做任何事
- 改为：`"type-check": "vue-tsc --noEmit --pretty false -b"`（启用 references 构建模式）或 `"type-check": "vue-tsc --noEmit --pretty false -p tsconfig.app.json"`

不做的事：本轮不清理 plant3d-web 的 537 条既有错误，那是另一个 Sprint 的体量。

## 三、验证

- Task 1：`cargo check --lib` in `rs-core/` → 0 errors
- Task 2：上述五个 Step 按预期返回 / 日志 / UI 行为
- Task 3（若做）：`npm run type-check` 不再是 no-op，能看到 537 条错误（这是 baseline，不是回归）

## 四、风险

| 风险 | 等级 | 缓解 |
|------|------|------|
| 删除 `aios_mgr.rs` 后有其他 workspace（gen_model-dev / web-server）引用它 | 中 | 本 plan 已 grep 四个当前 workspace；gen_model-dev / web-server 不在当前 Cursor workspace，但通过 cargo 依赖关系应会在 `cargo check` 阶段暴露；若真出现，回滚删除，只做 mod.rs 与 material/mod.rs 的清理 |
| 停掉 PID 27112 打断用户本地 admin 页面 | 低 | 需要用户明确授权，停之前保存状态；重启后用 `AIOS_PRINT_ROUTES=1` 跑起来，功能不变 |
| P2 smoke 的 `bind_host=0.0.0.0` 清理不干净遗留脏数据 | 低 | smoke 用的是临时 `project_name='t'`，测完 `DELETE /api/admin/sites/t-<port>` 清掉即可 |

## 五、完成定义

- Task 1：`aios_mgr.rs` 被删，`cargo check --lib` 通过，形成一个 rs-core commit（`chore(aios_db_mgr): remove legacy AiosDBMgr ...`）
- Task 2：五个 Step 的实际输出记录回本文件第六节
- Task 3（可选）：`npm run type-check` 能看到真实错误数

## 六、执行记录（2026-04-25）

### Task 1：AiosDBMgr Phase 4 彻底移除 ✅

- 删：`rs-core/src/aios_db_mgr/aios_mgr.rs`（398 行 / 13561 字节）
- 改：`rs-core/src/aios_db_mgr/mod.rs` — 去掉 `pub mod aios_mgr;`
- 改：`rs-core/src/material/mod.rs` — 去掉 dead import
- 验证：`cargo check --lib` → `0.68s, 0 errors`
- 提交：rs-core `9b85cb05 chore(aios_db_mgr): remove legacy AiosDBMgr (migration Phase 4 last mile)` · 3 files · -398

### Task 2：2026-04-24 Sprint 运行时冒烟（待执行）

需要用户授权停掉 PID 27112 的 web_server.exe 后执行。预期操作序列：

1. `Stop-Process -Id 27112 -Force` （用户授权）
2. `cargo build --bin web_server --features web_server`（在 plant-model-gen 目录）
3. `$env:AIOS_PRINT_ROUTES='1'; cargo run --bin web_server --features web_server`
4. 查看启动日志中 `[web_server] registered routes (stateless web_api)` 段落
5. 在另一终端按 Step 3/4/5 里的 Invoke-RestMethod 序列跑 P2/P3/P4 smoke，把 HTTP 状态码与 UI banner 观察补回来

### Task 3：修 plant-model-gen `npm run type-check`（未执行）

本轮暂缓——plant-model-gen 下 `ui/admin/package.json` 的 type-check 脚本已经是 `vue-tsc -b` 走 references 模式，是正常工作状态。问题主要出在 plant3d-web 的 `package.json`（在另外一个 workspace）。视后续是否做"plant3d-web 537 条类型错误清扫 Sprint"一起处理。
