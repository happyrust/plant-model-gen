# Feature Specification: PE/ATT 版本化存储（SurrealDB RocksDB versioned）

**Feature Branch**: `022-versioned-pe-att-storage`

**Created**: 2026-07-13

**Status**: Draft

**Input**: User description: "使用本地 SurrealDB fork（D:\work\plant-code\surrealdb, dev-3.1）的 RocksDB versioned 能力，为 PE 和 ATT 数据开启版本化存储，替代之前的版本存储方式。"

## 背景与决策记录（grill 结论）

本 spec 由 2026-07-13 的 grill 会话产出，七项关键决策如下（Q1~Q6 为超时自决采纳推荐项，Q7 为用户手动确认）：

| # | 决策点 | 结论 |
|---|--------|------|
| 1 | 启用范围 | SUL_DB（项目主库，RocksDB）实例级开启 `versioned=true`；模型高频写数据靠已有 MODEL_KV 分离机制隔离 |
| 2 | retention | 默认 **`0`（无限保留，全量历史）**，做成 DbOption 可配置项，透传到 `surreal start`；可按站点改为 `90d`/`30d` 等（磁盘风险需评估） |
| 3 | 版本锚点 | 新建 `sesno_version_anchor` 表，每次增量落库事务完成后固化 `dbnum + sesno → 时间戳`；历史查询按 sesno 入参、内部换算时间戳 |
| 4 | 删除语义 | 保持硬 DELETE，删除前状态由 versioned 存储层通过 `VERSION $t` 回答 |
| 5 | 与交付单元版本关系 | 正交：本 feature 只管 PE/ATT **源行**历史（细粒度、retention）。导出交付单元版本不在本 feature 范围 |
| 6 | 存量迁移 | 重新解析建库：新建 versioned 数据目录 + `sync_pdms` 全量重灌 + 写首条锚点；`DbOption.versioned_storage` 默认 **false**（versioned 是建库属性，存量目录以 versioned=true 打开会因 comparator 不匹配启动失败），新建站点经管理端/配置显式开启（T001 偏差记录 + T022，详见 ops-notes 决策 6） |
| 7 | 查询接口 | 仅新增 CLI 子命令（`model-version` 下的 history 类命令），暂不开 HTTP API；封装层放 rs-core `version_query` 模块 |

**已核实的技术前提**：

- fork 的 RocksDB 引擎已实现 user-defined timestamps 版本化：`rocksdb://<path>?versioned=true&retention=90d`（`RocksDbConfig.versioned/retention`，配置键 `datastore_versioned` / `datastore_retention`）
- SurrealQL `SELECT ... VERSION $t` 时间旅行查询已贯通到 kvs 层
- 能力边界：`retention=0` 无限保留；GC 以 60 秒粒度推进 `full_history_ts_low` 水位线；HLC 时间戳仅进程内单调；读时间戳低于水位线报 InvalidArgument
- `db-data/run_surrealkv_versioned.ps1` + `db-data/test_version_data.surql` 已验证 pe 表 VERSION 查询、删除历史、children 历史可行
- PE/ATT 固定写 `project_primary_db()`（SUL_DB）；模型数据在 `MODEL_KV_ENABLED` 时走独立 KV 实例

## User Scenarios & Testing *(mandatory)*

### User Story 1 - 按 sesno 查询元素历史快照 (Priority: P1)

设计审查人员发现某设备参数异常，需要查看该元素在 sesno=N 时刻的完整 PE 与属性状态，与当前状态对比定位是哪次设计会话改坏的。

**Why this priority**: 这是版本化存储的核心价值——不重扫 PDMS 源文件即可回答"过去长什么样"。

**Independent Test**: 在 versioned 实例上跑一次增量（sesno N→N+1，含属性修改），然后用 CLI 按 sesno=N 查询该元素，应返回修改前的属性值。

**Acceptance Scenarios**:

