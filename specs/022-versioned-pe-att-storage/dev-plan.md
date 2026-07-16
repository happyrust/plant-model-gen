# 开发执行计划: PE/ATT 版本化存储(022)

**Date**: 2026-07-13 | **基于**: spec.md / plan.md / tasks.md + 当日代码现状核对

**定位**: tasks.md 是任务清单;本文件回答"下一步怎么排期、代码落点在哪一行、跨仓顺序、谁来做、怎么验"。

---

## 0. 现状核对(2026-07-16 二次核对，修正 07-13 版本的重大偏差)

> **07-13 dev-plan 的判断"进度比 tasks.md 更靠前"是对的，但漏了一个致命事实：M1+M2 的代码从未提交，全部躺在 `git stash@{0}`（`dirty-wt-before-narrow-023`，07-16 18:59 创建）里。当前分支 HEAD 上没有任何 022 代码。** 下面是修正后的实况。

| 任务 | tasks.md 原状态 | 真实代码状态（2026-07-16） |
|------|--------------|---------|
| T001~T007 (M1 实例级开关) | 已勾选 | **已实现但仅在 stash@{0}，HEAD 上没有**。`options.rs` 的 `versioned_storage`/`version_retention`/`rocksdb_conn_str`/`current_versioned_params` + 5 处启动串透传;冒烟 07-13 已过(8031 versioned / 8032 对照)——但那次冒烟基于 stash 基线，落定后需重跑 |
| T008 锚点表 schema | 未勾选 | **已实现但仅在 stash@{0}**:`ensure_sesno_version_anchor_schema()`,SCHEMAFULL + UNIQUE INDEX (dbnum,sesno) + `note: option<string>` + `source ASSERT IN ['full','incremental']`;已挂进 `sync_pdms`/`sync_pdms_with_callback` 入口 |
| T009 增量锚点 | 未做 | **已实现但仅在 stash@{0}**:`write_sesno_version_anchor()` + `persist_pdms_increment_grouped` 成功路径调用 + `PdmsIncrementPersistStats.anchors` |
| T010 全量锚点 | 未做 | **已实现但仅在 stash@{0}**:`write_full_version_anchors()`,sender drop + insert 排空后写(anchored_at 晚于本批);`parse_single_db_file` 单文件路径待复核(R1) |
| T011 汇总 JSON | 未做 | **已实现但仅在 stash@{0}**:`main.rs` `"version_anchor": persist_stats.anchors` |
| T012 M2 验证 | 未做 | 属实，未做 |
| T013~T020 (M3) | 未做 | 属实。rs-core 只有旧 `version.rs`(`backup_data`)，无 `version_query` 模块;`model-version` CLI 存在但无 `history` 子命令组(stash 也没碰 `version_management/cli.rs`) |
| T021~T026 (M4/Polish) | 未做 | 属实。`specs/022` 下**没有 quickstart.md** |

### 0.1 stash@{0} 的构成（落定前必读）

`stash@{0}` 共 **349 个文件**，绝大多数与 022 无关，必须挑拣，**不能 `stash pop`**：
- **纯 022、可 `git apply --3way` 独立落定的 6 个文件**：`src/options.rs`(+74)、`src/cli_modes.rs`(+41/-11)、`src/web_server/db_startup_manager.rs`(+8/-2)、`src/web_server/handlers.rs`(+17/-6)、`src/bin/web_server.rs`(+29/-9)、`src/versioned_db/database.rs`(+94/-3)。
- **022 与其它 WIP 交织、需逐 hunk 拣选的 3 个文件**：
  - `src/data_interface/sesno_increment.rs`(+417/-27)：022 锚点(`VersionAnchorRecord`/`write_sesno_version_anchor`) + 非 022 的 session-scan 扫描(`PdmsSesnoSessionScanOutcome`/`scan_pdms_increment_sessions_*`) + 依赖 `increment_record.rs` 的 `cleanup_refnos` 字段。且 HEAD 已被 `ed16d8ab` 改过此文件(删除 `IncrementCollectProgress` 用法)，stash 基线更旧——**必然三方冲突，需手工合**。
  - `src/main.rs`(+333/-11)：022 的 `version_anchor` 汇总(1 行) + 大量 session-scan 汇总/打印 + 023 的 `affected_delivery_units`/`build_incremental_affected_delivery_units`。**022 部分极小，其余都不是 022**。
  - `src/web_server/managed_project_sites.rs`(+44/-9)：022 的 `site_versioned_params`/`site_rocksdb_conn_str`;HEAD 侧 `ed16d8ab` 也动过此文件(preset)，但区域不同，`--3way` 应能自动合。
