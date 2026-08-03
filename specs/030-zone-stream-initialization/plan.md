# ZoneStream 按 ZONE 双缓冲初始化 — 开发计划（worktree 版）

> 输入：用户设计稿《ZoneStream：按 ZONE 双缓冲初始化方案》。
> 目标产物：`ADR-0016` + `specs/030-zone-stream-initialization` + 可运行的 ZoneStream 初始化模式。
> 约束基线：`AGENTS.md`（禁 `cargo test`，web 走 HTTP/POST 验证，DB 走 aios-database CLI + JSON）。

---

> **状态：D0 决策已定案（2026-08-03），基线已提交。** 见 §1 的结论块与 §0 的「基线落地记录」。

## 0. 现状核查（写计划前实测，非假设）

| # | 事实 | 证据 | 对计划的影响 |
|---|---|---|---|
| F1 | `GenerationOutputBackfill` / `ZoneScopeSeal` / `zone_stream` 在 `src/` 全域 **0 命中** | 全仓 grep | 设计稿「复用 ADR-0015/spec029 定义的 seam」目前**无实现可复用**，seam 必须由本期或 spec029 先落地 |
| F2 | `docs/adr/0015-*.md` 与 `specs/029-zone-slice-kvmem-backfill/` 均为 **未跟踪文件**（`??`） | `git status` | 新 worktree 从 HEAD 切出**看不到这两份文档** |
| F3 | 主工作树 **158 项未提交**：67 个 `src/*.rs` 已修改 + `src/web_server/generation_lock.rs`、`src/versioned_db/pe_graph_kvmem.rs`、`pe_graph_seed.rs` 等未跟踪新文件 | `git status --porcelain` | spec029 声明的前提「2026-08-03 裁剪解析 + scoped 生成修复」**就在这堆未提交改动里**；从 `HEAD=b7f8d755` 切 worktree 会**丢掉前提** |
| F4 | 当前分支 `codex/fix-visible-insts-parquet`，与 origin 同步；已存在 10 个 worktree，其中 6 个 `prunable` | `git worktree list` | 需先 `git worktree prune` 清理，再按统一命名新建 |
| F5 | spec 028（形态母体）在另一 worktree `plant-model-gen-mem-gen-cache`（`feat/028-mem-generation-cache`） | `git worktree list` | ZoneStream 与 028 的抽象会正面相撞，需明确收编方向 |
| F6 | `src/web_server/managed_project_sites.rs` = **601 KB 单文件**，`spawn_parse_process`(L9945) / `spawn_generation_process`(L10189) / `ensure_site_db_started`(L11840) 都在其中 | 文件大小 + grep | 这是**并行 worktree 的头号冲突热点**，必须限制只有一个 worktree 能改它 |

### 由 F1–F3 得出的阻塞项

**在建立 worktree 之前必须先解决基线问题**，否则 worktree 里做的一切都建立在缺失前提之上。见 §1 的 D0。

### 基线落地记录（2026-08-03，按 D0-A 执行完毕）

分支 `codex/fix-visible-insts-parquet`，在 `b7f8d755` 之上切出三个语义提交：

| 提交 | 内容 |
|---|---|
| `5de89938` `fix(gen):` | 61 文件 / +6059 −1448。`pe_graph_kvmem`、`pe_graph_seed`、`generation_lock` 三个新模块；`pe_owner` 确立为层级唯一持久表示；`generation_read` 与 `gen_model` 读路径对齐 |
| `e2a40040` `docs(adr):` | ADR-0012..0015 + `specs/029` 入库；`CONTEXT.md` 新增 ref0 / 层级投影种子 / PE 层级关系，重写「生成缓存库」 |
| `ca760cbe` `chore(repo):` | 删除 `docs/plans/` 51 份归档计划与 `tests/tree_cata_hash_stats.rs`；补 smoke 脚本与 DbOption；`.gitignore` 忽略 `/.codex-tmp/` |

> 刻意未入库：`docs/verification/*.png` 共 11 张验证截图。该目录既有 2 个跟踪文件都是 `.md`，仓库无跟踪验证截图的先例，故保持未跟踪。
> 若判定为误删，`docs/plans` 的清理集中在 `ca760cbe` 单个提交，`git revert ca760cbe` 即可整体恢复。

