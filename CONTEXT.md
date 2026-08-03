# plant-model-gen 领域术语表

PDMS/E3D 工厂设计数据的解析、模型生成与版本化服务。本表是全仓唯一术语真相源；实现细节不在此记录（见 `specs/` 与 `docs/adr/`）。

## 解析

**ref0（引用号库前缀）**:
构成 refno 的库归属前缀；每个 ref0 只归属于一个 dbnum，而一个 dbnum 可以拥有多个 ref0。
_Avoid_: dbnum、完整 refno、可复用前缀

**初始化解析（Initialization Parse）**:
项目首次写入新数据目录时，对一轮稳定源文件集合读取当时最新完整状态的过程；它不回放历史会话，也不形成数据版本。业务版本开始后不得在原数据目录重新执行初始化解析。
_Avoid_: 全量历史解析、full 版本提交、原地重灌

**数据库增量接入（Incremental Database Onboarding）**:
已有版本历史的项目接入一个此前不存在的 dbnum 时，在增量流程内导入该库当前完整态并建立增量起版基线的过程；它不允许借用初始化解析旁路写入已有项目。
_Avoid_: 局部全量解析、补库、原地初始化

## 版本与历史

**sesno（设计会话号）**:
PDMS/E3D 源库的业务版本号，随每次设计会话递增。数据历史使用 `(dbnum, sesno)` 定位；最小交付单元的模型历史使用 `(dbnum, unit_refno, sesno)` 定位。
_Avoid_: 版本号、session id

**会话元数据导入（Session Metadata Import）**:
显式读取源库中的设计会话号、发生时间等描述信息，供日期展示与审计辅助使用；它不重建 PE/ATT 历史状态，也不形成数据版本或模型提交。
_Avoid_: 历史解析、历史数据导入、版本重放

**数据版本（数据历史）**:
一个 dbnum 的 PE/ATT 源数据在指定 sesno 上的完整历史状态，身份为 `(dbnum, sesno)`。
_Avoid_: 数据快照、源版本

**版本锚点（Version Anchor）**:
`sesno_version_anchor` 中把业务 sesno 映射到 SurrealDB MVCC 数据库时刻的已发布边界，供 `VERSION AT` 历史读取和水位计算使用；它不复制 PE/ATT，也不构成第二个版本库。数据锚点的新写来源只有增量起版基线与增量提交，`full` 仅兼容存量；`model_gen` 锚点声明模型覆盖水位。
_Avoid_: 物理快照、权威副本、版本数据复制

**增量起版基线（Incremental Version Baseline）**:
一个 dbnum 首次执行版本化增量时，在应用变更之前固化的完整当前态，sesno 等于当时的已提交水位；它为第一次增量提供可查询的变更前状态，只能由增量流程建立，不能由全量解析建立。
_Avoid_: full 锚点、解析版本、迁移快照

**输入版本清单（Input Version Manifest）**:
一次模型生成运行打开读取时观测到的各 dbnum 已提交水位记录（`dbnum → sesno`），写入运行结果用于解释与复现来源；它是观测记录，不是绑定契约，不参与失败关闭或覆盖校验。
_Avoid_: 绑定契约、fail-closed 清单、目标 sesno、最新版本

**生成读取时刻（Generation Read Instant）**:
一次增量模型生成绑定的单一 MVCC as-of 时刻（取同轮最后一个数据提交锚点的数据库时间）；运行内所有 dbnum 和所有查询都看到该时刻的一致切面。初始化全量生成不绑定时刻，活读 staging 当前态。
_Avoid_: 版本读取会话、动态快照、每次查询取最新

**生成契约（Generation Contract）**:
决定同一输入版本清单如何生成模型结果的规则集合，涵盖会影响结果的算法与配置；它用于解释和重现结果，不取代 sesno 模型历史。
_Avoid_: 临时运行参数、任务配置

**模型生成运行（Model Generation Run）**:
一次初始化生成、增量生成、欠账追赶或受控修复的执行尝试。运行记录追加保存其输入水位、生成契约、原因、操作者与结果，但不构成新的模型版本身份；同一 sesno 修复后的最新成功结果仍属于原模型提交。
_Avoid_: 模型版本、release、重试次数

