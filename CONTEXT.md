# plant-model-gen 领域术语表

PDMS/E3D 工厂设计数据的解析、模型生成与版本化服务。本表是全仓唯一术语真相源；实现细节不在此记录（见 `specs/` 与 `docs/adr/`）。

## 版本与历史

**sesno（设计会话号）**:
PDMS/E3D 源库的业务版本号，随每次设计会话递增。全系统唯一的业务版本坐标，配合 dbnum 使用：`(dbnum, sesno)`。
_Avoid_: 版本号、session id

**数据版本（数据历史）**:
PE/ATT 源数据行在 versioned 存储中的历史状态，按 `(dbnum, sesno)` 经数据锚点定位。
_Avoid_: 数据快照、源版本

**模型版本（模型历史）**:
模型表（几何实例与关系）在 versioned 存储中的历史状态，按 `(dbnum, sesno)` 经模型锚点定位。模型库本身是可由数据重新生成的缓存。
_Avoid_: 交付单元版本、unit version、release

**版本提交（Version Commit）**:
一个 dbnum 在目标 sesno 上完整、已验证且可供历史读取的 PE/ATT 源数据状态。
_Avoid_: 增量批次、保存结果

**版本锚点（Version Anchor）**:
业务版本号与存储时间戳之间的唯一桥梁，键 `(dbnum, sesno, source)`。source 表达锚定语义：`full`/`incremental` = 该 sesno 的数据就绪时刻；`model_gen` = 该 sesno 的模型生成完成时刻。锚点发布后不可变；“有锚点 = 该语义下的一致快照”。
_Avoid_: 可变 checkpoint、最新时间戳、版本标记

**提交指纹（Commit Fingerprint）**:
由规范化变更、sesno 区间与源观测信息确定性计算出的版本提交身份。
_Avoid_: 行数、文件时间戳

**待提交版本（Commit Pending）**:
写入可能已开始，但验证和版本锚点发布尚未完成的候选版本提交。同一 dbnum 存在待提交版本时，后续 sesno 提交必须被阻断。
_Avoid_: 部分成功、仅告警成功

**旧式锚点（Legacy Anchor）**:
提交指纹机制启用前创建、仅为读取兼容而保留的版本锚点；不得重写，也不得视为可复现版本提交的证明。
_Avoid_: 回填提交

**已提交水位（Committed Watermark）**:
一个 dbnum 已发布版本锚点的最高 sesno，是增量收集唯一允许的续跑起点。仅对锚点机制启用前的 dbnum，才回退到 `dbnum_info_table` 最大 sesno。
_Avoid_: 文件最新 sesno、缓存 header sesno、直接使用 dbnum_info_table 最大 sesno

**导出物**:
按锚点从库内导出的一次性交付文件（parquet/glb 等）。导出物没有独立的版本身份，其版本就是导出时使用的锚点坐标。
_Avoid_: release 包、交付单元、release_id

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

## 已废除术语（历史文档中出现时一律按废除理解）

- **交付单元版本 / unit version / release_id**：由 DuckLake 交付链承载的版本身份，已整体退役（ADR-0001）
- **历史重放 / 物理基线**：用源文件重放历史的机制，被存储层时间旅行取代
- **模型 record id 的 sesno 槽位**：`[ref0, ref1, sesno]` 形状的应用层版本化残留，已删除
