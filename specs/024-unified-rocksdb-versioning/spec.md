# Feature Specification: 版本化统一（RocksDB versioned 单一真相源）

**Feature Branch**: `024-unified-rocksdb-versioning`

**Created**: 2026-07-20

**Status**: Draft

**Input**: 用户决策："让数据版本和模型版本都统一使用目前的 rocksdb versioned 方案，清理以前的旧实现，重构代码。"

**Upstream**: `docs/adr/0001-unified-rocksdb-versioning.md`（决策记录）、根 `CONTEXT.md`（术语）、specs/022（PE/ATT versioned，保留并扩展）、specs/023（DuckLake 交付单元版本，**被本 spec 退役**）

## 背景与决策记录（2026-07-20 grill 会话，D1–D10 全部经用户逐项确认）

| # | 决策点 | 结论 |
|---|--------|------|
| D1 | 统一边界 | 彻底统一：所有版本诉求由 RocksDB versioned + 锚点回答；023 DuckLake 交付版本链退役 |
| D2 | 023 消费面 | HTTP API（model_version_api）/ 离线部署发布链 / 发布类 CLI / 对账状态机全退役；交付改为按锚点导出（新 `model-version export`）；source observation 哈希门禁内联保留，manifest 归档链删除 |
| D3 | 锚点身份 | 同表扩展：`sesno_version_anchor:[dbnum, sesno, source]`，source ∈ {full, incremental, model_gen}；model_gen 在生成管线全部下游成功后写，失败不写，同 sesno 重跑 UPSERT 覆盖 |
| D4 | 旧增量路径 | notify watcher（init_watcher / execute_incr_update / exec_watcher）整体退役，watch-incremental 成为唯一增量入口；MQTT 文件分发拆出独立保留 |
| D5 | retention | `0`（无限保留）为默认并作为退役 DuckLake 的 ADR 前提；有限窗口仅磁盘受限站点手工例外 + 文档警示；窗口外唯一兜底 = 源文件重解析 |
| D6 | 查询入口 | CLI only；rs-core `version_query` 扩 `model_snapshot_at` / `model_diff`；前置门禁 = range record-id + VERSION 冒烟验证 |
| D7 | 错误吞噬修复 | 纳入且为第一任务：`exec_statements` / 锚点写入 / `delete_increment_element` / pre_cleanup chunk 全面 `.check()` 化 |
| D8 | 迁移 | 硬切换零兼容层：versioned 重建边界 = 新 record id 形状边界；存量 versioned 测试站配一次性清库脚本；删 legacy 双读 |
| D9 | DuckLake 代码边界 | 交付链 + ModelWriter DuckLake/Parquet 写入后端全退（ModelWriterMode 收敛 Surreal/DrainOnly，TransformWriteBackend 去 DuckLake）；web 查看器 Parquet 导出器保留 |
| D10 | 收尾 | 增量入口 per-project 文件锁；element_changes / --target-sesno 死路径删除；文档三件套（本 specs + CONTEXT.md + ADR-0001） |

**已核实的技术前提**：

- 模型表与 PE/ATT 同库（SurrealKV/MODEL_KV 分离已移除），实例级 versioned 已同时覆盖两者——模型历史在存储层已存在，缺的是锚点语义与查询入口
- 模型 record id 的 sesno 槽位实际恒为 0（生成管线的 RefnoEnum 不携带 sesno；`refno_with_sesno` / `model_refno_id_with_sesno` 无调用者），删除无语义损失
- 022 已验证 pe 表点查 VERSION；range record-id 扫描 + VERSION 组合**未验证**（P5 门禁）
- 现存语句级错误吞噬：`query().await?` 不带 `.check()` 时 SurrealDB 语句失败不传播，威胁"有锚点 = 一致快照"不变量

## User Scenarios & Testing *(mandatory)*

### User Story 1 - 按 sesno 查询历史模型 (Priority: P1)