**最小交付单元（Minimum Delivery Unit）**:
独立追踪模型变化和交付的最小完整模型边界。其根由 BRAN、HANG、EQUI、WALL 或 FLOOR 确定，HVAC 是纳入该边界的模型分类而不是另一种根。
_Avoid_: 任意元素、dbnum 汇总包、release unit

**模型提交（Model Commit）**:
最小交付单元在一次 sesno 上的完整模型状态，身份为 `(dbnum, unit_refno, sesno)`；每次变化都是小版本提交，不是人为发布。即使几何未变化，E3D 写入的新 sesno 仍形成可追踪的模型提交。
_Avoid_: model_version_id、release、发布版本、content_hash

**模型删除提交（Tombstone Model Commit）**:
最小交付单元在指定 sesno 已不存在的模型提交；它保留删除事实，但不引用模型导出物。
_Avoid_: 空模型、缺失导出物、加载失败

**版本提交（Version Commit）**:
一个 dbnum 在目标 sesno 上完整、已验证且已发布、可供历史读取的 PE/ATT 数据版本。
_Avoid_: 增量批次、保存结果

**提交指纹（Commit Fingerprint）**:
由规范化变更、sesno 区间与源观测信息确定性计算出的版本提交身份。
_Avoid_: 行数、文件时间戳

**待提交版本（Commit Pending）**:
写入可能已开始，但验证和版本发布尚未完成的候选版本提交。同一 dbnum 存在待提交版本时，后续 sesno 提交必须被阻断。
_Avoid_: 部分成功、仅告警成功

**已提交水位（Committed Watermark）**:
一个 dbnum 已发布且可完整读取的数据版本的最高 sesno，是增量收集唯一允许的续跑起点。初始化后尚无数据锚点时，实现可暂以 `dbnum_info` 的 latest_sesno 作为首次收集起点，但该回退值不构成数据版本，直到增量流程发布起版基线。
_Avoid_: 文件最新 sesno、缓存 header sesno、直接使用源文件最大 sesno

**导出物**:
从指定数据版本或模型提交产生的交付文件（parquet/glb 等）。最小交付单元导出继承 `(dbnum, unit_refno, sesno)`；暂未拆分的其他模型类型仍按 dbnum 汇总导出。
_Avoid_: release 包、release_id

## 增量链路

**增量更新**:
以 sesno 区间从源 db 文件收集元素变更、落库 PE/ATT、并按变更分类驱动模型重生成的流程。唯一常驻入口是 watch-incremental 轮询。
_Avoid_: 热更新、实时同步

**元素变更（Element Change）**:
从源 db 文件收集 sesno 区间时观察到的单元素分类操作，包含新增、修改或删除及其 noun、owner 和模型分类。它只存在于收集结果及报告中，不是独立可查询的存储。
_Avoid_: element_changes 表记录、increment record、IncrementInfo

**增量分类日志（IncrGeoUpdateLog）**:
一次增量收集产出的变更 refno 分桶（prim / loop_owner / bran_hanger / basic_cata / delete），是增量模型生成的种子。

**几何影响属性（Geometry-Affecting Attribute）**:
其改动会改变模型生成器的几何输入、从而需要重算受影响元素几何的属性；判据是该属性是否驱动定位、目录/规格选型、尺寸或管路布线等几何输入。只改非几何影响属性（名称、描述等业务元数据）的变更不触发几何重算。判定的权威语义与工程取舍见 ADR-0009。
_Avoid_: wnoevt（内核标志本身）、任意可变属性、显著属性表

**目录反向波及闭包（Catalogue Reverse Impact Closure）**:
元件库定义（如 SCOM 及其下几何/尺寸）发生几何影响变更时，沿引用关系反向查出所有引用它的设计实例、并把这些实例纳入模型重生成目标的过程；它是「依赖闭包清单」（出向 DESI→元件库）的反方向。设计实例改动自身引用只重算该实例，不经此闭包向同引用兄弟扇出。
_Avoid_: 依赖闭包清单（那是出向）、cata manifest、共享目录全扇出

