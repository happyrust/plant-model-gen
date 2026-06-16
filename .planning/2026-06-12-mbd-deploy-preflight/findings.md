# Findings & Decisions: MBD 部署前选择与依赖补齐

## Requirements

- 用户希望站点部署时支持指定 MBD。
- UI 需要提供所有 MBD 候选项的下拉框。
- 选择 MBD 后，系统应根据 SYST/MBD 中得到的依赖 DB file path 做完整性检查。
- 如果缺少 DB file，UI 应提供添加项目文件或项目目录的选项。
- 添加后刷新检查，直到所有必需 DB file 都可获取。
- 只有依赖完整后才允许开始部署。

## Research Findings

- 当前 Admin UI 的主要入口是 `ui/admin/src/components/sites/SiteDrawer.vue`。
- `SiteDrawer.vue` 已有“工程组成”能力：用户可扫描根路径、导入多个 `projects[]`、标注 `design/library` 角色，并指定唯一主工程。
- `SiteDrawer.vue` 已有“本次解析预览”，会调用 `sitesApi.previewParsePlan()`，请求体来自 `PreviewManagedSiteParsePlanRequest`。
- 当前预览请求支持 `manual_db_nums`、`manual_db_files`、`parse_db_types`、`auto_parse_related_dbnums`、`cata_partial_parse`、`db_index_path`。
- `src/web_server/admin_handlers.rs` 暴露 `/api/admin/projects/scan`，但实际扫描由 `parse_sidecar_client::scan_projects()` 转发给 aios-database sidecar。
- `src/parse_sidecar.rs::scan_projects_under_root()` 会读取 DB 文件头，返回候选工程、dbnums、db_types 和冲突信息。
- `web_server` 当前原则是不直接读取 E3D DB 文件；`scan_projects_under_root()` 在 web_server 侧已明确返回错误，提示调用 sidecar。
- `src/web_server/managed_project_sites.rs::build_site_config()` 当前写 `DbOption.toml` 时，`mdb_name` 来自 `DatabaseConfig::from_db_option(&aios_core::get_db_option())` 的模板默认值，不来自站点配置。
- `ManagedProjectSite` 当前没有 `mdb_name` 或 `mdb_module` 字段，无法持久化用户在部署 UI 中的 MBD 选择。
- `parse_plan_inputs_hash()` 当前包含 project、manual_db_nums、parse_db_types、force_rebuild_system_db、auto_parse_related_dbnums、cata_partial_parse，但没有 MBD 相关输入。
- 现有 MDB/MBD 查询语义可参考：
  - `src/mdb.rs::get_project_mdb()`
  - `src/api/project_mdb.rs::query_db_nums_of_mdb()`
  - `src/api/project_mdb.rs::query_db_quick_info()`
  - `src/api/children.rs::query_mdb_all_dbnums()`
- `src/api/project_mdb.rs` 中的查询基于 `PROJECT_MDB` 表，支持按 `MDB_NAME` 和 `db_type` 获取 `DB_NUM`、`WORLD_REFNO`、排序等信息。

## Technical Decisions

| Decision | Rationale |
|----------|-----------|
| MBD 候选/依赖检查由 sidecar 实现 | sidecar 已负责工程扫描、文件头读取、db file resolve，符合当前架构边界。 |
| UI 增加 MBD 选择区块，而不是复用 `manual_db_nums` 输入框 | MBD 是更高层部署范围事实源，直接塞进手动 DB nums 会让用户看不到缺失文件和依赖状态。 |
| `mdb_name` 和 `mdb_module` 应持久化到站点模型 | 选择结果需要写入 `DbOption.toml`，也需要在编辑站点时回显。 |
| 缺失必需 DB file 时阻断部署 | 这是用户明确要求的部署前完整性保障；不能等 parse/generate 失败后再暴露。 |
| 首版建议 MBD 选择和手动 DB Nums 二选一 | 两个范围事实源同时启用会带来合并语义、优先级和 UI 可解释性问题。 |
| `parse_plan_inputs_hash()` 必须包含 MBD 输入 | 切换 MBD 会改变解析范围和 db_index 依赖闭包，不能复用旧 preview manifest。 |