- **另有 8 个 src 文件**(`mbd_pipe_api`/`e3d_tree_api`/`post_gen_export`/`spec_info`/`orchestrator`/`pdms_inst`/`tree_index_manager`/`increment_record`)属 023 收窄或其它 WIP，**不属于 022**（`increment_record.rs` 的 `cleanup_refnos` 是 `sesno_increment` session-scan 的依赖，属"非 022 但被交织文件牵连"）。
- **其余 ~332 个是文档/skill 删除**，与本 feature 无关，落定 022 时一律不带。

### 0.2 落定策略（迭代 0，见 §1）

对 6 个纯 022 文件用 `git stash show -p stash@{0} -- <file> | git apply --3way`；对 3 个交织文件走逐 hunk 手工拣选（只取带 `versioned`/`retention`/`anchor`/`version_anchor` 标记的 hunk，session-scan 与 023 的 hunk 留在 stash 里）。**stash 不 drop**，直到落定分支 cargo check 全绿 + 冒烟通过后再决定是否保留。

**待替代的"之前的版本存储方式"已定位**:rs-core `src/rs_surreal/version.rs` 的 `backup_data()`/`backup_owner_relate()`(`fn::backup_data` 写 history 表),本仓唯一调用方 `src/data_interface/increment_manager.rs:11`;另有 `db_option.is_sync_history()` 驱动的 ses 历史索引路径(`database.rs:835/1315/2113`)。退役动作排在迭代 3,MVP 前不动它。

---

## 1. 迭代划分与排期

### 迭代 0 —— 抢救并落定 M1+M2(最高优先,预计 0.5 天,单人串行) 【新增，2026-07-16】

**动机**：M1+M2 代码只存在于 `stash@{0}`，一旦误 drop 全丢；且 tasks.md 谎报 M1 已完成。必须先把它干净落定到分支，后续 M3/M4 才有地基。

**步骤**：
1. 从当前 HEAD(`ed16d8ab`) 切新分支 `feat/022-versioned-pe-att-storage`。
2. 6 个纯 022 文件三方合并：
   ```
   git stash show -p "stash@{0}" -- src/options.rs src/cli_modes.rs \
     src/web_server/db_startup_manager.rs src/web_server/handlers.rs \
     src/bin/web_server.rs src/versioned_db/database.rs | git apply --3way
   ```
3. 3 个交织文件逐 hunk 拣选（`git apply --3way` 失败的 hunk 手工处理）：
   - `managed_project_sites.rs`：优先整体 `--3way`，区域不同应自动合；冲突则只取 `site_versioned_params`/`site_rocksdb_conn_str`/三处启动透传。
   - `sesno_increment.rs`：只取 022 锚点相关（`VersionAnchorRecord`、`write_sesno_version_anchor`、`PdmsIncrementPersistStats.anchors`、`persist_pdms_increment_grouped` 尾部锚点调用），**放弃** session-scan 全套（`PdmsSesnoSessionScan*`、`scan_pdms_increment_sessions_*`）与 `IncrementCollectProgress` 回退（HEAD 已删）。
   - `main.rs`：只取 `"version_anchor": persist_stats.anchors` 一行汇总；**放弃** session-scan 汇总/打印与 023 `affected_delivery_units`。
   - 交织文件如牵连 `increment_record.rs` 的 `cleanup_refnos`：仅当锚点代码不依赖它时不带；若编译需要则单独加该字段（2 行）。
4. `cargo check` 全绿（仓库规则：不跑 cargo test）。修复三方合并残留。
5. **补 T012 验证**：8031/8030 起 versioned 实例 → `incremental-sesno --json` → `SELECT * FROM sesno_version_anchor` 核对 (dbnum,sesno,anchored_at) 时间序晚于本批写入；模拟失败路径确认无残留锚点。
6. 提交（feat: 022 M1+M2 落定），勾正 tasks.md 的 `[s]`→`[x]`、订正 M1「已提交」事实。**stash 暂不 drop**，确认无误后再清。

**产出**：M1+M2 真正进入分支，SC-002 可验收。

### 迭代 1 —— M3 查询封装 + history CLI(原迭代 2，P1/P2，预计 1~1.5 天，双人并行)

见 §1 下方原「迭代 2」小节（rs-core `version_query` + 本仓 history CLI）。

### 迭代 2 —— M4 存量切换 + 旧机制退役 + Polish(原迭代 3，P2，预计 1 天)

见 §1 下方原「迭代 3」小节。

---

### （原）迭代 1 —— M2 锚点固化：**已并入迭代 0**，以下为原始落点记录，落定时按此逐行核对

