# Phase 1 — Tubi & friends 走 ModelWriterBackend trait

## 出处

`03-writer-architecture.md` 的「Canonical tables × backend stages」表标了 5 个 Phase 1 gap：`raw_tubi_info` / `raw_tubi_relate` / `raw_aabb(tubi)` / `raw_trans` / `raw_vec3(tubi)` / `raw_refno_assoc_index`。这些表当前由 `cata_model.rs` 与 `refno_assoc_index.rs` 直接调 SurrealQL，绕过 `ModelWriterBackend`，所以未来任何非 Surreal backend（DuckLake / Parquet / compare）都拿不到这些行。

本文档把闭合这些 gap 的工作拆成可派工的目标包草稿。它是 mission Phase 1 收尾的下个 goal 候选，不是已经启动的执行计划。

## Outcome

让 `ModelWriterBackend` 拥有完整的 Phase 1 持久化阶段集，所有 canonical raw 表的写入都通过 trait 流出；默认 Surreal 行为按字节级兼容现状，drain-only 安全无副作用，验证以 CLI JSON + web_server POST 为主。

## Context

- 主线代码已存在的阶段：`write_base_batch / persist_mesh_results / persist_inst_relate_aabb / reconcile_missing_neg_relations / run_boolean_bridge`。
- `cata_model.rs::gen_cata_geos` 内部直接调：
  - `crate::fast_model::utils::save_transforms_to_surreal(&tubi_trans_map)`
  - `crate::fast_model::utils::save_aabb_to_surreal(&tubi_aabb_map)`
  - `crate::fast_model::utils::save_pts_to_surreal(&tubi_pts_map)`
  - SurrealQL `tubi_relates.join("")` query
- `refno_assoc_index.rs` 单独维护 refno_assoc_index 的写入；当前不通过 trait。
- 现有 guard：`enable_surreal_outputs = db_option.use_surrealdb && db_option.model_writer_mode.writes_to_surreal()` —— drain-only 模式下整块写入被短路。

## Constraints

- 不能改变默认 Surreal 路径的可观察结果（行顺序、错误时机、SQL 语义、日志格式都要保持）。
- drain-only 模式必须仍然不写 SurrealDB，且新阶段一定要 push skipped report 到 stage_reports（与现有 cleanup/mesh_persist/... 一致）。
- 不引入 DuckLake/Parquet 真实实现；只让 trait 可以覆盖这些表。
- 不动 mesh / boolean 算法本身，只动持久化职责的归属。
- 不允许在迁移过程中删除或重写已有 SurrealDB 数据。

## Non-Goals

- 不实现真实 DuckLake/Parquet backend；不补 compare 模式。
- 不重写 `cata_model.rs::gen_cata_geos` 的业务逻辑（5000+ 行），只把上面 4 个外部写入收编进 trait。
- 不优化 transform / aabb / pts 的序列化方式，保留现状以便对照 byte-level parity。

## Ask Before

- git commit / push / 创建 PR（按 brief.md 默认约束继承）。
- 任何可能改变 SurrealQL relate 语义或主键的修改。
- 把接口预留升级为真实 DuckLake/Parquet/compare backend 实现。

## Slices

| Slice | 目的 | 主要文件 | Done when | 风险 |
|---|---|---|---|---|
| 1 | 设计 trait 新阶段 | `model_writer.rs` | `persist_tubi_data` / `persist_transforms` / `update_refno_assoc_index` 三个阶段方法在 trait 上存在，含 request/report 类型，默认实现为安全 skipped report | 接口一次扩太大编译面广 |
| 2 | Surreal backend 包装现有 helper | `model_writer.rs::SurrealModelWriterBackend` | 三个新阶段在 Surreal backend 里调用现有 `save_transforms_to_surreal` / `save_aabb_to_surreal` / `save_pts_to_surreal` / `model_primary_db().query(tubi_relates)` / `refno_assoc_index::write_*`，行为兼容 | helper 错误传播方式不一致 |
| 3 | DrainOnly backend 实现 NoOp | `model_writer.rs::DrainOnlyModelWriterBackend` | 三个新阶段在 drain-only 里全部 record_skipped，原因字符串明确指明这是 tubi/transform/refno_assoc | 不小心调到 Surreal helper |
| 4 | cata_model.rs 把直调改成 trait 调用 | `cata_model.rs::gen_cata_geos` | line 6202-6260 块改为 `model_writer.persist_tubi_data(...)`；`enable_surreal_outputs` guard 改成 `db_option.model_writer_mode.writes_to_surreal()` 判断（或全交给 backend 决定） | 6000+ 行函数改错；调用点拿不到 `Arc<dyn ModelWriterBackend>` 实例 |
| 5 | refno_assoc_index 走 trait | `refno_assoc_index.rs` + 调用点 | 写入路径都通过 backend.update_refno_assoc_index(...) | 现有 cleanup 路径同时依赖 refno_assoc_index，顺序敏感 |
| 6 | 验证面 | `model_writer_verify`、web_server POST | CLI `--exec drain-only` 含三个新阶段的 skipped report；web_server POST 跑一次真实 BRAN 模型生成 → SurrealDB 内 tubi_info/tubi_relate/inst_relate_aabb 等行数前后一致 | web_server 启动失败、SurrealDB schema 不匹配 |

