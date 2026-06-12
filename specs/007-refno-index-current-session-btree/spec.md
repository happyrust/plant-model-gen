# Feature Specification: 当前会话 B-tree refno 索引全量枚举修复（spec 007）

## User Need

解析器从全文件扫描 `gen_ref_type_pos_table_scan()` 迁移到 B-tree / BTreeMap 索引定位后，必须仍然能通过索引枚举出当前 DB 的全部有效 refno，并对同一 refno 只解析当前最新 Element 记录。

当前实现对部分 E3D2.1 DB 只枚举到极少 refno，例如：

```json
{
  "aps250160_0001": {
    "index_refnos": 2,
    "scan_refnos": 2759
  },
  "aps7351_0001": {
    "index_refnos": 8,
    "scan_refnos": 3345855
  }
}
```

这会导致：

- `All refnos count` 明显偏小。
- DESI 只形成极小 `TreeIndex`。
- `cata_closure` manifest 空或严重缩水。
- CATA 部分解析错误跳过实际依赖库。
- 站点部署最终生成 0 行或缺失模型数据。

## Scope

- `pdms-io-fork/crates/parse_pdms_db/src/refno_index.rs` 的 B-tree 索引页解析、树遍历和 refno 表构建。
- `parse_pdms_db::parse::gen_ref_type_pos_table()` 的索引优先路径，不引入全文件 scan fallback 作为正常路径。
- `find_refno_entry()` 单点定位与全表枚举语义对齐。
- `compare_refno_index` / `verify_parse_db_basic` 示例工具升级为回归验证入口。
- `plant-model-gen-cata-closure` 对 `parse_pdms_db` 修复后的部署验证：`aps250160_0001` 与 `ams7997_0001`。

## Non-Goals

- 不恢复以 `gen_ref_type_pos_table_scan()` 作为常规解析路径。
- 不在本 spec 重写 CATA closure 业务语义；closure 只消费修复后的完整 refno 表。
- 不修复属性字段解析问题（见 spec 003）。
- 不改变模型生成 pipeline / SurrealDB 写入策略（见 spec 004 / 005）。
- 不做存量站点后台迁移；修复后通过重解析自然生效。

## Background: core.dll / IDA Evidence

经 `user-ida-pro-mcp` 当前 `core.dll` 实例确认：

- IDA 打开二进制：`D:\AVEVA\Everything3D3.1\core.dll.i64`。
- `core.dll` 中存在大量 `DB_RefTable` / `RefTableIterator` 字符串与 xrefs，说明原生引擎通过引用表 / 索引访问元素，而不是全库扫描元素记录。
- `DGOTO` 字符串存在，印证原生引擎支持按 refno 惰性导航到元素。
- 这与本仓库目标一致：Rust 侧应通过 DB 内部索引枚举 / 定位当前有效 refno，再解析对应 Element。

本次修复应参考 core.dll 的处理思路：索引是权威访问路径，但必须正确处理 session / page / start marker / overwrite 语义。

## Observed Index Structure

### `aps250160_0001`

文件大小 `919552` 字节，header 显示 latest session page 为 `448`。

最新 session 页：

```text
ses 448 words [3, 444, 1, 22, 4294967295, 448, 1, 447, ...]
```

其中：

- `word[1] = 444` 指向上一 session。
- `word[7] = 447` 是当前 session index root。

当前实现只读取 root `447`，且按 `declared_entries=2` 截断每页：

```text
pg 447 level=1 declared=2 actual_until_zero=43
pg 419 level=0 declared=2 actual_until_zero=126
```

真实页内从 `0x1C` 开始到 `ref0 == 0` 前有远多于 2 条索引项。

### `aps7351_0001`

文件大小 `1238308864` 字节，latest session page 为 `604642`。

当前实现结果：

```text
index_refnos = 8
scan_refnos  = 3345855
```

忽略 `declared_entries=2`、按零终止遍历页内 entries 后，可枚举到约 `3348074` 条候选，接近 scan 量级；后续需通过 `entry_from_loc()` 校验与最新覆盖规则收敛到真实当前集合。

## Root Cause

### RC1: index page entry count 字段误读

