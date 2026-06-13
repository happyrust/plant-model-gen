# Tasks: RVM 导入完善与模型生成准确性对拍(spec 009)

## T001 — 基线复现与样本档案

- 用现有 `--import-rvm` 导入样本
  `D:\work\plant-code\plant-model-gen\test_data\rvm\2013286704_476 .rvm`
  (dbnum=250160,--verbose),记录节点/几何统计与伪 refno 形态。
- `--export-rvm-semantic-debug` 导出当前 identity_source 证据。
- 归档样本元信息:E3D 2.1 导出、单 BRAN `/03SKID1-PIPE-SUCTION/B1`
  (=2013286704/476)、组名为 NAME 路径、未命名成员为默认命名。

验收:

- 基线统计与伪 refno 样例写入本 spec;后续修改可对照。

实测(2026-06-13 01:15,target-spec009 debug 构建,relation store
默认 `output/model_relations`):

- 统计:File=1 / Model=1 / Group=15 / Geometry=22 / cleaned=0;
- 伪 refno 形态:`1074432847012148`(=(250160<<32)|hash)已固化;
- BRAN `/03SKID1-PIPE-SUCTION/B1` 组自带 2 个 geometry(**TUBE 管段挂在
  BRAN 组上,RVM 不为 TUBE 生成独立组**)——L1 对齐规则修正:
  BRAN 自身 geos ↔ `tubings.parquet`;
- 11 个成员组与站点库子序列完全对应(FLANGE 1/REDUCER 1/TEE 1/ELBOW 1/
  FLANGE 2/GASKET 1/VALVE 1/GASKET 2/FLANGE 3/FLANGE 4/GASKET 3 ↔
  477 FLAN/478 REDU/479 TEE/480 ELBO/481 FLAN/482 GASK/483 VALV/
  484 GASK/485 FLAN/1325 FLAN/1326 GASK),默认命名序号按**同 NOUN 计数**,
  反推规则实锤;
- GASKET 组 geos=0(垫片无几何导出)→ L1 需豁免"零几何成员";
- VALVE 1 组 geos=12(阀门多原语);
- group path 含文件头前缀(`AVEVA Everything3D .../APS/...`),
  名称匹配需取路径末段。

## T002 — ModelRelationStore 扩列

- `inst_relate` 增列 `name/noun/identity_source/resolved`,
  建表 SQL + 旧库幂等 ALTER 迁移。
- `InstRelateRecord` 扩字段;语义导出 SELECT(`export_rvm_semantic_debug.rs:324`)
  改读真实列。

验收:

- 新库/旧库各导入一次均成功;语义导出包含 name/noun。

## T003 — 导入期身份解析器

- `rvm_import.rs` 新增 `RefnoResolver`:
  - 命名元素:站点 SurrealDB 按 name+dbnum 查 `pe`,owner 链消歧;
  - 未命名成员:`<NOUN> <n> of <OWNER_NAME>` → owner 第 n 个同 NOUN 子元素;
  - 失败回退 stable_refno + `resolved=0`;
  - 离线(无连接)模式整体回退,不破坏既有调用。
- 导入统计输出 `resolved/unresolved` 计数。

验收:

- 单测覆盖:命名解析、默认命名解析、失败回退三态;
- 样本导入 resolved=100%,BRAN 解析为 `2013286704/476`。

## T003.5 — 部署测试旁证(2026-06-13 凌晨,release 包 aps250160_0001)

监控 release 包重新部署任务 `e62554dc` 期间获得:

- **CATA 部分解析验证通过**(用户指定的部署测试目标):
  `gen-cata-closure 完成: 7 个 CATA 库 / seeds=2901 / visited=898 /
  missing=5 / rounds=6`;manifest 对齐解析计划 `CATA files 0 -> 7,
  covered_dbnums=[7000,7001,7014,7320,250193,250700,250701]`——
  按需闭包而非全量。生成成功(`manifest_250160.json` 写出,
  sidecar `job_done exit_code=0`)。
- **新缺陷发现**:生成 01:15 成功后走了"HTTP 终态轮询失败→websocket
  终态事件兜底"路径,随后 DeployManagedSite 任务**悬挂**(Running 至
  02:50+ 不进入 start 阶段,无活动子进程),且 admin 任务不支持取消,
  站点被锁死;重启 web_server 后任务对账为 Failed。
  建议另开 spec:终态兜底路径的管线续行 + 任务取消能力。