**本计划文件本身位于 `.plannotator/`，而该目录在 `.gitignore:187` 被忽略**，因此不会随分支进入 worktree。Phase 1 建立 `specs/030-zone-stream-initialization/` 时需放一份跟踪版计划进去。

---

## 1. 前置决策（已定案 2026-08-03）

| 决策 | 结论 |
|---|---|
| **D0-A** 基线 | **A1** — 整理成 3 个语义提交并推 origin，worktree 从该提交切出。已执行，见 §0 基线落地记录 |
| **D0-B** backfill seam | **B1** — 由 spec 030 首次实现 `GenerationOutputBackfill`，并回标 spec029 为「seam 由 030 实现」 |
| **D0-C** spec 028 | **C1** — 030 视为 028 的超集，028 worktree 冻结，spec 028 标记 `merged-into-030` |
| **D0-D** worktree 拓扑 | **3 个** — 1 主线 + 2 并行（读路由、回填），依 §2.2 单写者规则 |

<details>
<summary>原始备选方案（存档）</summary>

### D0-A 基线提交策略（**阻塞**）
ZoneStream 依赖「裁剪解析 + scoped 生成」修复与 ADR-0015/spec029 文档，三者都未提交。三选一：

- **A1（推荐）**：在主工作树把当前改动整理成一个基线提交（或 2–3 个语义提交：`fix(model): …` / `docs(adr): 0015 + spec029` / `feat(lock): generation_lock`），推到 `origin`，ZoneStream worktree 从该提交切出。
- **A2**：`git stash` 后从干净 HEAD 切 worktree —— **不可行**，会丢前提（F3）。
- **A3**：ZoneStream worktree 从主工作树当前状态复制未提交改动 —— 可跑但无法 review、无法回溯，拒绝。

### D0-B `GenerationOutputBackfill` seam 归属（**阻塞**）
- **B1（推荐）**：本期在 ZoneStream 内**首次实现** seam（trait + SurrealQL 顺序实现），并回标 spec029 为「seam 由 030 实现」。理由：029 站点切片场景与 030 初始化场景的回填清单几乎同构，先做 030 的更完整。
- **B2**：先把 spec029 的 seam 单独落地并合入主干，030 再消费。多一次合流，但 029 可独立验收。

### D0-C spec 028 收编方向
- **C1（推荐）**：030 是 028 的超集实现，028 worktree 冻结，spec 028 标记 `merged-into-030`。
- **C2**：028 先合主干，030 在其上改造为「ZONE 粒度轮转」。

</details>

---

## 2. worktree 拓扑与分支策略

### 2.1 命名与落点
沿用仓库现有的**同级目录**惯例（`plant-model-gen-<topic>`），不用 `.worktrees/`（现存两个均已 prunable）。

| worktree | 分支 | 职责 | 主要触碰 | 冲突面 |
|---|---|---|---|---|
| `plant-model-gen-zone-stream`（主线） | `feat/030-zone-stream` | 文档、配置模式、任务/状态机、编排、发布与可见性、管理页 | `web_server/models.rs`、`managed_project_sites.rs`、`handlers.rs`、`web_api/*` | **独占 `managed_project_sites.rs`** |
| `plant-model-gen-zs-read-route`（并行） | `feat/030-composite-read-session` | 显式 client 化 + 复合读 session + 清除绕过 session 的全局读 | `generation_read/*`、`fast_model/gen_model/*` | 与主线基本不重叠 |
| `plant-model-gen-zs-backfill`（并行） | `feat/030-generation-output-backfill` | `GenerationOutputBackfill` trait + 双 WS 顺序回填 + attempt manifest | **新模块** `src/zone_stream/backfill/*` | 新增文件为主，冲突最小 |