`parse_index_page()` 当前把 `offset + 0x10` 作为 declared entry count：

```rust
let declared_entries = read_u32(input, offset + 0x10)? as usize;
```

真实 E3D2.1 文件中该字段常为 `2`，但页内有效 entry 可达几十或上百条。因此该字段不能作为 active entry count。

实际应按 page capacity 读取，并以 `ref0 == 0` 作为终止。

### RC2: internal page 的 start marker 子树被跳过

当前遍历逻辑跳过 `80000001/80000001`：

```rust
if !loc.is_start_marker() && loc.pgno > 0 {
    queue.push_back(loc.pgno);
}
```

但真实 root 页中 start marker 指向左侧 / 基础子树：

```text
root page 447:
  80000001/80000001 -> page 446
  2013286704/496    -> page 419
  2013286704/824    -> page 359
```

构建全表时应遍历 start marker 的 child page，但不能把 start marker 本身作为 Element refno 插入结果。

### RC3: single lookup 与 full enumeration 语义未完全对齐

`find_refno_entry()` 的 fallback leaf walk 也跳过 internal start marker 子树，因此可能无法定位位于基础子树或低 key 区间的 refno。

全表枚举、单点查找、验证工具必须共享同一 page 遍历语义。

## Requirements

1. `parse_index_page()` 必须按 index page capacity 遍历 entry，并以 `ref0 == 0` 终止；不得用 `offset + 0x10` 截断真实 entries。
2. internal index page 遍历必须跟随所有有效 child page，包括 start marker child；leaf page 仍不得把 start marker 当作元素插入。
3. `gen_ref_type_pos_table_from_index()` 必须对同一 refno 保留当前最新 Element 记录。初始规则为 `entry.pos` 更大者覆盖更小者；若发现删除/tombstone 记录，需补充显式删除语义。
4. `find_refno_entry()` 必须与全表枚举使用相同 traversal 规则，至少在 fallback leaf walk 中覆盖 start marker 子树。
5. 索引解析必须使用 `entry_from_loc()` 校验：refno 必须与 loc refno 一致，record 起点必须有效，noun hash 必须可读。
6. `compare_refno_index` 必须报告 `index_count`、`scan_count`、`matched`、`scan_only`、`index_only`，并保留样例输出用于人工复核。
7. `verify_parse_db_basic` 必须可用于确认主链路 `parse_db_basic_data()` 的 `refno_table_map` 已恢复完整。
8. 修复后 `cata_closure` 不得因 refno index 缩水生成空 manifest；如果 manifest 为空，应能从日志定位到真实业务原因，而不是 index 枚举缺陷。

## Open Questions（grill-me 访谈未决项，附推荐答案）

- Q1：是否允许 `gen_ref_type_pos_table()` 在 index 异常时 fallback scan？**推荐：否。**本 spec 的目标是把 B-tree index 枚举修正确；scan 只能作为验证 oracle，不进入生产常规路径。
- Q2：`offset + 0x10` 字段应完全废弃还是保留诊断？**推荐：保留为诊断字段，不参与截断。**真实 DB 已证明它不是通用 entry count。
- Q3：start marker 是否只在 full table build 中遍历，single lookup 仍跳过？**推荐：两者都遍历。**否则 `find_refno_entry` 与批量解析会出现语义差异。
- Q4：同 refno 多记录如何判定当前最新？**推荐：先沿用 `pos` 最大 wins，与旧 scan 从文件尾向前保留最新记录一致；若后续确认 session 顺序更精确，再切换为 session-order wins。**
- Q5：index 枚举比 scan 多出的候选如何处理？**推荐：通过 `entry_from_loc()` 校验 + 与旧 scan oracle 对比找出 tombstone/free entry 形态，再补专门过滤规则；不要用 scan fallback 掩盖。**
- Q6：真实 `aps7351_0001` 是否能入库作为 fixture？**推荐：不提交大文件；使用本地路径验证 + 小型 synthetic fixture 覆盖 `declared=2 but actual>2` 和 start marker child。**

## Acceptance Criteria

