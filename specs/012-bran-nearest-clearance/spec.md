# Feature Specification: BRAN 弯头到最近墙/柱自动测距空间查询

**Feature Branch**: `012-bran-nearest-clearance`

**Created**: 2026-06-14

**Status**: Draft

**Input**: User description: "分析现在的空间查询接口是否考虑了专业的筛选了，比如我想实现针对一个 BRAN 的弯头，自动测量离他最近的墙、柱子的功能。帮我使用 grill-me 分析，然后编写 spec-kit"

## Current State and Grill-Me Decisions

### Existing API Findings

- `GET /api/sqlite-spatial/query` already supports `mode=bbox|refno|position`, `distance`/`radius`, `nouns`, `spec_values`, pagination, `include_self`, and `shape=cube|sphere`.
- Query results already include `refno`, `noun`, `spec_value`, `aabb`, and AABB-to-AABB minimum `distance`, sorted nearest-first.
- The current "专业过滤" is only numeric `spec_values` from the SQLite `items.spec_value` column. It does not provide a named discipline model, type aliases, target groups, or wall/column semantic roles.
- The legacy spatial endpoint in `src/web_server/handlers.rs` is less capable and should not be the basis for this feature.
- The current distance is bounding-box clearance, not geometry-to-surface clearance. This is acceptable for MVP only if the response declares `distance_method=aabb_clearance_mm`.

### Grill-Me Decision Tree

| Question | Recommended Answer |
|----------|--------------------|
| Is `spec_values` enough for professional filtering? | No. Keep it as a low-level filter, but add named filters so users can ask for wall/column without knowing numeric spec codes. |
| Should the first implementation be wall/column specific or generic nearest target query? | Generic grouped nearest target query, with a BRAN elbow preset as the first supported use case. |
| What is the target object vocabulary? | Start with `target_groups=wall,column`, mapped to configured noun sets and optional `spec_values`. Return the resolved filters in the response. |
| How should "BRAN elbow" be identified? | Accept an explicit source `refno`; validate that source noun is `BRAN`, then classify it as elbow using available attributes/catalogue metadata when present. If elbow classification is unavailable, allow `source_kind=bran_component` with a warning. |
| What distance should be reported? | MVP reports AABB clearance in millimetres. Later versions may add mesh/convex exact surface distance. |
| What if walls/columns are in another dbnum? | Default to same dbnum as source, with `scope=same_dbnum|all_loaded|explicit_dbnums`. |
| What if multiple walls/columns tie? | Stable order by distance, preferred dbnum, target group priority, then refno. |
| What should the endpoint return for no hits? | `success=true`, empty `nearest_by_group`, `warnings=["no_target_found"]`, and the query bbox used. |

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Find nearest wall and column for a BRAN elbow (Priority: P1)

An engineer selects a BRAN elbow instance and asks the system to return the nearest wall and nearest column within a configurable search radius.

**Why this priority**: This is the user-requested workflow and validates whether current spatial filtering can support discipline-aware clearance checks.

**Independent Test**: Build a small SQLite spatial index with one BRAN source, two wall candidates, two column candidates, and unrelated PIPE/EQUI candidates. Query the source with target groups `wall,column` and assert only the nearest wall and nearest column are returned.

**Acceptance Scenarios**:

1. **Given** a BRAN source refno and indexed wall/column AABBs within 5000mm, **When** the user queries nearest targets for `wall,column`, **Then** the response contains one nearest wall and one nearest column with distance in millimetres and source/target AABBs.
2. **Given** nearer PIPE geometry and farther WALL geometry, **When** the user requests `target_groups=wall`, **Then** PIPE is excluded and the nearest WALL is returned.
3. **Given** no indexed columns within the radius, **When** the user requests `target_groups=wall,column`, **Then** wall may be populated, column is empty, and the response includes a non-fatal warning for the missing column group.

---

### User Story 2 - Explain and audit professional filters (Priority: P2)

