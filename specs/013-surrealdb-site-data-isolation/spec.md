# Feature Specification: SurrealDB 站点数据目录隔离

**Feature Branch**: `013-surrealdb-site-data-isolation`

**Created**: 2026-06-15

**Status**: Draft

**Input**: User description: "写 SurrealDB 项目目录隔离 的 spec kit，在站点部署时，数据库文件夹时和站点名称对应的"

## Current State and Grill-Me Decisions

### Existing Behavior Findings

- 受管站点运行目录以 `runtime/admin_sites/<site_id>` 为根。
- 站点输出文件已经按项目名落到 `runtime/admin_sites/<site_id>/output/<project_name>`。
- SurrealDB RocksDB 数据目录当前由站点记录中的 `db_data_path` 决定；旧站点常见路径是 `runtime/admin_sites/<site_id>/data/surreal.db`。
- 在重复使用相同站点名和端口的快测场景中，旧 RocksDB 内容可能被复用，导致不同项目或不同测试案例的数据状态混在同一站点根目录下。
- 现场误判发生在 `avevamarinesample-8080` 与 `avevaplantsample-8080` 两个测试案例之间；用户要求数据库文件夹必须与站点名称对应，便于肉眼识别和隔离。

### Grill-Me Decision Tree

| Question | Recommended Answer |
|----------|--------------------|
| 这个 spec 是否同时覆盖 250160 生成事务冲突？ | 否。本 spec 只覆盖 SurrealDB 数据目录与站点名称对应的隔离规则；生成事务冲突另起修复项。 |
| 数据目录应按项目名还是站点名命名？ | 按站点名称或其规范化 slug 命名，因为用户在部署 UI、日志和运行目录中以站点为管理单位。 |
| 是否迁移历史站点数据？ | 不在本 spec 内自动迁移。历史站点继续使用原路径，重新部署或重建站点后按新规则生成目录。 |
| 输出目录是否也要改变？ | 不改变。输出仍按现有 `output/<project_name>` 组织，本 spec 只约束 SurrealDB 数据目录。 |
| file 与 ws 模式是否使用同一目录规则？ | 是。离线嵌入式 file 模式和 ws 运行时都必须引用同一个站点专属 `db_data_path`。 |
| quick deploy/API 是否默认自动解析依赖 DB？ | 否。默认只解析请求指定的目标 DB，用于快速测试；只有显式传 `auto_parse_related_dbnums=true` 才自动纳入依赖 DB。 |

## User Scenarios & Testing *(mandatory)*

### User Story 1 - 新建站点时数据库目录与站点名称对应 (Priority: P1)

管理员创建或快速部署一个站点后，能在运行目录中直接看到与站点名称对应的 SurrealDB 数据文件夹，避免把不同项目案例的数据混在一起。

**Why this priority**: 这是用户明确提出的核心需求，直接影响部署排查和重复快测的可信度。

**Independent Test**: 创建站点 `AvevaPlantSample`，端口为 `8080`，验证 metadata 和配置中的数据库路径都位于 `runtime/admin_sites/avevaplantsample-8080/projects/avevaplantsample/data/surreal.db` 或等价的站点名称规范化目录下。

**Acceptance Scenarios**:

1. **Given** 用户创建站点名称为 `AvevaPlantSample` 且 web 端口为 `8080`，**When** 站点配置文件生成，**Then** SurrealDB 数据目录包含规范化站点名 `avevaplantsample`，并且不再默认写入站点根目录的通用 `data/surreal.db`。
2. **Given** 用户创建另一个站点 `AvevaMarineSample`，**When** 该站点部署，**Then** 它的数据库目录与 `AvevaPlantSample` 的数据库目录不同，即使二者使用相同的项目结构和相同的部署流程。
3. **Given** 用户查看 `metadata.json`，**When** 部署完成或配置生成完成，**Then** metadata 中能看到实际 `db_data_path`，用于确认数据库文件夹归属。

---

### User Story 2 - 快速部署重复运行不会误复用其他站点数据库 (Priority: P1)

开发者通过 quick deploy 重复运行不同项目案例时，数据库目录应由当前站点名称决定，避免上一次其他站点的 RocksDB 数据影响本次解析/生成。

