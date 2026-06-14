# Implementation Plan: EQUI/TMPL 模板原语生成补齐(spec 011)

## Approach

先把缺失问题收敛成可复跑的边界诊断,再做最小修复:

1. 以 `ZONE 2013286704/821` 为唯一失败样本,固定目标 refno 集合;
2. 依次检查 TreeIndex 可达性、IndexTree 第二阶段 grouped 输入、LOOP/CATE/PRIM
   页面输入、处理器输出、writer/export 落盘;
3. 只修导致 `EQUI -> TMPL` 与 `SUBE -> TMPL` 参数化几何丢失的边界,不扩大目标 noun;
4. 用 `--compare-rvm` 回归 821 ZONE,再复跑 476 BRAN 样本确保 spec 009 不回退。

## Phase 0 — 失败样本与目标集合固化

目标样本:

- RVM:`D:\work\plant-code\plant-model-gen\test_data\rvm\2013286704_821.rvm`
- 站点:`quicktest-250160-8080`
- dbnum:`250160`
- root refno:`2013286704/821`

首批必须追踪的目标 refno:

- `CYLI`: `2013286704/906`, `2013286704/998`
- `DISH`: `2013286704/907`, `2013286704/999`
- `SNOU`: `2013286704/908`, `2013286704/1000`
- `PYRA`: `2013286704/941`, `2013286704/942`, `2013286704/1033`, `2013286704/1034`
- `NOZZ`: `2013286704/852`, `2013286704/909`, `2013286704/944`, `2013286704/1001`

现有证据:

```powershell
cargo run --example dump_tree_node --features rvm-import -- `
  "D:\work\plant-code\plant-model-gen-cata-closure\dist\package\Plant3D-AIOS-win-x64\release\runtime\admin_sites\quicktest-250160-8080\output\AvevaPlantSample\scene_tree\250160.tree" `
  2013286704 821 904 906 907 908 941 942 998 999 1000 1033 1034
```

结果:这些节点存在且从 `821` 可达,不是 `.tree` 缺节点或 orphan。

## Phase 1 — 生成边界诊断

新增一个轻量诊断入口,优先放在 `examples/` 或 CLI debug mode:

```text
diagnose-index-tree-generation <tree-file> <root-refno> <target-refnos...>
```

输出结构:

- `tree`: exists / ancestor_chain / direct_children;
- `grouped`: `collect_target_refnos_grouped` 是否命中目标 refno,以及 noun hash/name;
- `category`: 最终归入 loop / cate / prim 的集合;
- `page_input`: 目标 refno 是否进入 `process_loop_refno_page` / `process_cate_refno_page` /
  `process_prim_refno_page`;
- `writer`: 是否产生 `ShapeInstancesData.refno`;
- `parquet`: 是否存在于 `instances.parquet`。

诊断原则:

- 用固定 refno 白名单输出,不要打印全量树。
- 调试日志统一带前缀 `[spec011]`,修复收口前全部清理或转为正式诊断输出。
- 优先复用 `TreeIndexManager`,避免回退 SurrealDB 递归查询制造第二套语义。

## Phase 2 — 根据边界结果修复

### 情况 A: grouped 未命中

检查 `collect_target_refnos_grouped` 的 noun 过滤与 `TreeIndex` meta.noun:

- 若目标节点 noun hash 正确但未分组,修 TreeIndex grouped 查询逻辑;
- 若目标节点 noun hash 异常,回到解析期 `TreeNodeMeta.noun` 构建;
- 加测试:构造小型树 `ZONE -> EQUI -> TMPL -> CYLI/DISH/SNOU`,
  root 查询应命中三个 PRIM 节点。

### 情况 B: grouped 命中但分类/页面输入丢失

检查 `GNERAL_PRIM_NOUN_NAMES` 与 `get_entry_nouns` / `IndexTreeConfig` 过滤:

- 明确 `CYLI/DISH/SNOU/PYRA` 归 `prim_refnos`;
- `NOZZ` 若走 CATE 或普通件路径,保持既有分类,只补漏处理路径;
- 避免用字符串配置覆盖默认集合导致类别缺项。

### 情况 C: 页面输入命中但处理器无输出

检查对应处理器:

- `prim_model` 是否能读取目标 refno 的参数;
- `query_provider` 是否因 owner/DB meta 映射跳过目标;
- world transform / SJUS 等依赖缺失时是否 silent skip。

修复应让失败显式进入 `cache_miss_report` 或诊断输出,不再无声漏掉。

### 情况 D: 处理器有输出但 writer/export 缺失

检查 `ShapeInstancesData` 写入链:

- writer 是否因 `geo_type` / `aabb` / transform NaN 跳过;
- Parquet export 是否过滤了这些 refno;
- relation store / SurrealDB 是否有行但导出遗漏。

## Phase 3 — 回归对拍

修复后运行:

```powershell
cargo run --features "rvm-import,model-writer-drain" --bin aios-database -- `
  -c "runtime\admin_sites\quicktest-250160-8080\DbOption-generate" `
  --compare-rvm `
  --dbnum 250160 `
  --root-refno "2013286704/821" `
  --parquet-dir "runtime\admin_sites\quicktest-250160-8080\output\AvevaPlantSample\parquet\250160"
```

验收:

- `EQUI/TMPL` 目标 PRIM 在 `instances.parquet` 中存在;
- `missing_in_gen` 中不再出现 `CYLI/DISH/SNOU/PYRA/NOZZ` 的目标 refno;
- datum 和 BRAN 口径差异保留在解释/豁免桶内。

实测结果:

- 821 复验报告:
  `runtime/rvm-compare/rvm-compare-8647000551151633205-20260614-083451.json`。
- 本 spec 目标全部 `matched`:
  `NOZZ` 4 个、`CYLI/DISH/SNOU` 6 个、`PYRA` 4 个。
- 总 `missing_in_gen=21`,均为非本 spec 目标或既有口径差异。

再复跑 `2013286704/476`:

- matched / extra / noun mismatch 不回退;
- RVM 导入侧 identity 解析仍为 100%。

实测结果:

- 重新导入 `2013286704_476 .rvm`: `resolved=15 unresolved=0`。
- compare 报告:
  `runtime/rvm-compare/rvm-compare-8647000551151632860-20260613-145148.json`。
- `matched=8`, `extra_in_gen=0`, `noun_mismatch=0`;
  `missing_in_gen=1` 为 spec 009 已记录 BRAN/TUBE 挂组口径差异。

## Risks

- R1:目标节点虽然在 `.tree` 可达,但其参数读取依赖父级 template 状态。缓解:
  诊断输出同时包含 ancestor chain 与处理器输入依赖。
- R2:`NOZZ` 既可能是独立 CATE/普通件,也可能依赖模板上下文。缓解:先把
  `CYLI/DISH/SNOU/PYRA` 作为硬验收,`NOZZ` 单独定界后处理。
- R3:修复入口集合可能扩大生成范围。缓解:用目标 refno 和 476 样本做回归,
  不改 datum 目标集合。

## Rollback

诊断入口是增量工具;生成修复应保持小范围改动。若回归失败,回滚修复代码并保留
spec 诊断结论作为下一轮输入。
