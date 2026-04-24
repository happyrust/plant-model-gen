# 下一冲刺开发计划 (2026-04-24)

## 一、当前状态快照

### 各仓库最新提交

| 仓库 | 最新提交 | 日期 | 摘要 |
|------|---------|------|------|
| plant-model-gen | `1ea666e` | 04-24 | BRAN展开+SurrealQL空值防护+ptset批量API+实时实例查询 |
| plant3d-web | `9bcfd9f` | 04-24 | DTX object nearest measure UI |
| rs-core | `982bb16` | 04-16 | bump rusqlite 0.32→0.33 |

### 待清理的工作树

| 仓库 | 未提交改动 | 说明 |
|------|-----------|------|
| plant-model-gen | `.cursor/rules` 少量配置 | 影响小，可随下次功能提交附带 |
| plant3d-web | **66 文件，+4586/-1254 行** | 涉及 review/测量/MBD/截图/API 等多模块，需分批提交 |
| rs-core | `src/rs_surreal/inst.rs` 1 文件 | 待确认是否就绪 |

### 已完成但未收尾的里程碑

| 项目 | 状态 | 出处 |
|------|------|------|
| PDMS Hardening M4（冒烟脚本） | ✅ 完成并验证 | `2026-04-24-pdms-hardening-m3-m5` |
| PDMS Hardening M3（前端404诊断） | ✅ 完成 | 同上 |
| PDMS Hardening M5（路由清单打印） | ⏳ 下一步 | 同上 |
| 批注流转门禁 V1 前后端 | ✅ 完成（已提交） | `2026-04-21-next-iteration` |
| 异地协同 Phase 1-4 | ✅ 完成（已提交） | git log 04-22/23 |
| plant3d-web 大量 WIP | ⚠️ 未提交 | git status |

---

## 二、下一冲刺目标（优先级排序）

### Sprint 目标：稳定化 + 可观测性 + 测量工具完善

---

### P0：plant3d-web WIP 分批提交与整理（0.5天）

**问题**：66个文件的未提交改动涉及 review/测量/MBD/API 多个功能域，长期不提交风险高。

**方案**：
1. 按功能域分 3~4 批提交：
   - **Batch 1 — 测量工具**：`MeasurementPanel.vue`, `XeokitMeasurementPanel.vue`, `MeasurementOverlayBar.vue`, `useXeokitMeasurementTools.ts`, `useXeokitMeasurementStyleStore.ts`, `xeokitMeasurementFormat.ts`, `XeokitElevation*.ts`, 测量教程文档
   - **Batch 2 — MBD 管道标注**：`MbdPipePanel.vue`, `mbdPipeApi.ts`, `branchLayoutEngine.*`, `computePipeAlignedOffsetDirs.*`, `useMbdPipeAnnotationThree.ts`, `bran-test-data.*`
   - **Batch 3 — 三维校审/Review**：`ReviewPanel.vue`, `ReviewCommentsTimeline.vue`, `TaskReviewDetail.vue`, `reviewPanelActions.ts`, `reviewRecordReplay.ts`, `DesignerCommentHandlingPanel.vue`, `review/adapters/*`, `review/domain/*`, 开发文档
   - **Batch 4 — 其他**：`useModelGeneration.ts`, `useToolStore.ts`, `useScreenshot.ts`, `ViewerPanel.vue`, `PtsetPanel*.vue`, `ribbonConfig.ts`, API 文件, `auth.ts`
2. 每批提交后 `npm run type-check` 验证

**验收**：git status 干净，各批次独立可回溯

---

### P1：PDMS Hardening M5 — 启动期路由清单打印（0.5天）

**问题**：web_server 启动时无法看到已注册路由，排障困难。

**方案**（来自 `2026-04-24-pdms-hardening-m3-m5`）：
1. `src/web_api/mod.rs` 新增 `stateless_web_api_route_paths()` 返回静态路由列表
2. `src/web_server/mod.rs` 启动时拼接打印完整路由清单
3. debug build 默认打印，release 需 `AIOS_PRINT_ROUTES=1`

**验收**：
- `cargo build --bin web_server --features web_server` 通过
- 启动日志出现 `registered routes` 段落，包含 PDMS 5 条关键路径

---

### P2：Admin 站点安全默认值 + 状态机加固（1天）

