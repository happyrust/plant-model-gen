# Implementation Plan: RVM 导入完善与模型生成准确性对拍(spec 009)

## Approach

把 RVM 从"能进 SQLite 的孤岛数据"升级为"能按真实 refno 与生成侧 join 的基准",
再在其上建三层对比引擎:

1. 身份解析前移到导入期(站点库 name 查询 + E3D 默认命名反推),relation store
   直接落真实 refno;
2. 对比引擎只做"按 refno join 后逐层比对",不在对比期做任何模糊匹配;
3. 单 BRAN 样本(`2013286704/476`)全链路验收后,容差与映射表沉淀为配置。

## Phase 0 — 基线复现

固化当前行为,作为修改前对照:

```powershell
cargo run --features rvm-import --bin aios-database -- `
  --import-rvm "D:\work\plant-code\plant-model-gen\test_data\rvm\2013286704_476 .rvm" `
  --dbnum 250160 --verbose
```

记录:节点统计、生成的伪 refno 形态(`(250160<<32)|hash`)、
`--export-rvm-semantic-debug` 的 identity_source 字段。

## Phase 1 — Relation Store 扩列

`src/model_relation_store.rs`:

- `inst_relate` 增列:`name TEXT`、`noun TEXT`、`identity_source TEXT`
  (`surreal_name` / `default_name_rule` / `stable_hash`)、`resolved INTEGER`。
- 建表 SQL 更新 + 旧库 `ALTER TABLE ... ADD COLUMN` 幂等迁移
  (打开时检测列缺失再补)。
- `InstRelateRecord` 扩字段,既有调用点(语义导出 SELECT 已引用 noun/name)
  改为读真实列。

## Phase 2 — 导入期身份解析器

`src/rvm_import.rs` 新增 `RefnoResolver`:

1. **命名元素**:`SELECT id FROM pe WHERE name = $name AND dbnum = $dbnum`
   (站点 SurrealDB;连接参数走既有 `-c` 配置或新增 `--resolve-db` 参数);
   多解时用 owner 链逐级名称约束消歧。
2. **未命名成员**:解析 `<NOUN> <n> of <OWNER_NAME>` →
   先解析 OWNER refno,再查 owner 子元素中第 n 个 `noun == NOUN` 的 pe
   (顺序 = `pe_owner` 序,与 `fn::default_name` 的 order 语义一致)。
3. **失败回退**:保留 `stable_refno`,`identity_source=stable_hash`、
   `resolved=0`;导入统计打印 `resolved=X unresolved=Y`。
4. NOUN 词表(2026-06-13 站点库实测):站点 `pe.noun` 是 PDMS 四字短名词,
   RVM 默认命名用全称,需映射表:FLANGE→FLAN、ELBOW→ELBO、REDUCER→REDU、
   GASKET→GASK、VALVE→VALV、TEE→TEE、TUBE→TUBI、BRANCH→BRAN、
   EQUIPMENT→EQUI…;未知全称按前 4 字符截断兜底,未命中记 unresolved。

数据侦察实证(站点 quicktest-250160-8080,surreal 8022):

- `pe` id 形态:`` pe:`2013286704_476` ``(字符串 key `ref0_ref1`);
- 命名元素 `name` 为带斜杠全名(`/03SKID1-PIPE-SUCTION/B1`),
  与 RVM 组名**逐字符一致**,可直接精确匹配;
- 未命名成员 `name = ""`(空串);
- BRAN 476 子序列(pe_owner 序):477 FLAN→478 REDU→479 TEE→480 ELBO→
  481 FLAN→482 GASK→483 VALV→484 GASK→485 FLAN→1325 FLAN→1326 GASK;
- `fn::default_name(477)` = `FLAN 1`,序号语义与 RVM `FLANGE 1 of …` 一致,
  反推规则可落地。

无站点连接时(离线导入)跳过解析,全部走回退——保持现有调用方不破坏。

## Phase 3 — `--compare-rvm` 对比引擎

