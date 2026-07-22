# 022 Version Commit 验证：full sync 锚点 fail-closed + 全量 cargo check 取证

- 日期：2026-07-19 23:07–23:14 (UTC+8)
- 方式：静态代码阅读（不改代码）+ `cargo check`（仓库规则禁止 cargo test）
- 工作区状态：含 specs/023 未提交改动（version_commit.rs 的 pe_owner_rows、pe_owner 边维护等），属预期，未回退、未 commit。

---

## 事一：full sync 锚点 fail-closed 链路（静态取证）

### 结论

三项验证全部通过：

1. **锚点失败 anyhow 向上传播（fail-closed，不吞错）** ✓
2. **PendingCommit 分支用 `recover_version_commit` 以同 fingerprint 恢复** ✓
3. **调用时序注释与 `sync_total_async_threaded*` 实际 join 顺序一致**（含 specs/023 pe_owner 边同通道说明）✓

唯一的"非阻断"分支是锚点成功之后的 `pe_owner_version_meta` 写入失败只 warn 不传播（`database.rs:743-754`），这是注释里明确声明的设计（读侧回退 `pe.children` 天然安全），不属于吞错。

### ① 锚点失败向上传播

`write_full_version_anchors`（`src/versioned_db/database.rs:685-758`）签名返回 `anyhow::Result<Vec<VersionAnchorRecord>>`，内部对 `commit_version` 的三分支处理：

```709:727:src/versioned_db/database.rs
        let outcome = match commit_version(request.clone(), || async { Ok(counts.clone()) }).await {
            Ok(outcome) => outcome,
            Err(VersionCommitError::PendingCommit { pending_sesno, .. })
                if pending_sesno == *sesno =>
            {
                recover_version_commit(request, || async { Ok(counts) })
                    .await
                    .with_context(|| {
                        format!(
                            "full sync 数据已写入，但 pending 锚点恢复失败(dbnum={dbnum} sesno={sesno})"
                        )
                    })?
            }
            Err(error) => {
                return Err(anyhow::Error::new(error).context(format!(
                    "full sync 数据已写入，但锚点发布失败(dbnum={dbnum} sesno={sesno})"
                )));
            }
        };
```

- 恢复分支失败：`.with_context(...)?` 传播（line 714-720）。
- 其它任何错误（LeaseBusy / FingerprintConflict / LegacyAnchor / CountMismatch / ApplyFailed / RecoveryNotFound / Storage，见 `version_commit.rs:69-117`）：`return Err(...)`（line 722-726）。
- **不同 sesno 的 PendingCommit 不满足 guard `pending_sesno == *sesno`，落入通用 Err 分支同样传播**——不会跨 sesno 乱恢复。
- 两个调用点均以 `?` 上抛：`database.rs:2078` 与 `database.rs:3004`（`write_full_version_anchors(&pending_full_version_anchors).await?;`）。上层是 `sync_total_async_threaded_with_callback` / `sync_total_async_threaded` 的返回值，继续向 CLI/调用方传播。
- 函数 doc（line 683-684）明文约定："锚点失败必须向上传播。……在锚点成功前不能把本次 full sync 宣告为可供历史查询的完整提交。"实现与之一致。

### ② PendingCommit 分支：同 fingerprint 恢复

- fingerprint 只计算一次（`database.rs:690-698`，输入 `full-sync-v1:{dbnum}:{sesno}` + 文件路径/长度/mtime 证据），放进 `request`；`commit_version(request.clone(), ...)` 与 `recover_version_commit(request, ...)` 用的是**同一个 request 值 → 同一 fingerprint**。
- `recover_version_commit`（`version_commit.rs:187-196`）→ `commit_version_inner(request, recovered=true, ...)` → `require_matching_pending`（`version_commit.rs:422-448`）：要求存在 `version_commit_state` 行满足 `fingerprint == request.fingerprint && status ∈ {preparing, pending}`，否则 `RecoveryNotFound` 错误（同样传播）。**恢复路径被 fingerprint 门禁住，不可能拿别人的 pending 顶包。**
- 幂等/冲突防线（`existing_idempotent_outcome`，`version_commit.rs:363-420`）：已有锚点 fingerprint 相同 → 幂等成功返回；不同 → `FingerprintConflict`；无 fingerprint 旧锚点 → `LegacyAnchor`。锚点不可变语义成立。
- apply 失败 / count 不匹配 / 建锚失败均先 `mark_commit_pending` 再返回 Err（`version_commit.rs:266-298`），为下次同 fingerprint 恢复留下现场——与 ① 的 fail-closed 语义闭环。

### ③ 调用时序注释 vs 实际 join 顺序

函数 doc（`database.rs:671-681`）约定：

> 必须在本轮写库任务全部 join 之后（sync_total_async_threaded* 内 `drop(sender)` + 等待 insert_handles 排空之后）调用……
> specs/023 T008：pe_owner 边（PERelateJson）走同一 sender/sink 通道，上述 join 约束同样保证"边全部落库先于 full 锚点固化"。

实际代码两条路径均一致：

**路径 A：`sync_total_async_threaded_with_callback`（`database.rs:1273`）**