### 2.2 纪律
1. **单写者规则**：`managed_project_sites.rs`、`web_server/models.rs` 只允许主线 worktree 修改；并行 worktree 若需要挂接点，先由主线预留 trait/函数签名再消费。
2. **合流方向单向**：并行分支 → 主线分支 → 主干。并行分支不得互相 merge。
3. **rebase 节奏**：并行分支每完成一个 Phase 就 rebase 到主线分支最新，避免长尾冲突。
4. **构建目录是共享的，不是隔离的**（2026-08-03 实测更正）：环境变量
   `CARGO_TARGET_DIR=D:\Rust\target` 全局生效，三个 worktree **共用同一个 target 目录**，
   `.cargo/config.toml` 另有 `rustc-wrapper = "sccache"`。因此：
   - **不要并行跑 cargo**：同一 target 目录上的两个 cargo 进程会互相阻塞在 cargo lock 上；
     并行只体现在「编辑 + 思考」，编译必须串行。
   - 在 worktree 之间来回构建会反复重编 `aios_database` 本体（外部依赖与 `pdms-io-fork`
     由 sccache 复用），单次 `cargo check` 约 2 分钟、`cargo build --bin aios-database` 约 2.5 分钟。
   - **已决定隔离**（2026-08-03）：每条线在自己的 session 里先 dot-source
     `. scripts/dev/use-worktree-target.ps1`，把 `CARGO_TARGET_DIR` 指到
     `<base>/target-<worktree 名>`，换取真正的并行编译；代价是三份完整产物占磁盘。
     注意环境变量优先级高于 `.cargo/config.toml` 的 `build.target-dir`，改配置文件无效。
5. **运行时隔离**：ZoneStream 会拉 surreal sidecar，各 worktree 必须用**不同 `db_port` 段**与不同 `runtime_dir`，否则会互相杀进程（参考 `stop_site_ws_db_for_exclusivity` 的按端口杀逻辑）。
6. 开工前 `git worktree prune` 清掉 6 个 prunable 条目。

---

## 3. 不变量（实施中不得违背，来自设计稿）

- **I1 Legacy 零影响**：默认值、旧入口、旧 UI、解析与生成执行顺序完全不变；旧 Parse/Generate 入口**内部不得增加模式判断**，分流只发生在 managed-site 编排入口。
- **I2 ZONE 是最小流水边界**：不做半 ZONE 生成；同一时刻**最多一个** generator，**最多一个** backfill。
- **I3 重叠只允许一处**：「解析下一 ZONE」可与「生成/回填当前 ZONE」重叠，不做三阶段全并发、不做双 generator。
- **I4 路由必须精确**：复合 session 禁止「两库都查、取先返回者」；重复 / 缺失 / 同 ID 不同内容一律**硬失败**。
- **I5 不复用旧模式开关**：不碰 `pipeline_db_mode`、`GenerationReadBackendKind`、`ModelWriterMode`。
- **I6 不伪造 Ready**：ZONE 级只发 `ZoneScopeSeal`；dbnum 级 `pe_owner/seed Ready` 只在整库审计通过后真实发布。
- **I7 发布原子性**：`model_gen` baseline anchor 与 create-only dbnum publication 必须**同一个 Surreal 事务**。
- **I8 未发布即不可见**：公共入口一律经 Published gate，未发布点查返回 `404`（不是空结果）。
- **I9 失败不静默**：未知模式值启动失败；parquet feature 缺失或 exporter `skipped` 在 ZoneStream 中**视为失败**；不回退 Legacy。
- **I10 三类持久状态封顶**：只新增 `initialization_runs`（SQLite）、`initialization_zone_checkpoint`、`initialization_dbnum_publication`（目标 RocksDB），不再造第四个事实源。

---

## 4. 阶段与任务

### Phase 1 — 文档与契约固化（主线）
- T1.1 `docs/adr/0016-zone-stream-initialization-mode.md`：记录模式分流、双 slot、发布顺序、三类持久状态、不纳入首版清单；链接 ADR-0015 / spec029。
- T1.2 `specs/030-zone-stream-initialization/spec.md`：User Need / Evidence / Scope / Non-Goals / Requirements / Acceptance Criteria，结构对齐 spec029。
- T1.3 `CONTEXT.md` 补术语：**ZONE 流水初始化**、**ZONE 工作区**、**共享生成依赖库**、**dbnum 发布**、**ZONE 检查点**。
- T1.4 冻结 **contract hash 定义**（哪些字段进 hash：模式、预算、ZONE plan、源清单、契约版本）—— Resume 判等直接依赖它，必须先定死。
- **出口**：文档合入主线分支，并行 worktree 从此点切出。

