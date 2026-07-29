use aios_core::tool::db_tool::db1_dehash;
use aios_core::{RefU64, RefnoEnum, SurrealQueryExt, project_primary_db};
use axum::{
    Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::str::FromStr;
use std::sync::Arc;
use surrealdb::types::SurrealValue;
use tokio::time::{Duration, timeout};

use crate::data_interface::db_meta_manager::db_meta;
use crate::data_interface::db_meta_manager::resolve_dbnum_for_refno;
use crate::versioned_db::pe_owner_tree::PeOwnerTreeStore;

#[derive(Clone)]
pub struct E3dTreeApiState {
    pub db_manager: Arc<crate::data_interface::tidb_manager::AiosDBManager>,
}

pub fn create_e3d_tree_routes(state: E3dTreeApiState) -> Router {
    Router::new()
        .route("/api/e3d/world-root", get(get_world_root))
        .route("/api/e3d/node/{refno}", get(get_node))
        .route("/api/e3d/children/{refno}", get(get_children))
        .route("/api/e3d/ancestors/{refno}", get(get_ancestors))
        .route("/api/e3d/subtree-refnos/{refno}", get(get_subtree_refnos))
        .route("/api/e3d/visible-insts/{refno}", get(get_visible_insts))
        .route("/api/e3d/site-nodes/{refno}", get(get_site_nodes))
        .route("/api/e3d/search", post(search_nodes))
        .with_state(state)
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TreeNodeDto {
    pub refno: RefnoEnum,
    pub name: String,
    pub noun: String,
    pub owner: Option<RefnoEnum>,
    pub children_count: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NodeResponse {
    pub success: bool,
    pub node: Option<TreeNodeDto>,
    pub error_message: Option<String>,
    /// specs/023：带 sesno 请求时的版本元信息
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<TreeVersionInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChildrenResponse {
    pub success: bool,
    pub parent_refno: RefnoEnum,
    pub children: Vec<TreeNodeDto>,
    pub truncated: bool,
    pub error_message: Option<String>,
    /// specs/023：带 sesno 请求时的版本元信息
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<TreeVersionInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AncestorsResponse {
    pub success: bool,
    pub refnos: Vec<RefnoEnum>,
    pub error_message: Option<String>,
    /// specs/023：带 sesno 请求时的版本元信息
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<TreeVersionInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SubtreeRefnosResponse {
    pub success: bool,
    pub refnos: Vec<RefnoEnum>,
    pub truncated: bool,
    pub error_message: Option<String>,
    /// specs/023：带 sesno 请求时的版本元信息
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<TreeVersionInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VisibleInstsResponse {
    pub success: bool,
    pub refno: RefnoEnum,
    pub refnos: Vec<RefnoEnum>,
    pub error_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug: Option<VisibleInstsDebug>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VisibleInstsDebug {
    pub candidates_count: usize,
    pub filtered_count: usize,
    pub visible_count: usize,
    pub source: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchRequest {
    pub keyword: String,
    pub nouns: Option<Vec<String>>,
    pub limit: Option<i32>,
    /// specs/023：search 暂不支持版本模式（二期）；传入即返回 VersionUnsupported
    #[serde(default)]
    pub sesno: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchResponse {
    pub success: bool,
    pub items: Vec<TreeNodeDto>,
    pub error_message: Option<String>,
}

// ========================
// Site Nodes API (xeokit Node 层级)
// ========================

/// AABB 包围盒
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NodeAabb {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

/// Site Node 数据（用于前端构建 xeokit Node 层级）
#[derive(Debug, Serialize, Deserialize)]
pub struct SiteNodeDto {
    pub refno: RefnoEnum,
    pub parent: Option<RefnoEnum>,
    pub noun: String,
    pub name: Option<String>,
    pub aabb: Option<NodeAabb>,
    pub has_geo: bool,
}

/// Site Nodes API 响应
#[derive(Debug, Serialize, Deserialize)]
pub struct SiteNodesResponse {
    pub success: bool,
    pub nodes: Vec<SiteNodeDto>,
    pub total: usize,
    pub error_message: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChildrenQuery {
    pub limit: Option<i32>,
    /// specs/023：可选版本参数（per-dbnum sesno）；不传 = 现状 TreeIndex 路径
    pub sesno: Option<u32>,
}

// ========================
// specs/023: 版本化树查询辅助
// ========================

/// 带 `sesno` 的树接口响应附带的版本元信息（contracts/tree-version-api.md）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeVersionInfo {
    pub requested_sesno: u32,
    pub resolved_sesno: u32,
    /// true = 精确命中锚点；false = 回退到「最近不大于」锚点
    pub exact: bool,
    /// 实际层级数据源："pe_owner" | "pe_children_fallback"
    pub source: String,
}

/// 版本解析结果：锚点命中 + VERSION 时刻 + 数据源分界。
#[derive(Debug, Clone)]
pub struct ResolvedTreeVersion {
    pub dbnum: u32,
    pub requested_sesno: u32,
    pub resolved_sesno: u32,
    pub exact: bool,
    /// RFC3339 时刻字符串，可直接拼进 `VERSION d'<...>'`
    pub anchored_at: String,
    /// pe_owner 历史可信起点（`pe_owner_version_meta`）；缺失 = 一律回退 pe.children
    pub maintained_since_sesno: Option<u32>,
}

impl ResolvedTreeVersion {
    /// 数据源选择（FR-008）：可信分界之后走 pe_owner 边，否则回退 pe.children。
    pub fn use_pe_owner(&self) -> bool {
        self.maintained_since_sesno
            .map(|since| self.resolved_sesno >= since)
            .unwrap_or(false)
    }

    pub fn source_str(&self) -> &'static str {
        if self.use_pe_owner() {
            "pe_owner"
        } else {
            "pe_children_fallback"
        }
    }

    /// 拼接 `VERSION d'<anchored_at>'` 子句。
    pub fn version_clause(&self) -> String {
        format!("VERSION d'{}'", self.anchored_at)
    }

    pub fn to_info(&self) -> TreeVersionInfo {
        TreeVersionInfo {
            requested_sesno: self.requested_sesno,
            resolved_sesno: self.resolved_sesno,
            exact: self.exact,
            source: self.source_str().to_string(),
        }
    }
}

/// 版本解析/查询错误 → HTTP 语义（对齐 `/api/model-history/*`，contracts 通用错误表）。
#[derive(Debug)]
pub enum TreeVersionError {
    /// 404：该 dbnum 无任何 ≤sesno 的锚点（含非 versioned 站点）
    AnchorMissing(String),
    /// 410：锚点时刻低于 retention GC 水位线
    Expired(String),
    /// 400：接口不支持版本模式（FR-010）
    VersionUnsupported(String),
    /// 502：底层查询失败
    QueryFailed(String),
}

impl TreeVersionError {
    pub fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::AnchorMissing(m) => (StatusCode::NOT_FOUND, "AnchorMissing", m),
            Self::Expired(m) => (StatusCode::GONE, "Expired", m),
            Self::VersionUnsupported(m) => (StatusCode::BAD_REQUEST, "VersionUnsupported", m),
            Self::QueryFailed(m) => (StatusCode::BAD_GATEWAY, "QueryFailed", m),
        };
        (
            status,
            Json(serde_json::json!({
                "ok": false,
                "error": { "code": code, "message": message },
            })),
        )
            .into_response()
    }
}

/// 把 VERSION 查询的底层错误分类为 Expired / QueryFailed。
///
/// 启发式与 rs-core `version_query::is_history_expired_message` 同源（该函数未导出）：
/// GC 水位线越界在 kvs 层表现为 InvalidArgument / full_history_ts_low 类消息。
pub fn classify_version_query_error(message: &str) -> TreeVersionError {
    let lower = message.to_ascii_lowercase();
    let expired = lower.contains("invalidargument")
        || lower.contains("invalid argument")
        || lower.contains("below the garbage collection")
        || lower.contains("full_history_ts_low")
        || lower.contains("retention")
            && (lower.contains("version") || lower.contains("history") || lower.contains("gc"));
    if expired {
        TreeVersionError::Expired(format!(
            "该 sesno 历史已超出 retention 窗口，请改用源 db 文件重扫或放宽 version_retention：{message}"
        ))
    } else {
        TreeVersionError::QueryFailed(message.to_string())
    }
}

/// 版本入口：锚点解析（specs/022 体系，禁止绕过锚点裸查 VERSION）+ pe_owner 可信分界读取。
pub async fn resolve_tree_version(
    dbnum: u32,
    sesno: u32,
) -> Result<ResolvedTreeVersion, TreeVersionError> {
    match aios_core::resolve_data_anchor(dbnum, sesno).await {
        Ok(Some(hit)) => {
            let maintained_since_sesno =
                crate::versioned_db::pe_owner_meta::get_maintained_since(dbnum)
                    .await
                    .unwrap_or_default();
            Ok(ResolvedTreeVersion {
                dbnum,
                requested_sesno: sesno,
                resolved_sesno: hit.sesno,
                exact: hit.exact,
                anchored_at: hit.anchored_at,
                maintained_since_sesno,
            })
        }
        Ok(None) => Err(TreeVersionError::AnchorMissing(format!(
            "未找到 dbnum={dbnum} sesno<={sesno} 的 sesno_version_anchor（非 versioned 站点或 sesno 过早）"
        ))),
        Err(e) => Err(TreeVersionError::QueryFailed(format!(
            "resolve_anchor failed: {e}"
        ))),
    }
}

/// 由 refno 解析版本入口（树接口通用前置）。
pub async fn resolve_tree_version_for_refno(
    refno: RefnoEnum,
    sesno: u32,
) -> Result<ResolvedTreeVersion, TreeVersionError> {
    let dbnum = resolve_dbnum_for_refno(refno).map_err(|e| {
        TreeVersionError::QueryFailed(format!("resolve_dbnum_for_refno failed: {e}"))
    })?;
    resolve_tree_version(dbnum, sesno).await
}

#[derive(Debug, Deserialize)]
pub struct WorldRootQuery {
    pub sesno: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct NodeQuery {
    pub sesno: Option<u32>,
}

/// 版本模式不支持的接口用于拦截 sesno 参数（FR-010）。
#[derive(Debug, Deserialize)]
pub struct VersionGuardQuery {
    pub sesno: Option<u32>,
}

/// 版本化 PE 展示属性行（批量点查投影）。
#[derive(Debug, Deserialize, SurrealValue)]
struct VersionedPeRow {
    id: RefnoEnum,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    noun: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    owner: Option<RefnoEnum>,
}

/// 版本模式：查询 t 时刻的直接子节点 refnos（顺序 = 该版本同胞顺序）。
///
/// 数据源选择（FR-008）：可信分界内走 pe_owner 图遍历；否则回退 pe.children 字段。
/// 注意：**禁止 `pe_owner:[..]..` id 区间扫 + VERSION**（research C3：语法接受但
/// 静默返回当前态）；`ORDER BY id` 必须在 VERSION 之前（反之为解析错误）。
async fn query_children_refnos_versioned(
    parent: RefnoEnum,
    ver: &ResolvedTreeVersion,
) -> Result<Vec<RefnoEnum>, TreeVersionError> {
    let parent_key = parent.to_pe_key();
    if ver.use_pe_owner() {
        let sql = format!(
            "SELECT VALUE in FROM {parent_key}<-pe_owner ORDER BY id {};",
            ver.version_clause()
        );
        project_primary_db()
            .query_take::<Vec<RefnoEnum>>(&sql, 0)
            .await
            .map_err(|e| classify_version_query_error(&e.to_string()))
    } else {
        let sql = format!(
            "SELECT VALUE children FROM {parent_key} {};",
            ver.version_clause()
        );
        let rows = project_primary_db()
            .query_take::<Vec<Option<Vec<RefnoEnum>>>>(&sql, 0)
            .await
            .map_err(|e| classify_version_query_error(&e.to_string()))?;
        Ok(rows.into_iter().flatten().next().unwrap_or_default())
    }
}

/// 版本模式：批量点查 t 时刻的 PE 展示属性；t 时刻不存在的记录不返回行。
async fn fetch_pe_snapshots_versioned(
    refnos: &[RefnoEnum],
    ver: &ResolvedTreeVersion,
) -> Result<HashMap<RefnoEnum, VersionedPeRow>, TreeVersionError> {
    let mut out = HashMap::new();
    for chunk in refnos.chunks(500) {
        let keys = chunk
            .iter()
            .map(|r| r.to_pe_key())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT id, name, noun, owner FROM [{keys}] {};",
            ver.version_clause()
        );
        let rows = project_primary_db()
            .query_take::<Vec<VersionedPeRow>>(&sql, 0)
            .await
            .map_err(|e| classify_version_query_error(&e.to_string()))?;
        for row in rows {
            out.insert(row.id, row);
        }
    }
    Ok(out)
}

/// 版本模式 children（contracts/tree-version-api.md §children）。
async fn get_children_versioned(parent_refno: RefnoEnum, sesno: u32, limit: i32) -> Response {
    let ver = match resolve_tree_version_for_refno(parent_refno, sesno).await {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let mut child_refnos = match query_children_refnos_versioned(parent_refno, &ver).await {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let truncated = child_refnos.len() > limit as usize;
    if truncated {
        child_refnos.truncate(limit as usize);
    }
    let snapshots = match fetch_pe_snapshots_versioned(&child_refnos, &ver).await {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let children = child_refnos
        .iter()
        .enumerate()
        .map(|(idx, r)| {
            let snap = snapshots.get(r);
            let noun = snap
                .and_then(|s| s.noun.clone())
                .unwrap_or_else(|| "UNKNOWN".to_string());
            let mut name = snap.and_then(|s| s.name.clone()).unwrap_or_default();
            // 与 fn::default_name 一致：name 为空时生成 "{noun} {order+1}"
            if name.trim().is_empty() {
                name = format!("{} {}", noun, idx + 1);
            }
            TreeNodeDto {
                refno: *r,
                name,
                noun,
                owner: Some(parent_refno),
                // 版本模式下逐子统计孙辈代价高，恒为 None（契约边界）
                children_count: None,
            }
        })
        .collect();
    Json(ChildrenResponse {
        success: true,
        parent_refno,
        children,
        truncated,
        error_message: None,
        version: Some(ver.to_info()),
    })
    .into_response()
}

/// 版本模式单节点快照（contracts/tree-version-api.md §node）。
async fn get_node_versioned(refno: RefnoEnum, sesno: u32) -> Response {
    let ver = match resolve_tree_version_for_refno(refno, sesno).await {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let snapshots = match fetch_pe_snapshots_versioned(&[refno], &ver).await {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    match snapshots.get(&refno) {
        Some(snap) => {
            let noun = snap.noun.clone().unwrap_or_else(|| "UNKNOWN".to_string());
            let mut name = snap.name.clone().unwrap_or_default();
            if name.trim().is_empty() {
                // 版本模式无同胞序上下文，空名回退 refno 字符串
                name = refno.to_string();
            }
            Json(NodeResponse {
                success: true,
                node: Some(TreeNodeDto {
                    refno,
                    name,
                    noun,
                    owner: snap.owner,
                    children_count: None,
                }),
                error_message: None,
                version: Some(ver.to_info()),
            })
            .into_response()
        }
        None => Json(NodeResponse {
            success: false,
            node: None,
            error_message: Some(format!("Node not found at sesno {}", ver.resolved_sesno)),
            version: Some(ver.to_info()),
        })
        .into_response(),
    }
}

/// 版本模式 ancestors（contracts/tree-version-api.md §ancestors）。
///
/// 走 `pe.owner` 字段 VERSION 点查逐级上溯（点查最廉价，不走边）；
/// 返回顺序与现状一致：根→父，不含自身。深度上限 20。
async fn get_ancestors_versioned(refno: RefnoEnum, sesno: u32) -> Response {
    const MAX_ANCESTOR_DEPTH: usize = 20;
    let ver = match resolve_tree_version_for_refno(refno, sesno).await {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    let mut chain: Vec<RefnoEnum> = Vec::new();
    let mut seen: HashSet<RefnoEnum> = HashSet::new();
    seen.insert(refno);
    let mut cur = refno;
    for _ in 0..MAX_ANCESTOR_DEPTH {
        let sql = format!(
            "SELECT VALUE owner FROM {} {};",
            cur.to_pe_key(),
            ver.version_clause()
        );
        let owners = match project_primary_db()
            .query_take::<Vec<Option<RefnoEnum>>>(&sql, 0)
            .await
        {
            Ok(v) => v,
            Err(e) => return classify_version_query_error(&e.to_string()).into_response(),
        };
        let Some(owner) = owners.into_iter().flatten().next() else {
            break;
        };
        // owner 自指或成环 → 停（防脏数据死循环）
        if !seen.insert(owner) {
            break;
        }
        chain.push(owner);
        cur = owner;
    }
    chain.reverse();
    Json(AncestorsResponse {
        success: true,
        refnos: chain,
        error_message: None,
        version: Some(ver.to_info()),
    })
    .into_response()
}

/// 版本模式 subtree-refnos（contracts/tree-version-api.md §subtree-refnos）。
///
/// children BFS 逐层（复用版本 children 数据源选择），沿用 max_depth/limit/truncated 语义。
async fn get_subtree_refnos_versioned(
    root_refno: RefnoEnum,
    sesno: u32,
    include_self: bool,
    max_depth: i32,
    limit: usize,
) -> Response {
    let ver = match resolve_tree_version_for_refno(root_refno, sesno).await {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };

    let mut out: Vec<RefnoEnum> = Vec::new();
    if max_depth > 0 {
        let mut visited: HashSet<RefnoEnum> = HashSet::new();
        visited.insert(root_refno);
        let mut frontier = vec![root_refno];
        'bfs: for _ in 0..max_depth {
            if frontier.is_empty() || out.len() > limit {
                break;
            }
            let mut next = Vec::new();
            for parent in frontier {
                let children = match query_children_refnos_versioned(parent, &ver).await {
                    Ok(v) => v,
                    Err(e) => return e.into_response(),
                };
                for child in children {
                    if visited.insert(child) {
                        out.push(child);
                        next.push(child);
                        if out.len() > limit {
                            break 'bfs;
                        }
                    }
                }
            }
            frontier = next;
        }
    }

    if include_self {
        out.insert(0, root_refno);
    }
    let truncated = out.len() > limit;
    if truncated {
        out.truncate(limit);
    }

    Json(SubtreeRefnosResponse {
        success: true,
        refnos: out,
        truncated,
        error_message: None,
        version: Some(ver.to_info()),
    })
    .into_response()
}

/// 版本模式 world-root：仅单 dbnum 上下文生效（契约 §world-root）。
async fn get_world_root_versioned(sesno: u32) -> Response {
    let Some(world) = resolve_offline_world_refno() else {
        return TreeVersionError::QueryFailed(
            "版本模式 world-root 要求单 dbnum 上下文（manual_db_nums 或 db_meta 可推导），当前无法解析".to_string(),
        )
        .into_response();
    };
    let dbnum = resolve_dbnum_for_refno(world).unwrap_or_else(|_| (world.refno().0 >> 32) as u32);
    let ver = match resolve_tree_version(dbnum, sesno).await {
        Ok(v) => v,
        Err(e) => return e.into_response(),
    };
    Json(NodeResponse {
        success: true,
        node: Some(TreeNodeDto {
            refno: world,
            name: "*".to_string(),
            noun: "WORL".to_string(),
            owner: None,
            children_count: None,
        }),
        error_message: None,
        version: Some(ver.to_info()),
    })
    .into_response()
}

#[derive(Debug, Deserialize)]
pub struct SubtreeQuery {
    pub include_self: Option<bool>,
    pub max_depth: Option<i32>,
    pub limit: Option<i32>,
    /// specs/023：可选版本参数；不传 = 现状 TreeIndex 路径
    pub sesno: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct AncestorsQuery {
    /// specs/023：可选版本参数；不传 = 现状 TreeIndex 路径
    pub sesno: Option<u32>,
}

fn parse_refno_path(raw: &str) -> Result<RefnoEnum, StatusCode> {
    RefU64::from_str(raw.trim())
        .map(RefnoEnum::from)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

async fn get_world_root(
    Query(query): Query<WorldRootQuery>,
    State(_state): State<E3dTreeApiState>,
) -> Response {
    // specs/023：带 sesno 走版本分支；不传 sesno 行为与现状完全一致
    if let Some(sesno) = query.sesno {
        return get_world_root_versioned(sesno).await;
    }

    let db_option = aios_core::get_db_option();
    let mdb_name = db_option.mdb_name.clone();

    let (world, world_error) = if let Some(refno) = resolve_offline_world_refno() {
        (refno.refno(), None)
    } else {
        let world_query = timeout(
            Duration::from_secs(2),
            aios_core::mdb::get_world_refno(mdb_name),
        )
        .await;
        match world_query {
            Ok(Ok(r)) => (r.refno(), None),
            Ok(Err(e)) => match resolve_offline_world_refno() {
                Some(refno) => (refno.refno(), Some(format!("get_world_refno failed: {e}"))),
                None => {
                    return Json(NodeResponse {
                        success: false,
                        node: None,
                        error_message: Some(format!("get_world_refno failed: {e}")),
                        version: None,
                    })
                    .into_response();
                }
            },
            Err(_) => match resolve_offline_world_refno() {
                Some(refno) => (
                    refno.refno(),
                    Some("get_world_refno timed out; using offline world refno".to_string()),
                ),
                None => {
                    return Json(NodeResponse {
                        success: false,
                        node: None,
                        error_message: Some("get_world_refno timed out".to_string()),
                        version: None,
                    })
                    .into_response();
                }
            },
        }
    };

    // pe 表可能不包含 WORL/SITE 数据；因此这里优先返回可用的根 refno + noun。
    let node = match query_node(world.into()).await {
        Ok(Some(mut n)) => {
            if let Some(children_count) =
                try_offline_world_children_count(RefnoEnum::from(world)).await
            {
                n.children_count = Some(children_count);
            }
            Some(n)
        }
        Ok(None) | Err(_) => Some(TreeNodeDto {
            refno: world.into(),
            name: "*".to_string(),
            noun: "WORL".to_string(),
            owner: None,
            children_count: try_offline_world_children_count(RefnoEnum::from(world)).await,
        }),
    };

    Json(NodeResponse {
        success: true,
        node,
        error_message: world_error,
        version: None,
    })
    .into_response()
}

async fn get_node(
    Path(refno): Path<String>,
    Query(query): Query<NodeQuery>,
    State(_state): State<E3dTreeApiState>,
) -> Response {
    let refno = match parse_refno_path(&refno) {
        Ok(v) => v,
        Err(status) => return status.into_response(),
    };
    // specs/023：带 sesno 走版本分支
    if let Some(sesno) = query.sesno {
        return get_node_versioned(refno, sesno).await;
    }
    let node = match query_node(refno).await {
        Ok(v) => v,
        Err(_) => None,
    };
    if node.is_none() {
        return Json(NodeResponse {
            success: false,
            node: None,
            error_message: Some("Node not found".to_string()),
            version: None,
        })
        .into_response();
    }

    Json(NodeResponse {
        success: true,
        node,
        error_message: None,
        version: None,
    })
    .into_response()
}

async fn get_children(
    Path(parent_refno): Path<String>,
    Query(query): Query<ChildrenQuery>,
    State(_state): State<E3dTreeApiState>,
) -> Response {
    let parent_refno = match parse_refno_path(&parent_refno) {
        Ok(v) => v,
        Err(status) => return status.into_response(),
    };
    let limit = query.limit.unwrap_or(200).clamp(1, 2000);

    // specs/023：带 sesno 走版本分支（实时 VERSION 查询）；不传 sesno 走现状 TreeIndex 路径
    if let Some(sesno) = query.sesno {
        return get_children_versioned(parent_refno, sesno, limit).await;
    }

    let parent_type = get_type_name(parent_refno).await;

    let mut children: Vec<TreeNodeDto> =
        if parent_type == "WORL" || is_offline_world_refno(parent_refno) {
            let db_option = aios_core::get_db_option();
            let mdb_name = db_option.mdb_name.clone();

            let mut children =
                match aios_core::get_mdb_world_site_ele_nodes(mdb_name, aios_core::DBType::DESI)
                    .await
                {
                    Ok(eles) if !eles.is_empty() => eles
                        .into_iter()
                        .map(|mut ele| {
                            ele.owner = parent_refno;
                            TreeNodeDto {
                                refno: ele.refno,
                                name: ele.name,
                                noun: ele.noun,
                                owner: Some(parent_refno),
                                children_count: Some(i32::from(ele.children_count)),
                            }
                        })
                        .collect(),
                    _ => offline_world_children(parent_refno).await,
                };
            hydrate_tree_node_names(&mut children).await;
            children
        } else {
            query_children_dtos_pe_owner(parent_refno)
                .await
                .unwrap_or_default()
        };

    let truncated = (children.len() as i32) > limit;
    if children.len() > limit as usize {
        children.truncate(limit as usize);
    }

    Json(ChildrenResponse {
        success: true,
        parent_refno,
        children,
        truncated,
        error_message: None,
        version: None,
    })
    .into_response()
}

/// specs/023 M1：latest children 的 pe_owner 主路径——
/// 同胞顺序 = 边序（`ORDER BY id`），noun/children_count 批量查询（不逐子回环）。
async fn query_children_dtos_pe_owner(parent_refno: RefnoEnum) -> anyhow::Result<Vec<TreeNodeDto>> {
    let child_refnos = PeOwnerTreeStore::query_children(parent_refno).await?;
    if child_refnos.is_empty() {
        return Ok(Vec::new());
    }
    let metas = PeOwnerTreeStore::fetch_node_metas(&child_refnos).await?;
    let counts = PeOwnerTreeStore::query_children_counts(&child_refnos).await?;

    let mut out: Vec<TreeNodeDto> = Vec::with_capacity(child_refnos.len());
    for (idx, r) in child_refnos.iter().enumerate() {
        let noun = metas.get(r).map(|m| m.noun.clone()).unwrap_or_default();
        let mut name = crate::fast_model::query_provider::get_pe(*r)
            .await
            .ok()
            .flatten()
            .map(|pe| pe.name)
            .unwrap_or_default();
        // 与 fn::default_name 一致：name 为空时生成 "{noun} {order+1}"（order 来自边序）
        if name.trim().is_empty() {
            name = format!("{} {}", noun, idx + 1);
        }
        out.push(TreeNodeDto {
            refno: *r,
            name,
            noun,
            owner: Some(parent_refno),
            children_count: Some(counts.get(r).copied().unwrap_or(0) as i32),
        });
    }
    Ok(out)
}

async fn hydrate_tree_node_names(nodes: &mut [TreeNodeDto]) {
    for (idx, node) in nodes.iter_mut().enumerate() {
        let pe_name = crate::fast_model::query_provider::get_pe(node.refno)
            .await
            .ok()
            .flatten()
            .map(|pe| pe.name)
            .unwrap_or_default();

        if !pe_name.trim().is_empty() {
            node.name = pe_name;
            continue;
        }

        if node.name.trim().is_empty() || node.name == node.refno.to_string() {
            node.name = format!("{} {}", node.noun, idx + 1);
        }
    }
}

fn resolve_offline_world_refno() -> Option<RefnoEnum> {
    let db_option = aios_core::get_db_option();
    if let Some(dbnum) = db_option
        .manual_db_nums
        .as_ref()
        .and_then(|dbnums| dbnums.first().copied())
    {
        return Some(RefnoEnum::from(RefU64((dbnum as u64) << 32)));
    }

    let _ = db_meta().ensure_loaded();
    let mut dbnums = db_meta().get_all_dbnums();
    if dbnums.is_empty() {
        return None;
    }
    dbnums.sort_unstable();
    dbnums.dedup();
    dbnums
        .into_iter()
        .next()
        .map(|dbnum| RefnoEnum::from(RefU64((dbnum as u64) << 32)))
}

fn is_offline_world_refno(refno: RefnoEnum) -> bool {
    resolve_offline_world_refno()
        .map(|world| world == refno)
        .unwrap_or(false)
}

async fn try_offline_world_children_count(world_refno: RefnoEnum) -> Option<i32> {
    let children = offline_world_children(world_refno).await;
    Some(children.len() as i32)
}

async fn offline_world_children(parent_refno: RefnoEnum) -> Vec<TreeNodeDto> {
    offline_world_children_pe_owner(parent_refno)
        .await
        .unwrap_or_default()
}

/// D4：offline world 子节点的 DB 主路径。
///
/// 1) 先按 pe_owner/children 取 parent 的直接子（parent 可能就是真实 WORL 行）；
/// 2) 合成 world（`dbnum<<32`）无 pe 行时，按 db_meta/manual dbnum 清单逐库找
///    `pe WHERE noun='WORL'` 行并取其 children；仍空则回退该库 SITE 行清单。
async fn offline_world_children_pe_owner(
    parent_refno: RefnoEnum,
) -> anyhow::Result<Vec<TreeNodeDto>> {
    let direct = query_children_dtos_pe_owner(parent_refno).await?;
    if !direct.is_empty() {
        return Ok(direct);
    }

    if !is_offline_world_refno(parent_refno) {
        return Ok(Vec::new());
    }

    let mut dbnums = aios_core::get_db_option()
        .manual_db_nums
        .clone()
        .unwrap_or_default();
    if dbnums.is_empty() {
        let _ = db_meta().ensure_loaded();
        dbnums = db_meta().get_all_dbnums();
    }
    dbnums.sort_unstable();
    dbnums.dedup();

    let mut roots: Vec<RefnoEnum> = Vec::new();
    for dbnum in dbnums {
        let store = PeOwnerTreeStore::new(vec![dbnum]);
        let worlds = store.query_noun_refnos("WORL", None).await?;
        let mut db_roots: Vec<RefnoEnum> = Vec::new();
        for world in worlds {
            db_roots.extend(PeOwnerTreeStore::query_children(world).await?);
        }
        if db_roots.is_empty() {
            // 库内无 WORL 行（或 WORL 无子）：回退该库 SITE 清单（与旧 by_scan 路径对齐）
            db_roots = store.query_noun_refnos("SITE", None).await?;
        }
        roots.extend(db_roots);
    }
    roots.sort_by_key(|r| r.refno().0);
    roots.dedup();
    if roots.is_empty() {
        return Ok(Vec::new());
    }

    let metas = PeOwnerTreeStore::fetch_node_metas(&roots).await?;
    let counts = PeOwnerTreeStore::query_children_counts(&roots).await?;
    Ok(roots
        .into_iter()
        .map(|r| TreeNodeDto {
            refno: r,
            name: r.to_string(),
            noun: metas.get(&r).map(|m| m.noun.clone()).unwrap_or_default(),
            owner: Some(parent_refno),
            children_count: Some(counts.get(&r).copied().unwrap_or(0) as i32),
        })
        .collect())
}

fn configured_project_output_root() -> std::path::PathBuf {
    crate::versioned_db::db_meta_info::get_current_project_name()
        .map(|project_name| {
            crate::versioned_db::db_meta_info::get_project_output_dir(&project_name)
        })
        .unwrap_or_else(crate::versioned_db::db_meta_info::get_output_root)
}

fn legacy_project_output_root() -> std::path::PathBuf {
    let db_option = aios_core::get_db_option();
    if db_option.project_name.trim().is_empty() {
        std::path::PathBuf::from("output")
    } else {
        std::path::PathBuf::from("output").join(db_option.project_name.trim())
    }
}

#[cfg_attr(not(feature = "parquet-export"), allow(dead_code))]
fn resolve_local_dbnum_dir(dbnum: u32, required_file: &str) -> Option<std::path::PathBuf> {
    let project_output_root = configured_project_output_root();
    let legacy_project_output_root = legacy_project_output_root();
    let mut candidates = vec![
        project_output_root.join("parquet").join(dbnum.to_string()),
        project_output_root
            .join("instances")
            .join(dbnum.to_string()),
    ];
    if legacy_project_output_root != project_output_root {
        candidates.push(
            legacy_project_output_root
                .join("parquet")
                .join(dbnum.to_string()),
        );
        candidates.push(
            legacy_project_output_root
                .join("instances")
                .join(dbnum.to_string()),
        );
    }
    candidates.push(std::path::PathBuf::from("output/parquet").join(dbnum.to_string()));
    candidates.push(std::path::PathBuf::from("output/instances").join(dbnum.to_string()));

    candidates
        .into_iter()
        .find(|candidate| candidate.join(required_file).exists())
}

#[cfg(not(feature = "parquet-export"))]
fn filter_visible_candidates_from_parquet(
    _candidates: &[RefnoEnum],
    _dbnum: u32,
    _bran_hang_load_roots: &HashSet<RefnoEnum>,
) -> Option<(Vec<RefnoEnum>, String)> {
    // 未编译 parquet-export 时跳过 parquet 过滤，调用方回退 inst_relate 查询。
    None
}

#[cfg(feature = "parquet-export")]
fn filter_visible_candidates_from_parquet(
    candidates: &[RefnoEnum],
    dbnum: u32,
    bran_hang_load_roots: &HashSet<RefnoEnum>,
) -> Option<(Vec<RefnoEnum>, String)> {
    use polars::prelude::*;

    let (path, source) = if let Some(dir) = resolve_local_dbnum_dir(dbnum, "geo_instances.parquet")
    {
        (dir.join("geo_instances.parquet"), "parquet_geo_instances")
    } else if let Some(dir) = resolve_local_dbnum_dir(dbnum, "instances.parquet") {
        (dir.join("instances.parquet"), "parquet_instances")
    } else {
        return None;
    };

    let file = std::fs::File::open(&path).ok()?;
    let df = ParquetReader::new(file).finish().ok()?;
    let refno_col = df.column("refno_str").ok()?.str().ok()?;
    let wanted = candidates
        .iter()
        .map(|r| r.to_string())
        .collect::<HashSet<_>>();
    let mut available = HashSet::<String>::new();
    for value in refno_col.into_iter().flatten() {
        if wanted.contains(value) {
            available.insert(value.to_string());
        }
    }

    let mut out = Vec::new();
    for r in candidates.iter().copied() {
        let key = r.to_string();
        let matched = bran_hang_load_roots.contains(&r)
            || available.contains(&key)
            || (key.contains('/') && available.contains(&key.replace('/', "_")));
        if matched {
            out.push(r);
        }
    }
    out.sort();
    out.dedup();
    Some((out, source.to_string()))
}

async fn get_ancestors(
    Path(refno): Path<String>,
    Query(query): Query<AncestorsQuery>,
    State(_state): State<E3dTreeApiState>,
) -> Response {
    let refno = match parse_refno_path(&refno) {
        Ok(v) => v,
        Err(status) => return status.into_response(),
    };
    // specs/023：带 sesno 走版本分支
    if let Some(sesno) = query.sesno {
        return get_ancestors_versioned(refno, sesno).await;
    }
    let ancestors_result = PeOwnerTreeStore::query_ancestors(refno).await;
    let ancestors = match ancestors_result {
        Ok(v) => v,
        Err(e) => {
            return Json(AncestorsResponse {
                success: false,
                refnos: vec![],
                error_message: Some(format!("query_ancestor_refnos failed: {e}")),
                version: None,
            })
            .into_response();
        }
    };

    Json(AncestorsResponse {
        success: true,
        refnos: ancestors,
        error_message: None,
        version: None,
    })
    .into_response()
}

async fn get_subtree_refnos(
    Path(root_refno): Path<String>,
    Query(query): Query<SubtreeQuery>,
    State(_state): State<E3dTreeApiState>,
) -> Response {
    let root_refno = match parse_refno_path(&root_refno) {
        Ok(v) => v,
        Err(status) => return status.into_response(),
    };
    let include_self = query.include_self.unwrap_or(true);
    let max_depth = query.max_depth.unwrap_or(64).clamp(0, 256);
    let limit = query.limit.unwrap_or(50_000).clamp(1, 200_000) as usize;

    // specs/023：带 sesno 走版本分支（children BFS 实时 VERSION 查询）
    if let Some(sesno) = query.sesno {
        return get_subtree_refnos_versioned(root_refno, sesno, include_self, max_depth, limit)
            .await;
    }

    let mut out: Vec<RefnoEnum> = if max_depth <= 0 {
        Vec::new()
    } else {
        match PeOwnerTreeStore::query_descendants(root_refno, Some(max_depth as usize)).await {
            Ok(v) => v,
            Err(e) => {
                return Json(SubtreeRefnosResponse {
                    success: false,
                    refnos: vec![],
                    truncated: false,
                    error_message: Some(format!("pe_owner query_descendants failed: {e}")),
                    version: None,
                })
                .into_response();
            }
        }
    };

    if include_self {
        out.insert(0, root_refno);
    }

    let truncated = out.len() > limit;
    if out.len() > limit {
        out.truncate(limit);
    }

    Json(SubtreeRefnosResponse {
        success: true,
        refnos: out,
        truncated,
        error_message: None,
        version: None,
    })
    .into_response()
}

/// specs/023 FR-010：几何/实例数据 latest-only，带 sesno 显式拒绝而非静默返回当前态。
async fn get_visible_insts(
    Path(refno): Path<String>,
    Query(guard): Query<VersionGuardQuery>,
    State(state): State<E3dTreeApiState>,
) -> Response {
    if guard.sesno.is_some() {
        return TreeVersionError::VersionUnsupported(
            "visible-insts 数据源（几何实例/instances json/parquet）为 latest-only，不支持版本模式"
                .to_string(),
        )
        .into_response();
    }
    match get_visible_insts_inner(Path(refno), State(state)).await {
        Ok(json) => json.into_response(),
        Err(status) => status.into_response(),
    }
}

async fn get_visible_insts_inner(
    Path(refno): Path<String>,
    State(state): State<E3dTreeApiState>,
) -> Result<Json<VisibleInstsResponse>, StatusCode> {
    let refno = parse_refno_path(&refno)?;
    // 1) 先拿“深度可见实例”（可能包含无几何的组节点）
    // 层级查询统一走 indextree（TreeIndex）
    let mut candidates = if is_offline_world_refno(refno) {
        let mut out = Vec::new();
        for child in offline_world_children(refno).await {
            match crate::fast_model::query_compat::query_deep_visible_inst_refnos(child.refno).await
            {
                Ok(mut values) => out.append(&mut values),
                Err(e) => {
                    return Ok(Json(VisibleInstsResponse {
                        success: false,
                        refno,
                        refnos: vec![],
                        error_message: Some(format!(
                            "query_deep_visible_inst_refnos failed for child {}: {e}",
                            child.refno
                        )),
                        debug: None,
                    }));
                }
            }
        }
        out.sort();
        out.dedup();
        out
    } else {
        match crate::fast_model::query_compat::query_deep_visible_inst_refnos(refno).await {
            Ok(v) => v,
            Err(e) => {
                return Ok(Json(VisibleInstsResponse {
                    success: false,
                    refno,
                    refnos: vec![],
                    error_message: Some(format!("query_deep_visible_inst_refnos failed: {e}")),
                    debug: None,
                }));
            }
        }
    };

    // 请求根本身也可能承载几何（例如 BRAN/HANG 的 tubi_relate）。
    if !candidates.contains(&refno) {
        candidates.push(refno);
    }
    let candidates_count = candidates.len();

    let bran_hang_load_roots: HashSet<RefnoEnum> =
        match PeOwnerTreeStore::fetch_node_metas(&candidates).await {
            Ok(metas) => candidates
                .iter()
                .copied()
                .filter(|candidate| {
                    metas
                        .get(candidate)
                        .map(|m| {
                            let noun = m.noun.trim().to_ascii_uppercase();
                            noun == "BRAN" || noun == "HANG"
                        })
                        .unwrap_or(false)
                })
                .collect(),
            Err(_) => HashSet::new(),
        };

    // 2) 优先用 instances_{dbnum}.json 做“可加载几何”过滤：与前端实际加载数据保持一致。
    //    - 这可以避免 query_deep_visible_inst_refnos 返回“组节点/无几何节点”，导致前端 instances 缺失。
    //    - 若文件不存在，再回退到 inst_relate 的几何实例查询做过滤。
    fn parse_dbno(r: RefnoEnum) -> Option<u32> {
        crate::data_interface::db_meta_manager::resolve_dbnum_for_refno(r)
            .ok()
            .or_else(|| {
                if is_offline_world_refno(r) {
                    Some((r.refno().0 >> 32) as u32)
                } else {
                    None
                }
            })
    }

    fn collect_component_refnos(v: &serde_json::Value, out: &mut HashSet<String>) {
        // 兼容多种 compact JSON 格式：递归收集所有 key=="refno" 的字符串
        match v {
            serde_json::Value::Object(map) => {
                for (k, val) in map {
                    if k == "refno" {
                        if let Some(s) = val.as_str() {
                            out.insert(s.to_string());
                        }
                    }
                    collect_component_refnos(val, out);
                }
            }
            serde_json::Value::Array(arr) => {
                for item in arr {
                    collect_component_refnos(item, out);
                }
            }
            _ => {}
        }
    }

    // NOTE:
    // - instances_{dbnum}.json 位于当前配置项目输出目录：<output_root>/<project_name>/instances/
    // - 历史兼容：也支持旧路径 output/<project_name>/instances/ 与 output/instances/
    // - 文件读取/解析成功时：即使结果为空，也不回退 inst_relate（避免 inst_relate 缺失时接口直接报错）
    let visible_dbnum = parse_dbno(refno);
    let (refnos, file_ok) = if let Some(dbnum) = visible_dbnum {
        let project_output_root = configured_project_output_root();
        let legacy_project_output_root = legacy_project_output_root();
        let instances_path_new = project_output_root
            .join("instances")
            .join(format!("instances_{dbnum}.json"));
        let instances_path_legacy_project = legacy_project_output_root
            .join("instances")
            .join(format!("instances_{dbnum}.json"));
        let instances_path_old = std::path::Path::new("output")
            .join("instances")
            .join(format!("instances_{dbnum}.json"));

        let bytes = fs::read(&instances_path_new)
            .or_else(|_| fs::read(&instances_path_legacy_project))
            .or_else(|_| fs::read(&instances_path_old));
        if let Ok(bytes) = bytes {
            match serde_json::from_slice::<serde_json::Value>(&bytes) {
                Ok(json) => {
                    let mut available = HashSet::<String>::new();
                    collect_component_refnos(&json, &mut available);

                    let mut out = Vec::new();
                    for r in candidates.iter().copied() {
                        let key = r.to_string();
                        let matched = if bran_hang_load_roots.contains(&r) {
                            true
                        } else if available.contains(&key) {
                            true
                        } else if key.contains('/') {
                            available.contains(&key.replace('/', "_"))
                        } else {
                            false
                        };
                        if matched {
                            out.push(r);
                        }
                    }

                    out.sort();
                    out.dedup();
                    (out, true)
                }
                Err(_) => (Vec::new(), false),
            }
        } else {
            (Vec::new(), false)
        }
    } else {
        (Vec::new(), false)
    };

    // 文件读取/解析成功时：直接使用文件过滤结果（允许为空）
    // 文件缺失/解析失败：优先使用同一 output_root 下的 parquet 过滤，再回退 inst_relate 几何实例过滤。
    let (refnos, source) = if file_ok {
        (refnos, "instances_json".to_string())
    } else if let Some(dbnum) = visible_dbnum {
        if let Some((parquet_refnos, parquet_source)) =
            filter_visible_candidates_from_parquet(&candidates, dbnum, &bran_hang_load_roots)
        {
            (parquet_refnos, parquet_source)
        } else {
            match crate::fast_model::export_model::model_exporter::query_geometry_instances(
                &candidates,
                true,  // enable_holes：这里只用于过滤是否存在几何实例
                false, // verbose
            )
            .await
            {
                Ok(v) => {
                    let mut out = v.into_iter().map(|q| q.refno).collect::<Vec<_>>();
                    out.extend(bran_hang_load_roots.iter().copied());
                    out.sort();
                    out.dedup();
                    (out, "surreal_geometry".to_string())
                }
                Err(e) => {
                    return Ok(Json(VisibleInstsResponse {
                        success: false,
                        refno,
                        refnos: vec![],
                        error_message: Some(format!("query_geometry_instances failed: {e}")),
                        debug: None,
                    }));
                }
            }
        }
    } else {
        match crate::fast_model::export_model::model_exporter::query_geometry_instances(
            &candidates,
            true,  // enable_holes：这里只用于过滤是否存在几何实例
            false, // verbose
        )
        .await
        {
            Ok(v) => {
                let mut out = v.into_iter().map(|q| q.refno).collect::<Vec<_>>();
                out.extend(bran_hang_load_roots.iter().copied());
                out.sort();
                out.dedup();
                (out, "surreal_geometry".to_string())
            }
            Err(e) => {
                return Ok(Json(VisibleInstsResponse {
                    success: false,
                    refno,
                    refnos: vec![],
                    error_message: Some(format!("query_geometry_instances failed: {e}")),
                    debug: None,
                }));
            }
        }
    };
    let visible_count = refnos.len();

    Ok(Json(VisibleInstsResponse {
        success: true,
        refno,
        refnos,
        error_message: None,
        debug: Some(VisibleInstsDebug {
            candidates_count,
            filtered_count: candidates_count.saturating_sub(visible_count),
            visible_count,
            source,
        }),
    }))
}

/// specs/023 FR-010：search 版本模式二期，带 sesno 显式拒绝。
async fn search_nodes(
    State(state): State<E3dTreeApiState>,
    Json(request): Json<SearchRequest>,
) -> Response {
    if request.sesno.is_some() {
        return TreeVersionError::VersionUnsupported(
            "search 暂不支持版本模式（noun 表为当前态；版本化搜索属二期范围）".to_string(),
        )
        .into_response();
    }
    match search_nodes_inner(State(state), Json(request)).await {
        Ok(json) => json.into_response(),
        Err(status) => status.into_response(),
    }
}

async fn search_nodes_inner(
    State(_state): State<E3dTreeApiState>,
    Json(request): Json<SearchRequest>,
) -> Result<Json<SearchResponse>, StatusCode> {
    let keyword = request.keyword.trim();
    if keyword.is_empty() {
        return Ok(Json(SearchResponse {
            success: true,
            items: vec![],
            error_message: None,
        }));
    }

    let limit = request.limit.unwrap_or(50).clamp(1, 200) as usize;

    // 不使用 pe 全表搜索，必须指定具体 noun 表查询
    const DEFAULT_SEARCH_NOUNS: &[&str] = &[
        "EQUI", "PIPE", "BRAN", "NOZZ", "VALV", "PUMP", "TANK", "INST", "ZONE", "STRU", "SUBS",
        "FRMW", "SITE",
    ];

    let nouns: Vec<String> = match request.nouns.as_ref() {
        Some(v) if !v.is_empty() => v.clone(),
        _ => DEFAULT_SEARCH_NOUNS.iter().map(|s| s.to_string()).collect(),
    };

    let mut items: Vec<TreeNodeDto> = Vec::new();
    for noun in &nouns {
        if items.len() >= limit {
            break;
        }

        let rows = match aios_core::query_noun_hierarchy(noun, Some(keyword), None).await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("query_noun_hierarchy failed for noun={noun}: {e}");
                continue;
            }
        };

        for row in rows {
            if items.len() >= limit {
                break;
            }
            items.push(TreeNodeDto {
                refno: row.id,
                name: row.name,
                noun: row.noun,
                owner: Some(row.owner),
                children_count: row.children_cnt,
            });
        }
    }

    Ok(Json(SearchResponse {
        success: true,
        items,
        error_message: None,
    }))
}

async fn query_node(refno: RefnoEnum) -> anyhow::Result<Option<TreeNodeDto>> {
    let pe = aios_core::get_pe(refno).await?;
    let Some(pe) = pe else {
        return Ok(None);
    };

    let mut name = pe.name;
    // 与 fn::default_name 一致：name 为空时生成 "{noun} {order+1}"
    if name.trim().is_empty() {
        let order = PeOwnerTreeStore::query_children(pe.owner)
            .await
            .ok()
            .and_then(|siblings| siblings.iter().position(|r| *r == refno))
            .unwrap_or(0);
        name = format!("{} {}", pe.noun, order + 1);
    }

    Ok(Some(TreeNodeDto {
        refno: pe.refno,
        name,
        noun: pe.noun,
        owner: Some(pe.owner),
        children_count: None,
    }))
}

async fn get_type_name(refno: RefnoEnum) -> String {
    aios_core::get_type_name(refno)
        .await
        .unwrap_or_else(|_| "UNKNOWN".to_string())
}

// ========================
// Site Nodes Handler
// ========================

/// 查询 scene_node 表的返回结构
#[derive(Debug, Deserialize, SurrealValue)]
struct SceneNodeRow {
    pub id: i64,
    pub parent: Option<i64>,
    pub has_geo: bool,
    pub is_leaf: bool,
    pub aabb_min: Option<Vec<f64>>,
    pub aabb_max: Option<Vec<f64>>,
}

/// 获取 SITE 的所有 Node 层级数据（用于前端构建 xeokit Node 层级）
/// specs/023 FR-010：scene_node 数据源非版本化，带 sesno 显式拒绝。
async fn get_site_nodes(
    Path(site_refno): Path<String>,
    Query(guard): Query<VersionGuardQuery>,
    State(state): State<E3dTreeApiState>,
) -> Response {
    if guard.sesno.is_some() {
        return TreeVersionError::VersionUnsupported(
            "site-nodes 数据源（scene_node）为非版本化表，不支持版本模式".to_string(),
        )
        .into_response();
    }
    match get_site_nodes_inner(Path(site_refno), State(state)).await {
        Ok(json) => json.into_response(),
        Err(status) => status.into_response(),
    }
}

async fn get_site_nodes_inner(
    Path(site_refno): Path<String>,
    State(_state): State<E3dTreeApiState>,
) -> Result<Json<SiteNodesResponse>, StatusCode> {
    let site_refno = parse_refno_path(&site_refno)?;
    // 1. 获取 SITE 的所有子孙节点（通过 BFS 遍历 contains 关系）
    const MAX_DEPTH: usize = 20;
    const MAX_NODES: usize = 10000;
    const CHUNK_SIZE: usize = 500;

    let site_id = site_refno.refno().0 as i64;
    let mut all_ids: Vec<i64> = vec![site_id];
    let mut frontier: Vec<i64> = vec![site_id];
    let mut visited: std::collections::HashSet<i64> = std::collections::HashSet::new();
    visited.insert(site_id);

    for _ in 0..MAX_DEPTH {
        if frontier.is_empty() || all_ids.len() >= MAX_NODES {
            break;
        }

        let mut next_frontier: Vec<i64> = Vec::new();
        for chunk in frontier.chunks(CHUNK_SIZE) {
            let in_list = chunk
                .iter()
                .map(|id| format!("scene_node:{}", id))
                .collect::<Vec<_>>()
                .join(",");

            let sql = format!("SELECT VALUE record::id(out) FROM [{}]->contains", in_list);
            let children: Vec<i64> = project_primary_db()
                .query_take(&sql, 0)
                .await
                .unwrap_or_default();

            for child_id in children {
                if all_ids.len() >= MAX_NODES {
                    break;
                }
                if visited.insert(child_id) {
                    all_ids.push(child_id);
                    next_frontier.push(child_id);
                }
            }
        }
        frontier = next_frontier;
    }

    // 2. 批量查询 scene_node 详细信息（包括 aabb）
    let mut nodes: Vec<SiteNodeDto> = Vec::with_capacity(all_ids.len());

    for chunk in all_ids.chunks(500) {
        let id_list = chunk
            .iter()
            .map(|id| format!("scene_node:{}", id))
            .collect::<Vec<_>>()
            .join(",");

        // 查询 scene_node 表，同时关联 aabb 表获取包围盒
        let sql = format!(
            r#"SELECT 
                record::id(id) as id,
                parent,
                has_geo,
                is_leaf,
                aabb.min as aabb_min,
                aabb.max as aabb_max
            FROM [{}]"#,
            id_list
        );

        let rows: Vec<SceneNodeRow> = project_primary_db()
            .query_take(&sql, 0)
            .await
            .unwrap_or_default();

        for row in rows {
            let refno = RefnoEnum::from(RefU64(row.id as u64));
            let parent = row.parent.map(|p| RefnoEnum::from(RefU64(p as u64)));

            // 获取 pe 表的 noun 和 name
            let (noun, name) = match aios_core::get_pe(refno).await {
                Ok(Some(pe)) => (pe.noun, Some(pe.name)),
                _ => ("UNKNOWN".to_string(), None),
            };

            // 构建 AABB
            let aabb = match (&row.aabb_min, &row.aabb_max) {
                (Some(min), Some(max)) if min.len() >= 3 && max.len() >= 3 => Some(NodeAabb {
                    min: [min[0], min[1], min[2]],
                    max: [max[0], max[1], max[2]],
                }),
                _ => None,
            };

            nodes.push(SiteNodeDto {
                refno,
                parent,
                noun,
                name,
                aabb,
                has_geo: row.has_geo,
            });
        }
    }

    let total = nodes.len();
    Ok(Json(SiteNodesResponse {
        success: true,
        nodes,
        total,
        error_message: None,
    }))
}