**Why this priority**: 快测是当前定位问题的主要入口，目录混用会导致错误归因和难以复现。

**Independent Test**: 连续运行 `AvevaPlantSample` 与 `AvevaMarineSample` 的快速部署，确认两个站点的 `DbOption-parse.toml`、`DbOption-generate.toml`、`DbOption.toml` 中的 SurrealDB path 分别指向各自站点名称目录。

**Acceptance Scenarios**:

1. **Given** 已存在 `avevamarinesample-8080` 的数据库目录，**When** 用户部署 `avevaplantsample-8080`，**Then** 新站点不会引用 marine 站点的数据目录。
2. **Given** quick deploy 使用 `force_recreate=true`，**When** 重新创建同名站点，**Then** 新配置继续使用与当前站点名称对应的数据库目录。
3. **Given** quick deploy 使用 file pipeline 模式，**When** parse/generate sidecar 启动，**Then** sidecar 使用站点专属 `db_data_path`，而不是进程当前目录下的共享 RocksDB 路径。
4. **Given** quick deploy/API 请求没有显式设置 `auto_parse_related_dbnums`，**When** 用户指定单个目标 DB 进行快速测试，**Then** 系统默认不自动解析依赖 DB 文件。
5. **Given** quick deploy/API 请求显式设置 `auto_parse_related_dbnums=true`，**When** 用户需要完整依赖解析，**Then** 系统按现有依赖发现规则纳入相关 DB 文件。

---

### User Story 3 - 运行时互斥与可观测性保持一致 (Priority: P2)

管理员启动、停止、重新解析或重新生成站点时，file/ws 两种模式都围绕同一个站点专属数据库目录做互斥判断和日志提示。

**Why this priority**: SurrealDB RocksDB 目录有排他锁；路径规则变更不能破坏现有 file/ws 互斥保护。

**Independent Test**: 启动一个 ws 模式站点后再触发 file 模式解析，确认系统先释放同一站点专属数据库目录的 ws 持有者，再执行离线解析。

**Acceptance Scenarios**:

1. **Given** 某站点 ws 运行时正在持有其数据库目录，**When** 用户触发需要 file 模式访问同一目录的解析或生成，**Then** 系统按现有互斥策略停止或复用正确的站点持有者。
2. **Given** 两个不同站点同时存在，**When** 一个站点执行解析，**Then** 不会停止另一个站点的 SurrealDB 进程，除非它们配置了相同的数据库目录。
3. **Given** 站点启动失败，**When** 管理员查看日志和 metadata，**Then** 能看到与站点名称对应的数据库路径，便于判断是路径冲突、锁冲突还是数据污染。

### Edge Cases

- 站点名称包含空格、大小写、下划线、中文或特殊符号时，目录名必须通过稳定规则规范化，并保持可读。
- 站点名称为空或规范化后为空时，必须拒绝创建或使用明确的默认站点 slug，不能生成空目录。
- 同名站点使用不同端口时，站点根目录仍由 `site_id` 区分；数据库子目录也必须保持站点名称可识别。
- 历史站点已有 `runtime/admin_sites/<site_id>/data/surreal.db` 时，不应在本功能中静默移动数据。
- 配置文件、metadata、SurrealDB 启动命令和 sidecar 启动参数必须指向同一个数据库目录。
- `project_name` 与 `site_name` 不一致时，数据库目录使用站点名称，输出目录仍可使用项目名。

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: 系统 MUST 为新建受管站点生成与站点名称对应的 SurrealDB 数据目录。
- **FR-002**: 系统 MUST 使用稳定、可重复的站点名称规范化规则生成数据库目录名。
- **FR-003**: 系统 MUST 在 `DbOption.toml`、`DbOption-parse.toml`、`DbOption-generate.toml` 中写入同一个站点专属 SurrealDB 数据路径。
- **FR-004**: 系统 MUST 在 `metadata.json` 中输出实际 `db_data_path`。
- **FR-005**: 系统 MUST 确保 quick deploy 创建的新站点使用站点专属数据库目录。
- **FR-006**: 系统 MUST 确保 file pipeline 模式和 ws runtime 模式共享同一个站点专属 `db_data_path`。
- **FR-007**: 系统 MUST 继续基于 `db_data_path` 进行 RocksDB 目录持有者登记、互斥检查、停止和复用判断。
- **FR-008**: 系统 MUST 保持现有输出目录语义，不因数据库目录隔离改变 `output/<project_name>` 的组织方式。
- **FR-009**: 系统 MUST 不自动迁移历史站点的旧数据库目录，除非用户显式执行迁移流程。
- **FR-010**: 系统 MUST 在站点名称规范化结果冲突或不可用时给出清晰错误或使用已定义的唯一化策略。
- **FR-011**: 系统 MUST 保证两个不同站点默认不会生成相同的数据库目录。
- **FR-012**: 系统 MUST 提供可通过 HTTP quick deploy 或站点创建流程验证的回归用例。
- **FR-013**: quick deploy/API 请求 MUST 默认不自动解析依赖 DB 文件，除非请求显式声明启用自动依赖解析。
- **FR-014**: 当请求显式启用自动依赖解析时，系统 MUST 保留现有依赖解析能力，并在 manifest/metadata 中反映实际纳入的 DB 文件。