### Phase 2 — 配置与模式分流骨架（主线）
- T2.1 `InitializationPipelineMode { Legacy, ZoneStream }`，TOML/站点配置值 `legacy | zone-stream`；缺省 `legacy`；**未知值启动失败**（`src/options.rs` + `web_server/models.rs`）。
- T2.2 `zone_stream_memory_budget_mib`，默认 `4096`，可在 Resume 前调大。
- T2.3 SQLite 迁移：`managed_project_sites` 增列 + `db_mode_from_string` 同款的容错读取（参考 `managed_project_sites.rs` L2662 附近的行映射）。
- T2.4 初始化开始后**禁止切换模式**（改配置返回明确错误，要求新建目标目录/站点）。
- T2.5 编排入口分流：新增 `src/zone_stream/orchestrator.rs`，在 managed-site 编排层按模式跳转；**不改** `spawn_parse_process`(L9945) / `spawn_generation_process`(L10189) 内部逻辑。
- **验证**：`cargo build --bin aios-database`；HTTP 建站点分别传 `legacy` / `zone-stream` / `bogus`，第三个应启动/保存失败。

> **Phase 2 完成记录（2026-08-03，提交 `e2cdb02`）**
>
> 模式枚举落在 `src/options.rs`（与 `ModelWriterMode` 同源、不受 `web_server` feature 门控），
> `web_server::models` 只做转出；SQLite 与 TOML 两侧共用同一个 `parse_initialization_pipeline_mode`，
> 避免同一字面量在两处给出不同结论。SQLite schema 升到 v10。
>
> 已验证（`aios-database -c <config>` CLI，符合 AGENTS.md 不用 cargo test）：
>
> | 配置 | 结果 |
> |---|---|
> | `initialization_pipeline = "bogus"` | 启动失败，`Error: 未知 initialization_pipeline=bogus；仅支持 legacy 或 zone-stream，系统不会静默回退`，exit 1 |
> | `initialization_pipeline = "zone-stream"` | 接受，启动日志打印 `initialization_pipeline: zone-stream` |
> | `zone_stream_memory_budget_mib = 0` | 启动失败，`Error: zone_stream_memory_budget_mib 不能为 0；预算需覆盖 deps 与两个 slot` |
> | 完全不配置 | `initialization_pipeline: legacy`，Legacy 默认未变 |
>
> **尚未验证**：站点侧 HTTP 建站/改站的三值行为，以及「初始化已开始禁止切模式」的拒绝路径。
> 这两项需要管理端 API 起服务后走 HTTP，且与 Phase 3 的 `TaskType::ZoneStreamInitialization`
> 及 409 映射同批验证更经济，故顺延到 Phase 3 出口。

### Phase 3 — 任务、状态机与运行记录（主线）
- T3.1 `TaskType::ZoneStreamInitialization`（`web_server/models.rs` L82 枚举）：`POST /api/admin/tasks` = Start，`/cancel` = Stop，`/retry` = Resume。
- T3.2 ZoneStream 站点调用旧 Parse/Generate 任务返回 **409**；Legacy 站点行为不变。
- T3.3 `ManagedInitializationStatus`（含 `PartiallyReady`）作为**独立枚举**，不并入 `ManagedSiteStatus`（L755）；站点可同时 `Running + PartiallyReady`。
- T3.4 SQLite `initialization_runs`：run/site、状态、源/契约/ZONE 规划哈希、目标 dbnums、两 slot 状态、当前 attempt manifest、错误、完整指标 JSON。
- T3.5 目标写入全程持有 project mutation lock（对接未跟踪的 `src/web_server/generation_lock.rs`）。
- **验证**：HTTP 三个动作各跑一次，`initialization_runs` 行状态转移正确；旧任务 409 可复现。

### Phase 4 — mem sidecar 与双 slot 主管（主线）
- T4.1 每次运行启动 **loopback-only** Surreal kv-mem WS sidecar：动态端口 + run-scoped namespace，含 `deps` / `slot-a` / `slot-b` 三个逻辑库。
- T4.2 两个常驻 slot supervisor（A/B），维护 slot generation 与防 ABA lease。
- T4.3 每个 ZONE 起**新的短命生成子进程**，退出即清 transform / CATA / DbMeta / 报告等全局缓存影响。
- T4.4 sidecar 生命周期接入既有 reaper（`specs/017-sidecar-process-reaper`），崩溃后**不恢复内存内容**，依据源清单 + RocksDB checkpoint 重建。
- T4.5 端口与 runtime 目录隔离，避免与并行 worktree/其他站点互杀。
- **验证**：CLI 起一次 run，`ss/netstat` 确认仅 loopback；kill sidecar 后 Resume 能重建。