**重生成清理（pre_cleanup）**:
覆盖式重建前删除目标 refno 及后代的旧模型产物的步骤，是"插入即忽略"写入模式的对偶操作；被删数据的历史由 versioned 存储层保留。

## 模型生成

**生成缓存库（Generation Cache DB）**:
一次初始化模型生成持有的内存数据库实例，按 db 粒度加载模型生成所需的源数据，并在内存预算内换入换出。持久库仍是唯一真相源；生成缓存库内的数据没有独立版本身份，初始化模型生成结束后整体释放。
_Avoid_: mem 沙箱、内存副本库、缓存 surrealdb

**层级投影种子（Hierarchy Projection Seed）**:
初始化解析为单个 dbnum 及确定解析范围产生的当前态元素身份与层级元数据投影，用于快速灌入生成缓存库；它不是数据版本或权威副本，水位或解析范围不匹配时必须弃用。
_Avoid_: rkyv 缓存、树版本、层级权威库

**PE 层级关系（PE Ownership Relation）**:
`pe_owner` 是 PE 父子层级的唯一持久关系表示：边由 child 指向 owner，并携带同胞顺序；`pe.owner` 供祖先链读取，`pe` 不保存 children 列表。缺失或未通过完整性校验的 `pe_owner` 是数据错误，不允许回退到重复邻接表。
_Avoid_: pe.children、双写层级、children 兼容回退

**基线 sesno（Base Sesno）**:
生成缓存库中一个 db 副本灌入时对应的已提交水位，是该副本数据新鲜度的唯一判据；副本落后于持久库已提交水位时即视为过期。
_Avoid_: 缓存版本号、快照 sesno

**依赖闭包清单（Dependency Closure Manifest）**:
一个 DESI dbnum 进行模型生成所需的外部依赖 refno 集合（元件库及模板 DESI 库），按依赖 dbnum 分组；解析期计算，生成期仅作预灌范围提示，允许滞后，漂移由生成期兜底链自愈。
_Avoid_: 工程级 union 闭包清单、输入版本清单、cata manifest

**模型生成水位（Model Generation Watermark）**:
一个 dbnum 已发布 model_gen 锚点的最高 sesno，表示模型产物已完整覆盖到的数据版本；它只在模型生成及后处理全部成功，或已确认本次变更对模型中性并发布 no-op 锚点后推进。
_Avoid_: 生成进度、最后生成时间

**模型生成欠账（Model Generation Debt）**:
增量数据提交时留存、尚未被成功模型生成消费的变更种子记录；追赶只认欠账记录，欠账不阻断后续数据版本提交。
_Avoid_: 生成失败列表、重试队列、历史全量补偿

**未跟踪历史（Untracked History）**:
欠账跟踪启用之前发生的数据版本与模型状态差距；默认忽略、不告警、不自动重建，仅绑定既有数据锚点的受控 catch-up/repair 可追平。
_Avoid_: 历史欠账、存量洞

## 已废除术语（历史文档中出现时一律按废除理解）

- **权威版本库 / 版本化读副本**：DuckLake 权威 + Surreal 读副本的双库分工（ADR-0002 时期）；版本真相已收敛为 Surreal MVCC + sesno 锚点单源（ADR-0007），不存在第二个版本库
- **版本读取会话 / 权威 snapshot 绑定 / 副本 snapshot 绑定**：绑定 DuckLake snapshot 的固定版本读取机制（ADR-0003 时期）；由「生成读取时刻」取代（ADR-0008），不再有 snapshot_id 或后端绑定
- **release / release_id / model_version_id**：人为发布批次或额外代理版本号；模型变化直接以最小交付单元的 sesno 提交表达
- **历史重放 / 物理基线**：用源文件重放历史的机制，被存储层时间旅行取代
- **模型 record id 的 sesno 槽位**：`[ref0, ref1, sesno]` 形状的应用层版本化残留，已删除