## T004 — `--compare-rvm` CLI 与三层对比引擎

- 新模块 `src/rvm_compare.rs`;CLI 参数:dbnum/root-refno/容差/报告目录。
- L1 成员清单(含 TUBE↔tubi_relate 顺序对齐);
- L2 原语参数级(类型映射表 + 容差);
- L3 空间级(平移/旋转/AABB IoU);
- JSON 报告 + 控制台摘要;退出码 0/1。

验收:

- `cargo check --features rvm-import` 通过;
- 对样本 BRAN 能产出三层完整报告。

实测(2026-06-13 03:12,离线 smoke,RVM 侧为未解析基线数据):

- `--compare-rvm --dbnum 250160 --root-refno 2013286704/476 --parquet-dir <站点包>`
  跑通:Parquet 三表读取、owner 链子树、JSON 报告、差异退出码全部工作;
- gen 子树 = **8 个成员**(恰好等于 RVM 11 成员中有几何的 8 个:
  FLAN×4/REDU/TEE/ELBO/VALV;3 个 GASK 零几何不入 instances)、
  root BRAN 的 `gen_tubi_segments=5`——两侧数据口径自洽;
- 当前 extra_in_gen=8 / matched=0 是**预期值**(基线 RVM 数据 resolved=0,
  无法按 refno join);待 T003 在线解析导入后复跑应转为 matched=8;
- RefnoEnum::from_str("2013286704/476") 与 instances.parquet `refno_u64`
  编码一致(子树/TUBI 查询命中实证)。

## T005 — 样本全链路验收(站点 quicktest-250160-8080)

实测(2026-06-13 03:20,站点库重启后在 8023):

- **T003 在线解析:resolved=15 / unresolved=0**——4 个命名元素走
  `surreal_name`(BRAN 正确解析为 `2013286704_476`),11 个未命名成员走
  `default_name_rule`,与站点库子序列逐一吻合(477~485/1325/1326)。✅
- **compare 复跑(matched 视角)**:matched=8、extra_in_gen=0、
  noun_mismatch=0——成员/类型级**完全一致**。✅
- **missing_in_gen=1 为口径差异**:BRAN 自身(RVM 把 2 段 TUBE 几何挂
  BRAN 组,gen 侧 BRAN 不入 instances,管段在 tubings.parquet 5 段)。
- **AABB 全员超 1mm 容差,但结构分析表明是全局坐标基准平移**:
  所有成员中心偏移 ≈ (+858272, -874377, -301000) mm,x/y 散布仅
  75/92mm、z 除 VALVE 外 <1mm——RVM 导出基准与 gen world 原点差一个
  常量,非几何错位。扣除平移后尺寸差 90~320mm(L1 LOD 网格 AABB vs
  精确原语包围盒的预期量级);**VALVE z 向 861mm 为唯一显著外点**
  (候选原因:手轮/执行机构几何归属),待单独核查。
- 后续增强(Backlog):compare 增加 `--auto-align`(中位数平移自动
  对齐后再比)、TUBE 段空间并集对比、VALVE 外点定位。

**VALVE 外点定位结论(2026-06-13 03:40,`analyze_valve.py`)**:
不是生成缺陷——RVM 侧 VALVE 的 **12 个原语 bbox_world 全部退化为点**
(z_size=0,组级 AABB 失真为 1mm 点),而 gen 侧 VALVE AABB
(320×320×862mm)是合理阀体+手轮形状。根因在导入端:`rvm_import.rs`
直接采信 rvm-rs 的 `geometry.bbox_world`,该值对部分带 transform 的
原语未展开。其余成员 90~320mm 的"尺寸差"同源(RVM bbox 普遍偏小)。
**修复方向**:导入端弃用 rvm-rs bbox_world,改由原语参数 + transform
自算包围盒(`rvm_obj_export.rs` 已有 11 种原语 mesh 化代码可复用)。
**最终判定强化**:成员/类型级完全一致;几何级两项"差异"
(全局基准平移、AABB 尺寸差)均定位为对拍链路自身问题,
**未发现生成端缺陷**。

**bbox 修复实施与缩放根因(2026-06-13 04:10 续)**:

1. 导入端已改为 mesh 化自算 AABB + 退化盒落 NULL
   (`rvm_import.rs::compute_payload_aabb`);