> **Phase 4 进度（2026-08-03）**：T4.1 与 T4.2 的模块已落地并通过 `cargo check`，
> 但**尚未接入 orchestrator**（要等 Phase 6 的 deps epoch 与 ZONE 规划才有调用点），
> 因此当前对运行时零影响。
>
> - `zone_stream::sidecar`：动态 loopback 端口（内核分配，避免与站点管理的按端口杀进程逻辑
>   相撞）、run-scoped namespace `zs_<run_id>`、每次运行随机口令、就绪等待带超时、
>   pid 文件落在 `<runtime_dir>/zone-stream/<run_id>/` 供孤儿清理脚本发现、
>   `shutdown()` 显式停 + `Drop` 兜底。
> - `zone_stream::slot`：`SlotLease`（slot + 单调 generation）解决 slot 复用后的 ABA 问题 ——
>   过期 lease 硬失败，不做跨 slot 回退查询（D4）；`advance` 只允许
>   Parsing → Sealed → Generating → Backfilling 单向前进；`SlotPair::downstream_busy()`
>   表达 D2 的「同一时刻至多一个 generator/backfill」。
>
> **未做**：T4.3 短命生成子进程、T4.4 与 specs/017 reaper 的正式对接（当前只有 pid 文件 +
> Drop 兜底）、以及把两者接进编排循环。

### Phase 5 — 复合读 session 与精确路由（并行 worktree `zs-read-route`）
- T5.1 `SurrealVersionedReadSession` 改为**持有显式 client**（`src/generation_read/surreal.rs`），去掉隐式全局连接依赖。
- T5.2 新增复合 session：设计 PE/ATT/owner/transform → **当前 slot**；共享依赖 → **`deps`**；路由表为不可变 route map。
- T5.3 落实 **I4**：禁止双查取先到者；重复 / 缺失 / 同 ID 异内容硬失败并带可定位错误。
- T5.4 snapshot 身份 = run + slot generation + 防 ABA lease + deps epoch + 源哈希 + route map。
- T5.5 清除生成路径中**绕过 session 的全局读取**：owner、LOOP 高度、CATA ref、arrive/leave、transform fallback（`fast_model/gen_model/{session_query,resolve,context,cata_model}.rs` 等）。
- T5.6 Legacy adapter 仍绑定原主库，**结果逐位不变**（回归对拍）。
- **验证**：Legacy 路径跑一次 dbnum 7997 生成，与改造前产物 digest 一致。

### Phase 6 — deps epoch 与 ZONE 规划（主线）
- T6.1 dbnum 按现有升序；ZONE 按稳定的层级/refno 顺序。
- T6.2 每个 dbnum 先算**全部 ZONE 的依赖并集**，装载 SYSTEM、CATA 及闭包所需 DICT 元数据，生成不可变 `deps_epoch/hash` 后才启动 ZONE 流水。
- T6.3 `ZoneScopeSeal`：证明子树、祖先链、CATA 闭包、transform 完整；**不修改也不伪造** dbnum 级 `pe_owner Ready`（I6）。
- T6.4 ZONE plan 哈希入 `initialization_runs`，供 Resume 判等。
- **验证**：CLI 打印 deps epoch/hash 与 ZONE plan，两次运行同源应稳定一致。

### Phase 7 — 生成子进程绑定与产物回填（并行 worktree `zs-backfill`，依赖 D0-B）
- T7.1 生成子进程的全局模型写库**绑定当前 slot**；磁盘产物写 `<runtime_dir>/zone-stream/<run_id>/` 私有目录；生成过程**不直接写目标 RocksDB**。
- T7.2 `GenerationOutputBackfill` trait（首版：双 WS + SurrealQL 顺序回填）。搬运清单沿用 spec029 三组：源数据侧 / 模型产物侧 11 张 + `inst_info` / 值对表。
- T7.3 生成 barrier 后**先固化完整 Zone attempt manifest**，再开始目标写入。
- T7.4 幂等语义分层：
  - ZONE 独占的设计/关系记录 → 重试时**按 manifest 删除后重放**；
  - 祖先 / 共享依赖 / 内容寻址记录 → **只允许同 ID 同 fingerprint 的幂等写**，单个 ZONE 不得删除。