审查人员发现某设备当前模型异常，需要查看该元素在 sesno=N 时的模型（几何实例与关系）与当前对比，定位是哪次设计变更或哪次重生成引入的。

**Why this priority**: 这是"模型版本统一到 versioned"的核心新增能力；数据历史 022 已交付。

**Independent Test**: 对同一 refno 跑两次生成（中间修改属性触发增量），`model-version history model-snapshot --refno E --sesno N` 应返回旧一代模型行。

**Acceptance Scenarios**:

1. **Given** sesno N、M 各有 model_gen 锚点且元素 E 的模型在 M 代变化，**When** 按 sesno=N 查询 model-snapshot，**Then** 返回 N 代的 inst_relate/geo_relate 行而非当前
2. **Given** 元素 E 在 M 代被删除（模型产物被清理），**When** 按 sesno=N 查询，**Then** 返回删除前完整模型行
3. **Given** 查询的 sesno 无 model_gen 锚点，**When** 执行查询，**Then** 按"最近不大于"回退并在输出注明实际锚点

---

### User Story 2 - 生成完成自动固化模型锚点 (Priority: P1)

增量或重生成完成后，系统自动写 model_gen 锚点；生成中途失败不写，此时按上一个 model_gen 锚点仍能读到完整的上一代模型。

**Why this priority**: 锚点是模型历史业务可用的前提；"regen 失败可回看上一代"是当前完全不存在的恢复能力。

**Independent Test**: 跑一次 `--generate-model` 增量，锚点表出现同 sesno 的 incremental 与 model_gen 两条且 model_gen 时间戳更晚；人为让生成失败（如断开 mesh 阶段），无新 model_gen 锚点。

**Acceptance Scenarios**:

1. **Given** 增量落库+生成全部成功，**When** 收尾，**Then** `sesno_version_anchor:[dbnum, sesno, 'model_gen']` 存在且晚于本批全部模型写入
2. **Given** 生成中途失败（清理已执行、写入未完成），**When** 查询上一 model_gen 锚点，**Then** 读到完整上一代模型
3. **Given** 同 sesno 重跑 regen 成功，**When** 观察锚点，**Then** 同一条 model_gen 锚点时间戳被覆盖更新

---

### User Story 3 - 按锚点导出交付 (Priority: P2)

交付人员不再维护发布目录与 release 对账，需要某站点某 sesno 的交付文件时，直接 `model-version export --dbnum D --sesno N` 一次性导出。

**Why this priority**: 替代退役的 023 交付链的最小能力。

**Acceptance Scenarios**:

1. **Given** sesno N 有 model_gen 锚点，**When** 执行 export，**Then** 产出该代模型的导出物，输出中记录使用的锚点坐标
2. **Given** 请求的 sesno 无锚点且无可回退锚点，**When** 执行 export，**Then** 明确报错（不静默导出当前态冒充）

---

### User Story 4 - 单一增量入口与并发保护 (Priority: P2)

运维只需要维护 watch-incremental 一个常驻入口；误操作并发跑第二个增量进程会被文件锁拒绝，而不是产生交错写入与错误锚点。

**Acceptance Scenarios**:

1. **Given** watch-incremental 正在运行，**When** 手动再跑 incremental-sesno 同项目，**Then** 获取锁失败、明确报错退出，锚点表无交错记录
2. **Given** 旧 notify watcher 代码路径已删除，**When** 全仓搜索，**Then** 无 init_watcher/execute_incr_update 符号，sync_live 配置走 watch 轮询语义

---

### Edge Cases

