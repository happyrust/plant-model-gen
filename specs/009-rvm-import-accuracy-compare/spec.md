# Feature Specification: RVM 导入完善与模型生成准确性对拍(spec 009)

## User Need

把 AVEVA E3D 官方导出的 RVM 文件作为**几何基准**,与本仓 `gen_model` 生成的模型数据
做结构化对拍,量化验证模型生成的准确性。

首个基准样本:`D:\work\plant-code\plant-model-gen\test_data\rvm\2013286704_476 .rvm`
(注意文件名含空格)——E3D 2.1 于 2026-06-12 23:29 导出,72KB,单 BRAN
`/03SKID1-PIPE-SUCTION/B1`,即 AvevaPlantSample dbnum=250160 的 refno `2013286704/476`,
与站点 `quicktest-250160-8080` 的生成数据同源。

## Current State(已探明)

`rvm-import` feature 已有的闭环:

- `--import-rvm <RVM_PATH> --dbnum N [--att ...]`(`src/rvm_import.rs`):
  `rvm-rs` 解析 RVM/ATT → 遍历 File/Model/Group 树 → 写 `ModelRelationStore` SQLite
  (inst_relate / inst_geo / geo_relate),几何按 11 种 RVM 原语序列化 JSON payload
  (kind/detail/transform/bbox/color)。
- `--export-rvm-semantic-debug --dbnum N --root-refno X`
  (`src/fast_model/export_model/export_rvm_semantic_debug.rs`):从 relation store
  导出语义调试 JSON。
- OBJ 网格导出(`src/rvm_obj_export.rs`):RVM 原语 → 三角网格,可视化对拍用。

RVM 样本内部形态(实测):组名为 **E3D NAME 路径**(`/SITE-EQUIPMENT-AREA03` →
`/03SKID1` → `/03SKID1-PIPE-SUCTION` → `/03SKID1-PIPE-SUCTION/B1`),
未命名成员为 **E3D 默认命名**(`FLANGE 1 of BRANCH /03SKID1-PIPE-SUCTION/B1`)。

## Gaps(挡住准确性对拍的三件事)

1. **身份无法对齐**:导入用 `stable_refno(dbnum, hash(path))` 生成伪 refno
   (`rvm_import.rs:266-273`),与生成侧真实 refno(`2013286704/476`)无关,
   两侧数据无法按 refno join。
2. **没有对比工具**:只有导入/导出,没有 compare 命令。
3. **语义信息进库即丢**:`InstRelateRecord` 只有
   refno/inst_id/parent_refno/world_matrix(`model_relation_store.rs:304-309`),
   RVM 组名(name)与可推导的类型(noun)未保留。

## Decisions(grill-me,2026-06-13)

| 问题 | 决策 |
|------|------|
| Q1 身份对齐 | A:导入时解析真实 refno——站点 SurrealDB 按 name 查 `pe`(命名元素);未命名成员按 E3D 默认命名 `<NOUN> <n> of <OWNER>` 在 owner 同类型子序列中定位(与站点库 `fn::default_name` 同语义);失败回退 stable_refno 并标记 `unresolved` |
| Q2 对比内容 | A:三层——①成员清单(missing/extra)②原语参数级(类型映射+容差)③空间级(world 平移/旋转偏差、AABB 中心/尺寸/IoU);JSON 报告+控制台摘要,容差可配 |
| Q3 gen 侧基准 | A(2026-06-13 实测修正):**Parquet 导出包**(`instances/geo_instances/transforms/aabb/tubings.parquet`)。原推荐"站点 SurrealDB"被实测推翻——站点库 `inst_relate`/`inst_info`/`inst_geo` 行数为 0、`trans`/`aabb` 表不存在,仅 `tubi_relate`(16)与 `ses` 有数据;而 Parquet 包完备(808 实例)。站点库仅用于身份解析(`pe`/`pe_owner`)与 TUBI 交叉验证 |
| Q4 交付形态 | A:新 CLI `--compare-rvm`;以 `2013286704/476` 为首个验收样本 |

## Scope

1. `model_relation_store.rs`:`inst_relate` 表扩列
   `name TEXT / noun TEXT / identity_source TEXT / resolved INTEGER`
   (新建即含,旧库 `ALTER TABLE` 兼容迁移);`InstRelateRecord` 同步扩字段。
