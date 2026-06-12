# Tasks: 当前会话 B-tree refno 索引全量枚举修复（spec 007）

## T001 — 固化最小复现

- 运行 `verify_parse_db_basic` 对 `aps250160_0001`。
- 运行 `compare_refno_index --full` 对 `aps250160_0001`。
- 记录当前输出：
  - `index_refnos=2`
  - `scan_refnos=2759`
  - `scan_only=2757`
- 运行 `verify_parse_db_basic` 对 `aps7351_0001`，记录 `index_refnos=8` / `scan_refnos=3345855`。

验收：

- 复现命令和输出摘要写入本 spec 或 implementation notes。
- 后续修复可用同一命令对比。

## T002 — 修复 index page entry 读取

- 修改 `crates/parse_pdms_db/src/refno_index.rs::parse_index_page`。
- 不再用 `offset + 0x10` 作为 entry count。
- 按 `capacity` 读取到 `ref0 == 0`。
- 保留 index page noun / bounds 校验。

验收：

- synthetic test 覆盖 `declared=2` 但实际 4 条 entry 的页面。
- 测试期望读取 4 条，而不是 2 条。

## T003 — 修复 internal start marker 子树遍历

- 修改 `gen_ref_type_pos_table_from_index()`：
  - leaf page：跳过 start marker entry。
  - internal page：所有非零 child page 入队，包括 start marker child。
- 修改 `find_loc_by_leaf_walk()` 使用同一 internal traversal。

验收：

- synthetic test 覆盖 root start marker child + normal child。
- 两个 child leaf 的元素都进入 table。

## T004 — 修复单点查找候选选择

- 修改 `choose_child_pages()`：
  - target 小于首个 valid key 时加入 start marker child。
  - 乱序/删除空洞兜底时允许加入 start marker child。
  - 候选去重，避免重复递归。
- 确认 `find_refno_entry()` 对 start marker child 下的 refno 能命中。

验收：

- synthetic test 覆盖 `find_refno_entry()` 查找 start marker child 内 refno。
- `compare_refno_index` 的 `scan_only_samples` 代表 refno 可被定位或进入明确 tombstone 豁免。

## T005 — 保持 latest Element 合并语义

- 保留 `BTreeMap<RefU64, EleDataEntry>`。
- 同 refno 多条记录以 `entry.pos` 最大 wins。
- 增加注释说明该规则与旧 scan 反向遍历保留最新记录等价。
- 如果真实验证发现 tombstone/free entry，补最小过滤函数并增加样例测试。

验收：

- 同 refno 新旧记录 synthetic test 只保留新记录。
- 若 tombstone 规则暂不明确，记录为 known gap，不静默吞掉差异。

## T006 — 升级诊断工具

- 更新 `examples/compare_refno_index.rs`：
  - 输出 `index_only` 样例的 pos / noun_hash。
  - 可选输出 start marker child traversal 统计。
- 确认 `examples/verify_parse_db_basic.rs` 仍能跑主链路对比。

验收：

- 工具输出能解释 `index_count` 与 `scan_count` 差异来源。
- 不依赖站点部署即可复核 refno 表问题。

## T007 — 真实文件回归验证

- 对 `aps250160_0001` 运行：

```powershell
cargo run --manifest-path D:\work\plant-code\pdms-io-fork\crates\parse_pdms_db\Cargo.toml --example verify_parse_db_basic -- --db-file D:\AVEVA\Projects\E3D2.1\AvevaPlantSample\aps000\aps250160_0001
```

- 对 `aps7351_0001` 运行同一命令。
- 对 `aps250160_0001` 运行 `compare_refno_index --full`。

验收：

- `aps250160_0001` 不再返回 `index_refnos=2`。
- `scan_only` 从 2757 降为 0 或全部有明确豁免。
- 大 CATA 的 `index_refnos` 与 scan 量级一致。

## T008 — 站点部署验证

- [x] 在 `plant-model-gen-cata-closure` release 包内替换修复后的 `aios-database.exe`，并用
  `quicktest-250160-8081` 的 `DbOption-parse` 做最小重解析验证。
- [x] 检查验证日志：
  - `aps250160_0001 All refnos count` 不再为 2。
  - `aps7351_0001 All refnos count` 不再为 8，而是 3345855。
- [x] 检查 CATA closure manifest：
  - 不因 refno index 漏读为空。
  - CATA 部分解析覆盖目标依赖库。
- [x] 重跑 `quicktest-7997-8080` 的 CATA closure pass，确认当前 7997 解析输入不再因 refno
  index 漏读产生空/缩水 manifest。
- [ ] 重跑完整生成，检查 `TreeIndex` / Parquet 生成产物不再因 refno index 缺陷缩水。

验收：

- 250160 的 release 包最小解析验证已通过：`aps250160_0001` 为 2748 refnos，CATA manifest 覆盖
  `250193` 的 16 个 refno。
- 7997 的 closure pass 已通过：`ams7997_0001` 为 157260 refnos，CATA manifest 覆盖 8 个
  CATA 库、15040 个 refno。
- 完整 Parquet 重跑仍待补充；当前站点已有非空 Parquet 产物，但不是本次闭包 job 生成。

## T009 — 文档与变更记录

- 在 `CHANGELOG.md` 记录 refno index 修复。
- 在 spec 007 更新真实验证结果。
- 若发现 `project_name=acp000` 导致 manifest 路径错位，另开 spec / issue，不塞进本修复。

验收：

- spec / plan / tasks 与实际实现一致。
- 后续维护者能按命令复现问题和验证修复。
