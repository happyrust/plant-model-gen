# API Contract: 模型树版本查询（/api/e3d/* 增量契约）

**Date**: 2026-07-19 | **Plan**: `../plan.md`

对既有 `/api/e3d/*` 接口的**向后兼容增量**：全部新增可选 query 参数 `sesno`（u32，per-dbnum 会话号）。不传 `sesno` 时请求/响应与现状完全一致（本契约不适用）。

## 通用语义

### 版本解析

1. 由路径 refno 解析 dbnum（world-root 场景取配置 dbnum）
2. `resolve_anchor(dbnum, sesno)`：命中精确锚点或"最近不大于"回退锚点
3. 锚点时刻 `$t = fn::sesno_version(dbnum, resolved_sesno)`，全部数据读取带 `VERSION $t`

### 通用响应扩展

带 `sesno` 的成功响应均附加：

```json
"version": {
  "requested_sesno": 120,
  "resolved_sesno": 118,
  "exact": false,
  "source": "pe_owner"        // 或 "pe_children_fallback"
}
```

### 通用错误

| 场景 | HTTP | code | 说明 |
|------|------|------|------|
| 该 dbnum 无任何 ≤sesno 的锚点 / 非 versioned 站点 | 404 | `AnchorMissing` | 与 `/api/model-history/resolve-anchor` 对齐 |
| 锚点时刻低于 retention GC 水位线 | 410 | `Expired` | 与 `/api/model-history/snapshot` 对齐 |
| sesno 参数非法（非数字等） | 400 | `BadRequest` | |
| 接口不支持版本模式 | 400 | `VersionUnsupported` | 见下"不支持的接口" |

## 各接口契约

### GET /api/e3d/children/{refno}?sesno=N&limit=200

t 时刻 refno 的直接子节点，顺序 = 该版本 PDMS 同胞顺序。

```json
{
  "success": true,
  "parent_refno": "17496/1206",
  "children": [
    { "refno": "17496/2001", "name": "ZONE-A", "noun": "ZONE",
      "owner": "17496/1206", "children_count": null }
  ],
  "truncated": false,
  "error_message": null,
  "version": { "requested_sesno": 120, "resolved_sesno": 120, "exact": true, "source": "pe_owner" }
}
```

约束：`children_count` 在版本模式下恒为 `null`（成本边界，spec Edge Cases）；`name/noun` 取自 `VERSION $t` 的 PE 快照，已删除节点照常返回其当时值。

### GET /api/e3d/node/{refno}?sesno=N

t 时刻单节点快照（name/noun/owner）。节点在 t 时刻不存在 → `success:false, error_message:"Node not found at sesno N"`（HTTP 200，语义与现状 Node not found 一致）。

### GET /api/e3d/ancestors/{refno}?sesno=N

t 时刻的祖先链（根→父）。实现走 `pe.owner` VERSION 点查逐级上溯，深度上限 20。

### GET /api/e3d/subtree-refnos/{refno}?sesno=N&include_self=true&max_depth=64&limit=50000

t 时刻子树 refno 集合；BFS 逐层 children，`truncated` 语义与现状一致。

### GET /api/e3d/world-root?sesno=N

版本模式下仅当能解析出单一 dbnum 上下文时生效（manual_db_nums 首项 / 单库站点）；多 dbnum 世界树的版本组合由前端按 dbnum 分别请求子树（spec Edge Cases）。

### 不支持的接口（FR-010）

`GET /api/e3d/site-nodes/{refno}?sesno=N`、`GET /api/e3d/visible-insts/{refno}?sesno=N`、`POST /api/e3d/search`（body 带 sesno）→ 400 `VersionUnsupported`，message 说明该数据源为 latest-only（scene_node / 几何实例）。

## 版本列表（复用，不新增）

前端版本选择器数据源：`GET /api/model-history/anchors?dbnum=N`（specs/022 既有）。

## 实现层强约束（评审检查点）

- children 主查询：`SELECT VALUE in FROM pe:<owner><-pe_owner ORDER BY id VERSION $t`
  （数组 id 按 [owner, 序号] 结构序排序；`ORDER BY record::id(id)[1]` 在 fork 上是解析错误——incr_shapes smoke 实测）
- **禁止** `pe_owner:[..]..[..] VERSION` id 区间扫（research C3：语义错误）
- 回退查询：`SELECT VALUE children FROM pe:<owner> VERSION $t`（`maintained_since_sesno` 分界，见 data-model.md）
- 属性批量点查：`SELECT name, noun, owner FROM [pe:a, pe:b, ...] VERSION $t`