### Key Entities *(include if feature involves data)*

- **Managed Site**: 管理端创建和部署的站点。关键属性包括 `site_id`、`site_name`、`project_name`、端口、运行目录和状态。
- **Database Data Path**: 站点专属 SurrealDB RocksDB 路径。关键属性包括规范化站点名称、父运行目录、文件模式和 ws 模式共享性。
- **Runtime Metadata**: 写入站点运行目录的可观测信息。关键属性包括 `site_id`、`project_name`、`runtime_db_mode`、`pipeline_db_mode` 和 `db_data_path`。
- **Sidecar Job**: parse/generate 运行任务。关键属性包括站点归属、配置路径、数据库模式和使用的 `db_data_path`。
- **Data Directory Owner**: 当前持有某个 RocksDB 目录的进程或模式登记。关键属性包括路径、站点、PID、访问模式和用途。

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: 新建 `AvevaPlantSample` 站点后，100% 的生成配置文件和 metadata 都指向包含 `avevaplantsample` 规范化名称的数据库目录。
- **SC-002**: 连续部署两个不同站点时，两个站点的 `db_data_path` 必须不同，并且各自路径可从站点名称直接识别。
- **SC-003**: 使用 quick deploy 创建站点后，管理员无需读取代码即可通过 `metadata.json` 判断 SurrealDB 数据属于哪个站点。
- **SC-004**: file 模式解析/生成与 ws 模式运行时继续围绕同一 `db_data_path` 做互斥；不会因路径规则变化导致同站点双开 RocksDB。
- **SC-005**: 历史站点未显式迁移时仍可按原配置读取旧目录，新建站点按新目录规则生成。
- **SC-006**: 未传 `auto_parse_related_dbnums` 的单 DB 快速测试请求只解析目标 DB 与强制预解析项，不额外纳入自动依赖 DB。
- **SC-007**: 显式传 `auto_parse_related_dbnums=true` 的请求仍能纳入依赖 DB，且行为与现有依赖解析期望一致。

## Assumptions

- 站点名称是用户在部署和运维时识别案例的主要语义名称。
- `site_id` 仍用于隔离站点根目录，数据库子目录使用站点名称提高可读性。
- 历史站点迁移有数据丢失风险，因此本 spec 不做自动迁移。
- quick deploy 是本功能的关键验证入口。
- 站点名称规范化应沿用仓库已有 slug 规则，避免引入第二套命名语义。
- 快速测试优先追求启动和验证速度，因此默认不做依赖 DB 自动扩展。

## Out of Scope

- 修复 `inst_info in 字段冲突` 或其他模型生成事务错误。
- 自动清理、移动或合并已有 SurrealDB RocksDB 数据目录。
- 改变输出目录、Parquet 目录或 scene tree 目录的项目名组织方式。
- 改变 SurrealDB 认证、端口分配、远端部署路径策略。
- 为历史站点提供批量迁移 UI。
