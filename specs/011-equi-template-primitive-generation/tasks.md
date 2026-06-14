# Tasks: EQUI/TMPL 模板原语生成补齐(spec 011)

## T001 — 固化 821 ZONE 失败样本与目标 refno 集

- 记录样本路径、站点、dbnum、root refno。
- 从 spec 009 对拍报告回填豁免桶外 missing 清单。
- 固化首批追踪 refno:
  `906/907/908/941/942/998/999/1000/1033/1034` 和
  `852/909/944/1001`。

验收:

- 本 spec 中的目标集合与 RVM 对拍 missing 明细一致。
- 后续诊断/测试均引用同一份目标集合。

实测(2026-06-13):

- `dump_tree_node` 已确认 `821/904/906/907/908/941/942/998/999/1000/1033/1034`
  均存在于 release 站点 `250160.tree`。
- `906/907/908/941/942/998/999/1000/1033/1034` 均能追溯到
  `2013278512/0 -> 2013286704/11 -> 2013286704/821` 祖先链。

## T002 — 增强树诊断工具

- 扩展 `examples/dump_tree_node.rs` 或新增 `examples/diagnose_index_tree_generation.rs`。
- 输出每个目标 refno 的:
  - exists;
  - noun hash / noun name;
  - ancestor chain;
  - direct children;
  - 是否在 root subtree 中。

验收:

- 对 release 站点 `250160.tree` 一条命令可复现 T001 实测。
- 输出足够短,可直接贴入 spec 或 CI 日志。

实测(2026-06-13):

- `examples/dump_tree_node.rs` 已增强:
  - 打印 noun hash / noun name;
  - 打印默认生成目标集合的 grouped summary;
  - 对每个目标 refno 打印 category 与 grouped_hit。
- `cargo run --example dump_tree_node --features rvm-import -- <250160.tree> 2013286704 821 ...`
  编译运行通过。

## T003 — 诊断 IndexTree 第二阶段 grouped 输入

- 在诊断工具中复用 `TreeIndexManager.collect_target_refnos_grouped`。
- 输入 root=`2013286704/821`,nouns=默认 `get_entry_nouns` 结果。
- 对目标 refno 输出:
  - grouped 是否命中;
  - 命中的 noun hash / noun name;
  - 将归入 loop/cate/prim 哪一类。

验收:

- 修复前明确定位目标 refno 是否已进入 grouped。
- 若 grouped 未命中,能输出目标节点实际 noun,为修复提供证据。

实测(2026-06-13):

- `diagnostic_root=2013286704_821`,默认目标 noun 数 `59`,grouped refnos `49`。
- grouped summary 命中:
  - `CYLI:2`
  - `DISH:2`
  - `SNOU:2`
  - `PYRA:4`
  - `NOZZ:4`
- 目标 `906/907/908/941/942/998/999/1000/1033/1034`
  均为 `category=prim grouped_hit=true`。
- 结论:缺失不在 TreeIndex/grouped/分类入口,下一步查 PRIM 页面输入之后。

## T004 — 诊断页面输入与处理器输出

- 在 LOOP/CATE/PRIM 页面处理边界添加临时 `[spec011]` 或正式 debug 事件。
- 追踪目标 refno 是否进入:
  - `process_loop_refno_page`;
  - `process_cate_refno_page`;
  - `process_prim_refno_page`;
  - `ShapeInstancesData` sender。
- 诊断结束后清理临时日志,或保留为受 flag 控制的 debug 输出。

验收:

- 修复前能明确目标 refno 是“未进入页面输入”还是“页面输入后无输出”。
- 不打印全量模型数据。

实施进展(2026-06-13):

- `prim_model.rs` 新增 env-gated 探针:
  `AIOS_SPEC011_REFNOS=2013286704/906,...`。
- 探针只在命中目标 refno 时输出:
  - `[spec011][prim] page_input ... watched_hits=[...]`
  - `skip=world_transform_missing`
  - `skip=create_csg_shape_failed`
  - `skip=invalid_inst_geo`
  - `inserted type=...`
- 正常生成未设置环境变量时无额外日志。
- `cargo check --features rvm-import --bin aios-database` 通过(仅依赖仓既有 warning)。
- 后续探针进一步确认:
  - 目标 `CYLI/DISH/SNOU/PYRA` 均进入 `page_input`。
  - 修复前全部在 `build_inst_geo_from_shape` 前后因
    `shape_check_valid_false` / `invalid_inst_geo` 被跳过。
  - 参数摘要显示标准尺寸字段为 0:
    `CYLI DIAM/HEIG=0`, `DISH DIAM/HEIG=0`,
    `SNOU HEIG/DTOP/DBOT=0`, `PYRA HEIG/XBOT/...=0`。