1. **Given** versioned 实例已有 sesno N 与 N+1 两个锚点且元素 E 在 N+1 被修改，**When** CLI 查询 `history snapshot --refno E --sesno N`，**Then** 返回 N 时刻的 PE + ATT 值（而非当前值）
2. **Given** 元素 E 在 sesno N+1 被硬删除，**When** 按 sesno=N 查询 E，**Then** 返回删除前的完整记录；按当前查询则返回不存在
3. **Given** 查询的 sesno 早于 retention 窗口（锚点时间戳低于 GC 水位线），**When** 执行查询，**Then** 返回明确的"历史已过期"错误提示（不 panic、不返回空结果冒充）

---

### User Story 2 - 增量落库自动固化 sesno 锚点 (Priority: P1)

运维执行 `incremental-sesno` 增量同步后，系统自动记录本次 sesno 对应的数据库时间戳，后续所有按 sesno 的历史查询都依赖该锚点。

**Why this priority**: 锚点是 sesno（业务版本）与 HLC 时间戳（存储版本）之间唯一的桥梁，没有锚点 VERSION 能力对业务不可用。

**Independent Test**: 跑一次增量落库，查询 `sesno_version_anchor` 表应有一条新记录，且其时间戳晚于本次全部 UPSERT/DELETE。

**Acceptance Scenarios**:

1. **Given** 一次增量落库（dbnum D，sesno 从 N 到 M）成功完成全部 PE/ATT 写入与删除，**When** 落库收尾，**Then** `sesno_version_anchor` 写入 `{dbnum: D, sesno: M, anchored_at: <时刻>}` 且该时刻晚于所有本批写入
2. **Given** 增量落库中途失败（部分 UPSERT 已执行），**When** 观察锚点表，**Then** 不产生 sesno=M 的锚点（避免半成品状态被当成一致快照）
3. **Given** 全量重灌（sync_pdms）完成，**When** 建库收尾，**Then** 写入该 dbnum 当前 sesno 的首条锚点

---

### User Story 3 - 按 sesno 区间做批量 diff (Priority: P2)

增量审查场景：给定 refno 集合与 sesno 区间 [N, M]，输出每个元素新旧两个快照的字段级差异，服务发布审查与 affected 证据生成。

**Why this priority**: 支撑现有 patch_only 发布链的 affected 证据从"元数据启发式提取"升级为"存储层可证 diff"，但不阻塞 P1 的基础能力。

**Independent Test**: 对已知修改过的 refno 集合执行 `history diff --from-sesno N --to-sesno M`，输出应仅包含实际变化的字段。

**Acceptance Scenarios**:

1. **Given** 元素 E 在区间内 BORE 从 150 改为 200 且其余字段不变，**When** 执行 diff，**Then** 输出仅含 BORE 一项变更（old=150, new=200）
2. **Given** 元素 F 在区间内被删除，**When** 执行 diff，**Then** 输出 F 标记为 deleted，附删除前快照

---

### User Story 4 - 存量站点切换到 versioned 实例 (Priority: P2)

管理员将既有站点切换到 versioned 存储：新建数据目录、全量重灌、验证、切流。

**Why this priority**: 没有迁移路径，能力只对新站点可用。

**Independent Test**: 对一个测试站点执行切换流程，切换后当前态查询结果与旧库一致，且增量链路照常工作。

**Acceptance Scenarios**:

1. **Given** 旧站点（非 versioned 数据目录），**When** 按切换流程新建 versioned 目录并 sync_pdms 重灌，**Then** PE/ATT 记录数与旧库一致，且首条锚点已写入
2. **Given** 切换完成后的站点，**When** 照常跑 incremental-sesno，**Then** 增量落库、锚点固化、模型生成全链路无回归

---

### Edge Cases