- T7.5 从目标库 **read-back 校验行数与 digest** 通过后，才创建 `initialization_zone_checkpoint`（create-only、同 payload 幂等、异 payload 冲突）。
- **验证**：单 ZONE 回填后 CLI 逐表比对行数；中途 kill 再跑，结果与一次成功等价。

### Phase 8 — 内存预算与反压（主线）
- T8.1 总预算覆盖 mem sidecar + deps + 两个 slot + 临时批次。
- T8.2 超限 → **停止接收下一 ZONE** 形成反压（不杀当前 ZONE）。
- T8.3 `deps + 单个 ZONE` 仍超限 → **当前 dbnum 失败**；不拆 PIPE、不回退 Legacy（I9）。
- T8.4 峰值分项（deps / slot / 临时 / 总）入指标。
- **验证**：把预算调到极小，观察反压与 dbnum 失败两条分支各触发一次。

### Phase 9 — dbnum 发布与可见性 gate（主线）
- T9.1 目标 RocksDB 新表 `initialization_dbnum_publication`（唯一 Published 注册表 + 公共 API allowlist）。
- T9.2 发布顺序**固定六步**：① 全 ZONE Verified → ② 整库层级/reference/模型关系审计并发布真实 dbnum 级 `pe_owner/seed Ready` → ③ 导出 Parquet + 构建 spatial index（先私有 staging，校验 SHA/schema/row count 后提升到最终路径）→ ④ 复核源文件集合 / canonical path / header60 / sesno / 逐文件 SHA-256 → ⑤ **同一 Surreal 事务**写 `model_gen` baseline anchor + create-only publication → ⑥ 首个 Published 后启动只读 web/viewer；全部完成后释放写锁并启用 watcher/增量。
- T9.3 初始化**不创建 data anchor**；单个 ZONE **永不发布 model anchor**。
- T9.4 可见性 gate：`/api/databases`、db status、`/api/v1/dbnums`、树/模型/Parquet/refno 点查、`/files/output` 全部过 gate，未发布点查 **404**；管理 API 仍可见 Pending/Failed。
- T9.5 `PartiallyReady` 阶段完全只读：watch / 增量 / 手工生成一律 **409**。
- **验证**：7329 发布后可访问且 7997 全公共入口不可见；7997 发布后两者皆可用。

### Phase 10 — Stop / Resume / 错误分级（主线）
- T10.1 Stop 在**当前解析或写回批次边界**生效；task `Cancelled`，run `Interrupted`；未完成 ZONE 不写 checkpoint。
- T10.2 Resume 仅在**源 manifest + contract hash + ZONE plan 三者一致**时继续同一 run：清空槽位、按 attempt manifest 清理半写独占行、跳过 Verified ZONE 与 Published dbnum。
- T10.3 可恢复错误（worker / 网络 / 导出）→ 只停当前 dbnum，其他 dbnum 继续，之后可 Resume。
- T10.4 不可恢复错误（源漂移、同 ID 异 fingerprint、目标非空或已有业务锚点、契约不一致）→ 停止全部剩余工作，已发布 dbnum 保持只读，要求显式创建/重置新目标后重跑。
- **验证**：在解析 / 生成 / 部分回填 / checkpoint 前后 / 导出 / 发布六个边界各 kill 一次。

### Phase 11 — 指标与管理页（主线）
- T11.1 指标 JSON 带 schema version、mode、run/source/contract hash。
- T11.2 记录：初始化总耗时、首个 dbnum 可用时间、每 dbnum Published 时间；依赖发现/装载、每 ZONE 解析/seal/生成/回填/验证/导出耗时；overlap 时长与比例；行数/字节数；重试次数；deps/slot/临时/总内存峰值。
- T11.3 管理页：Start/Stop/Resume 按钮、各 dbnum/ZONE 阶段、耗时摘要、运行指标 JSON 下载；**首版不做图表**。
- **验证**：管理页摘要与下载 JSON 逐字段一致。

