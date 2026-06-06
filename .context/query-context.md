# SigMap Query Context
Generated: 2026-06-06T00:48:25.705Z

## .worktrees\model-persistence-trait\docs\plans\2026-05-09-model-write-trait-followup\v4-candidates.md
```
h1 v4 候选议题 (草稿，待 plannotator 正式立项)
h2 0. 状态前提
h2 1. 高优先级 (v4 主目标)
h3 1.1 DuckLake 真实写入实装
h3 1.2 Parquet 真正 `.parquet` 物化（替换 JSONL fallback）
h2 2. 中优先级
h3 2.1 `inst_relate_bool` / `inst_relate_cata_bool` Phase 2 boolean canonical records ⚠️ PARTIAL (PR #22 schema scaffold only)
h3 2.2 `async fn in trait` 调研报告 ✅ DONE (PR #21 — decision: NO migrate)
h3 2.3 DrainOnly stats `Mutex` → atomic ✅ DONE (PR #20)
h2 3. 低优先级（视情况打包到 v4 或更后）
h3 3.1 PR #11 合并时的 rebase 处理（A.3 deferred）
h3 3.2 9 个 mission docs 的格式标准化
h3 3.3 BridgeContext 升级到更彻底拆分
h2 4. v3 残余事项不在 v4 范围
h2 5. v4 拆 PR 建议节奏（草稿）
h2 6. 下一步建议
code-fence plain
```

## .worktrees\pe-transform-backends\src\web_api\e3d_tree_api.rs
```
pub struct E3dTreeApiState
pub struct TreeNodeDto
pub struct NodeResponse
pub struct ChildrenResponse
pub struct AncestorsResponse
pub struct SubtreeRefnosResponse
pub struct VisibleInstsResponse
pub struct SearchRequest
pub struct SearchResponse
pub struct NodeAabb
pub struct SiteNodeDto
pub struct SiteNodesResponse
pub struct ChildrenQuery
pub struct SubtreeQuery
pub fn create_e3d_tree_routes(state: E3dTreeApiState) → Router
```

## .worktrees\model-persistence-trait\docs\development\model-writer-storage\05-parquet-writer.md
```
h1 Parquet Writer
h2 Role
h2 Layout
h2 Write behavior
h2 Validation
h2 Phase boundary
code-fence text
code-fence plain
```

## .worktrees\model-persistence-trait\src\fast_model\gen_model\model_writer\parquet.rs
```
pub struct CanonicalParquetWriter
pub struct CanonicalParquetWriterConfig
pub struct CanonicalParquetBatchSummary
pub struct CanonicalParquetTableSummary
impl CanonicalParquetWriter
pub fn new(config: CanonicalParquetWriterConfig) → Self
pub fn write_raw_batch(&self, batch_id: u64, batch: &CanonicalRawBatch,) → anyhow::Result<CanonicalPar...
pub fn summary_file_path(&self, batch_id: u64) → PathBuf
```

## .cursor\agents\trellis-check.md
```
h1 Check Agent
h2 Recursion Guard
h2 Trellis Context Loading Protocol
h2 Context
h2 Core Responsibilities
h2 Important
h2 Workflow
h3 Step 1: Get Changes
h3 Step 2: Check Against Specs
h3 Step 3: Self-Fix
h3 Step 4: Run Verification
h2 Report Format
h2 Self-Check Complete
h3 Files Checked
h3 Issues Found and Fixed
h3 Issues Not Fixed
h3 Verification Results
h3 Summary
code-fence bash
code-fence plain
```
