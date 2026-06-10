# Implementation Plan

## Approach

最小增量:后端在 site.file adapter 入口做 site_id 兜底(进程内 `current_site_id()`),
前端仅移除告警并显示站点名。不改查询契约、不动 003 已验收行为。

## Files

- `src/web_api/logs_api.rs`
  - `query_site_file_logs`:`query.site_id` 缺省 → `crate::web_server::web_listen::current_site_id()`,
    取不到时 bail 出明确错误;类型目录 filters 改为 `site_id?`。
- `../plant3d-web/src/components/review/LogDrawer.vue`
  - 移除"站点日志需要 site_id"告警分支;挂载时 `getCurrentSiteIdentity()` 显示站点名(失败静默)。

## Validation

- `cargo check`;重启 3102 dev 实例 HTTP 冒烟(无 site_id / 显式 site_id 双路径)。
- `pnpm vitest run src/api/logsApi.test.ts`。
- agent-browser:Vite(VITE_BACKEND_PORT=3102)+ localStorage 开 flag → 抽屉三 tab 实测截图。