## T005 — 修复丢失边界

- 若 grouped 未命中:修 TreeIndex grouped 查询或树 meta noun 构建。
- 若分类/页面输入丢失:修 `get_entry_nouns` / noun 分类 / `IndexTreeConfig` 过滤。
- 若处理器输入后丢失:修对应 LOOP/CATE/PRIM 参数读取或 silent skip。
- 若 writer/export 丢失:修 ShapeInstancesData 写入或 Parquet 过滤。

验收:

- 目标 `CYLI/DISH/SNOU/PYRA` refno 进入 `instances.parquet`。
- 无 `[spec011]` 临时日志残留。

实施进展(2026-06-13):

- 已确认第一层解析计划缺陷:
  - 目标 PRIM 的 `ORRF` 指向 `15207_*`。
  - `db_index.sqlite` 显示 `ref0=15207 -> dbnum=7015/acp7015_0001`。
  - `acp7015_0001` 是 `AvevaCatalogue` 的外部模板 `DESI` 库,旧 precise
    closure 只收口 `CATA`,因此 runtime manifest 未包含 `7015`。
- 已修复 closure:
  - `CataClosureConfig::precise()` 允许被引用的外部模板 `DESI` 入闭包。
  - 每个源 DESI 闭包排除自身 dbnum,避免把主设计库写入 manifest。
  - sync filter 对 manifest 覆盖到的任意 dbnum 做部分解析。
  - parse plan 对齐逻辑把 manifest-covered `DESI` 也纳入
    `included_db_files/auto_related_db_files`。
- 已确认第二层生成缺陷:
  - `acp7015_0001` 落库后,源 PRIM 标准尺寸仍为 0;raw PDMS 解析也为 0。
  - 实际模板参数在实例父级 `EQUI/SUBE.DESP` 和
    `TMPL -> DDSE -> DDAT(DKEY/DDPR)` 中。
  - 例如 `EQUI:2013286704_851 DESP=[1200,800,200,100,600,80,150]`,
    DDAT 映射 `HEIG<-DESP[1]`, `DIAM<-DESP[2]`,
    `DHEI<-DESP[3]`, `DRAD<-DESP[4]`, `SHEI<-DESP[5]`,
    `SDIA<-DESP[6]`。
- 已在 PRIM 生成期增加 TMPL 参数补齐:
  - `CYLI`: `DIAM=DIAM`, `HEIG=HEIG-DHEI-SHEI`。
  - `DISH`: `DIAM=DIAM`, `HEIG=DHEI`, `RADI=DRAD`。
  - `SNOU`: `HEIG=SHEI`, `DTOP=DIAM`, `DBOT=SDIA`。
  - `PYRA`: `XBOT/XTOP=STHI`, `YBOT=DIMD*2`(无 `DIMD` 时回退
    `SWID`),`YTOP=max(YBOT-SWID,STHI)`,`HEIG=SHEI`,
    `Z_OFFSET=SDIS+SHEI/2`。
- 探针复跑通过:
  - `906/907/908/941/942/998/999/1000/1033/1034`
    均输出 `template_params_applied ...`。
  - 全部输出 `inserted type=...`;未再出现 `shape_check_valid_false`。
- `NOZZ` 缺失已确认并修复:
  - 失败路径为 `NOZZ.CATR -> SPCO.CATR -> SCOM` 的 catalogue 引用链未在
    `resolve_desi_comp` fallback 场景下归一化,导致设计件自身被当作 `tubi_scom`
    求解,表现为 `is_tubi=true` 且 `gm_out=0/ngm_out=0`。
  - 新增 catalogue ref 归一化后,`2013286704/852` trace 输出
    `gm_in=4 gm_out=4`, `convert_geo=4/4`, `is_tubi=false`。
  - 821 Parquet 抽查确认 `852/909/944/1001` 均进入 `instances.parquet`,
    且每个 `NOZZ` 有 2 行 `geo_instances`。

IDA 旁证(2026-06-14,`D:\AVEVA\Everything3D3.1\core.dll.i64`):

- 当前 `user-ida-pro-mcp` active 实例为 `core.dll`。`core.dll` 明确包含
  `ATT_DESP/ATT_DDPR/ATT_DDDF/ATT_DKEY`,以及
  `NOUN_TMPL/NOUN_DDSE/NOUN_DDAT/NOUN_CYLI/NOUN_DISH/NOUN_SNOU/NOUN_PYRA`。