An engineer can inspect which nouns and `spec_values` were actually used for wall and column filtering, so results are auditable and can be corrected per project.

**Why this priority**: Current `spec_values` are numeric and opaque. Professional filtering must be explainable before it can be trusted.

**Independent Test**: Query the filter metadata endpoint or response `resolved_filters` and confirm that `wall` and `column` expand to configured noun and spec filters.

**Acceptance Scenarios**:

1. **Given** a project filter config that maps `wall` to nouns `WALL,PANE` and `column` to `COLU,SCTN`, **When** the nearest query runs, **Then** the response echoes these resolved filters.
2. **Given** a candidate whose noun matches but whose `spec_value` is excluded, **When** strict professional filtering is enabled, **Then** that candidate is excluded and the diagnostics identify the filter reason in debug mode.

---

### User Story 3 - Preserve current generic spatial query behavior (Priority: P3)

Existing users of `/api/sqlite-spatial/query` continue to use bbox/refno/position spatial queries with `nouns` and `spec_values` filters.

**Why this priority**: The feature should extend spatial querying without breaking existing UI and diagnostics.

**Independent Test**: Run existing `sqlite_spatial_api` unit tests and add regression coverage for current `nouns`, `spec_values`, `shape=sphere`, and pagination behavior.

**Acceptance Scenarios**:

1. **Given** an existing bbox query with `nouns=PIPE`, **When** the feature is deployed, **Then** the response schema and sorting behavior remain compatible.
2. **Given** an existing refno query with `include_self=false`, **When** the feature is deployed, **Then** the source refno is still excluded.

### Edge Cases

- Source refno does not exist in `aabb_index`, return `success=false` with a clear source-not-found error.
- Source exists in `aabb_index` but not `items`, return unknown source metadata and allow query only if caller opts into `allow_unknown_source=true`.
- Source noun is not `BRAN`, return `success=false` unless caller explicitly uses a generic nearest query mode.
- Search radius is zero, negative, missing, NaN, or too large, reject invalid values and clamp or reject values above configured maximum.
- Candidate intersects the BRAN AABB, report distance `0.0` and mark `intersects=true`.
- A wall/column candidate lacks `spec_value`, include it only when the group mapping allows noun-only fallback.
- Multiple target groups resolve to the same candidate, return it under each matching group but keep one canonical candidate record by refno.
- Cross-dbnum query is requested while only one dbnum is indexed, return a warning showing effective indexed scope.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST provide a nearest-target spatial query that accepts a source refno, search radius, target groups, and scope.
- **FR-002**: System MUST support `wall` and `column` target groups for MVP.
- **FR-003**: System MUST resolve each target group into auditable noun and optional `spec_value` filters.
- **FR-004**: System MUST preserve existing `/api/sqlite-spatial/query` parameters and behavior for bbox/refno/position queries.
- **FR-005**: System MUST return nearest candidates grouped by target group, sorted by distance ascending with deterministic tie-breaks.
- **FR-006**: System MUST include source metadata, source AABB, target AABB, `distance_mm`, `distance_method`, and query bbox in the response.
- **FR-007**: System MUST identify and exclude the source object by default.
- **FR-008**: System MUST default nearest BRAN measurement scope to the source dbnum and allow explicit wider scopes.
- **FR-009**: System MUST report whether a BRAN source was positively classified as an elbow, classified as generic BRAN, or not BRAN.
- **FR-010**: System MUST expose warnings for partial results, missing target groups, unknown source metadata, and fallback filtering.
- **FR-011**: System MUST allow project-specific filter mappings for wall/column without requiring code changes.
- **FR-012**: System MUST include unit tests for noun filtering, `spec_value` filtering, target group resolution, nearest-per-group selection, same-dbnum scoping, and no-hit behavior.
- **FR-013**: System MUST document in API response fields that MVP distance is AABB clearance, not exact mesh surface distance.