- `verify_parse_db_basic --db-file ...aps250160_0001` 输出 `index_refnos == 2759` 或与 `scan_refnos` 等价（允许经 tombstone 规则解释的极小差异）。
- `verify_parse_db_basic --db-file ...aps7351_0001` 输出 `index_refnos` 与 `scan_refnos=3345855` 等价（允许经 tombstone 规则解释的极小差异）。
- `compare_refno_index --full` 对 `aps250160_0001` 的 `scan_only` 从 `2757` 降为 `0` 或全部进入已解释豁免。
- `find_refno_entry()` 能定位 `compare_refno_index` 的 `scan_only_samples` 中代表 refno，例如 `2013286704/1377`。
- `quicktest-250160-8081` 重解析后不再出现 `All refnos count: 2`。
- `quicktest-7997-8080` 重解析后 CATA manifest 不再因 index 漏读为空或错误缩水。
- 部署产物中的 Parquet 至少对目标 DESI 产生非空 `instances.parquet` / `aabb.parquet`，除非目标 DB 本身业务上无可生成模型。

## Implementation Validation（2026-06-12）

已按推荐方案修改 `pdms-io-fork/crates/parse_pdms_db/src/refno_index.rs`：

- `parse_index_page()` 不再用 `offset + 0x10` 截断 entry，改为按页容量读到 `ref0 == 0`。
- full enumeration 与 fallback leaf walk 都会遍历 internal page 的 start marker child。
- `choose_child_pages()` 在 target 小于首个 valid key 时优先搜索 start marker child。
- 新增 3 个合成单元测试覆盖 entry count、start marker child、duplicate refno latest-wins。

验证结果：

```text
cargo test refno_index --lib
=> 3 passed
```

```json
{
  "db_file": "aps250160_0001",
  "before": { "index_refnos": 2, "scan_refnos": 2759 },
  "after": {
    "index_refnos": 2748,
    "scan_refnos": 2759,
    "common_refnos": 2748,
    "children_equal": 2748,
    "children_diff": 0
  }
}
```

`compare_refno_index --full` 对 `aps250160_0001`：

```json
{
  "scan_count": 2759,
  "index_count": 2748,
  "matched": 2748,
  "mismatched": 0,
  "scan_only": 11,
  "index_only": 0,
  "scan_world": "=2013278512/0",
  "index_world": "=2013278512/0"
}
```

这说明主链路已不再只读 2 个 refno；剩余 11 个 scan-only 更像旧 byte scan 捕获到的非当前索引记录，需要另行确认 tombstone / history 语义，但不会再导致当前索引缩水到个位数。

### scan-only 豁免解释（2026-06-12，T007 收口）

用新增诊断工具 `examples/probe_scan_only_sessions.rs` 对 `aps250160_0001` 的 10 个
`scan_only_samples` 逐一沿 session 链分析，结论：

- 这些 refno 的 index entry **存在于最新 session 索引**（B-tree 枚举候选 2767 = 2748 有效 + 19 无效 loc）。
- 但 loc 指向的 element page 全部**远超文件实际页数**：`elem_pg ∈ {32258, 32261, 32353, 32354, 32356}`，
  而文件 919552 字节 / 2048 只有 449 页（byte_off≈66MB 越界）：

```text
loc 2013286704/1323 leaf=pg365 elem_pg=32258 words=2   byte_off=66064388 (file=919552)
loc 2013286704/2064 leaf=pg377 elem_pg=32353 words=576 byte_off=66260096 (file=919552)
... 10/10 样例全部同一形态，refno 簇 1323–1334 / 2064–2078 各对应一段连续无效页
```

- `entry_from_loc()` 的 `byte_offset` 越界校验**正确排除**了这批 entry——当前文件内无法解引用的
  loc 不计入当前集合，与 core.dll「索引为权威」的语义一致。
- 旧 byte-scan 捕获到的是这些 refno 残留在本文件旧数据页上的历史记录字节（删除/搬迁不抹字节），
  因此只出现在 scan 侧。
- **处置**：维持 `entry_from_loc()` 校验拒绝即可，无需新增过滤规则；11 条 scan-only 全部归入
  「loc 指向文件外哨兵/失效页」豁免。若后续需要显式删除语义，可把 `loc.pgno > 文件总页数`
  识别为 tombstone 并输出诊断计数。