- `sub_5991200`(`DB_CopyManager::createHierarchy`)递归复制 primary-list
  成员;当成员 `type == NOUN_TMPL` 时,额外检查 `climb(...,NOUN_TMAR)` 后
  调整递归标志,说明原版对 `TMPL` 子树有特殊层级语义,不是只靠目标 noun
  入口查询。
- `sub_596EC70` 的 DB_Element 规则测试显示:`DESP` 是
  `vector<double>` 属性,可通过 `DB_Rule/DB_Expression` 与子元素属性联动;
  测试中 `executeRule(ATT_DESP)` 后断言 `DESP[0]` 变为 `1030`,
  修改子 `CYLI` 的 `DIAM` 后再变为 `2030`。这支持本实现按
  `DESP + DDAT/DDPR` 在生成期补齐模板 PRIM,但也提示长期应从字符串解析
  升级为更完整的表达式求值。
- `core.dll` 中有固定属性 `ATT_XBOT/ATT_XTOP/ATT_YBOT/ATT_YTOP`,
  但未见 `SWID/STHI/DIMD/SDIS` 固定属性;这些名称按现有样本证据视为
  `DDAT.DKEY` 参数键,生成期需映射到真实 PRIM 字段。

## T006 — 增加最小回归测试

- 优先添加 TreeIndex 小型树单测:
  `ZONE -> EQUI -> TMPL -> CYLI/DISH/SNOU` 从 ZONE root 查询必须命中 PRIM。
- 若根因在处理器或 writer,增加对应最小单测或 example smoke。

验收:

- 测试在修复前失败、修复后通过;若无法构造正确 seam,在本任务中记录原因。

实测(2026-06-14):

- 根因最终落在 PRIM 处理器的模板参数补齐,TreeIndex/grouped 已由 T003
  证明命中,因此本轮补充 `prim_model.rs` 的最小纯函数回归测试。
- 新增 4 个单测覆盖:
  - `DESP[n]` 为 1-based 索引,`DESP[0]`/越界返回 `None`;
  - `CYLI` 从模板参数补齐 `DIAM`,并按 `HEIG-DHEI-SHEI` 计算主体高度与 Z 偏移;
  - `DISH` 补齐 `DIAM/HEIG/RADI`,并按主体高度设置 Z 偏移;
  - `SNOU/PYRA` 补齐关键尺寸、偏移字段,缺少必需参数时拒绝生成。
- 验证命令:
  `cargo test --features rvm-import spec011 --lib`。
- 结果:4 passed / 0 failed。现有 `pdms-io-fork` 依赖仍输出既有 warning,
  本轮未新增失败。

## T007 — 821 ZONE 对拍验收

- 重跑 821 ZONE 生成。
- 重新导入 `2013286704_821.rvm`(identity resolved=100%)。
- 运行 `--compare-rvm` 生成 spec011 报告。

验收:

- 豁免桶外 `EQUI/TMPL` 模板 PRIM missing=0。
- 若仍有差异,逐项归类为真实缺陷、口径差异或 RVM 导入侧限制。

实测(2026-06-13):

- 821 root 重新生成完成,Parquet 导出:
  - `instances.parquet`: 943 行;
  - `geo_instances.parquet`: 1182 行;
  - `aabb.parquet`: 1019 行。
- 目标集合 Parquet 抽查:
  - `NOZZ`: `2013286704_852/_909/_944/_1001` 均存在,每个 2 行几何;
  - `CYLI/DISH/SNOU/PYRA`: `906/907/908/941/942/998/999/1000/1033/1034`
    均存在,每个 1 行几何。
- RVM compare 复验报告:
  `runtime/rvm-compare/rvm-compare-8647000551151633205-20260614-083451.json`。
  汇总从旧报告 `missing_in_gen=25` 降到 `21`,新增匹配 4 个 `NOZZ`。
- `EQUI/TMPL` 模板目标均为 `matched`:
  - `NOZZLE 1 of EQUIPMENT /03SKID3-EQUIP1`;
  - `NOZZLE 1 of TMPLATE 1 of EQUIPMENT /03SKID3-EQUIP1`;
  - `CYLINDER/DISH/SNOUT` in `/03SKID3-EQUIP1`;
  - `PYRAMID 1/2` in subequipment of `/03SKID3-EQUIP1`;
  - 同组 `/03SKID3-EQUIP2` 目标。