### Phase 12 — 端到端验收（主线合流后）
数据集 **AvevaMarineSample `7329 + 7997`**，7997 覆盖 ZONE `=24381/144870`。

- V1 7329 先发布并可访问；此时 7997 未发布，所有公共入口均隐藏；随后两者全部可用。
- V2 Legacy 默认配置、旧 UI/任务、解析与生成顺序完全不变。
- V3 同一时刻最多一个 generator/backfill，且「下一 ZONE 解析」与「当前下游阶段」存在**真实重叠**（用指标里的 overlap 比例证明）。
- V4 六个边界 kill → Resume、清理、幂等性均正确。
- V5 单 dbnum 失败后继续后续 dbnum；源漂移停止全局；内存反压与超限；共享记录 fingerprint 冲突；重复运行结果 digest 稳定。
- V6 anchor 与 publication 同事务；未发布数据与静态产物不可见；PartiallyReady 阶段所有写入口被拒。
- V7 管理页摘要与下载 JSON 一致，关键耗时可供后续 Legacy 对比。
- **首版只跑 ZoneStream 性能基线，不做 Legacy 对照，不设固定加速门槛。**

---

## 5. 风险与缓解

| # | 风险 | 缓解 |
|---|---|---|
| R1 | **基线缺失**：worktree 从 HEAD 切出丢掉未提交的生成修复（F3） | D0-A 先落基线提交；worktree 建立后立刻验证 dbnum 7997 scoped 生成产物量级与 spec029 记录一致 |
| R2 | **601KB 单文件冲突**（F6） | §2.2 单写者规则；并行分支只消费主线预留的签名 |
| R3 | **Legacy 回归**：清除全局读取（T5.5）改到共用代码 | T5.6 逐位对拍 digest；Legacy adapter 绑定原主库不动 |
| R4 | **kv-mem 乐观 MVCC 写冲突**（ADR-0015 已记载） | 回填一律顺序写；生成期写并发沿用已收敛配置 |
| R5 | **回填半态** | attempt manifest 先固化 + 独占行按 manifest 删除重放 + read-back 校验后才 checkpoint |
| R6 | **发布事务跨越两种存储**（RocksDB registry + Surreal anchor） | 以 Surreal 事务为原子边界，RocksDB publication 为 create-only 幂等注册；冲突即失败不覆盖 |
| R7 | **028/029/030 三份抽象打架**（F5） | D0-C 先定收编方向，spec 状态字段显式标注 |
| R8 | **端口/进程互杀**（多 worktree 同跑） | 各 worktree 固定不同端口段与 runtime 目录 |

---

## 6. 明确不纳入首版

单 ZONE 切片站点、半 ZONE 生成、三阶段全并发、双 generator、PIPE 动态切分、已有站点迁移、SST 直灌、嵌入式 mem 承载体、PartiallyReady 期间增量写入、Legacy 性能实跑。

---

## 7. 交付顺序

```text
D0 决策 → 基线提交 → Phase 1（文档/契约）
                        ├─ 主线: Phase 2 → 3 → 4 → 6 → 8 → 9 → 10 → 11
                        ├─ 并行: Phase 5（读路由）  ──合流──┐
                        └─ 并行: Phase 7（回填）    ──合流──┤
                                                            └→ Phase 12 端到端验收
```

合流检查点：Phase 4 结束（sidecar 可用）时合入 Phase 5；Phase 6 结束（seal 可用）时合入 Phase 7。每个 Phase 出口都要求 `cargo build --bin aios-database` 通过 + 该 Phase 的 HTTP/CLI 验证项通过。

---

## 8. 待确认（D0-A..D 已定案，见 §1）

1. **`contract hash` 字段集**（T1.4）：Resume 判等直接依赖它，必须在 Phase 1 定死。当前提案纳入
   —— 模式（`zone-stream`）、`zone_stream_memory_budget_mib`、ZONE plan 哈希、源文件清单
   （canonical path + header60 + sesno + 逐文件 SHA-256）、契约 schema 版本。
   开放点：**内存预算是否该进 hash**？进了则调大预算就无法 Resume 同一 run（与设计稿
   「允许失败后调大预算再 Resume」冲突），**建议不进 hash，仅记录在 run 里**。
2. **`docs/verification/*.png`** 11 张验证截图是否要入库（当前按仓库惯例保持未跟踪）。