新模块 `src/rvm_compare.rs` + CLI 接线(`main.rs` / `cli_modes.rs`):

```text
--compare-rvm --dbnum 250160 --root-refno 2013286704/476
  [--relation-store-root <dir>] [-c 站点配置]
  [--tol-translation-mm 1.0] [--tol-length-pct 0.1] [--tol-angle-deg 0.1]
  [--report-dir runtime/rvm-compare]
```

实现分层:

- **L1 成员清单**(基准修正:Parquet):RVM 侧 root 子树(relation store)
  vs gen 侧 `instances.parquet` 按 refno 过滤 root 子树
  (子树成员集合由站点库 `pe_owner` 遍历得出)∪ `tubings.parquet`;
  TUBE 对齐规则:RVM `TUBE n of BRANCH` ↔ tubings 按 BRAN 内顺序,
  另用站点库 `tubi_relate` 交叉验证。
- **L2 参数级**:类型映射表 `RvmKind ↔ gen 参数几何类型`,逐参数容差;
  gen 侧参数从 `geo_instances.parquet` / `instances.parquet` 读取
  (参数列不足时回退读取 mesh AABB 做 L3-only 比对并标记)。
- **L3 空间级**:gen 侧 world transform 取 `transforms.parquet`、
  AABB 取 `aabb.parquet`;RVM 侧取 payload transform/bbox;
  计算平移/旋转偏差与 AABB 中心/尺寸/IoU。
- 报告结构:`{ summary: {matched, missing, extra, param_mismatch,
  spatial_mismatch}, items: [...逐项明细...], tolerances: {...} }`;
  控制台打印 summary;全过退出 0,有差异退出 1。

## Phase 4 — 样本全链路验收

前置:站点 `quicktest-250160-8080` 在线(surreal 8022),生成数据为
2026-06-12 22:30 成功重跑产物(tubi_relate=16)。

1. 重新导入样本 RVM(带解析):期望 resolved=全部、BRAN==`2013286704/476`。
2. 跑 `--compare-rvm`:分析 L1/L2/L3 差异;
   - 已知预期差异:RVM 含 Obstruction/Insulation 几何时进豁免桶;
   - TUBE 段数与 `tubi_relate`(16 条 / 本 BRAN 范围内的子集)对齐验证。
3. 容差调整与差异定性:真实生成缺陷 → 记录为后续 spec 候选,不在本 spec 修。
4. 报告样例归档到 spec 目录。

## Phase 5 — 文档与收口

- `CHANGELOG.md` 记录;spec 009 回填验收数据。
- README/CLI help 增加 `--compare-rvm` 用法与样本命令。
- 残留风险与后续(批量对拍、ATT 属性对比)列入 spec Backlog 段。

## Risks

- R1:E3D 默认命名格式随语言/版本变化。缓解:解析器规则可配,
  样本驱动扩展;unresolved 不阻断导入。
- R2:TUBI 对齐顺序假设(RVM TUBE 序 == gen tubi index 序)可能不成立。
  缓解:先按顺序对齐 + 空间(L3)交叉验证;不一致时降级为空间最近邻并标记。
- R3:同名元素多解。缓解:owner 链逐级约束;仍多解则标记 ambiguous 不强配。
- R4:RVM 导出设置差异(是否含保温/障碍)。缓解:geo_type 分桶豁免 + 报告注明。
- R5:`rvm-rs` 对个别原语(FacetGroup 大网格)解析性能。缓解:样本仅 72KB;
  批量场景再优化。

## Rollback

新增模块与 CLI 均为增量;relation store 扩列为幂等 ADD COLUMN,
回滚即不再读新列,旧数据不受影响。

## Done Definition

- 样本 RVM 导入 resolved=100%、BRAN refno 正确;
- `--compare-rvm` 三层报告产出且可复跑,样本差异全部容差内或逐项可解释;
- CHANGELOG / spec / CLI 文档同步;
- 不触碰 gen_model 生成逻辑。
