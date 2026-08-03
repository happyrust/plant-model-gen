---
status: proposed
---

# 用 rkyv 种子装载初始化模型生成的 kv-mem 层级投影

## Context

初始化模型生成需要大量元素元数据过滤、祖先链、子节点和后代遍历。现有实现同时维护持久 SurrealDB、`HierarchySnapshot`、`HierView`、`PeDbnumSnapshot` 和 query provider，多套层级语义增加内存、装载时间与结果漂移风险。

持久 SurrealDB 始终是唯一真相源。rkyv 只负责把初始化解析已经验证的单个 dbnum 当前态投影序列化到磁盘；初始化模型生成再把需要的 dbnum 投影装入一个临时 SurrealDB kv-mem 站点，复用 SurrealQL 和图查询。该缓存没有独立业务版本，生成结束后整体释放。

## Decision

### 范围与启用边界

- 第一阶段只服务初始化全量模型生成。增量生成、欠账追赶、历史版本生成、受控修复和 HTTP 查询保持原路径。
- 新增显式 `gen_all_geos_data_for_initialization` 入口，仅初始化主流程调用它。禁止根据空 refno、live 读取或配置组合猜测初始化身份。
- `DbOptionExt` 增加 `generation_cache_mode = off | shadow | on`，默认 `off`。
- `off` 完全使用持久库；`shadow` 同时查询 kv-mem 与持久库、只消费持久库结果；`on` 优先消费 kv-mem，缓存不可用时整次查询回退持久库。

### pe_owner 是唯一层级来源

- 持久库和 kv-mem 的 `pe` 都不保存或读取 `children`。`pe` 只保留 `owner` 与表达源元素是否有子节点的 `child_count`。
- children 和 descendants 统一查询 `pe_owner`；ancestors 统一沿 `pe.owner` 查询。删除所有 `pe.children` 兼容回退。
- `pe_owner` 固定为 `in = child`、`out = owner`、`id = [owner, order]`。根节点的 `owner` 规范化为自身，但不创建自环边。
- `pe_owner_version_meta` 保留 `maintained_since_sesno` 表达 pe_owner 从哪个水位起持续由正式写入链维护，并增加 `bulk_state = NotReady | Ready`、`verified_sesno`、`node_count`、`edge_count` 和规范化 `hierarchy_hash`。后五项只表达最近一次 full/rebuild 全量审计证据。
- 初始化解析以每个已物化 PE 的 `owner` 决定边成员，`children_map` 只提供同胞顺序提示；缺失顺序按 refno 稳定补齐。worker 完成后串行固化 `owner`、`child_count` 和 `pe_owner`，再从持久数据回读计算实际哈希；节点数、边数或哈希不一致，以及 `pe_owner_version_meta` 发布失败，都使该 dbnum 初始化解析失败并阻止模型生成。
- rkyv 种子发布失败只影响缓存，可回退到已经验证的持久 `pe_owner`。
- 旧数据缺少完整 `pe_owner` 或可信元数据时必须先执行现有 rebuild CLI；rebuild 以 `pe.owner` 反推成员关系（已有边保留其 ordinal，新增孩子按 refno 补位，与种子构建器同一约定），彻底不再读 `pe.children`，也不能静默回退旧 `pe.children`。

### 种子发布协议