**问题**（来自 `2026-04-21-next-iteration-plan` P0）：
- SurrealDB 默认绑定 `0.0.0.0` 存在安全风险
- `root/root` 弱凭据未拒绝
- 状态机约束不足（可重复启动、解析中可启动等）

**方案**：
1. **安全默认值收口** (`managed_project_sites.rs`)
   - 默认绑定收窄为 `127.0.0.1`
   - 创建/更新时拒绝 `root/root` 弱凭据
   - `SiteDrawer.vue` 去掉默认凭据填充
2. **状态机动作约束** (`managed_project_sites.rs`)
   - `parse`: Running/Starting/Stopping 或 parse_status=Running 时拒绝
   - `start`: 禁止重复启动
   - `stop`: 允许在有活动进程时执行
   - `delete`: 任一进程活跃时拒绝
3. **前端按钮联动** (`SiteDataTable.vue`, `SiteDetailHeader.vue`, `site-status.ts`)

**验收**：admin API HTTP 请求验证 + `scripts/test-admin-deployment.ps1`

---

### P3：Viewer URL 配置化（0.5天）

**问题**（来自 `2026-04-21-next-iteration-plan` P1）：`localhost:3101` 硬编码。

**方案**：
- `SiteDataTable.vue`/`SiteDetailView.vue` 中 Viewer URL 改为读取环境变量 `VIEWER_BASE_URL` 或 admin 配置项
- 保留 `backendPort/backend + output_project` query 协议不变

**验收**：修改环境变量后 Viewer 链接正确变化

---

### P4：错误反馈增强（0.5天）

**问题**（来自 `2026-04-21-next-iteration-plan` P2）：动作失败后用户看不到原因。

**方案**：
- `sites.ts` store 增加动作级错误状态
- `SiteDetailView.vue` 增加 toast/banner 展示失败原因
- 后端 `admin_handlers.rs` 细化错误码

**验收**：刻意触发各类错误场景，前端均能显示可理解的错误信息

---

### P5：rs-core 待清理项（持续）

| 项目 | 优先级 | 说明 |
|------|--------|------|
| `inst.rs` 未提交改动 | 高 | 确认并提交 |
| AiosDBMgr → QueryProvider 迁移 | 中 | 跨仓影响大，需独立排期 |
| sweep/CSG 后续完善 | 低 | 功能已可用，优化为渐进式 |

---

## 三、执行顺序与时间线

```
Day 1 (04-25)
├─ P0: plant3d-web WIP 分批提交 (上午)
└─ P1: PDMS M5 路由清单打印 (下午)

Day 2 (04-28)
├─ P2: Admin 安全默认值 + 状态机 — 后端 (全天)
└─ P5: rs-core inst.rs 提交 (穿插)

Day 3 (04-29)
├─ P2: Admin 安全 — 前端联动 (上午)
└─ P3: Viewer URL 配置化 (下午)

Day 4 (04-30)
├─ P4: 错误反馈增强 (上午)
└─ 回归验证 + 部署 (下午)
```

---

## 四、风险与规避

| 风险 | 等级 | 规避 |
|------|------|------|
| P0 分批提交时引入编译/类型错误 | 中 | 每批提交后 `npm run type-check` |
| P2 状态机改动影响已有站点管理流程 | 中 | 先只做"约束拒绝"，不改数据模型 |
| P1 路由列表与实际路由不同步 | 低 | 从 `assemble_stateless_web_api_routes` 自动提取 |
| rs-core QueryProvider 迁移范围膨胀 | 高 | 本冲刺只做评估，不动手 |

---

## 五、本冲刺不做的事

- rs-core AiosDBMgr → QueryProvider 大重构（需独立方案）
- plant3d-web 全仓 fetchJson 风格统一（超出 PDMS scope）
- Playwright E2E 全量校审测试（单独排期）
- 异地协同 Phase 5+（等 Phase 1-4 稳定后再推进）

---

## 六、与上游计划的关系

| 本轮任务 | 上游计划文件 | 关系 |
|---------|------------|------|
| P1 M5 | `2026-04-24-pdms-hardening-m3-m5` | 直接承接，完成最后一个里程碑 |
| P2 | `2026-04-21-next-iteration-plan` P0 | 直接承接 |
| P3 | `2026-04-21-next-iteration-plan` P1 | 直接承接 |
| P4 | `2026-04-21-next-iteration-plan` P2 | 直接承接 |
| P0 | 无 | 新增，工程卫生 |
