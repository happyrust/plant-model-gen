use axum::{
    Router,
    extract::{Path, Query},
    http::StatusCode,
    response::Json,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use aios_core::RefnoEnum;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
struct RoomEntry {
    refno: RefnoEnum,
    display_name: String,
    full_code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RoomTreeNodeId {
    Refno(RefnoEnum),
    Str(String),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RoomTreeNodeDto {
    pub id: RoomTreeNodeId,
    pub name: String,
    pub noun: String,
    pub owner: Option<RoomTreeNodeId>,
    pub children_count: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NodeResponse {
    pub success: bool,
    pub node: Option<RoomTreeNodeDto>,
    pub error_message: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChildrenResponse {
    pub success: bool,
    pub parent_id: RoomTreeNodeId,
    pub children: Vec<RoomTreeNodeDto>,
    pub truncated: bool,
    pub error_message: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AncestorsResponse {
    pub success: bool,
    pub ids: Vec<RoomTreeNodeId>,
    pub error_message: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchRequest {
    pub keyword: String,
    pub limit: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchResponse {
    pub success: bool,
    pub items: Vec<RoomTreeNodeDto>,
    pub error_message: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChildrenQuery {
    pub limit: Option<i32>,
}

const ROOM_ROOT_ID: &str = "room-root";
const ROOM_GROUP_PREFIX: &str = "room-group:";

fn room_root_node() -> RoomTreeNodeDto {
    RoomTreeNodeDto {
        id: RoomTreeNodeId::Str(ROOM_ROOT_ID.to_string()),
        name: "ROOM".to_string(),
        noun: "ROOM_ROOT".to_string(),
        owner: None,
        children_count: None,
    }
}

fn group_node_id(group: &str) -> String {
    format!("{ROOM_GROUP_PREFIX}{group}")
}

fn parse_group_name(id: &str) -> Option<&str> {
    id.strip_prefix(ROOM_GROUP_PREFIX)
}

async fn query_arch_room_groups() -> anyhow::Result<BTreeMap<String, Vec<RoomEntry>>> {
    let rooms_from_relate = aios_core::room::algorithm::query_rooms_from_room_relate().await?;
    let mut map: BTreeMap<String, Vec<RoomEntry>> = BTreeMap::new();

    fn push_room_code(
        map: &mut BTreeMap<String, Vec<RoomEntry>>,
        refno: RefnoEnum,
        room_code: &str,
    ) {
        let split = room_code.split('-').collect::<Vec<_>>();
        if split.len() < 2 {
            return;
        }
        let Some(first) = split.first() else {
            return;
        };
        let Some(last) = split.last() else {
            return;
        };
        let group = if first.len() > 1 {
            first[1..].to_string()
        } else {
            first.to_string()
        };
        map.entry(group)
            .or_default()
            .push(RoomEntry {
                refno,
                display_name: last.to_string(),
                full_code: room_code.to_string(),
            });
    }

    for room in rooms_from_relate {
        let code = room.name;
        push_room_code(&mut map, room.id, &code);
    }

    // 如果 room_panel_relate 为空，则回退到 noun_hierarchy 查询（FRMW/SBFR）
    if map.is_empty() {
        let mut items = aios_core::query_noun_hierarchy("FRMW", Some("-RM"), None).await?;
        if items.is_empty() {
            items = aios_core::query_noun_hierarchy("SBFR", Some("-RM"), None).await?;
        }
        for item in items {
            push_room_code(&mut map, item.id, &item.name);
        }
    }

    for (_, rooms) in map.iter_mut() {
        rooms.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    }
    Ok(map)
}

pub fn create_room_tree_routes() -> Router {
    Router::new()
        .route("/api/room-tree/root", get(get_room_tree_root))
        .route("/api/room-tree/children/{id}", get(get_room_tree_children))
        .route("/api/room-tree/ancestors/{id}", get(get_room_tree_ancestors))
        .route("/api/room-tree/search", post(search_room_tree))
}

async fn get_room_tree_root() -> Result<Json<NodeResponse>, StatusCode> {
    Ok(Json(NodeResponse {
        success: true,
        node: Some(room_root_node()),
        error_message: None,
    }))
}

async fn get_room_tree_children(
    Path(id): Path<String>,
    Query(query): Query<ChildrenQuery>,
) -> Result<Json<ChildrenResponse>, StatusCode> {
    let limit = query.limit.unwrap_or(2000).clamp(1, 20000) as usize;

    if id == ROOM_ROOT_ID {
        let map = match query_arch_room_groups().await {
            Ok(v) => v,
            Err(e) => {
                return Ok(Json(ChildrenResponse {
                    success: false,
                    parent_id: RoomTreeNodeId::Str(id),
                    children: vec![],
                    truncated: false,
                    error_message: Some(format!("query_arch_room_groups failed: {e}")),
                }));
            }
        };

        let mut children = map
            .iter()
            .map(|(g, rooms)| RoomTreeNodeDto {
                id: RoomTreeNodeId::Str(group_node_id(g)),
                name: g.clone(),
                noun: "ROOM_GROUP".to_string(),
                owner: Some(RoomTreeNodeId::Str(ROOM_ROOT_ID.to_string())),
                children_count: Some(rooms.len().min(i32::MAX as usize) as i32),
            })
            .collect::<Vec<_>>();

        let truncated = children.len() > limit;
        if children.len() > limit {
            children.truncate(limit);
        }

        return Ok(Json(ChildrenResponse {
            success: true,
            parent_id: RoomTreeNodeId::Str(ROOM_ROOT_ID.to_string()),
            children,
            truncated,
            error_message: None,
        }));
    }

    if let Some(group) = parse_group_name(&id) {
        let map = match query_arch_room_groups().await {
            Ok(v) => v,
            Err(e) => {
                return Ok(Json(ChildrenResponse {
                    success: false,
                    parent_id: RoomTreeNodeId::Str(id),
                    children: vec![],
                    truncated: false,
                    error_message: Some(format!("query_arch_room_groups failed: {e}")),
                }));
            }
        };

        let rooms = map.get(group).cloned().unwrap_or_default();

        let mut children = rooms
            .into_iter()
            .map(|room| RoomTreeNodeDto {
                id: RoomTreeNodeId::Refno(room.refno),
                name: room.display_name,
                noun: "ROOM".to_string(),
                owner: Some(RoomTreeNodeId::Str(group_node_id(group))),
                children_count: Some(0),
            })
            .collect::<Vec<_>>();

        let truncated = children.len() > limit;
        if children.len() > limit {
            children.truncate(limit);
        }

        return Ok(Json(ChildrenResponse {
            success: true,
            parent_id: RoomTreeNodeId::Str(id),
            children,
            truncated,
            error_message: None,
        }));
    }

    Ok(Json(ChildrenResponse {
        success: false,
        parent_id: RoomTreeNodeId::Str(id.clone()),
        children: vec![],
        truncated: false,
        error_message: Some(format!("unknown node id: {id}")),
    }))
}

async fn get_room_tree_ancestors(Path(id): Path<String>) -> Result<Json<AncestorsResponse>, StatusCode> {
    if id == ROOM_ROOT_ID {
        return Ok(Json(AncestorsResponse {
            success: true,
            ids: vec![RoomTreeNodeId::Str(ROOM_ROOT_ID.to_string())],
            error_message: None,
        }));
    }

    if parse_group_name(&id).is_some() {
        return Ok(Json(AncestorsResponse {
            success: true,
            ids: vec![
                RoomTreeNodeId::Str(id),
                RoomTreeNodeId::Str(ROOM_ROOT_ID.to_string()),
            ],
            error_message: None,
        }));
    }

    // treat as room refno
    let target = RefnoEnum::from(id.as_str());
    if !target.is_valid() {
        return Ok(Json(AncestorsResponse {
            success: false,
            ids: vec![],
            error_message: Some(format!("invalid refno: {id}")),
        }));
    }

    let map = match query_arch_room_groups().await {
        Ok(v) => v,
        Err(e) => {
            return Ok(Json(AncestorsResponse {
                success: false,
                ids: vec![],
                error_message: Some(format!("query_arch_room_groups failed: {e}")),
            }));
        }
    };

    for (group, rooms) in map {
        if rooms.iter().any(|r| r.refno == target) {
            return Ok(Json(AncestorsResponse {
                success: true,
                ids: vec![
                    RoomTreeNodeId::Refno(target),
                    RoomTreeNodeId::Str(group_node_id(&group)),
                    RoomTreeNodeId::Str(ROOM_ROOT_ID.to_string()),
                ],
                error_message: None,
            }));
        }
    }

    Ok(Json(AncestorsResponse {
        success: false,
        ids: vec![],
        error_message: Some("room not found in ARCH groups".to_string()),
    }))
}

async fn search_room_tree(Json(request): Json<SearchRequest>) -> Result<Json<SearchResponse>, StatusCode> {
    let keyword = request.keyword.trim();
    if keyword.is_empty() {
        return Ok(Json(SearchResponse {
            success: true,
            items: vec![],
            error_message: None,
        }));
    }

    let limit = request.limit.unwrap_or(50).clamp(1, 200) as usize;
    let q = keyword.to_lowercase();

    let map = match query_arch_room_groups().await {
        Ok(v) => v,
        Err(e) => {
            return Ok(Json(SearchResponse {
                success: false,
                items: vec![],
                error_message: Some(format!("query_arch_room_groups failed: {e}")),
            }));
        }
    };

    let mut out: Vec<RoomTreeNodeDto> = Vec::new();

    for (group, rooms) in map {
        if out.len() >= limit {
            break;
        }

        let group_lc = group.to_lowercase();
        let group_id = group_node_id(&group);

        for room in rooms {
            if out.len() >= limit {
                break;
            }
            let name_lc = room.display_name.to_lowercase();
            let full_lc = room.full_code.to_lowercase();
            if group_lc.contains(&q) || name_lc.contains(&q) || full_lc.contains(&q) {
                out.push(RoomTreeNodeDto {
                    id: RoomTreeNodeId::Refno(room.refno),
                    name: room.display_name,
                    noun: "ROOM".to_string(),
                    owner: Some(RoomTreeNodeId::Str(group_id.clone())),
                    children_count: Some(0),
                });
            }
        }
    }

    Ok(Json(SearchResponse {
        success: true,
        items: out,
        error_message: None,
    }))
}
