# Research: SurrealDB 站点数据目录隔离

## Decision: 数据目录按站点名称 slug 命名

**Rationale**: 用户在管理端、日志和运行目录中以站点为部署单位识别测试案例。`project_name` 可能与 `site_name` 不一致，且输出目录已经使用项目名；数据库目录使用站点名称可以避免两个概念混在一起。

**Alternatives considered**:

- 使用 `project_name`: 与输出目录一致，但当一个项目创建多个站点或站点名用于区分测试案例时不够清晰。
- 使用 `site_id` only: 已经体现在站点根目录，但子目录 `data/surreal.db` 仍不可读。
- 使用 dbnum: 对单 DB 快测清晰，但多 DB 或全量站点无法表达完整归属。

## Decision: quick deploy/API 默认不自动解析依赖 DB

**Rationale**: quick deploy 的主要价值是快速验证一个目标 DB。默认纳入依赖 DB 会增加解析时间，并让测试范围从单目标变成多库闭包，容易影响排查。

**Alternatives considered**:

- 默认继续自动解析依赖 DB: 更接近完整部署，但慢且容易混入无关变量。
- 总是禁用依赖解析: 破坏需要完整依赖的部署场景。
- 使用显式开关: 默认快速，显式 `auto_parse_related_dbnums=true` 时保留完整能力。

## Decision: 不自动迁移历史数据库目录

**Rationale**: RocksDB 目录可能正在被进程持有，且包含历史站点状态。静默迁移有数据损坏和锁冲突风险。

**Alternatives considered**:

- 自动移动旧目录: 用户体验简单，但风险高。
- 复制旧目录: 会产生双份状态，增加误用概率。
- 新规则只对新建/重建站点生效: 最小风险，适合当前修复范围。

## Decision: file/ws 模式共享同一个 db_data_path

**Rationale**: 现有互斥机制以 RocksDB 数据目录作为真源。继续共享路径可以保证离线解析、生成和运行时看到同一份数据，也能复用已有锁保护。

**Alternatives considered**:

- file/ws 使用不同目录: 可避免锁冲突，但会产生数据同步问题。
- 仅 ws 使用项目目录: 解析生成仍可能污染共享目录。
- 所有模式统一项目目录: 行为一致，便于验证。