### Key Entities *(include if feature involves data)*

- **Source Component**: The selected model instance, usually a BRAN elbow. Key attributes include refno, dbnum, noun, source classification, AABB, and optional catalogue/spec metadata.
- **Target Group**: A semantic group such as wall or column. Key attributes include group name, allowed nouns, optional allowed `spec_values`, priority, and fallback behavior.
- **Nearest Target Candidate**: A candidate indexed object matched by spatial radius and target group filters. Key attributes include refno, noun, spec value, AABB, distance, intersection flag, and filter match reason.
- **Measurement Result**: The grouped output for one source query. Key attributes include source, query bbox, nearest_by_group, all_candidates when requested, warnings, and distance method.
- **Filter Configuration**: Project-level mapping from professional/semantic names to low-level nouns and `spec_values`.

## API Contract Draft

### Request

`GET /api/sqlite-spatial/nearest-clearance`

| Parameter | Required | Description |
|-----------|----------|-------------|
| `source_refno` | yes | Refno string such as `250160_123456`. |
| `target_groups` | yes | Comma-separated group names. MVP requires `wall,column`. |
| `radius` | no | Search radius in millimetres. Default 5000, maximum configurable. |
| `scope` | no | `same_dbnum` default, `all_loaded`, or `explicit_dbnums`. |
| `dbnums` | no | Comma-separated dbnums when `scope=explicit_dbnums`. |
| `max_per_group` | no | Default 1. |
| `distance_method` | no | MVP supports `aabb_clearance_mm`. |
| `strict_source_kind` | no | Default true for BRAN elbow flow. |
| `debug` | no | Include filter diagnostics and rejected candidate counts. |

### Response Shape

```json
{
  "success": true,
  "source": {
    "refno": "250160_123456",
    "noun": "BRAN",
    "classification": "bran_elbow|bran_component|not_bran|unknown",
    "aabb": {}
  },
  "distance_method": "aabb_clearance_mm",
  "unit": "mm",
  "query_bbox": {},
  "resolved_filters": {
    "wall": {"nouns": ["WALL", "PANE"], "spec_values": []},
    "column": {"nouns": ["COLU", "SCTN"], "spec_values": []}
  },
  "nearest_by_group": {
    "wall": [{"refno": "250160_200001", "noun": "WALL", "spec_value": 0, "distance_mm": 420.0, "intersects": false, "aabb": {}}],
    "column": [{"refno": "250160_300001", "noun": "COLU", "spec_value": 0, "distance_mm": 880.0, "intersects": false, "aabb": {}}]
  },
  "warnings": []
}
```

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: In a deterministic fixture, nearest wall and nearest column for a BRAN source are selected correctly with no unrelated noun leakage.
- **SC-002**: 100% of nearest query responses include `distance_method`, `unit`, `source`, `query_bbox`, and `resolved_filters`.
- **SC-003**: Existing `sqlite_spatial_api` unit tests continue to pass without response regressions for current query modes.
- **SC-004**: A 10,000-candidate local SQLite fixture returns nearest wall/column results within 500ms on a developer workstation.
- **SC-005**: When no wall or column exists in range, the API returns an empty group with warnings rather than failing the whole query.

## Assumptions

- AABB-based clearance is acceptable for MVP, and exact mesh surface distance is a future enhancement.
- Wall and column naming varies by project, so the MVP must use configurable mappings rather than hard-coded nouns only.
- `items.spec_value` remains available in the SQLite spatial index, but may be zero or missing for some candidates.
- Coordinates and distances are in millimetres, matching existing spatial index behavior.
- The initial consumer is an engineering/debug API or UI, not a safety-critical final construction tolerance report.

## Out of Scope

- Exact mesh, convex hull, or face-to-face distance calculation.
- Automatic generation of wall/column geometry if those targets are not already indexed.
- Frontend measurement UI changes beyond consuming the new API contract.
- Persisting measurement records to a database.
