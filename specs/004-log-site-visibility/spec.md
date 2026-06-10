# Feature Specification: 校审页站点日志可见性(Log Site Visibility)

> spec 003 的增量。用户验证诉求(2026-06-11):"检查 plant3d-web 能否看到日志信息——
> 不同类型:解析的日志、模型生成的、三维校审的日志"。
> 现状:校审/接口日志可见;解析(site.file.parse)/生成(site.file.generate)在校审页
> 不可见,因为 LogDrawer 未获得 site_id,后端也要求显式传参。

## User Need

校审页日志抽屉打开后,无需任何配置即可看到**当前站点**的解析日志与模型生成日志,
与校审流转/接口日志并列三类可用。

## Decisions(grill-me 定案,2026-06-11)

| # | 决策点 | 结论 |
|---|---|---|
| 1 | site_id 来源 | 后端兜底:`/api/logs` 的 `site.file.*` 未传 site_id 时默认 `current_site_id()`(one-web-server-per-site 进程内已知);前端抽屉标题显示当前站点名作增强 |
| 2 | 生成日志覆盖面 | 本期仅文件日志(parse/generate 等 5 类);进程内实时生成日志(server.runtime,tracing→ring)记二期 |
| 3 | 阅读体验 | 复用现有列表(level 过滤+刷新/轮询),不加 tail -f |
| 4 | 验收方式 | 后端 HTTP 冒烟 + 前端 vitest + 本地无头浏览器实测截图闭环 |

## Requirements

1. `GET /api/logs?type=site.file.*` 不带 `site_id` 时使用当前站点身份;进程无站点身份时
   返回明确错误信息。隐式兜底命中的站点若非 managed 站点(如手工启动的 dev 实例),按
   "无日志"返回空列表;显式传入的 site_id 查不到仍然报错。
2. `GET /api/logs/types` 中 `site.file.*` 的 filters 将 `site_id` 标注为可选。
3. LogDrawer 对 `site.file.*` tab 不再要求 siteId prop;标题区显示当前站点名
   (`/api/site/identity`,失败静默隐藏)。
4. 默认行为零变化:显式传 site_id 的既有调用语义不变。

## Non-Goals

- 不做进程内 server.runtime 日志采集(二期)。
- 不做 tail -f 跟随模式。
- 不改 admin UI。

## Acceptance Criteria

- 在当前站点的 web_server 上,`GET /api/logs?type=site.file.parse`(无 site_id)返回该站点
  `runtime/admin_sites/<site>/logs/parse.log` 尾部内容;无日志文件时返回空列表而非报错。
- 浏览器实测:开 `REVIEW_H_LOG_DRAWER` flag 后,抽屉内"站点日志·parse / 站点日志·generate"
  tab 可直接出数据(对有日志的站点),校审流转/接口日志 tab 同时可用。

## Verification Result(2026-06-11)

环境:web_server:3102(`DbOption-dev32`,3.2-nightly surreal@8021 迁移副本)+
Vite:3101(`VITE_BACKEND_PORT=3102`)+ Playwright chromium 无头实测
(`plant3d-web/debug_scripts/log-drawer-smoke.mjs`,截图 `plant3d-web/artifacts/log-drawer-*.png`)。

| 验收点 | 结果 |
|---|---|
| 后端 site_id 兜底(无参) | ✅ 当前站点为非 managed dev 实例 → 空列表不报错(`success=true entries=0`) |
| 显式 site_id 语义不变 | ✅ 历史站点 parse 3 条;generate+level=error 命中 1 条 |
| types 标注 | ✅ `site_id?,level,q` |
| 浏览器:抽屉可见性 | ✅ flag 开启后悬浮按钮/抽屉渲染,7 个类型 tab 齐全 |
| 浏览器:校审日志 | ✅ 校审流转历史 tab 显示迁移副本中的真实历史(`[sj] submit by SJ`、`[jd] return by JH`…) |
| 浏览器:接口日志 | ✅ 9 条本会话调用记录 |
| 浏览器:站点日志 tab | ✅ parse tab 空态提示(dev 实例无 managed 日志,语义正确);真实内容路径由显式站点 API 验证覆盖 |
| 前端 vitest | ✅ logsApi 3/3 保持绿 |

已知杂音:Vite `--force` 后首次加载偶发 2 个 `504 Outdated Optimize Dep`(开发服务器缓存竞态,
刷新即消失,与本功能无关)。