| 步骤 | 行号 | 代码 |
|---|---|---|
| 建通道 | 1359 | `let (sender, receiver) = flume::unbounded();` |
| 起 16 个写库 worker | 1362-1368 | `insert_handles.push(tokio::task::spawn(...))`，worker 消费 `SenderJsonsData`（含 `PERelateJson`，1406/1433） |
| 解析循环登记锚点 | 2013-2019 | `pending_full_version_anchors.push((dbnum, sesno, evidence))` |
| 关通道 | 2073 | `drop(sender)` |
| join 全部 worker | 2074-2076 | `while let Some(result) = insert_handles.next().await { result.map_err(...)?; }` |
| **写锚点** | 2078 | `write_full_version_anchors(&pending_full_version_anchors).await?;` |

**路径 B：`sync_total_async_threaded`（`database.rs:2102`）**

| 步骤 | 行号 | 代码 |
|---|---|---|
| 建通道 | 2172 | `let (sender, receiver) = flume::unbounded();` |
| 起写库 worker | 2173+ | worker 消费 `SenderJsonsData`（含 `PERelateJson`，2211/2232） |
| 解析任务（spawn）内登记锚点 | 2391 / 2917-2923 / 2994 | 锚点向量由解析任务收集并作为返回值带出 |
| join 解析任务 | 2996-2997 | `.await.map_err(...)??` |
| 关通道 | 2998 | `drop(sender)` |
| join 全部 worker | 3000-3002 | 同上，worker 错误以 `?` 传播 |
| **写锚点** | 3004 | `write_full_version_anchors(&pending_full_version_anchors).await?;` |

两条路径都是 **解析完成 → drop(sender) → insert_handles 全部 join（写库 worker 错误也 fail-closed 传播）→ 才写 full 锚点**，与注释一字不差。

**pe_owner 边同通道证据**（specs/023）：`src/versioned_db/pe.rs:436-477` 的 `save_pe_relates` 把边语句经同一个 `flume::Sender<SenderJsonsData>` 以 `SenderJsonsData::PERelateJson` 发出（pe.rs:465/471）；两条路径的 sink worker 分别在 `database.rs:1406-1433` / `2211-2232` 处理该变体（specs/023 T007：每 owner 先 DELETE 后 INSERT RELATION，幂等）。因此"边落库先于锚点固化"由同一个 join 屏障保证，注释成立。

---

## 事二：全量编译取证（cargo check，禁 test）

机器背景：本机内存紧张预案（-j 1 / cargo clean）未触发，无 os error 1455、无 rustc 崩溃、无 rmeta 损坏。

### 1) `cargo check -j 2`（默认 features）

- **退出码：0**（`CARGO_CHECK_DEFAULT_EXIT=0`），耗时 ~18s（依赖缓存命中，仅重查 aios-database）
- **aios-database：无 error、无 warning**（日志中 `Checking aios-database v0.3.34` 后直接 `Finished`）
- 警告均来自工作区外依赖 `parse_pdms_db`（80 条）/ `pdms_io`（28 条），与本任务无关
- 日志：`db-data/cargo_check_default_022verify.log`

### 2) `cargo check -j 2 --no-default-features --features sync-cli`

- **首跑（23:08:44 起）：退出码 101**，唯一错误：

  ```text
  error[E0405]: cannot find trait `SurrealValue` in this scope
      --> src\version_management\cli.rs:4290:41
  4290 |     #[derive(Debug, serde::Deserialize, surrealdb::types::SurrealValue)]
  ```

  定性：**并发编辑中间态，非本分支持久缺陷**。`cli.rs` 的 LastWriteTime 为 23:11:42（晚于首跑开始），是同组成员 specs/023 未提交工作正在进行的编辑；首跑后再读文件，该处已改为 `use surrealdb::types::SurrealValue;`（cli.rs:4289）+ 短名 derive（cli.rs:4291），与 Cargo.toml:76-77 "SurrealValue derive 宏默认使用 ::surrealdb_types 路径，需显式引入 surrealdb-types" 的既有约定吻合。
  日志：`db-data/cargo_check_synccli_022verify.log`

- **复跑（23:13）：退出码 0**（`CARGO_CHECK_SYNCCLI_RERUN_EXIT=0`），耗时 ~16s；**aios-database 无 error、无 warning**（同样 `Checking aios-database v0.3.34` → `Finished`）。
  日志：`db-data/cargo_check_synccli_022verify_rerun.log`

### 汇总

| 命令 | 退出码 | aios-database 错误 | 备注 |
|---|---|---|---|
| `cargo check -j 2` | 0 | 无（也无警告） | 一次通过 |
| `cargo check -j 2 --no-default-features --features sync-cli`（首跑） | 101 | E0405 ×1（cli.rs:4290） | 队友并发编辑中间态 |
| 同上（复跑） | 0 | 无（也无警告） | 以复跑为准：通过 |

**最终结论**：两套 feature 组合下 aios-database 均编译干净；full sync 锚点链路 fail-closed 成立（锚点失败必传播、PendingCommit 同 fingerprint 门禁恢复、锚点固化严格晚于全部 PE/ATT/pe_owner 边写入 join）。