- full 写库或 rebuild 开始前，必须先把该 dbnum 的 `pe_owner_version_meta` 和 `pe_graph_seed_meta` 都标为 `NotReady`；任一步失败都禁止开始修改 PE/pe_owner。
- 解析期间种子只保留在内存；等待 PE worker 完成、串行固化 pe_owner 并通过上述完整性校验后，才写入和发布最终文件。
- PE/pe_owner 写入和规范化完整性哈希通过后，先发布 `pe_owner_version_meta = Ready`；该步骤失败时初始化解析失败并保持种子不可用。
- 最终文件不可变，文件名包含 dbnum、sesno，以及 scope hash 与 payload SHA-256 的 16 位 hex 前缀（避免深部署目录撞 Windows MAX_PATH）；完整哈希存入 `pe_graph_seed_meta`，加载器只认元数据记录的精确文件名。
- `pe_owner_version_meta` 已 Ready 后再原子发布种子文件，最后把 `pe_graph_seed_meta` 更新为 `Ready`，记录 sesno、full scope hash、精确文件名、payload SHA-256、节点数和边数。
- 加载器只接受 `Ready` 记录并打开其精确文件，禁止扫描目录猜测候选。崩溃时已落盘但没有 Ready 记录的文件是不可见孤儿。
- 任意时刻崩溃的最坏结果是缓存不可用或初始化生成因 pe_owner NotReady 而明确失败，不能读取半写层级。
- partial、closure 或增量写入只把对应 dbnum 的 `pe_graph_seed_meta` 标为 `NotReady`，不发布替代种子；partial/closure 裁剪解析额外按本次写入集 scoped 固化对应 parent 的 `pe_owner` 边与 `child_count`（待调解 parent = 写入集中的 parent ∪ 写入集节点在库内存在的 owner，成员取本次文件 `children_map` ∩ 库内存在行），但不发 `Ready`、不切换 `bulk_state`，以保证裁剪解析后 latest 层级查询可用、审计门控消费方继续 fail-closed。
- 普通增量不切换 `pe_owner_version_meta.bulk_state`，也不全库重算 hierarchy hash；旧审计记录保留其原 `verified_sesno`，不能冒充当前水位证据。
- `db_meta_info.json` 仍是 ref0 到 dbnum 的唯一 locator；种子只保存 ref0 集合摘要用于校验，不复制 locator。

### rkyv 文件格式

- `Cargo.toml` 精确锁定 `rkyv = 0.8.17`，显式固定 `aligned + little_endian + pointer_width_64`，并保留默认 bytecheck。
- schema 或格式特征变化必须提升种子 `format_version`；旧种子按未命中回退。
- 文件使用 64 字节固定前缀，保存 magic、格式版本、payload 长度和 payload SHA-256。
- rkyv payload 保存 dbnum、sesno、源文件名、full scope hash、ref0 set hash、node count、edge count，以及按 refno 排序的节点。
- 节点保存 refno、owner、同胞 order、noun、name、字符串语义的 cata_hash 和 child_count，不保存 children。
- 第一阶段只生成和读取 full 初始化种子；将来确需 partial 缓存时提升 `format_version` 并单独设计查询覆盖证明。
- 读取在 `spawn_blocking` 中完成：读入 aligned buffer，依次校验前缀、长度、SHA-256、rkyv archived access 和业务凭据，不把整个 payload 反序列化成原生 `Vec<Node>`。
- 每批最多把 500 个 archived 节点转换为 SurrealDB typed values，禁止拼接未转义 SQL。分片进入 `Ready` 后释放批次临时值和文件 buffer；第一阶段不使用 mmap。

### kv-mem 投影与查询语义

- 每次初始化模型生成只创建一个 kv-mem 站点。
- kv-mem 使用最小严格 schema：`pe SCHEMAFULL`；`pe_owner TYPE RELATION IN pe OUT pe ENFORCED`；仅建立 `(dbnum, noun)` 索引。
- kv-mem `pe` 只保存 dbnum、owner、noun、name、字符串形式的 cata_hash 和 child_count。
- 装载固定先写全部 `pe`，再写 `pe_owner`，最后校验节点数、边数和 ref0 集合摘要。
- shadow 比较必须保留现有 API 契约：children 按边 order；descendants 按输入根顺序、BFS 和同胞顺序；ancestors 保留各入口既有方向；点查和批量元数据按 refno 逐字段相等。
- 无分页枚举按集合比较；带 limit/分页的枚举增加稳定 `ORDER BY id` 后按序列比较。
- 任一 shadow 差异都令 `shadow_compatible = false`，禁止切换到 `on`。

### dbnum 分片、LRU 与回退