## Proposed API Shape

### MBD candidates

`POST /api/admin/projects/mdb-candidates`

Request:

```json
{
  "project_name": "AvevaPlantSample",
  "project_path": "D:/AVEVA/Projects/E3D2.1/AvevaPlantSample",
  "projects": [
    { "name": "AvevaPlantSample", "path": "...", "role": "design", "is_primary": true, "sort_order": 0 }
  ],
  "module": "DESI"
}
```

Response:

```json
{
  "candidates": [
    {
      "mdb_name": "/SAMPLE",
      "module": "DESI",
      "dbnums": [250160],
      "db_files": [
        {
          "dbnum": 250160,
          "db_type": "DESI",
          "file_name": "aps250160_0001",
          "file_path": "D:/AVEVA/Projects/E3D2.1/AvevaPlantSample/aps000/aps250160_0001",
          "status": "available",
          "source": "project_scan"
        }
      ],
      "missing_count": 0,
      "ambiguous_count": 0,
      "ready_to_deploy": true
    }
  ],
  "warnings": []
}
```

### Preview extension

Extend `PreviewManagedSiteParsePlanRequest`:

```ts
interface PreviewManagedSiteParsePlanRequest {
  mdb_name?: string
  mdb_module?: string
}
```

Extend preview response with:

```ts
interface ManagedSiteMbdDependencyCheck {
  mdb_name: string
  module: string
  required_db_files: MbdDbFileStatus[]
  missing_db_files: MbdDbFileStatus[]
  ambiguous_db_files: MbdDbFileStatus[]
  ready_to_deploy: boolean
}
```

## UI Findings

- `SiteDrawer.vue` 当前在“工程组成”之后进入“运行配置”和“解析范围”；MBD 选择更适合放在“工程组成”和“解析范围”之间。
- “本次解析预览”现在位于“解析范围”顶部；选择 MBD 后可将预览标题改成“基于 MBD 的解析预览”。
- 当前 `canSubmit` 只检查项目名、项目路径、重复项目名、多工程错误、凭据和端口；需要加入 `mbdDependencyReady`。
- 当前缺失 DB file 可复用已有输入能力：
  - 添加工程目录：更新 `projects[]` 后重新扫描。
  - 手动指定 DB Files：写入 `manual_db_files`，后端已能通过 sidecar resolve 为 dbnum。
- UI 状态建议：
  - 未选择 MBD：显示“可按解析类型或手动 DB Nums 部署；选择 MBD 后将启用依赖完整性检查”。
  - 检查中：显示“正在读取 MBD 依赖”。
  - ready：绿色卡片“依赖完整，可以部署”。
  - missing：红色卡片“缺少必需 DB 文件，无法部署”，列出 dbnum/db_type/来源 MBD。
  - ambiguous：黄色卡片“存在多个候选文件，请选择项目目录或手动指定文件”。

## Issues Encountered

| Issue | Resolution |
|-------|------------|
| 根目录已有旧 planning 文件，且内容属于 sidecar job 竞态修复 | 创建独立 `.planning/2026-06-12-mbd-deploy-preflight/`，不覆盖旧计划。 |
| MBD/MDB 命名在用户输入与代码中混用 | 计划文件使用用户说法 MBD，同时在代码字段建议用现有 `mdb_name` 命名以匹配 `DbOption.toml` 和现有 API。 |

## Resources

- `src/parse_sidecar.rs`
- `src/web_server/parse_sidecar_client.rs`
- `src/web_server/admin_handlers.rs`
- `src/web_server/managed_project_sites.rs`
- `src/web_server/models.rs`
- `src/mdb.rs`
- `src/api/project_mdb.rs`
- `src/api/children.rs`
- `ui/admin/src/components/sites/SiteDrawer.vue`
- `ui/admin/src/types/site.ts`
- `ui/admin/src/api/sites.ts`

## Visual/Browser Findings

- 未进行浏览器视觉验证；本轮为代码级规划。