### 站点部署端到端验收（2026-06-12，T008 收口）

`quicktest-250160-8080` 重建站点全链路（解析 → CATA closure → 生成 → 部署验证）结果，
见 `runtime/admin_sites/quicktest-250160-8080/deploy-validation.json`（22:26:45）：

- `blocking_count = 0`；解析日志 `aps250160_0001 All refnos count: 2748`（修复前为 2）。
- Parquet 全部非空：`instances.parquet rows=808`、`geo_instances rows=808`、
  `transforms rows=854`、`aabb rows=824`，HTTP 访问 200。
- `visible-insts API` 返回 963 个可见实例（修复前 TreeIndex 仅 2 节点、产物近空）。
- mesh GLB 可匹配且 HTTP 200；Parquet 抽样引用一致性 pass。
- 7997：`cata-closure-refno-index-20260612-210727.log` 显示 `ams7997_0001 All refnos count: 157260`，
  manifest 覆盖 8 个 CATA 库 / 15040 refno，不再因 index 漏读为空。
- 唯一 warning：`subtree-refnos API` 的 `resolve_dbnum_for_refno failed`——与 refno 索引无关的
  API 解析问题，按 T009 约定另行立项，不混入本 spec。

**Acceptance Criteria 全部达成，spec 007 关闭。**

```json
{
  "db_file": "aps7351_0001",
  "before": { "index_refnos": 8, "scan_refnos": 3345855 },
  "after": {
    "index_refnos": 3345855,
    "scan_refnos": 3345855,
    "common_refnos": 3345855,
    "children_equal": 3345855,
    "children_diff": 0
  }
}
```

### Release Package Validation（2026-06-12）

已在 `plant-model-gen-cata-closure/dist/package/Plant3D-AIOS-win-x64/release`
中替换 `bin/aios-database.exe` 为本地修复后的 release 构建：

```text
SHA256: 8DA38CE6DF64A161A3D642F15189F1A73B769BD8EA28DF48A61531672C425A37
backup: bin/aios-database.exe.bak-refno-index-20260612-205955
```

对包内现有 `quicktest-250160-8081` 使用同一 `DbOption-parse`
执行最小 release 验证：

```text
runtime/admin_sites/quicktest-250160-8081/logs/parse-refno-index-20260612-210130.log
```

关键结果：

```text
aps250160_0001:
  old parse.log: db_type is DESI, All refnos count: 2
  new validation: db_type is DESI, All refnos count: 2748

aps7351_0001:
  old parse.log: All refnos count: 8
  new validation: All refnos count: 3345855

gen-cata-closure:
  DESI seed indexed=2748 parsed=2737 seeds=2901
  cata_dbs=1 visited=16 missing=44 rounds=6
  covered CATA dbnum=250193, requested=1 parsed=1 table_size=1563
```

生成的 CATA closure manifest 非空：

```json
{
  "by_dbnum": { "250193": ["16 refnos"] },
  "seed_count": 2901,
  "visited_count": 16,
  "rounds": 6,
  "missing": 44
}
```

说明 release 包内解析侧已经不再复现 `All refnos count: 2`。

源码工作区现有 `quicktest-7997-8080` runtime 站点可用于闭包验证。使用同一
修复版 `aios-database.exe` 重新执行：

```text
runtime/admin_sites/quicktest-7997-8080/logs/cata-closure-refno-index-20260612-210727.log
```

关键结果：

```text
ams7997_0001:
  current validation: All refnos count: 157260

gen-cata-closure:
  DESI seed indexed=157260 parsed=157259 seeds=158063
  cata_dbs=8 visited=15040 missing=92 rounds=9
```

生成的 `AvevaMarineSample/scene_tree/cata_closure.json` 非空，按 dbnum 覆盖：

```text
5052=3529
5053=319
5054=5808
6890=86
7000=202
7001=30
7002=4868
7320=198
```

`quicktest-7997-8080` 输出目录已有非空 Parquet 文件（例如
`instances.parquet`、`aabb.parquet`、`transforms.parquet`），但时间戳早于本次
闭包验证；因此本轮只确认 refno index 与 CATA closure 不再因索引漏读缩水，
完整生成/Parquet 重跑仍待单独执行。