- **GC 水位线越界**：查询的锚点时间戳低于 `full_history_ts_low` 时，kvs 返回 InvalidArgument——CLI 必须捕获并翻译为"该 sesno 的历史已超出 retention=90d 窗口"
- **锚点缺失**：请求的 sesno 没有锚点（例如增量跳号、或早于 versioned 切换时刻）——按"最近不大于该 sesno 的锚点"回退并在输出中注明，找不到任何锚点则报错
- **同一 dbnum 并发增量**：锚点语义要求同一 dbnum 的增量落库串行（现状已满足，watch-incremental 单队列）；spec 要求在落库入口保持该约束
- **服务重启与 HLC**：HLC 仅进程内单调，重启后时间戳仍基于墙钟前进；锚点记录的是数据库侧 `time::now()`，不受客户端时钟影响
- **模型表未分离的站点**：未启用 MODEL_KV 的站点模型表也会被版本化，磁盘增长由 retention=90d 兜底；文档需注明建议此类站点评估磁盘余量或启用 KV 分离
- **retention 变更**：管理员调大/调小 retention 只影响 GC 水位线推进，不需要重建库

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: 系统 MUST 支持通过 DbOption 配置项开启 SUL_DB 的版本化存储，并在所有 `surreal start` 启动点（cli_modes 自启动、managed_project_sites、web_server、systemd/nohup 脚本模板）把 `?versioned=true&retention=<配置值>` 透传到连接串
- **FR-002**: retention MUST 默认为 `0`（无限保留，全量历史）且可按站点配置为有限窗口（如 `90d`/`30d`）；文档 MUST 警示 `retention=0` 下磁盘只增不减的风险
- **FR-003**: 系统 MUST 提供 `sesno_version_anchor` 表（字段：dbnum、sesno、anchored_at、来源标记 full/incremental），并在增量落库全部写入完成后、以及全量重灌完成后写入锚点
- **FR-004**: 增量落库失败时 MUST NOT 写入本次锚点
- **FR-005**: 历史查询 MUST 以 sesno 为业务入参，内部通过锚点表换算时间戳后发起 `VERSION` 查询；锚点缺失时按"最近不大于"回退并注明
- **FR-006**: 元素删除 MUST 保持硬 DELETE 语义，不引入软删除标记
- **FR-007**: 系统 MUST 提供 CLI 子命令（挂在 `model-version` 下）：`history snapshot`（单元素快照）、`history timeline`（元素跨锚点变更时间线）、`history diff`（refno 集合区间批量对比），全部支持 `--json` 输出
- **FR-008**: rs-core MUST 新增 `version_query` 封装模块（sesno→时间戳换算、VERSION 查询拼接、GC 越界错误翻译），plant-model-gen 侧只做 CLI 参数层
- **FR-009**: 存量站点切换 MUST 复用现有 sync_pdms 重灌链路，不新写一次性迁移工具；切换流程文档化到 quickstart
- **FR-010**: 现有 patch_only / 模型发布链 MUST 不受本 feature 影响（本 feature 不修改 register/reconcile 逻辑）。

### Key Entities

- **sesno_version_anchor**: 业务版本号与存储时间戳的映射锚点。`{ dbnum: u32, sesno: u32, anchored_at: datetime, source: "full" | "incremental" }`，主键 (dbnum, sesno)
- **PE / noun 表 / ATT_UDA**: 现有实体不变，历史版本由存储引擎透明保留，不新增字段（已注入的 sesno 字段保留，作为记录级冗余核对信息）

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: 对任意已有锚点的 sesno，单元素历史快照 CLI 查询在 2 秒内返回正确的 PE+ATT 值
- **SC-002**: 增量落库成功后锚点表 100% 有对应记录；落库失败后 0 条新锚点
- **SC-003**: 历史回溯场景不再需要重扫 PDMS 源 db 文件（对 retention 窗口内的 sesno）
- **SC-004**: 存量站点切换后，当前态查询结果与切换前一致（抽样 refno 集合逐字段比对通过），增量链路无回归
- **SC-005**: retention 窗口外的查询返回明确可读的过期错误，错误率 0（不出现 panic 或静默空结果）

## Assumptions

- 部署环境使用本 fork 构建的 surreal 二进制（官方发行版没有 RocksDB versioned 能力）
- 同一 dbnum 的增量落库保持串行（现有 watch-incremental 队列语义不变）
- PDMS 源 db 文件长期保留，作为 retention 窗口外历史的最终兜底
- 模型数据的版本化不在本 feature 范围内（依赖 MODEL_KV 分离；未分离站点接受模型表被一并版本化的磁盘代价）
- HTTP 历史查询 API 明确推迟（Q7 决策），未来需要时在 rs-core version_query 之上加 web_api 层即可