## Sequencing

Slice 1 先完成接口冻结。Slice 2、3 并行可写，但合并前要一起编译过。Slice 4 紧跟 Slice 2，要拿到 backend 实例向下传播。Slice 5 顺带做或独立做都可以。Slice 6 在 4/5 合入后做。

## Acceptance Criteria

- [ ] trait 新增 `persist_tubi_data` / `persist_transforms` / `update_refno_assoc_index` 三阶段，含 request/report 类型；编译通过。
- [ ] `cata_model.rs::gen_cata_geos` 不再直接调 `save_transforms_to_surreal` / `save_aabb_to_surreal` / `save_pts_to_surreal` / `model_primary_db().query(tubi_relates)`，证据是 grep 0 命中。
- [ ] `refno_assoc_index` 写入只通过 backend trait，证据是 grep 0 直调命中。
- [ ] drain-only 模式下 `model_writer_verify --exec --mode drain-only` 输出含 tubi/transform/refno_assoc 三个 skipped report。
- [ ] web_server 启动后 POST 跑一次真实模型生成，前后对 `inst_info` / `inst_relate` / `tubi_info` / `tubi_relate` / `inst_relate_aabb` / `trans` / `aabb` / `vec3` 八张表做行数与若干样本 record id 比对，结果一致。
- [ ] `progress.jsonl` 追加：CLI 命令、POST URL、行数比对 SQL 与结果。

## Required Evidence

| Requirement | Evidence | Where |
|---|---|---|
| 接口扩展 | trait + impl 编译通过 | `cargo check` 输出 |
| Surreal 兼容 | web_server POST 前后行数 SQL | `progress.jsonl` |
| DrainOnly 安全 | `--exec` JSON 含三个 skipped 阶段 | CLI stdout 副本 |
| cata 解耦 | grep 不再命中直调 | `progress.jsonl` |
| refno_assoc 解耦 | grep 不再命中直调 | `progress.jsonl` |

## Risk register

- **风险 R1**：cata_model.rs gen_cata_geos 内拿不到 Arc<dyn ModelWriterBackend>。当前函数签名通过 `db_option` 携带 ModelWriterMode，但没拿 backend 实例。Slice 4 需要在函数签名增加 `model_writer: Arc<dyn ModelWriterBackend>` 参数，或让 caller 在调用 gen_cata_geos 之前预先把 backend 注入到 cata_resolve_cache 之类的上下文。任意一种都需要 caller 改动。
- **风险 R2**：tubi 路径的 ID 生成依赖于 SurrealQL 语义（INSERT IGNORE / RELATE 隐式创建），换成 trait + 多 backend 时这些隐式语义需要变成显式 canonical record，否则 DuckLake/Parquet 写不出等价数据。Slice 1 设计接口时要明确把 INSERT IGNORE 等行为编进 request type 或文档。
- **风险 R3**：web_server POST 验证需要稳定的 BRAN 样例数据。本机环境如不可用，需在 `progress.jsonl` 显式记录跳过原因。

## Sketch of the trait additions (non-binding)

```rust
pub struct PersistTubiRequest<'a> {
    pub tubi_info_map: &'a DashMap<String, TubiInfoData>,
    pub tubi_trans_map: &'a DashMap<String, Transform>,
    pub tubi_aabb_map: &'a DashMap<String, Aabb>,
    pub tubi_pts_map: &'a DashMap<u64, String>,
    pub tubi_relate_sql_chunks: &'a [String],
}

#[async_trait]
pub trait ModelWriterBackend {
    // existing methods unchanged...

    async fn persist_tubi_data(
        &self,
        request: PersistTubiRequest<'_>,
    ) -> anyhow::Result<ModelWriterStageReport> {
        Ok(ModelWriterStageReport::skipped(
            "tubi_persist",
            "backend has no tubi persistence configured",
            0,
        ))
    }

    async fn persist_transforms(
        &self,
        trans_map: &DashMap<String, Transform>,
    ) -> anyhow::Result<ModelWriterStageReport> {
        Ok(ModelWriterStageReport::skipped(
            "transforms_persist",
            "backend has no transforms persistence configured",
            trans_map.len(),
        ))
    }

    async fn update_refno_assoc_index(
        &self,
        all_refnos: &[RefnoEnum],
    ) -> anyhow::Result<ModelWriterStageReport> {
        Ok(ModelWriterStageReport::skipped(
            "refno_assoc_index",
            "backend has no refno_assoc_index maintenance configured",
            all_refnos.len(),
        ))
    }
}
```

具体字段在 Slice 1 实施时按真实调用点 narrow。本草图只是给目标包审稿用，不预先冻结签名。

## How to start

把本文档作为 goal 输入，复用 `.factory/skill-runner` 或 plannotator-setup-goal skill 包装成 `goals/<slug>/{brief,plan,verification,blockers,goal-prompt}.md` 五件套；再以「执行 Slice 1」作为首个执行入口推进。
