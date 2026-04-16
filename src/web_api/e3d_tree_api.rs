use aios_core::tool::db_tool::db1_dehash;
use aios_core::{RefU64, RefnoEnum, SurrealQueryExt, project_primary_db};
use axum::{
    Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::sync::Arc;
use surrealdb::types::SurrealValue;

use crate::data_interface::db_meta_manager::db_meta;
use crate::fast_model::gen_model::tree_index_manager::{TreeIndexManager, load_index_with_large_stack};

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
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChildrenResponse {
    pub success: bool,
    pub parent_refno: RefnoEnum,
    pub children: Vec<TreeNodeDto>,
    pub truncated: bool,
    pub error_message: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AncestorsResponse {
    pub success: bool,
    pub refnos: Vec<RefnoEnum>,
    pub error_message: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SubtreeRefnosResponse {
    pub success: bool,
    pub refnos: Vec<RefnoEnum>,
    pub truncated: bool,
    pub error_message: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VisibleInstsResponse {
    pub success: bool,
    pub refno: RefnoEnum,
    pub refnos: Vec<RefnoEnum>,
    pub error_message: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchRequest {
    pub keyword: String,
    pub nouns: Option<Vec<String>>,
    pub limit: Option<i32>,
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
}

#[derive(Debug, Deserialize)]
pub struct SubtreeQuery {
    pub include_self: Option<bool>,
    pub max_depth: Option<i32>,
    pub limit: Option<i32>,
}

async fn get_world_root(
    State(_state): State<E3dTreeApiState>,
) -> Result<Json<NodeResponse>, StatusCode> {
    let db_option = aios_core::get_db_option();
    let mdb_name = db_option.mdb_name.clone();

    let (world, world_error) = match aios_core::mdb::get_world_refno(mdb_name).await {
        Ok(r) => (r.refno(), None),
        Err(e) => match resolve_offline_world_refno() {
            Some(refno) => (refno.refno(), Some(format!("get_world_refno failed: {e}"))),
            None => {
                return Ok(Json(NodeResponse {
                    success: false,
                    node: None,
                    error_message: Some(format!("get_world_refno failed: {e}")),
                }));
            }
        },
    };

    // pe 表可能不包含 WORL/SITE 数据；因此这里优先返回可用的根 refno + noun。
    let node = match query_node(world.into()).await {
        Ok(Some(mut n)) => {
            if let Some(children_count) = try_offline_world_children_count(RefnoEnum::from(world)) {
                n.children_count = Some(children_count);
            }
            Some(n)
        }
        Ok(None) | Err(_) => Some(TreeNodeDto {
            refno: world.into(),
            name: "*".to_string(),
            noun: "WORL".to_string(),
            owner: None,
            children_count: try_offline_world_children_count(RefnoEnum::from(world)),
        }),
    };

    Ok(Json(NodeResponse {
        success: true,
        node,
        error_message: world_error,
    }))
}

async fn get_node(
    Path(refno): Path<RefnoEnum>,
    State(_state): State<E3dTreeApiState>,
) -> Result<Json<NodeResponse>, StatusCode> {
    let node = match query_node(refno).await {
        Ok(v) => v,
        Err(_) => None,
    };
    if node.is_none() {
        return Ok(Json(NodeResponse {
            success: false,
            node: None,
            error_message: Some("Node not found".to_string()),
        }));
    }

    Ok(Json(NodeResponse {
        success: true,
        node,
        error_message: None,
    }))
}

async fn get_children(
    Path(parent_refno): Path<RefnoEnum>,
    Query(query): Query<ChildrenQuery>,
    State(_state): State<E3dTreeApiState>,
) -> Result<Json<ChildrenResponse>, StatusCode> {
    let limit = query.limit.unwrap_or(200).clamp(1, 2000);

    let parent_type = get_type_name(parent_refno).await;

    let mut children: Vec<TreeNodeDto> = if parent_type == "WORL" || is_offline_world_refno(parent_refno)
    {
        let db_option = aios_core::get_db_option();
        let mdb_name = db_option.mdb_name.clone();

        match aios_core::get_mdb_world_site_ele_nodes(mdb_name, aios_core::DBType::DESI).await {
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
            _ => offline_world_children(parent_refno),
        }
    } else {
        match TreeIndexManager::resolve_dbnum_for_refno(parent_refno) {
            Ok(dbnum) => {
                let manager = TreeIndexManager::with_default_dir(vec![dbnum]);
                let child_refnos = manager.query_children(parent_refno);

                let mut out: Vec<TreeNodeDto> = Vec::with_capacity(child_refnos.len());
                for (idx, r) in child_refnos.iter().enumerate() {
                    let noun = manager.get_noun(*r).unwrap_or_default();
                    let mut name = crate::fast_model::query_provider::get_pe(*r)
                        .await
                        .ok()
                        .flatten()
                        .map(|pe| pe.name)
                        .unwrap_or_default();
                    // 与 fn::default_name 一致：name 为空时生成 "{noun} {order+1}"
                    if name.trim().is_empty() {
                        name = format!("{} {}", noun, idx + 1);
                    }
                    let children_count = manager.query_children(*r).len() as i32;
                    out.push(TreeNodeDto {
                        refno: *r,
                        name,
                        noun,
                        owner: Some(parent_refno),
                        children_count: Some(children_count),
                    });
                }
                out
            }
            Err(_) => Vec::new(),
        }
    };

    let truncated = (children.len() as i32) > limit;
    if children.len() > limit as usize {
        children.truncate(limit as usize);
    }

    Ok(Json(ChildrenResponse {
        success: true,
        parent_refno,
        children,
        truncated,
        error_message: None,
    }))
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

fn try_offline_world_children_count(world_refno: RefnoEnum) -> Option<i32> {
    let children = offline_world_children(world_refno);
    Some(children.len() as i32)
}

fn offline_world_children(parent_refno: RefnoEnum) -> Vec<TreeNodeDto> {
    let tree_dir = TreeIndexManager::with_default_dir(vec![])
        .tree_dir()
        .to_path_buf();
    let mut out = offline_world_children_from_index(parent_refno, &tree_dir);
    if !out.is_empty() {
        return out;
    }
    out = offline_world_children_by_scan(parent_refno, &tree_dir);
    out.sort_by_key(|node| node.refno.refno().0);
    out
}

fn offline_world_children_from_index(
    parent_refno: RefnoEnum,
    tree_dir: &std::path::Path,
) -> Vec<TreeNodeDto> {
    let Ok(dbnum) = TreeIndexManager::resolve_dbnum_for_refno(parent_refno) else {
        return Vec::new();
    };
    let Ok(index) = load_index_with_large_stack(tree_dir, dbnum) else {
        return Vec::new();
    };
    if !index.contains_refno(parent_refno.refno()) {
        return Vec::new();
    }

    let mut child_counts: HashMap<RefU64, i32> = HashMap::new();
    for refno in index.all_refnos() {
        if let Some(meta) = index.node_meta(refno) {
            if meta.owner.0 != 0 {
                *child_counts.entry(meta.owner).or_insert(0) += 1;
            }
        }
    }

    index
        .all_refnos()
        .into_iter()
        .filter_map(|child| {
            let meta = index.node_meta(child)?;
            if meta.owner != parent_refno.refno() {
                return None;
            }
            let noun = db1_dehash(meta.noun);
            Some(TreeNodeDto {
                refno: RefnoEnum::from(child),
                name: RefnoEnum::from(child).to_string(),
                noun: noun.to_string(),
                owner: Some(parent_refno),
                children_count: Some(*child_counts.get(&child).unwrap_or(&0)),
            })
        })
        .collect()
}

fn offline_world_children_by_scan(
    parent_refno: RefnoEnum,
    tree_dir: &std::path::Path,
) -> Vec<TreeNodeDto> {
    let wanted_dbnum = TreeIndexManager::resolve_dbnum_for_refno(parent_refno).ok();
    let mut dbnums: Vec<u32> = Vec::new();
    if let Ok(entries) = fs::read_dir(tree_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            let Some(stem) = name.strip_suffix(".tree") else {
                continue;
            };
            let Ok(dbnum) = stem.parse::<u32>() else {
                continue;
            };
            if wanted_dbnum.is_none() || wanted_dbnum == Some(dbnum) {
                dbnums.push(dbnum);
            }
        }
    }
    dbnums.sort_unstable();
    dbnums.dedup();

    let mut out = Vec::new();
    for dbnum in dbnums {
        let Ok(index) = load_index_with_large_stack(tree_dir, dbnum) else {
            continue;
        };

        let mut child_counts: HashMap<RefU64, i32> = HashMap::new();
        for refno in index.all_refnos() {
            if let Some(meta) = index.node_meta(refno) {
                if meta.owner.0 != 0 {
                    *child_counts.entry(meta.owner).or_insert(0) += 1;
                }
            }
        }

        for refno in index.all_refnos() {
            let Some(meta) = index.node_meta(refno) else {
                continue;
            };
            let noun = db1_dehash(meta.noun);
            if noun != "SITE" {
                continue;
            }
            let owner_noun = index
                .node_meta(meta.owner)
                .map(|owner| db1_dehash(owner.noun))
                .unwrap_or_default();
            if owner_noun != "WORL" {
                continue;
            }

            out.push(TreeNodeDto {
                refno: RefnoEnum::from(meta.refno),
                name: RefnoEnum::from(meta.refno).to_string(),
                noun: noun.to_string(),
                owner: Some(parent_refno),
                children_count: Some(*child_counts.get(&meta.refno).unwrap_or(&0)),
            });
        }
    }

    out
}

async fn get_ancestors(
    Path(refno): Path<RefnoEnum>,
    State(_state): State<E3dTreeApiState>,
) -> Result<Json<AncestorsResponse>, StatusCode> {
    // 层级查询统一走 indextree（TreeIndex）
    let ancestors = match crate::fast_model::query_provider::get_ancestors(refno).await {
        Ok(v) => v,
        Err(e) => {
            return Ok(Json(AncestorsResponse {
                success: false,
                refnos: vec![],
                error_message: Some(format!("query_ancestor_refnos failed: {e}")),
            }));
        }
    };

    Ok(Json(AncestorsResponse {
        success: true,
        refnos: ancestors,
        error_message: None,
    }))
}

async fn get_subtree_refnos(
    Path(root_refno): Path<RefnoEnum>,
    Query(query): Query<SubtreeQuery>,
    State(_state): State<E3dTreeApiState>,
) -> Result<Json<SubtreeRefnosResponse>, StatusCode> {
    let include_self = query.include_self.unwrap_or(true);
    let max_depth = query.max_depth.unwrap_or(64).clamp(0, 256);
    let limit = query.limit.unwrap_or(50_000).clamp(1, 200_000) as usize;

    // 层级查询统一走 indextree（TreeIndex）
    use crate::fast_model::gen_model::tree_index_manager::TreeIndexManager;
    let mut out: Vec<RefnoEnum> = if max_depth <= 0 {
        Vec::new()
    } else {
        let Ok(dbnum) = TreeIndexManager::resolve_dbnum_for_refno(root_refno) else {
            return Ok(Json(SubtreeRefnosResponse {
                success: false,
                refnos: vec![],
                truncated: false,
                error_message: Some("resolve_dbnum_for_refno failed".to_string()),
            }));
        };
        let manager = TreeIndexManager::with_default_dir(vec![dbnum]);
        manager.query_descendants(root_refno, Some(max_depth as usize))
    };

    if include_self {
        out.insert(0, root_refno);
    }

    let truncated = out.len() > limit;
    if out.len() > limit {
        out.truncate(limit);
    }

    Ok(Json(SubtreeRefnosResponse {
        success: true,
        refnos: out,
        truncated,
        error_message: None,
    }))
}

async fn get_visible_insts(
    Path(refno): Path<RefnoEnum>,
    State(state): State<E3dTreeApiState>,
) -> Result<Json<VisibleInstsResponse>, StatusCode> {
    // 1) 先拿“深度可见实例”（可能包含无几何的组节点）
    // 层级查询统一走 indextree（TreeIndex）
    let mut candidates =
        match crate::fast_model::query_compat::query_deep_visible_inst_refnos(refno).await {
            Ok(v) => v,
            Err(e) => {
                return Ok(Json(VisibleInstsResponse {
                    success: false,
                    refno,
                    refnos: vec![],
                    error_message: Some(format!("query_deep_visible_inst_refnos failed: {e}")),
                }));
            }
        };

    // 兼容：如果没有子孙可见节点，至少包含自己
    if candidates.is_empty() {
        candidates.push(refno);
    }

    let bran_hang_load_roots: HashSet<RefnoEnum> = if let Ok(dbnum) =
        crate::fast_model::gen_model::tree_index_manager::TreeIndexManager::resolve_dbnum_for_refno(
            refno,
        ) {
        let manager =
            crate::fast_model::gen_model::tree_index_manager::TreeIndexManager::with_default_dir(
                vec![dbnum],
            );
        candidates
            .iter()
            .copied()
            .filter(|candidate| {
                manager
                    .get_noun(*candidate)
                    .map(|noun| {
                        let noun = noun.trim().to_ascii_uppercase();
                        noun == "BRAN" || noun == "HANG"
                    })
                    .unwrap_or(false)
            })
            .collect()
    } else {
        HashSet::new()
    };

    // 2) 优先用 instances_{dbnum}.json 做“可加载几何”过滤：与前端实际加载数据保持一致。
    //    - 这可以避免 query_deep_visible_inst_refnos 返回“组节点/无几何节点”，导致前端 instances 缺失。
    //    - 若文件不存在，再回退到 inst_relate 的几何实例查询做过滤。
    fn parse_dbno(r: RefnoEnum) -> Option<u32> {
        crate::fast_model::gen_model::tree_index_manager::TreeIndexManager::resolve_dbnum_for_refno(
            r,
        )
        .ok()
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
    // - instances_{dbnum}.json 位于项目输出目录：output/<project_name>/instances/
    // - 历史兼容：也支持旧路径 output/instances/
    // - 文件读取/解析成功时：即使结果为空，也不回退 inst_relate（避免 inst_relate 缺失时接口直接报错）
    let (refnos, file_ok) = if let Some(dbnum) = parse_dbno(refno) {
        let project_name = state.db_manager.db_option.project_name.clone();
        let instances_path_new = std::path::Path::new("output")
            .join(&project_name)
            .join("instances")
            .join(format!("instances_{dbnum}.json"));
        let instances_path_old = std::path::Path::new("output")
            .join("instances")
            .join(format!("instances_{dbnum}.json"));

        let bytes = fs::read(&instances_path_new).or_else(|_| fs::read(&instances_path_old));
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
    // 文件缺失/解析失败：回退到 inst_relate 几何实例过滤。
    let refnos = if file_ok {
        refnos
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
                out
            }
            Err(e) => {
                return Ok(Json(VisibleInstsResponse {
                    success: false,
                    refno,
                    refnos: vec![],
                    error_message: Some(format!("query_geometry_instances failed: {e}")),
                }));
            }
        }
    };

    Ok(Json(VisibleInstsResponse {
        success: true,
        refno,
        refnos,
        error_message: None,
    }))
}

async fn search_nodes(
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
        use crate::fast_model::gen_model::tree_index_manager::TreeIndexManager;
        let order = match TreeIndexManager::resolve_dbnum_for_refno(refno) {
            Ok(dbnum) => {
                let manager = TreeIndexManager::with_default_dir(vec![dbnum]);
                let siblings = manager.query_children(pe.owner);
                siblings.iter().position(|r| *r == refno).unwrap_or(0)
            }
            Err(_) => 0,
        };
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
async fn get_site_nodes(
    Path(site_refno): Path<RefnoEnum>,
    State(_state): State<E3dTreeApiState>,
) -> Result<Json<SiteNodesResponse>, StatusCode> {
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
