# Implementation Plan: 当前会话 B-tree refno 索引全量枚举修复（spec 007）

## Approach

把 `refno_index` 从“只读 latest root 的前 2 个 entry”修成真正的 B-tree traversal：

1. 正确读取 index page entries：按页容量读到零终止。
2. 正确遍历 internal page：start marker child 也要遍历。
3. 统一 full enumeration 与 single lookup 的 traversal 语义。
4. 用 BTreeMap / latest-wins 合并同 refno 多记录。
5. 用现有 scan 路径只做 oracle 验证，不作为生产 fallback。

## Evidence Summary

### `aps250160_0001`

当前：

```json
{
  "index_refnos": 2,
  "scan_refnos": 2759
}
```

真实索引页诊断：

```text
pg 447 level=1 declared=2 actual_until_zero=43
pg 419 level=0 declared=2 actual_until_zero=126
pg 446 level=0 declared=2 actual_until_zero=126
```

按零终止读取并遍历 start marker child 后：

```text
merged_count_ignore_declared = 2767
```

### `aps7351_0001`

当前：

```json
{
  "index_refnos": 8,
  "scan_refnos": 3345855
}
```

按零终止读取并遍历 start marker child 后：

```text
merged_count_ignore_declared = 3348074
```

## Phase 0 — Pin Reproduction

目标：保证后续每次修改都有确定反馈。

命令：

```powershell
cargo run --manifest-path D:\work\plant-code\pdms-io-fork\crates\parse_pdms_db\Cargo.toml --example verify_parse_db_basic -- --db-file D:\AVEVA\Projects\E3D2.1\AvevaPlantSample\aps000\aps250160_0001

cargo run --manifest-path D:\work\plant-code\pdms-io-fork\crates\parse_pdms_db\Cargo.toml --example compare_refno_index -- --db-file D:\AVEVA\Projects\E3D2.1\AvevaPlantSample\aps000\aps250160_0001 --full
```

期望当前复现：

```text
index_refnos=2
scan_refnos=2759
scan_only=2757
```

`aps7351_0001` 作为大库验证，运行较慢：

```powershell
cargo run --manifest-path D:\work\plant-code\pdms-io-fork\crates\parse_pdms_db\Cargo.toml --example verify_parse_db_basic -- --db-file D:\AVEVA\Projects\E3D2.1\AvevaPlantSample\aps000\aps7351_0001
```

## Phase 1 — Fix Page Entry Parsing

修改 `crates/parse_pdms_db/src/refno_index.rs::parse_index_page`：

- 删除 `declared_entries` 对 entry 数量的控制。
- 保留读取 `offset + 0x10` 仅用于 debug / future instrumentation。
- 按 `capacity = (page_size - 0x1C) / 16` 遍历。
- `ref0 == 0` 时终止当前页。

预期：

- `aps250160_0001` 的每个 leaf page 能读到约 126 条 loc。
- `aps7351_0001` 的 leaf/internal page 不再只读 2 条。

## Phase 2 — Traverse Start Marker Child

修改 full enumeration：

```rust
if page.level == 0 {
    // leaf: skip start marker as element
} else {
    // internal: enqueue every non-zero child page, including start marker child
}
```

修改 fallback leaf walk：

- internal page 遍历规则与 full enumeration 一致。
- leaf page 查找目标时仍跳过 start marker loc。

修改 `choose_child_pages`：

- 若 `target < valid[0].refno`，优先加入 start marker child。
- 在乱序 / 删除空洞情况下，start marker child 可作为候选兜底。
- 保持候选去重，避免递归循环。

## Phase 3 — Merge and Validate Current Entries

保持 `entry_from_loc()` 校验：

- `byte_offset` 有效。
- `skip_record_padding` 后至少 16 字节。
- record refno 等于 loc refno。
- noun hash 可读。

合并规则：

```rust
let should_insert = ordered
    .get(&refno)
    .map(|old| old.pos < entry.pos)
    .unwrap_or(true);
```

该规则与旧 scan 从文件尾向前、同 refno 保留最新记录的语义一致。

如果 Phase 3 后仍存在 `index_only` 大量多余：

- 抽样分析 `index_only` loc 指向的 record。
- 判断是否为删除/tombstone/free-area leftover。
- 只在明确格式后加过滤规则，不用 scan 结果硬裁剪。

## Phase 4 — Regression Fixtures and Tools

### Synthetic fixtures

新增或更新 `refno_index.rs` 单元测试：

- `parse_index_page_reads_until_zero_not_declared_count`
  - 构造 index page：`declared=2`，实际写入 4 条 entry，再以 `ref0=0` 终止。
  - 期望读取 4 条。
- `full_enumeration_traverses_start_marker_child`
  - root internal page 的 start marker 指向 leaf A，普通 key 指向 leaf B。
  - 期望 leaf A/B 的元素都出现在 table。
- `single_lookup_can_find_refno_under_start_marker_child`
  - 目标 refno 位于 start marker child。
  - 期望 `find_refno_entry()` 命中。

### Example tools

更新 `compare_refno_index`：

- 输出 `index_only_samples` 的 loc/entry 诊断。
- 可加 `--target-samples` 复查代表 refno 的 `find_refno_entry()`。

保留 `verify_parse_db_basic` 作为主链路验证。

## Phase 5 — Plant Deployment Validation

在 `plant-model-gen-cata-closure` 中重跑：

1. `quicktest-250160-8081`
   - 预期 `All refnos count` 不再为 2。
   - `TreeIndex` 不再只有 2 节点。
   - Parquet 不再全部 0 行，除非业务上目标 DB 无实例。
2. `quicktest-7997-8080`
   - 预期 `ams7997_0001` refno count 仍为 152656 或合理值。
   - CATA closure manifest 不因 refno index 漏读变空。
   - 新增元件库目录后，manifest 路径与 project_name 选择问题另行记录，不与本 spec 混修。

## Risks

- R1：按零终止读取可能包含 free-area leftover。缓解：`entry_from_loc()` 严格校验，后续用 `index_only` 样例识别 tombstone。
- R2：start marker child 遍历可能引入历史/缓存子树。缓解：通过 latest-wins 合并、entry 校验、scan oracle 对照确认。
- R3：大 CATA 索引枚举内存增长。缓解：当前 scan 已更慢；B-tree traversal 仍是目标优化路径。必要时后续引入 streaming table build。
- R4：单点查找候选过多。缓解：优先 lower_bound，start marker 只作为必要/兜底候选；fallback leaf walk 可接受 O(index pages)。

## Rollback

如果修复导致 `index_only` 大量不可解释，回滚到上一版 `refno_index.rs` 并保留 spec 诊断结果；不要改业务层掩盖问题。

## Done Definition

- spec 007 的 synthetic tests 覆盖 entry count 和 start marker traversal。
- `verify_parse_db_basic` 对 `aps250160_0001` 与 `aps7351_0001` 的 index/scan count 等价或差异有明确 tombstone 解释。
- `compare_refno_index --full` 对 `aps250160_0001` 不再有大量 `scan_only`。
- 站点部署日志不再出现 `aps250160_0001 All refnos count: 2`。
