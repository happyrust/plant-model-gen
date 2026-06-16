# Memory — plant-model-gen-cata-closure

> Generated: 2026-06-16 19:43:59  
> Total memories: **12**  
> Breakdown: decision: 3, event: 4, learning: 3, observation: 2

---

## Instructions

*Standing rules, constraints, and guidelines to always follow.*

*No memories of this type.*

---

## Facts

*Verified information, project status, and established truths.*

*No memories of this type.*

---

## Decisions

*Architectural choices, approach selections, and their rationale.*

### 决策：房间计算暂时不再作为模型生成后的自动必选步骤；默认关闭自动房间计算，仅保留显式/可选方式运行。...

决策：房间计算暂时不再作为模型生成后的自动必选步骤；默认关闭自动房间计算，仅保留显式/可选方式运行。实现上通过 AIOS_AUTO_ROOM_COMPUTE 环境变量 opt-in，默认跳过 run_room_compute_pipeline。

*Confidence: 1 | Status: active | Created: 2026-06-16T10:24:15*

### 项目身份边界：本系统中的项目可以包含多个 E3D 项目。系统项目名/部署项目名决定数据库名称、输出目...

项目身份边界：本系统中的项目可以包含多个 E3D 项目。系统项目名/部署项目名决定数据库名称、输出目录名称、外部访问名称等平台命名空间；E3D 项目名只表示源数据身份，两者不应冲突或互相覆盖。

*Confidence: 1 | Status: active | Created: 2026-06-16T10:09:44*

### 快速站点部署按 MBD 名称定位依赖工程路径不能只靠目录名或 dbfile 猜测；应以解析/读取 S...

快速站点部署按 MBD 名称定位依赖工程路径不能只靠目录名或 dbfile 猜测；应以解析/读取 SYST 与 GLOB/GLB 等系统库后的工程/MDB 关系为准，再快速确定依赖项目路径、目标 DB 和工程组成。

*Confidence: 1 | Status: active | Created: 2026-06-16T11:29:59*

---

## Goals

*Objectives, targets, and milestones to track progress.*

*No memories of this type.*

---

## Commitments

*Promises, obligations, and TODOs that need follow-through.*

*No memories of this type.*

---

## Preferences

*User and entity preferences for personalization.*

*No memories of this type.*

---

## Relationships

*Entity connections, team context, and collaboration patterns.*

*No memories of this type.*

---

## Context

*Session summaries, status updates, and conversation state.*

*No memories of this type.*

---

## Events

*Important conversations, milestones, and temporal occurrences.*

### MBD 候选发现已从只解析 SYST 扩展为解析 SYST/GLOB/GLB 系统库：src/dat...

MBD 候选发现已从只解析 SYST 扩展为解析 SYST/GLOB/GLB 系统库：src/data_interface/mdb_candidates.rs 新增 MDB_SOURCE_DB_TYPES、按 SYST->GLOB->GLB 优先级解析系统库并保留 source_file/source_db_type 证据字段，同时保留 syst_file 兼容旧响应；src/parse_sidecar.rs、src/web_server/models.rs、src/web_server/admin_handlers.rs 注释同步为 SYST/GLOB/GLB。已通过 cargo check --bin web_server --no-default-features --features ws,gen_model,manifold,project_hd,surreal-save,write-to-surrealdb,sqlite-index,web_server,parquet-export,rvm-import。

*Confidence: 1 | Status: active | Created: 2026-06-16T11:35:11*

### Configured MEMANTO for project plant-model-gen-cat...

Configured MEMANTO for project plant-model-gen-cata-closure with cloud backend and active project agent. API key is stored outside the repository via MOORCHEH_API_KEY user environment variable.

*Confidence: 1 | Status: active | Created: 2026-06-15T16:37:33 | Tags: `memanto`, `configuration`, `plant-model-gen-cata-closure`*

### 完成修复：自动房间计算已改为默认关闭，仅通过 AIOS_AUTO_ROOM_COMPUTE=1/tr...

完成修复：自动房间计算已改为默认关闭，仅通过 AIOS_AUTO_ROOM_COMPUTE=1/true/yes/on 显式启用；已重新编译并替换 release/bin/web_server.exe，验证 quicktest-250160-8080 生成成功后 room-compute.log 记录跳过，站点最终 Running/Parsed 且 last_error 为空。

*Confidence: 1 | Status: active | Created: 2026-06-16T10:31:30*

### 实现快速站点部署与站点配置的 MDB/工程名路径辅助输入：ui/admin/src/views/Si...

