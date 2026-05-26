# Blockers: DuckLake ModelWriter Backend

## Open Questions

- DuckDB Rust crate 的具体 version pin：执行 Slice 1 时按 `cargo search duckdb` 与 `cargo add duckdb --optional --no-default-features --features bundled` 解析当前最新 stable，写入 Cargo.toml 后立即冻结；若解析到的版本不支持 DuckLake `INSTALL ducklake; LOAD ducklake; ATTACH 'ducklake:...'` 链路，必须停下问用户：(a) 锁旧版 DuckDB 但确保 DuckLake extension 可用，(b) 切换到非-bundled DuckDB 并指定本机 DuckDB 路径，(c) 推迟本 goal 至 DuckDB / DuckLake 版本兼容后。
- `model_writer_verify` binary 与 web_server 路由文件位置已确认：`src/bin/model_writer_verify.rs`、`src/web_server/model_writer_verify.rs`、`src/web_server/mod.rs`（grep verified 2026-05-17）；Slice 5 在这三个文件就地扩 `ducklake` mode，不新增 binary、不新增公开路由。
- `output/<project>/model_writer_storage/ducklake/` 目录是否已被其他流程占用（参考 `output/AvevaMarineSample/profile/` 等历史目录）：执行 Slice 1 init 前先 `ls` 确认；若被占用需先与用户确认是否清理或换路径。
- 4 类 Known Gap 表的统一标识字符串：建议数组形态 `["raw_tubi_info", "raw_tubi_relate", "raw_aabb(tubi)", "raw_trans", "raw_vec3(tubi)", "raw_refno_assoc_index"]`（6 项 / 4 类），但 finalize 报告的 JSON key 命名（`known_gap_tables` vs `phase1_trait_gap_tables`）需在 Slice 1 设计时定下后续不再改。
- SQL parity 的"采样 record id"取样策略是否需要在不同 dbnum 之间稳定：本 plan 默认 `ORDER BY <primary key> LIMIT 3 OFFSET 0 / midpoint / last`，若 SurrealDB 与 DuckDB 对同一 hash 字符串排序不一致，需在 Slice 6 增加 deterministic 排序条款。

## Stop And Ask

- 需要 git commit / push / 创建 PR（按 brief.md 默认约束）。
- 需要删除、移动、批量重命名现有源码文件（含 `pe_transform_store.rs`、`Cargo.toml` 节移动、`model_writer.rs` 模块拆分）。
- 需要修改公开 CLI 参数且造成不兼容（含改 `--model-writer` 取值集合移除既有 `surreal` / `drain-only` 字面值）。
- 需要把 5 类 Phase 1 trait gap 表（tubi / transforms / refno_assoc）的写入纳入本 goal（即与 09-phase-1-tubi-trait-migration 合并）。
- 需要把 projection 9 张表或 projection 刷新 SQL 纳入本 goal。
- 需要引入 `duckdb` 之外的新外部依赖、启用其他重型 feature、或对 `Cargo.toml` 中本 goal 不需要的 feature 做改动。
- 需要修改 SurrealDB 依赖源、Cargo patch 节。
- 需要任何会改写 / 删除 / 清理 SurrealDB 中现有模型数据的 cleanup 语义变更。
- 需要连接远端服务器、修改运行环境（除 NASM / PATH 临时设置外）、或安装 DuckDB CLI 之外的系统依赖（如 vcpkg、cmake 子系统等）。
- 需要删除或重写 `pe_transform_store::register_ducklake` 空 stub。
- 发现默认 Surreal / DrainOnly 路径无法在新增 `ModelWriterMode::DuckLake` 后保持字节级兼容，需要产品 / 架构取舍。
- DuckLake parity 出现本 plan 未列入 Known Gap 范围内的差异（如非 tubi / 非 transforms / 非 refno_assoc 的表对账失败），需求用户决策是按代码 bug 修，还是补 Known Gap 范围，还是降级标记 fail。
- web_server POST 验证需要的 endpoint 还不是 `/api/model/writer-verify` 而需新增公开路由。
- `--refresh-transform 1112` 或同等真实生成 CLI 在本机不可用（含 SurrealDB / DuckDB / 数据样本缺失），需要换 dbnum 或换 generation 入口。