2. `rvm_import.rs`:
   - 保留组名进库;
   - 新增身份解析器:连接站点 SurrealDB(命令行给定连接/或 `-c` 站点配置),
     命名元素按 name 精确查 `pe`;未命名成员解析
     `<NOUN> <n> of <OWNER_NAME>` → 查 owner 的第 n 个同 NOUN 子元素;
   - 解析失败回退 stable_refno,`identity_source=stable_hash`、`resolved=0`,
     导入统计输出 resolved/unresolved 计数。
3. 新 CLI `--compare-rvm`(`cli_modes.rs` + `main.rs`):
   - 输入:RVM relation store(dbnum)、站点 SurrealDB 连接、root refno、容差参数;
   - 三层对比引擎,输出 JSON 报告(`runtime/rvm-compare/<root>-<ts>.json`)
     + 控制台摘要;
   - 退出码:0=容差内全过,1=存在差异(CI 可用)。
4. 首个验收:样本 BRAN 对 `quicktest-250160-8080` 站点库跑通全链路。

## Comparison Semantics(三层定义)

- **L1 成员清单**:RVM 侧 group 子树(FLANGE/ELBOW/TUBE/…) vs gen 侧
  `inst_relate` refno 子树 ∪ `tubi_relate`(gen 侧 TUBE 是隐式 TUBI 段,
  需把 RVM 的 TUBE 组与 `tubi_relate:[bran,idx]` 对齐);输出 matched /
  missing(基准有生成无)/ extra(生成有基准无)。
- **L2 参数级**:RVM 原语 ↔ gen `inst_geo` 参数化几何的类型映射表
  (Cylinder↔CYLI/TUBE 段、Snout↔REDU、CircularTorus↔ELBO/BEND、
  Box↔BOX、Dish↔DISH…);逐参数容差比较(长度/半径/角度)。
- **L3 空间级**:world transform 平移偏差(mm)与旋转偏差(deg);
  AABB 中心偏差、尺寸偏差、IoU。

默认容差(可配):平移 ≤1mm、长度/半径 ≤max(0.1mm, 0.1%)、角度 ≤0.1°、
AABB IoU ≥0.99。

## Non-Goals

- 不做三角网格级(mesh 顶点)diff——OBJ 双导出已可人工视觉对拍。
- 不做 ATT 属性值对比(本期只对几何与结构);ATT 解析入口保留现状。
- 不修改 `gen_model` 生成逻辑本身;对拍发现的生成缺陷另开 spec 修复。
- 不做全库批量对拍调度;单 root(BRAN/EQUI)粒度优先,批量为后续扩展。
- 不引入新的存储格式;继续用 ModelRelationStore SQLite 承载 RVM 侧数据。

## Open Question(侦察暴露,待另行排查,不阻塞本 spec)

站点 `quicktest-250160-8080` 生成配置为 `model_writer=surreal`、
`writes_to_surreal=true`,生成日志显示 base 批量写入逐批执行,
但事后站点库 `inst_relate`/`inst_info`/`inst_geo` 均为 0 行。
嫌疑:`utils.rs::ensure_inst_relate_relation_schema()` 每进程首调会
`REMOVE TABLE inst_relate` 重建(后续进程启动可能清空既有数据),
或写入目标连接与查询连接不一致。该疑点影响所有依赖站点库
inst 数据的 API,建议另开排查;本 spec 的对比基准已改用 Parquet,不受影响。

## Known Constraints / Risks

- E3D 默认命名格式假设为 `<NOUN> <n> of <OWNER_NAME>`(样本已证),
  若现场语言包不同需扩展解析;NOUN 词表取 rvm-rs 可见组名首词。
- RVM 导出选项(障碍/保温几何、ZONE 级别)影响成员集合:对比报告需按
  `geo_type`(Primitive/Obstruction/Insulation)分桶,Obstruction/Insulation
  默认豁免 missing 判定。
- RVM 单位为 mm(与 gen 侧一致,样本验证);坐标系同为 E3D world。
- 名称重复(同名元素)时按 name 查 pe 可能多解:取 owner 链路径逐级约束消歧。

## Acceptance Criteria

1. 导入样本 RVM 后:BRAN 及其全部成员真实 refno 解析率 100%(该样本),
   `resolved=1`,unresolved=0;BRAN 解析结果 == `2013286704/476`。
2. `--compare-rvm` 对站点 `quicktest-250160-8080` 输出报告:
   L1 成员清单 missing=0、extra=0(或差异全部落在 Obstruction/Insulation
   豁免桶并有说明)。
3. L2/L3 差异在默认容差内,或报告逐项列出可解释差异
   (含 refno、参数名、两侧值、偏差)。
4. 报告可复跑(同输入同输出),退出码语义正确;命令与样本路径写入文档。