- 一个 ref0 只属于一个 dbnum，一个 dbnum 可包含多个 ref0。定位后，缓存身份、装载、pin 和淘汰单位始终是完整 dbnum。
- 每个非根节点在发布和装载时都验证 child 与 owner 属于同一 dbnum；缺失或跨 dbnum 使该分片不可用。full 发布时若检测到悬空 owner（owner 不在本 dbnum 物化集内），持久 `pe_owner` 照常审计并发布 `Ready`，但跳过种子文件发布（`pe_graph_seed_meta` 保持 `NotReady`，记录悬空计数与样例），kv-mem 对该 dbnum 自然回退持久库。
- 查询或高层批次开始前解析全部 dbnum 并取得 lease。任一分片不可用时，整次查询回退持久库，禁止一次遍历混合两个数据源。
- dbnum 装载使用 singleflight，状态为 `Loading → Ready → Evicted`；只有 `Ready + unpinned` 分片可被 LRU 淘汰。
- 单个 kv-mem 站点同一时刻只允许一个 dbnum 分片执行装载写入；Ready 分片仍可并发查询。
- `pe` 和 `pe_owner` 每个 INSERT 最多 500 行，不新增批量大小配置。
- 站点配置 `generation_cache_max_mb`，默认 1024 MiB，仅在 shadow/on 生效。
- admission 估算为 `(rkyv_payload_bytes + node_count × 384 + edge_count × 192) × 1.25` 并向上取整。它是缓存记账上限，不是进程 RSS 硬限制；估算超限且无分片可淘汰时禁止 overcommit。
- shadow 记录装载前后进程内存、估算值、耗时、淘汰与回退，只用于校准，不自动放宽预算。
- 淘汰按 dbnum 删除 `pe`，依赖 SurrealDB 在端点删除时级联清理 `pe_owner`；确认该 dbnum 的 PE 行数为零后才释放预算。
- 种子缺失、损坏或凭据失配时，只在 `pe_owner_version_meta.bulk_state = Ready`、`verified_sesno` 等于当前初始化 sesno，且节点数、边数、hierarchy hash、端点存在性、单 owner 和同胞 order 唯一性审计全部通过后，才从持久 PE/pe_owner 重建 kv-mem 分片。
- 持久库重建不写回种子。若持久 `pe_owner` 审计失败，初始化模型生成失败；不能再回退 `pe.children`。

### 观测

- 复用现有 `CacheMissReport` 和 `cache_miss_report.json`，不新增报告文件。
- 报告增加可选 `generation_cache` 汇总，记录命中、回退、淘汰、shadow 差异数量、少量样例和 `shadow_compatible`。
- 详细内存变化与装载耗时只写结构化日志。
- 报告写入失败只告警，不影响权威模型生成，但该运行不能作为从 shadow 切换到 on 的验收证据。

### 文件治理

- 种子目录固定为 `output/<project>/scene_tree/pe_graph/`，与 `db_meta_info.json` 同属解析产物，不放入可单独改址的 model cache。
- 新 Ready 元数据提交成功后，只保留它指向的当前 dbnum 文件；旧内容哈希文件、不可见孤儿和 `.tmp` 文件做 best-effort 删除。
- 崩溃孤儿在该 dbnum 下次成功发布时清理。不新增启动扫描、后台清理线程或保留期限配置。

### 实施与验收

1. 第一切片只完成 rkyv 0.8.17 协议、PE/pe_owner 完整性、解析期发布和元数据绑定；缓存保持 off。
2. 第二切片完成 kv-mem、dbnum LRU、查询适配和初始化专用入口，只开放 shadow；模型仍消费持久库结果。
3. 第三切片在 shadow 全部一致后开放 on；验证完成后删除 `HierarchySnapshot`、`HierView`、`PeDbnumSnapshot` 和旧 query provider，并把本 ADR 标为 accepted。

每个切片都不新增或运行 cargo test，也不增加缓存专用调试 CLI。验证使用实际 CLI + JSON 与完整初始化模型生成：

- 在同一真实 multi-dbnum 数据和相同 generation contract 下执行 `off → shadow → on`。
- 三次 provenance、生成 refno 集合、模型/实例数量、`GenerationArtifactsSummary.geometry_artifact_hash` 和 `model_semantic_hash` 必须相同。
- shadow 必须查询差异为零、`shadow_compatible = true` 且报告成功落盘。
- 将预算降到只能容纳一个 dbnum，确认发生 LRU 淘汰与重载且产物不变。
- 分别制造 `NotReady` 和损坏种子，确认完整回退持久库且产物不变。
- 不比较可能含时间戳或不稳定排列的原始文件字节。

## Consequences

- 初始化模型生成只维护一套 SurrealQL/图查询语义，减少重复 Rust 层级快照和全量内存复制。
- PE/pe_owner 完整性从缓存优化的前置条件升级为初始化模型正确性的硬门槛。
- rkyv 和 kv-mem 的任意失败都不会改变权威数据；只要持久 pe_owner 已验证，生成可安全回退。
- 初次装载增加 rkyv 校验和 kv-mem 灌入成本，收益取决于同一 dbnum 的查询复用率。
- 纯 Rust 层级缓存、跨 dbnum 联合分片、partial 种子与覆盖证明、mmap、自动内存伸缩和后台文件清理均不在第一阶段实现。