- **model_gen 锚点的 sesno 归属**：生成由调用方按"该 dbnum 本次已落库的 sesno"写锚点（增量=actual_end_sesno；全量/regen=dbnum_info 当前 sesno）；生成范围不含某 dbnum 时不写该 dbnum 锚点
- **数据锚点在、模型锚点缺**（persist-only 增量或生成失败）：模型历史查询回退到上一 model_gen 锚点——正确反映模型库仍是旧一代
- **range VERSION 冒烟不过**：D6 方案回炉重议（阻塞 P5，不阻塞 P0–P4）
- **锚点表旧行**：id 形状从 [dbnum, sesno] 扩为 [dbnum, sesno, source]，存量 versioned 测试站由清库脚本重建锚点表（硬切换边界内）
- **有限 retention 站点**：窗口外 model-snapshot 报 HistoryExpired（复用 022 错误翻译），提示唯一兜底为源文件重解析

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: 增量落库与锚点写入路径 MUST 传播 SurrealDB 语句级错误（`.check()` 或等价），部分写入失败 MUST NOT 产生锚点
- **FR-002**: `sesno_version_anchor` 主键 MUST 为 `(dbnum, sesno, source)`，source MUST 支持 full/incremental/model_gen 三值
- **FR-003**: 模型生成成功收尾 MUST 为每个参与 dbnum 写 model_gen 锚点；任何下游阶段失败 MUST NOT 写
- **FR-004**: 模型 record id MUST 收敛为纯 refno 键（无 sesno 槽位）；全仓 MUST 无 `[ref0, ref1, sesno]` 形状构造与 legacy 双读
- **FR-005**: notify watcher 增量路径、element_changes 读取路径、`--target-sesno` 参数链 MUST 删除；watch-incremental MUST 为唯一增量入口
- **FR-006**: 增量落库入口 MUST 有 per-project 文件锁（含持有者 pid），获取失败拒绝启动
- **FR-007**: 023 交付链（version_management 发布模块群、model_version_api、发布类 CLI、DuckLake/Parquet ModelWriter 后端、ducklake_parity）MUST 退役；被存活路径引用的模块（source_observation 哈希门禁、set_status、update_log 等）MUST 保留
- **FR-008**: MUST 提供 `model-version export --dbnum --sesno`（按锚点导出）与 `model-version history model-snapshot / model-diff`（CLI，--json），封装层在 rs-core version_query
- **FR-009**: retention 默认 MUST 为 "0"；文档 MUST 警示有限窗口的历史丢弃语义
- **FR-010**: web 查看器 Parquet 导出器与 pe_transform 的 Parquet 后端 MUST 不受影响

### Key Entities

- **sesno_version_anchor**: `{ id: [dbnum, sesno, source], dbnum, sesno, source: 'full'|'incremental'|'model_gen', anchored_at, note? }`
- **模型 record id（新形状）**: 点表 `[ref0, ref1]`；geo_relate `[ref0, ref1, geo_index(, inst_hash)]`；tubi_relate `[ref0, ref1, tubi_index]`；neg/ngmr `[t0, t1, c0, c1, geo_index, rel_index]`

## Success Criteria *(mandatory)*

- **SC-001**: 一次 `--generate-model` 增量后，锚点表同 sesno 出现 incremental 与 model_gen 两条，时序正确（model_gen 最晚）
- **SC-002**: 人为注入语句级错误时，锚点零写入且进程报错退出（FR-001 验证）
- **SC-003**: model-snapshot 对任意有锚点 sesno 在 2s 内返回正确历史模型行；regen 失败窗口内按上一锚点读到完整上一代
- **SC-004**: 全仓无 sesno 槽位 id 构造、无 023 发布链符号、无 notify watcher 符号；`cargo check` 全绿
- **SC-005**: 并发第二个增量进程被文件锁拒绝，锚点表无交错
- **SC-006**: 按锚点导出产物与 regen 前基线导出逐行数一致（迁移无损）

## Assumptions

- fork 的 VERSION 查询支持 array record id range 扫描（P5 门禁验证；不过则 D6 回炉，不影响 P0–P4 交付）
- `model_version_api` 无外部平台消费方（用户已确认）
- 部署站点接受"升级 = versioned 重建/清库重生成"的硬切换边界
- rs-core（dev-3.1）可配合发版：version_query 扩展 + `rs_surreal/inst.rs` legacy tubi 查询修正