T009→T011 改动集中在同一条数据流上,不宜拆分并行,一人承接。

**T009 增量锚点** — `src/data_interface/sesno_increment.rs`
- 落点:`persist_pdms_increment_grouped()`(L781)成功路径末尾,即 `flush_increment_upserts`(L890)与 dbnum_info 更新(L905 `exec_statements(&dbnum_sqls, 200)`)全部完成之后、`Ok(stats)`(L907)之前。
- 写法:`UPSERT sesno_version_anchor:[{dbnum}, {sesno}] SET dbnum={}, sesno={}, source='incremental', anchored_at=time::now();` 走既有 `exec_sql_checked`(L689,内部 `project_primary_db()` + USE NS 前缀 + take_errors 检查),错误传播天然满足 FR-004(任何前序 `?` 失败都到不了锚点行)。
- sesno 取值:该函数按文件处理,`report.dbnum` + `report.actual_end_sesno` 即本批锚点;函数内 `dbnum_info` map(L799)是按 ref0 聚合的,仅用于 dbnum_info_table,锚点不要复用它(一个文件一个 dbnum,写一条即可)。
- 注意:调用方有两个——`persist_pdms_increment_files`(L910,file 路径)与 `persist_collected_pdms_increment_files`(L921,collected 路径,main.rs L393 实际走这条)。锚点写在 `persist_pdms_increment_grouped` 内部,两条路径自动同时覆盖。
- 幂等:UPSERT 到定长 record id,同 (dbnum,sesno) 重跑增量覆盖锚点时间戳,符合"重跑增量覆盖后写锚点"的风险对策。
- 前置:入口先调一次 `ensure_sesno_version_anchor_schema()`(已存在,database.rs:537),兑现 schema 注释里"锚点写入前会重试"的承诺。

**T010 全量锚点** — `src/versioned_db/database.rs`
- `sync_pdms` / `sync_pdms_with_callback` 收尾处,对每个成功解析的 dbnum 写 `source='full'` 锚点;latest_sesno 已在解析流程中取得(L1480/L2251 `io.get_latest_sesno()`,并已写入 db_meta `latest_sesno` 字段 L1770/L2659),收尾时从 db_meta 或解析结果聚合读取。
- `parse_single_db_file`(L2951)同样在尾部写单 dbnum 锚点(L3116 已有 latest_sesno)。

**T011 汇总 JSON** — `src/main.rs`
- `run_incremental_sesno` 的 summary 组装点 L623-658,在 `"data_persist": persist_stats` 旁新增 `"version_anchor": {dbnum, sesno, anchored_at, written: bool}`;数据从 T009 返回值带出(建议在 `PdmsIncrementPersistStats` 加 `anchors: Vec<(u32,u32)>` 或单独返回,取改动小者)。

**T012 验证**(CLI+SurrealQL,不写 cargo test)
- 8030 测试实例跑一次 `incremental-sesno --file ... --json` → 查 `SELECT * FROM sesno_version_anchor` 核对时间序(anchored_at 晚于本批全部写入);
- 断连/中途失败模拟 → 确认无新锚点;
- `sync_pdms` 小库全量 → 首条 full 锚点存在。

### 迭代 2 —— M3 查询封装 + history CLI(P1/P2,预计 1~1.5 天,双人并行)

**A 线:rs-core(D:\work\plant-code\rs-core,dev-3.1)**
- 新建 `src/rs_surreal/version_query.rs`,`mod.rs` 加 `pub mod version_query;`(参考现有模块声明区,不进 `pub use *` 泛导出,显式路径调用)。
- 实现(T013→T014→T015/T016):
  - `resolve_anchor(dbnum: u32, sesno: u32) -> Result<Option<AnchorHit>>`:精确命中否则 `WHERE dbnum=$d AND sesno<=$s ORDER BY sesno DESC LIMIT 1` 回退,`AnchorHit { sesno, anchored_at, exact: bool }`;
  - `snapshot_at(refno: RefnoEnum, sesno: u32) -> Result<ElementSnapshot>`:锚点换算 `$t` 后对 `pe:<refno>`、noun 表(先查 pe 快照拿 noun 再查 `<noun>:<refno>`)、`ATT_UDA:<refno>` 各发 `SELECT * FROM <target> VERSION <datetime>`;
  - `diff_range(refnos, from_sesno, to_sesno) -> Result<Vec<ElementDiff>>`:两端 snapshot 字段级对比,分类 changed/added/removed/deleted;
  - `timeline(refno, from_sesno, to_sesno) -> Result<Vec<TimelinePoint>>`:区间内逐锚点快照 hash 对比(首版接受 O(锚点数));
  - 错误类型:`HistoryError` enum(`Expired`/`AnchorMissing`/`Other(anyhow)`),识别引擎 InvalidArgument("read timestamp below full_history_ts_low" 类消息)翻译为 `Expired`——识别用 err 字符串 contains,fork 消息文本以 `db-data/run_surrealkv_versioned.ps1` 验证过的实例实测为准。
  - 连接:全部走 `SUL_DB`/`project_primary_db()` 同款入口,与 `version.rs`(旧 backup 机制)互不触碰。
