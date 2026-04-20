# 下一迭代开发计划 (2026-04-21)

## 当前状态总结

### 已完成（未提交）

| 模块 | 状态 | 关键文件 |
|------|------|---------|
| 批注流转门禁 V1（后端） | ✅ 完成 | `annotation_check.rs`, `workflow_sync.rs`, `review_form.rs` |
| 批注流转门禁 V1（前端） | ✅ 完成 | `ReviewPanel.vue`, `reviewPanelActions.ts` |
| workflow/sync form_id 增强 | ✅ 完成 | `workflow_sync.rs`, `types.rs`, `tests.rs` |
| Admin 站点管理 UI 改造 | ✅ 完成 | `SiteDataTable.vue`, `SiteDetailView.vue`, `SitesView.vue` |
| PMS 校审模拟器增强 | ✅ 完成 | `pmsReviewSimulator.ts`, `pmsReviewSimulatorLaunchPlan.ts` |

### 已有但未执行的计划

- `docs/plans/2026-04-19-admin-站点部署整改计划.md` — 5 阶段（安全收口 → 状态机加固 → 部署解耦 → Viewer 联动 → 可观测性）

## 下一步优先级排序

### P0：Admin 站点部署安全与状态机加固（Phase 1+2 合并）

**目标**：将已有整改计划中最紧迫的安全默认值 + 状态机约束一次落地。

**改动范围**：

1. **安全默认值收口** (`managed_project_sites.rs`)
   - SurrealDB 默认绑定从 `0.0.0.0` 收窄为 `127.0.0.1`
   - 创建/更新时拒绝 `root/root` 弱凭据组合
   - `SiteDrawer.vue` 去掉默认凭据填充

2. **状态机动作约束** (`managed_project_sites.rs`)
   - `parse`：Running/Starting/Stopping 或 parse_status=Running 时拒绝
   - `start`：禁止重复启动；解析中拒绝
   - `stop`：允许在有活动进程时执行
   - `delete`：任一进程活跃时拒绝
   - 前端 `site-status.ts` 同步对齐 `canStart/canStop/canParse/canDelete`

3. **前端按钮约束** (`SiteDataTable.vue`, `SiteDetailHeader.vue`)
   - Starting/解析中显示"停止"按钮
   - 各按钮 disabled 状态与后端一致

**验证方式**：通过 admin API HTTP 请求 + `scripts/test-admin-deployment.ps1`

### P1：Viewer URL 配置化（Phase 4 精简版）

**目标**：消除 `localhost:3101` 硬编码。

**改动范围**：
- `SiteDataTable.vue` / `SiteDetailView.vue` 中 Viewer URL 改为读取配置
- 约定 Viewer base URL 来源（环境变量 `VIEWER_BASE_URL` 或 admin 配置项）
- 保留 `backendPort/backend + output_project` query 协议不变

### P2：错误反馈与日志可追踪（Phase 5 精简版）

**目标**：关键动作失败后用户能看到原因。

**改动范围**：
- `sites.ts` store 增加动作级错误状态
- `SiteDetailView.vue` 增加 toast / banner 展示失败原因
- 后端 `admin_handlers.rs` 细化错误码（参数非法 / 状态冲突 / 端口冲突 / 启动失败）

## 实施顺序

```
P0 Phase 1+2 (安全+状态机)
    └─ 后端 managed_project_sites.rs
    └─ 前端 site-status.ts + SiteDrawer + SiteDataTable + SiteDetailHeader
    └─ HTTP 验证
P1 Phase 4 (Viewer URL)
    └─ 前端配置化
    └─ 后端配置项（如需要）
P2 Phase 5 (错误反馈)
    └─ store + 前端
    └─ 后端错误码
```

## 启动条件

当前 git 工作树有大量未提交改动（53 文件 / plant-model-gen, 28 文件 / plant3d-web），建议：
- 先对已完成的功能做一次 commit/tag 固化
- 然后在干净基线上实施上述计划