2. 深挖发现真根因:**RVM PRIM 矩阵旋转部分按米制存储(列长≈0.001),
   原语参数与平移为毫米**——直接相乘把几何坍缩成准点(所有成员的
   rvm-rs bbox_world 全部如此,先前各成员 90~320mm"尺寸差"实为
   gen 盒尺寸 vs 准点)。`rvm_obj_export.rs::parse_transform` 已加
   缩放检测归一(列长 ∈[5e-4,2e-3] 时旋转列 ×1000);
3. 修复后**尺寸差收敛到 0~27mm**(FLANGE 0~13.6、REDU 0~26、
   FLAN2/3/4 12.5),**形状级一致性得证**;TEE/ELBO/VALVE 残差
   54~358mm 与中心定位分裂(FLANGE 组与其余成员呈两组不同基准平移)
   暴露 rvm-rs **层级变换组合**问题(导入 walk 只累加 group.translation,
   未组合 CNTB 旋转;不同 PRIM 矩阵单位也不一致)——属上游
   `rvm-rs`(D:/work/plant-code/rvmparser/rvm-rs)工作项,另行处理;
4. L3 中心定位对比在 rvm-rs 层级矩阵修复前不可判;
   尺寸(形状)级与成员/类型级结论维持:**生成端无缺陷**。

- 站点 surreal(8022)在线,基于 2026-06-12 22:30 成功生成的数据
  (tubi_relate=16)。
- 重导入样本(带解析)→ `--compare-rvm` → 分析差异:
  - L1 missing/extra=0 或差异落入 Obstruction/Insulation 豁免桶;
  - L2/L3 容差内,或逐项给出可解释差异(真实生成缺陷记为后续 spec 候选);
  - TUBE 段与 tubi_relate 对齐验证。
- 报告样例归档 spec 目录。

验收:

- 验收数据回填本文件;复跑同输入同输出。

## T005.5 — 821 ZONE 案例:首个生成端缺陷实锤(2026-06-13 09:45)

用户报告 `/03SKID3`(ZONE `2013286704/821`,样本 `2013286704_821.rvm`)
生成结果与 E3D 截图不符(缺两个料仓设备、部分钢梁)。对拍 + 日志定界:

**对拍工具增强**(随本案例落地,已验证):

- 嵌套默认命名解析(`SNOUT 1 of TMPLATE 1 of EQUIPMENT /x`):owner 描述
  整名缓存命中 + 命名 owner 提取,遍历序父先子保证 owner 先解析;
- noun 多候选匹配(站点库截断名长度不定:TMPL/SUBE=4、GENSEC/JLDATU=6、
  SPINE=5):{映射短名, 全名, 截6, 截4} 四候选;
- 821 样本解析率:53/127 → **127/127(100%)**。

**生成端缺陷(missing=35 重新生成后复现,确定性)**:

- **EQUI 模板几何全缺**:EQUIP1/EQUIP2 的 `TMPL` 下 CYLI(906/998)、
  DISH(907/999)、SNOU(908/1000)、SUBE→TMPL→PYRA(941/942/1033/1034)
  及 NOZZ(852/909/944/1001)共 14 项在 instances.parquet 无行;
- 生成日志证据:`[Cate:...]` 批次只处理了 FIXING/GENSEC/EQUI/SUBE/
  PLOO/NOZZ/ELCONN;**CYLI/DISH/SNOU/PYRA 虽在目标 Noun 列表却入口
  查询为 0 个**——它们是 EQUI→TMPL 下的深层参数化几何,按 target_type
  的入口 roots 查询不可见;EQUI 作为 root 处理时只生成了部分直属件,
  未递归 TMPL 模板几何路径;
- 残余 17 项 JLDATU/PLDATU(datum 定位辅助件)不在目标 Noun 列表,
  属有意排除(可豁免,RVM 含其标记几何);4 个 BRAN 为既知口径差异。

**修复方向(建议开 spec 010)**:`cata_model` 的 EQUI/CATE 路径需覆盖
TMPL 子树参数化几何(CYLI/DISH/SNOU/PYRA…),或 IndexTree 入口查询
增补"EQUI 模板几何"的发现路径;以 821 ZONE 对拍 missing=0(豁免桶外)
为验收线。

## T006 — 文档与变更记录

- `CHANGELOG.md` 记录 spec 009;
- CLI help/README 增加 `--compare-rvm` 用法与样本命令
  (注意样本文件名含空格,需引号);
- Backlog:批量对拍调度、ATT 属性对比、网格级 diff。

验收:

- spec/plan/tasks 与实现一致,新人可按命令复现对拍。