- 提交并 push dev-3.1。

**B 线:plant-model-gen(可与 A 线并行起步,联调前用 `[patch]` 指本地 rs-core path)**
- `src/version_management/cli.rs`:`Command::new("model-version")`(L44)下加 `history` 子命令组(snapshot/timeline/diff,统一 `--json`),分发器 `handle_model_version_command`(L1775 起)接新分支;
- `HistoryError::Expired` → 输出"该 sesno 历史已超出 retention 窗口,请改用 DuckLake 存档或源文件重扫",`AnchorMissing` → 明确报错;回退命中(`exact:false`)在输出中注明;
- T019 `db-data/verify_versioned_pe_att.surql`:基于 `test_version_data.surql` 扩展锚点写入+VERSION 查询+删除历史三段(此文件独立,可第三人并行);
- 联调完成后 Cargo.toml 恢复 git 依赖并 `cargo update -p aios_core`。
- T020 验证:8030 实例三个子命令 vs 手工 SurrealQL 对照;构造低于水位线的时间戳确认 Expired 路径。

### 迭代 3 —— M4 存量切换 + 旧机制退役 + Polish(P2,预计 1 天)

- T021 quickstart.md 切换手册 + T022 站点编辑 versioned 开关防护(`managed_project_sites.rs`,已初始化站点返回"需要重灌")+ T023 全流程演练;
- **旧版本存储退役(新增,tasks.md 之外)**:`increment_manager.rs:11` 停用 `backup_data`/`backup_owner_relate` 调用(versioned 实例上历史由引擎回答);`is_sync_history()` 的 ses 索引路径保留但在 quickstart 注明 versioned 站点无需开启;rs-core `version.rs` 标记 deprecated,不删;
- T024 AGENTS.md 沉淀七项决策要点;T025 运维说明(磁盘水位/retention 调整/未开 MODEL_KV 警示);T026 终验。

---

## 2. 跨仓依赖与提交顺序

```
迭代1(本仓,独立可发) ──────────────┐
迭代2A(rs-core version_query) ──→ push dev-3.1 ──→ 迭代2B 联调(cargo update -p aios_core)──→ 迭代3
```

- 本仓 aios_core 依赖是 `git branch dev-3.1`(Cargo.toml:71),rs-core 未 push 前本仓拉不到——并行开发期用 `[patch."https://github.com/happyrust/rs-core.git"]` 指本地 path,联调完成移除;
- 迭代 1 与迭代 2A 无依赖,可同日并行开工;迭代 2B 的 CLI 骨架(参数解析、错误文案)也可先行,只在最后接 rs-core 真实现。

## 3. 分工建议(工作组恢复运行后)

| 角色 | 承接 |
|------|------|
| 成员甲 | 迭代 1 全部(T009/T010/T011/T012,单人串行) |
| 成员乙 | 迭代 2A rs-core version_query(T013~T016) |
| 成员丙 | 迭代 2B CLI 骨架 + T019 验证脚本(与乙并行) |
| 总指挥 | 迭代 2B 联调、迭代 3 排期、验收汇总 |

## 4. 验证环境约定

- 本地 fork 二进制(release 3.3.0-nightly):8030 主测试实例(versioned),8032 非 versioned 对照;
- 脚本:`db-data/run_surrealkv_versioned.ps1` 起实例,`db-data/test_version_data.surql` 基线,T019 扩展为 `verify_versioned_pe_att.surql`;
- 全程 CLI `--json` + SurrealQL 验证,不写 cargo test(仓库规则)。

## 5. 风险提示(计划层新增)

- `persist_pdms_increment_grouped` 对 `actual_start_sesno==0 || actual_end_sesno==0` 提前返回(L789)——锚点写入必须在该 guard 之后的正常路径,空批不写锚点;
- 一次 incremental-sesno 可能处理多个文件(多 dbnum),锚点按文件粒度各写一条,T011 汇总字段应为数组;
- fork 的 InvalidArgument 错误文本是翻译 `HistoryExpired` 的匹配依据,rs-core 实现时先在 8030 实例实测一条过期查询拿到准确消息再写匹配串,避免猜。
