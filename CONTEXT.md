# plant-model-gen 领域术语表

PDMS/E3D 工厂设计数据的解析、模型生成与版本化服务。本表是全仓唯一术语真相源；实现细节不在此记录（见 `specs/` 与 `docs/adr/`）。

## 版本与历史

**sesno（设计会话号）**:
PDMS/E3D 源库的业务版本号，随每次设计会话递增。数据历史使用 `(dbnum, sesno)` 定位；最小交付单元的模型历史使用 `(dbnum, unit_refno, sesno)` 定位。
_Avoid_: 版本号、session id

**数据版本（数据历史）**:
一个 dbnum 的 PE/ATT 源数据在指定 sesno 上的完整历史状态，身份为 `(dbnum, sesno)`。
_Avoid_: 数据快照、源版本

**输入版本清单（Input Version Manifest）**:
一次模型生成实际读取的全部数据版本集合，是 `dbnum → sesno` 的不可变映射，也是跨库依赖可重现的完整业务坐标；它是模型提交的来源证明，不取代模型提交身份。
_Avoid_: 目标 sesno、最新版本、单库版本号

**生成契约（Generation Contract）**:
决定同一输入版本清单如何生成模型结果的规则集合，涵盖会影响结果的算法与配置；它用于解释和重现结果，不取代 sesno 模型历史。
_Avoid_: 临时运行参数、任务配置

**最小交付单元（Minimum Delivery Unit）**:
独立追踪模型变化和交付的最小完整模型边界。其根由 BRAN、HANG、EQUI、WALL 或 FLOOR 确定，HVAC 是纳入该边界的模型分类而不是另一种根。
_Avoid_: 任意元素、dbnum 汇总包、release unit

**模型提交（Model Commit）**:
最小交付单元在一次 sesno 上的完整模型状态，身份为 `(dbnum, unit_refno, sesno)`；每次变化都是小版本提交，不是人为发布。即使几何未变化，E3D 写入的新 sesno 仍形成可追踪的模型提交。
_Avoid_: model_version_id、release、发布版本、content_hash

**模型删除提交（Tombstone Model Commit）**:
最小交付单元在指定 sesno 已不存在的模型提交；它保留删除事实，但不引用模型导出物。
_Avoid_: 空模型、缺失导出物、加载失败

**权威版本库（Authoritative Version Store）**:
唯一有权判定某个数据版本是否正式存在并推进已提交水位的版本库。
_Avoid_: 主库、默认后端

**版本化读副本（Versioned Read Replica）**:
复制权威版本库已发布的数据版本并保留历史查询能力的只读版本库；它可以落后但不得领先权威版本库，也不能自行发布新版本。
_Avoid_: 第二真相源、双主版本库、当前态缓存

**版本读取会话（Versioned Read Session）**:
一次业务运行绑定到固定输入版本清单和单一读取后端后的只读视图；会话存续期间即使有新版本发布，所有查询仍必须看到同一组数据版本。会话内禁止按 dbnum 或单次查询在权威版本库与版本化读副本之间回退。
_Avoid_: 每次查询取最新、动态快照、混合后端会话

**权威 snapshot 绑定（Authoritative Snapshot Binding）**:
输入版本清单在权威版本库中的物理定位信息。当前实现绑定一个 DuckLake 全局 `snapshot_id`；它只用于重现清单对应的存储状态，不取代任何 `(dbnum, sesno)` 数据版本身份。
_Avoid_: 用 snapshot_id 代替数据版本、任意历史版本拼接

**副本 snapshot 绑定（Replica Snapshot Binding）**:
权威 `snapshot_id` 在版本化读副本中的已验证复制提交点。只有连续复制水位已覆盖该 snapshot 且绑定校验成功时，副本才可为对应版本读取会话提供数据。
_Avoid_: 版本锚点、读取副本 latest、缺水位时静默回退

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
一个 dbnum 已发布且可完整读取的数据版本的最高 sesno，是增量收集唯一允许的续跑起点。
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

**重生成清理（pre_cleanup）**:
覆盖式重建前删除目标 refno 及后代的旧模型产物的步骤，是"插入即忽略"写入模式的对偶操作；被删数据的历史由 versioned 存储层保留。

## 模型生成

**生成缓存库（Generation Cache DB）**:
常驻的内存数据库实例，按 db 粒度加载模型生成所需的源数据、并承载生成过程的中间产物，受内存预算约束换入换出。持久库仍是唯一真相源；生成缓存库内的数据没有独立版本身份。
_Avoid_: mem 沙箱、内存副本库、缓存 surrealdb

**基线 sesno（Base Sesno）**:
生成缓存库中一个 db 副本灌入时对应的已提交水位，是该副本数据新鲜度的唯一判据；副本落后于持久库已提交水位时即视为过期。
_Avoid_: 缓存版本号、快照 sesno

## 已废除术语（历史文档中出现时一律按废除理解）

- **版本锚点 / 旧式锚点**：旧 SurrealDB 历史方案中从 `(dbnum, sesno, source)` 映射存储时间戳的桥梁，不再承担版本身份或一致性证明
- **release / release_id / model_version_id**：人为发布批次或额外代理版本号；模型变化直接以最小交付单元的 sesno 提交表达
- **历史重放 / 物理基线**：用源文件重放历史的机制，被存储层时间旅行取代
- **模型 record id 的 sesno 槽位**：`[ref0, ref1, sesno]` 形状的应用层版本化残留，已删除