- 剩余 `missing_in_gen=21` 不属于本 spec 的 `EQUI/TMPL` 模板目标,主要落在
  BRAN/STRUCTURE/PLDATUM 类旧差异;AABB mismatch 仍为既有全局坐标基准差异。

## T008 — 476 BRAN 回归

- 复跑 `2013286704/476` 样本对拍。

验收:

- spec 009 的成员/类型级结论不回退。
- 不因 spec 011 修复引入新的 extra/missing。

实测(2026-06-13):

- 重新导入 `D:\work\plant-code\plant-model-gen\test_data\rvm\2013286704_476 .rvm`:
  `resolved=15 unresolved=0`。
- 复跑 `2013286704/476` 生成与 Parquet 导出成功,退出码 0。
  日志仍有既有 catalog 布尔加载告警(`2013286704_1038/_1039/_1042/_1043`),
  但模型写入与 Parquet 导出完成。
- compare 报告:
  `runtime/rvm-compare/rvm-compare-8647000551151632860-20260613-145148.json`。
  结果:
  - `matched=8`;
  - `extra_in_gen=0`;
  - `noun_mismatch=0`;
  - `missing_in_gen=1`,为 spec 009 已记录的 BRAN 自身 TUBE 挂组口径差异
    (`/03SKID1-PIPE-SUCTION/B1`,RVM 将 2 段 TUBE geometry 挂在 BRAN 组,
    gen 侧 BRAN 不入 `instances.parquet`,管段在 `tubings.parquet` 5 段)。
- 结论:spec 009 成员/类型级结论未回退;spec 011 的 catalogue ref 归一化未引入
  476 BRAN 的新增 extra/noun mismatch。

## T009 — 文档与归档

- 回填 `spec.md` / `plan.md` / `tasks.md` 的最终根因、修复点、验收数据。
- 更新 `CHANGELOG.md`。
- 若诊断工具保留,补 CLI/example 用法说明。

验收:

- 新人可按本 spec 复现:失败样本 -> 修复验证 -> 回归对拍。

实测(2026-06-14):

- `spec.md` 已回填 Final Result:根因、修复点、821 对拍、476 回归和剩余差异边界。
- `plan.md` 已回填 Phase 3 复验命令与结果,明确 `missing_in_gen=21`
  均为非本 spec 目标或既有口径差异。
- `CHANGELOG.md` 已新增 spec 011 条目,覆盖 closure、模板参数补齐、NOZZ
  catalogue ref 归一化、821/476 验收数据。
- 诊断入口保留为 `AIOS_SPEC011_REFNOS` 环境变量门控探针,正常生成不输出
  `[spec011]` 日志;复现时可按 T004/T007 的命令和目标 refno 集合打开。
- 复验报告 JSON 属运行产物路径
  `runtime/rvm-compare/rvm-compare-8647000551151633205-20260614-083451.json`;
  当前仓库未提交该 `runtime/` 产物,文档记录报告名与关键统计用于追溯。

## T010 — 后续分流:datum marker 生成路径

spec 011 收口后,821 剩余 `missing_in_gen=21` 中有 17 个为
`JLDATU/PLDATU` datum 表示差异。本项不改写 spec 011 的完成口径,仅记录
推荐下一步已开始把 datum 从"解释/豁免"推进到可生成标记。

实施进展(2026-06-14):

- `index_tree_mode.rs` 将 `JLDATU/PLDATU` 作为 datum marker noun 追加到
  PRIM 发现集合,覆盖 grouped 分类、入口 noun 列表和 descendants 查询。
- `prim_model.rs` 对 `JLDATU/PLDATU` 绕开普通 CSG 参数构造,改为生成
  X/Y/Z 三根短圆柱 marker,并走现有 `ShapeInstancesData` 写入链。
- 新增单测 `datum_marker_geos_emit_three_visible_positive_axes`,断言 datum
  marker 输出 3 个 visible/positive inst,且三轴局部偏移稳定。
- 验证命令:
  `cargo test --features rvm-import fast_model::gen_model::prim_model::tests --lib`。
- 结果:5 passed / 0 failed。现有 `pdms-io-fork` 依赖仍输出既有 warning,
  本轮未新增失败。

下一步验收:

- 复跑 821 ZONE 生成 + `--compare-rvm`,确认 17 个 datum 是否从
  `missing_in_gen` 消失或转为可解释的 AABB/表示差异。
- 若 datum 进入 Parquet 后视觉尺寸/方向与 RVM 差异过大,另开专门 spec
  调整 marker 尺寸与对拍豁免规则,不要回滚已完成的 EQUI/TMPL 修复。