## Dangerous Or High-Risk Actions

- SurrealDB cleanup / 删除模型关系 / 重建 SurrealDB schema（包含 `inst_info` / `inst_relate` / `inst_geo` / `geo_relate` / `aabb` / `trans` / `vec3` / `inst_relate_aabb` / `neg_relate` / `ngmr_relate` 任一 8 张表的破坏性操作）。
- 远端服务器操作或任何需要凭据的操作（含 git push、远端 SurrealDB 连接、生产 DuckLake metadata 改写）。
- git push / force push / PR 创建。
- 写入到本机已有的 `pe_transform/` 路径（`output/AvevaMarineSample/pe_transform/`），可能与现有 pe_transform Parquet 文件冲突。
- 启用 DuckDB bundled feature 后第一次 cargo build：会拉 C 源代码长时间编译，可能与本机 cmake / NASM 工具链冲突；执行前先 `cargo check` 一遍单 ducklake feature 看是否触发 build script。
- 批量格式化或大范围重构与本目标无关文件（`rustfmt --check` 通过即可，不主动 `rustfmt --emit files`）。
- 启动 web_server 时 `target/debug/web_server.exe` 占用已知历史阻塞（参见 `model-writer-backend-abstraction/progress.jsonl@2026-05-11T14:21`）；启动前先检查端口与文件锁。
- DuckLake metadata 文件并发访问：若同时有 DuckDB CLI 或其它进程持有 `metadata.ducklake`，可能损坏 metadata；执行 init 前确认无其它进程持有。

## Known Blockers

- DuckDB bundled feature 编译需 cmake / 完整 MSVC 工具链。本机历史记录 `target-rus248` 编译需 NASM 在 `C:\Program Files\NASM` 加入 PATH；若启用 ducklake feature 触发更多 build script 失败，按 `progress.jsonl` 记录完整错误后停下询问。
- DuckLake 是 DuckDB extension，需 `INSTALL ducklake; LOAD ducklake;` 在线安装。本机 Rust 工具链运行时第一次执行此 SQL 可能需要网络访问 DuckDB extension repository；离线环境会失败。Slice 1 init 必须包含此 SQL 的错误处理与明确错误消息。
- web_server `target/debug/web_server.exe` 在历史会话被旧 :3100 服务锁定的现象（`model-writer-backend-abstraction/progress.jsonl@2026-05-11T14:21`）可能复现。已知恢复方案：使用独立 target 目录（如 `target-ducklake/`）+ `WEB_SERVER_PORT=3199`。
- `cargo check --lib` 大 feature 组合首次编译会非常慢（pe-transform-backends 历史显示完整 cargo build 44s+），加入 ducklake bundled 后可能 90s+；记录耗时即可，不视为阻塞。
- 当前主仓未提交改动列表（按 `model-writer-backend-abstraction/progress.jsonl@2026-05-11T15:34` 记录 + 当前 plant-model-gen 项目根的 `.tmp-pms-loggedin.png` 等本地 artifact）可能导致 `git diff --stat` 含无关项；执行 Slice 6 范围审计时需手动过滤本 goal 不相关文件，不能自动 commit。
- DuckDB 与 SurrealDB 的类型映射差异（u64 → BIGINT；hash 字符串编码；几何/transform 矩阵 JSON 序列化精度）已在 pe-transform-backends 实测出 max_delta=0.000854 的 float 精度差（`progress.md` Phase 10 finding）。本 goal 写入的 9 张 raw 表理论上无 float 精度问题（只含 id / hash / 布尔 / 计数），但 mesh AABB / Vec3 payload 可能含 float；Slice 6 parity SQL 需要为这些列设置容差或显式 exclude，并在 progress.jsonl 记录决策。
- `model_writer_verify` binary 在 `model-writer-backend-abstraction` goal 内已加入；若其后续会话被改名或挪位置，Slice 5 需要先 `rg "fn main" src/bin/` 重新定位。