实现快速站点部署与站点配置的 MDB/工程名路径辅助输入：ui/admin/src/views/SitesView.vue 新增 dbfile/MBD 两种快速创建模式并传 mbd_name/search_roots；ui/admin/src/components/sites/SiteDrawer.vue 新增按工程/MDB 名称补全 project_path/associated_project/scanRoot 的辅助区；ui/admin/src/types/site.ts 对齐后端 QuickDeployTestRequest 的 mbd_name/search_roots/projects 字段。已通过 npx vue-tsc -b；sigmap validate 成功但覆盖率仅 1%，属于现有 SigMap 配置覆盖不足。

*Confidence: 1 | Status: active | Created: 2026-06-16T11:14:35*

---

## Learnings

*Knowledge acquired from experience, corrections, and insights.*

### 修复站点重命名后 SurrealDB database 名称不同步的问题：managed_proje...

修复站点重命名后 SurrealDB database 名称不同步的问题：managed_project_sites 使用 site.project_name 作为运行库 database 名称，并将全量表单保存改为仅在解析相关值实际变化时重置 Stopped/Parsed 状态；已在 quicktest-250160-8080 验证 database=AvevaPlantSample_RenameCheck、viewer、manifest、E3D world-root/children 正常。

*Confidence: 1 | Status: active | Created: 2026-06-16T09:17:02 | Tags: `managed-sites`, `surrealdb`, `project-rename`*

### 修复 quicktest-250160-8080 CATA 部分解析计划对齐：src/web_ser...

修复 quicktest-250160-8080 CATA 部分解析计划对齐：src/web_server/managed_project_sites.rs 现在允许 cata_closure manifest 覆盖的 DESI 模板依赖进入 parse plan；release/bin/web_server.exe 已替换为 SHA256 D5066308908C650C7EB0A26FFEF622B99AFF4F75DB02FB9F58DB5948B546689C；运行验证显示 parse-plan-manifest.json 已包含 aps7015_0001、aps250124_0001、aps250162_0001，warnings 清空；后续生成仍显示 ptsets.parquet=0、missing_cata_hash_refnos=862，说明剩余问题在 ptset/cata_hash 数据链路而非 parse plan 缺库。

*Confidence: 1 | Status: active | Created: 2026-06-16T11:00:50*

### Spec Kit 016 已扩展覆盖站点重命名后的模型生成 precheck 回归：自定义 proj...

Spec Kit 016 已扩展覆盖站点重命名后的模型生成 precheck 回归：自定义 project_name 是输出/SurrealDB 命名空间，indextree/db_meta 自动补齐必须使用 included_projects 中真实源 E3D 工程；release 包中 release/bin/aios-database.exe 仍需替换为重新编译后的修复版再复测 quicktest-250160-8080。

*Confidence: 1 | Status: active | Created: 2026-06-16T09:51:33*

---

## Observations

*Patterns noticed, behavioral notes, and recurring themes.*

### 进一步确认 quicktest-250160-8080 的 CATA 部分解析问题：parse.lo...

进一步确认 quicktest-250160-8080 的 CATA 部分解析问题：parse.log 显示 gen-cata-closure 成功生成 4 个闭包库（7015、250124、250162、250193），但 managed_project_sites 对齐解析计划时从 CATA files 14 -> 1；正式 parse 阶段日志只出现 db_type is CATA 后读取 aps250193_0001。根因是 closure 精确模式允许 CATA + 外部模板 DESI 进入 manifest，但 align_parse_plan_cata_with_manifest 只接纳 db_index 中 db_type=CATA 的覆盖库，导致 db_type 被记录为 DESI 的 manifest 覆盖库没有进入正式解析计划。

*Confidence: 0.97 | Status: active | Created: 2026-06-16T10:43:13*

### 发现 quicktest-250160-8080 CATA 部分解析不完整的直接原因：cata_cl...

发现 quicktest-250160-8080 CATA 部分解析不完整的直接原因：cata_closure.json 覆盖 dbnums 7015/250124/250162/250193，但站点 db_index.sqlite 中 7015、250124、250162 的 db_type 被记录为 DESI，只有 250193 为 CATA；managed_project_sites 的 align_parse_plan_cata_with_manifest 只把 db_type=CATA 的覆盖库加入解析计划，因此 parse-plan-manifest 警告这些 dbnum 不是 CATA，最终 CATA 部分解析只纳入 aps250193_0001。生成日志显示 Parquet tubings=16、missing_mesh=0，但 ptsets=0 且 missing_cata_hash_refnos=862，BRAN/管道显示缺失更可能来自 CATA/ptset 语义数据不完整而非 mesh 文件缺失。

*Confidence: 0.95 | Status: active | Created: 2026-06-16T10:39:49*

---

## Artifacts

*Tool outputs, files, reports, and external references.*

*No memories of this type.*

---

## Errors

*Failure records, bugs, and lessons learned from mistakes.*

*No memories of this type.*

---

*End of memory export.*
