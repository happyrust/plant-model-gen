# SigMap Query Context
Generated: 2026-06-16T17:15:09.064Z

## .worktrees\pe-transform-backends\src\web_api\mbd_pipe_api.rs
```
pub struct MbdPipeQuery
pub struct MbdPipeResponse
pub struct MbdPipeData
pub struct MbdPipeStats
pub struct BranchAttrsDto
pub struct MbdPipeSegmentDto
pub struct MbdDimDto
pub struct MbdWeldDto
pub struct MbdSlopeDto
pub struct MbdLayoutHint
pub struct MbdCutTubiDto
pub struct MbdFittingDto
pub struct MbdTagDto
pub struct MbdBendDto
pub struct MbdPipeDebugInfo
pub struct MbdExportStats
pub struct MbdExportFailure
pub struct MbdManifest
pub struct MbdManifestEntry
pub enum MbdPipeSource
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

## .worktrees\pe-transform-backends\src\web_server\sqlite_spatial_api.rs
```
pub struct SqliteSpatialQueryParams
pub struct SpatialQueryResult
pub struct SpatialQueryResultItem
pub struct AabbDto
pub struct Vec3Dto
pub struct SpatialStatsResult
pub async fn api_sqlite_spatial_query(Query(params) → Json<SpatialQueryResult>
pub async fn api_sqlite_spatial_stats() → Json<SpatialStatsResult>
```

## .trellis\scripts\add_session.py
```
def get_latest_journal_info(dev_dir: Path) → tuple[Path | None, int, int]  # Get latest journal file info
def get_current_session(index_file: Path) → int  # Get current session number from index
def count_journal_files(dev_dir: Path, active_num: int) → str  # Count journal files and return table rows
def create_new_journal_file(dev_dir: Path, num: int, developer: str, today: str, max_lines: int) → Path  # Create a new journal file
def generate_session_content(session_num: int, title: str, commit: str, summary: str, extra_content: str, today: str, package: str | None, branch: str | None) → str  # Generate session content
def update_index(index_file: Path, dev_dir: Path, title: str, commit: str, new_session: int, active_file: str, today: str, branch: str | None) → bool  # Update index
def main() → int  # CLI entry point
```

## .trellis\spec\backend\error-handling.md
```
h1 Error Handling
h2 Overview
h2 Error Types
h2 Error Handling Patterns
h2 API Error Responses
h2 Common Mistakes
```
