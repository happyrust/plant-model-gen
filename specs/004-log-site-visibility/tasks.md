# Tasks

- [x] **T401 后端 site_id 兜底(已完成)**
  - `logs_api.rs::query_site_file_logs`:site_id 缺省时取 `web_listen::current_site_id()`;
    无站点身份时报明确错误;隐式兜底命中非 managed 站点 → 空列表(显式传参仍报错)。
  - `/api/logs/types` 中 site.file.* 的 filters 标注 `site_id?`。
- [x] **T402 前端抽屉适配(已完成)**
  - LogDrawer:移除缺 siteId 告警;打开时调 `getCurrentSiteIdentity()` 在标题区显示站点名
    (失败静默)。
- [x] **T403 验证(已完成,2026-06-11)**
  - HTTP 冒烟:无 site_id → 当前站点空列表;显式历史站点 parse=3 条 / generate+error=1 条。
  - vitest:logsApi 3/3 绿。
  - Playwright 无头实测(debug_scripts/log-drawer-smoke.mjs):抽屉 7 tab 渲染,
    校审流转历史显示真实历史记录,接口日志 9 条,parse tab 空态正确;截图 artifacts/。
- [x] **T404 回填 spec 验收结果(已完成)**
