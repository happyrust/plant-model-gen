# 更新日志

## [未发布]

### 变更

- **校审日志体系 specs/003~005 — 采集、统一查询与站点可见性** (2026-06-11)
  - 接口日志采集(spec 003):review/platform 域路由新增最外层中间件,记录
    request/response(4KB 截断 + truncated 标记)、状态码、耗时、request_id,并从
    query/path/body 提取 form_id/task_id;丢弃请求头,token/password/secret 字段打码;
    异步写 SurrealDB `api_request_log`(迁移 `20260610_002`),失败仅告警不影响业务;
    保留 7 天,惰性启动每小时清理
  - 统一日志查询 API(spec 003):`GET /api/logs/types`(按 JWT Role 裁剪)+
    `GET /api/logs?type=...`,adapter 适配四类数据源:`api.request` /
    `review.workflow`(复用 review_workflow_history 表)/ `server.runtime` /
    `site.file.{parse,generate,db,web,viewer}`(复用站点文件日志 tail);统一
    LogEntry 与游标分页;挂 review JWT,接口/站点/进程日志仅管理角色可见
  - 站点日志可见性(spec 004):`site.file.*` 未传 site_id 时默认当前站点
    (`current_site_id()`);隐式兜底命中非 managed 站点降级为空列表,显式传参语义不变
  - 进程内运行日志(spec 005):web_server 以 TeeLogger 包装 env_logger,info+ 写入
    5000 条环形缓冲(单条 2KB 截断),新类型 `server.runtime` 支持 level/q/时间窗/游标;
    控制台输出与过滤行为不变
  - 验收:隔离环境 HTTP 冒烟 + 真实站点回归 + Playwright 浏览器实测全过,
    详见 `specs/003-review-log-viewer` / `004-log-site-visibility` / `005-server-runtime-logs`

- **SurrealDB 客户端升级 3.1.0-alpha → 3.2.0-nightly** (2026-06-10)
  - 全量 `cargo update`:surrealdb 系列指向 fork dev-3.1 `#f01470af`
    (内部成员 3.2.0-nightly),RocksDB 10.6→11.0,revision 0.17→0.28,共 200+ 包刷新
  - ⚠️ 与 3.1.0-alpha server **WS 协议不兼容**(`Failed to decode value from fb value`),
    客户端/服务端必须同步升级;旧 rocksdb 数据可被 3.2 直接打开(956 万条 pe 副本实测),
    但打开即单向升级格式,升级前必须冷备份
  - Windows debug 版 surreal server 需 `/STACK:16777216` 重链接(主线程栈溢出),
    生产使用 release 构建
  - 生产切换步骤见 `docs/plans/2026-06-11-surrealdb-32-upgrade-runbook.md`

- **review 域 SurrealDB 上下文韧性(spec 002)** (2026-06-10)
  - review 主库连接改为进程级 OnceCell + 按调用克隆会话,移除每请求 `USE NS/DB` 切换;
    `review.ensure_context` 不再作为请求级超时点
  - WebSocket 模式下进程级启动一次 `aios_core` 心跳保活
  - review schema 迁移链落地(`review_schema_migrations` 表 + `rs_surreal/review/migrations/`)

- **CATA 按需闭包 manifest 生成 CLI 入口(T006c,feat/on-demand-cata-closure 分支)** (2026-06-10)
  - 新增子命令 `gen-cata-closure`(`--rescan-index` / `--out`):db_index.sqlite 缺失自动
    全量预扫 → 扫描工程根全部 DESI → refno 级引用闭包 → 原子写
    `output/<project>/scene_tree/cata_closure.json`;配合 `AIOS_CATA_CLOSURE_MODE=manifest`
    实现 CATA 部分解析
  - `db_index.rs` 抽出配置/工程根派生公共函数;index 落盘路径与 scene_tree 产物目录对齐
