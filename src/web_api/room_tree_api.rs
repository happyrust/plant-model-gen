use axum::{
    Router,
    extract::{Path, Query},
    http::StatusCode,
    response::Json,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use crate::versioned_db::pe_owner_tree::PeOwnerTreeStore;
use aios_core::{RefnoEnum, SurrealQueryExt, project_primary_db};
use std::collections::{BTreeMap, HashSet};
use surrealdb::types::SurrealValue;

use anyhow::anyhow;

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
const COMP_GROUP_PREFIX: &str = "comp-group:";
const ROOM_ITEM_PREFIX: &str = "room-item:";

/// 目标分组类型（owner 链中命中这些 noun 即分入对应组）
const GROUP_NOUNS: &[&str] = &["BRAN", "HANG", "EQUI"];

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

/// 构件分组虚拟节点 ID: comp-group:{room_refno}:{group_key}
fn comp_group_node_id(room_refno: &RefnoEnum, group_key: &str) -> String {
    format!("{COMP_GROUP_PREFIX}{}:{}", room_refno, group_key)
}

/// 解析 COMP_GROUP ID → (room_refno, group_key)
fn parse_comp_group(id: &str) -> Option<(RefnoEnum, String)> {
    let rest = id.strip_prefix(COMP_GROUP_PREFIX)?;
    let colon_pos = rest.rfind(':')?;
    let room_str = &rest[..colon_pos];
    let group_key = &rest[colon_pos + 1..];
    let refno = RefnoEnum::from(room_str);
    if refno.is_valid() {
        Some((refno, group_key.to_string()))
    } else {
        None
    }
}

/// 房间内交付单元放置节点 ID: room-item:{room_refno}:{delivery_refno}
fn room_item_node_id(room_refno: &RefnoEnum, item_refno: &RefnoEnum) -> String {
    format!("{ROOM_ITEM_PREFIX}{}:{}", room_refno, item_refno)
}

fn parse_room_item(id: &str) -> Option<(RefnoEnum, RefnoEnum)> {
    let rest = id.strip_prefix(ROOM_ITEM_PREFIX)?;
    let colon_pos = rest.rfind(':')?;
    let room_refno = RefnoEnum::from(&rest[..colon_pos]);
    let item_refno = RefnoEnum::from(&rest[colon_pos + 1..]);
    if room_refno.is_valid() && item_refno.is_valid() {
        Some((room_refno, item_refno))
    } else {
        None
    }
}

/// 构件信息（含分组 key）
struct RoomComponent {
    refno: RefnoEnum,
    noun: String,
    display_name: String,
    group_key: String,
}

#[derive(Clone)]
struct DeliveryCandidate {
    refno: RefnoEnum,
    noun: String,
    display_name: String,
}

fn delivery_candidate(
    refno: Option<String>,
    noun: Option<String>,
    name: Option<String>,
) -> Option<DeliveryCandidate> {
    let refno = RefnoEnum::from(refno?.as_str());
    if !refno.is_valid() {
        return None;
    }
    let noun = noun.unwrap_or_else(|| "PE".to_string());
    let display_name = name
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| refno.to_string());
    Some(DeliveryCandidate {
        refno,
        noun,
        display_name,
    })
}

fn build_room_component(candidates: Vec<DeliveryCandidate>) -> Option<RoomComponent> {
    let fallback = candidates.first()?.clone();
    let delivery = candidates
        .iter()
        .find(|c| GROUP_NOUNS.contains(&c.noun.to_uppercase().as_str()))
        .cloned()
        .unwrap_or(fallback);

    let upper = delivery.noun.to_uppercase();
    let group_key = if GROUP_NOUNS.contains(&upper.as_str()) {
        upper
    } else {
        "OTHER".to_string()
    };

    Some(RoomComponent {
        refno: delivery.refno,
        noun: delivery.noun,
        display_name: delivery.display_name,
        group_key,
    })
}

/// 根据 room refno 在分组表中查找其 display_name（如 "R301"）。
async fn find_room_display_name(room_refno: &RefnoEnum) -> anyhow::Result<String> {
    let map = query_arch_room_groups()
        .await
        .map_err(|e| anyhow!("query_arch_room_groups failed: {e}"))?;
    for (_group, rooms) in &map {
        if let Some(entry) = rooms.iter().find(|r| r.refno == *room_refno) {
            return Ok(entry.display_name.clone());
        }
    }
    Err(anyhow!("room refno not found in groups: {room_refno}"))
}

fn room_relate_room_filter(room_num: &str, room_refno: &RefnoEnum) -> String {
    format!(
        "room_num = '{}' \
         AND (in = pe:⟨{}⟩ OR in.owner = pe:⟨{}⟩ OR in.owner.owner = pe:⟨{}⟩ OR in.owner.owner.owner = pe:⟨{}⟩)",
        room_num, room_refno, room_refno, room_refno, room_refno
    )
}

fn group_nouns_sql() -> String {
    GROUP_NOUNS
        .iter()
        .map(|noun| format!("'{noun}'"))
        .collect::<Vec<_>>()
        .join(",")
}

/// 查询某房间在 room_relate 中关联的最小交付单元列表。
///
/// 先在 SQL 侧按最近的 BRAN/HANG/EQUI owner 层级折叠；没有命中交付单元的记录
/// 再落回 raw out，归入 OTHER，避免把几万条 raw room_relate 行拉回 Rust 后再去重。
async fn query_room_components(room_refno: &RefnoEnum) -> anyhow::Result<Vec<RoomComponent>> {
    #[derive(Debug, Deserialize, SurrealValue)]
    struct Row {
        refno: String,
        noun: Option<String>,
        name: Option<String>,
    }

    let room_num = find_room_display_name(room_refno).await?;
    let escaped_room_num = room_num.replace('\'', "''");
    let room_filter = room_relate_room_filter(&escaped_room_num, room_refno);
    let group_nouns = format!("[{}]", group_nouns_sql());
    let sql = format!(
        "LET $items = array::distinct(array::flatten([\
            (SELECT VALUE out FROM room_relate WHERE {room_filter} AND out.noun IN {group_nouns} GROUP BY out), \
            (SELECT VALUE out.owner FROM room_relate WHERE {room_filter} AND out.noun NOT IN {group_nouns} AND out.owner.noun IN {group_nouns} GROUP BY out.owner), \
            (SELECT VALUE out.owner.owner FROM room_relate WHERE {room_filter} AND out.noun NOT IN {group_nouns} AND out.owner.noun NOT IN {group_nouns} AND out.owner.owner.noun IN {group_nouns} GROUP BY out.owner.owner), \
            (SELECT VALUE out.owner.owner.owner FROM room_relate WHERE {room_filter} AND out.noun NOT IN {group_nouns} AND out.owner.noun NOT IN {group_nouns} AND out.owner.owner.noun NOT IN {group_nouns} AND out.owner.owner.owner.noun IN {group_nouns} GROUP BY out.owner.owner.owner), \
            (SELECT VALUE out.owner.owner.owner.owner FROM room_relate WHERE {room_filter} AND out.noun NOT IN {group_nouns} AND out.owner.noun NOT IN {group_nouns} AND out.owner.owner.noun NOT IN {group_nouns} AND out.owner.owner.owner.noun NOT IN {group_nouns} AND out.owner.owner.owner.owner.noun IN {group_nouns} GROUP BY out.owner.owner.owner.owner), \
            (SELECT VALUE out.owner.owner.owner.owner.owner FROM room_relate WHERE {room_filter} AND out.noun NOT IN {group_nouns} AND out.owner.noun NOT IN {group_nouns} AND out.owner.owner.noun NOT IN {group_nouns} AND out.owner.owner.owner.noun NOT IN {group_nouns} AND out.owner.owner.owner.owner.noun NOT IN {group_nouns} AND out.owner.owner.owner.owner.owner.noun IN {group_nouns} GROUP BY out.owner.owner.owner.owner.owner), \
            (SELECT VALUE out FROM room_relate WHERE {room_filter} AND out.noun NOT IN {group_nouns} AND out.owner.noun NOT IN {group_nouns} AND out.owner.owner.noun NOT IN {group_nouns} AND out.owner.owner.owner.noun NOT IN {group_nouns} AND out.owner.owner.owner.owner.noun NOT IN {group_nouns} AND out.owner.owner.owner.owner.owner.noun NOT IN {group_nouns} GROUP BY out)\
         ])); \
         SELECT record::id(id) AS refno, noun AS noun, fn::default_full_name(id) AS name FROM $items"
    );
    let rows: Vec<Row> = project_primary_db().query_take(&sql, 1).await?;

    let mut seen = HashSet::new();
    let mut out = rows
        .into_iter()
        .filter_map(|r| {
            let candidate = delivery_candidate(Some(r.refno), r.noun, r.name)?;
            build_room_component(vec![candidate])
        })
        .filter(|c| seen.insert(c.refno))
        .collect::<Vec<_>>();
    out.sort_by(|a, b| {
        a.group_key
            .cmp(&b.group_key)
            .then(a.display_name.cmp(&b.display_name))
            .then(a.refno.cmp(&b.refno))
    });
    Ok(out)
}

async fn model_children(parent_refno: RefnoEnum) -> Vec<RefnoEnum> {
    PeOwnerTreeStore::query_children(parent_refno)
        .await
        .unwrap_or_default()
}

async fn model_children_count(parent_refno: RefnoEnum) -> i32 {
    PeOwnerTreeStore::query_children_counts(&[parent_refno])
        .await
        .ok()
        .and_then(|counts| counts.get(&parent_refno).copied())
        .unwrap_or(0)
        .min(i32::MAX as usize) as i32
}

async fn query_room_item_children(
    room_refno: RefnoEnum,
    parent_refno: RefnoEnum,
    parent_id: &str,
) -> anyhow::Result<Vec<RoomTreeNodeDto>> {
    let child_refnos = PeOwnerTreeStore::query_children(parent_refno).await?;
    if child_refnos.is_empty() {
        return Ok(Vec::new());
    }
    let metas = PeOwnerTreeStore::fetch_node_metas(&child_refnos).await?;
    let counts = PeOwnerTreeStore::query_children_counts(&child_refnos).await?;
    let mut out = Vec::with_capacity(child_refnos.len());
    for (idx, child_refno) in child_refnos.into_iter().enumerate() {
        let noun = metas
            .get(&child_refno)
            .map(|m| m.noun.clone())
            .unwrap_or_default();
        let mut name = crate::fast_model::query_provider::get_pe(child_refno)
            .await
            .ok()
            .flatten()
            .map(|pe| pe.name)
            .unwrap_or_default();
        if name.trim().is_empty() {
            name = format!("{} {}", noun, idx + 1);
        }
        out.push(RoomTreeNodeDto {
            id: RoomTreeNodeId::Str(room_item_node_id(&room_refno, &child_refno)),
            name,
            noun,
            owner: Some(RoomTreeNodeId::Str(parent_id.to_string())),
            children_count: Some(
                counts
                    .get(&child_refno)
                    .copied()
                    .unwrap_or(0)
                    .min(i32::MAX as usize) as i32,
            ),
        });
    }
    Ok(out)
}

async fn model_self_and_ancestors(refno: RefnoEnum) -> Vec<RefnoEnum> {
    let mut out = Vec::new();
    let mut cur = refno;

    for _ in 0..64 {
        if !cur.is_valid() || out.contains(&cur) {
            break;
        }
        out.push(cur);

        let Ok(Some(pe)) = crate::fast_model::query_provider::get_pe(cur).await else {
            break;
        };
        if !pe.owner.is_valid() || pe.owner == cur {
            break;
        }
        cur = pe.owner;
    }

    out
}

async fn room_item_ancestor_ids(
    room_refno: RefnoEnum,
    item_refno: RefnoEnum,
) -> anyhow::Result<Vec<RoomTreeNodeId>> {
    let components = query_room_components(&room_refno).await?;
    let chain = model_self_and_ancestors(item_refno).await;

    let Some((component, component_index)) = chain.iter().enumerate().find_map(|(idx, refno)| {
        components
            .iter()
            .find(|c| c.refno == *refno)
            .map(|component| (component, idx))
    }) else {
        return Err(anyhow!(
            "room item not found in room tree: {room_refno}:{item_refno}"
        ));
    };

    let map = query_arch_room_groups()
        .await
        .map_err(|e| anyhow!("query_arch_room_groups failed: {e}"))?;
    let Some(group) = map
        .iter()
        .find_map(|(group, rooms)| rooms.iter().any(|r| r.refno == room_refno).then_some(group))
    else {
        return Err(anyhow!("room refno not found in groups: {room_refno}"));
    };

    let mut ids = chain[..=component_index]
        .iter()
        .map(|refno| RoomTreeNodeId::Str(room_item_node_id(&room_refno, refno)))
        .collect::<Vec<_>>();
    ids.push(RoomTreeNodeId::Str(comp_group_node_id(
        &room_refno,
        &component.group_key,
    )));
    ids.push(RoomTreeNodeId::Refno(room_refno));
    ids.push(RoomTreeNodeId::Str(group_node_id(group)));
    ids.push(RoomTreeNodeId::Str(ROOM_ROOT_ID.to_string()));
    Ok(ids)
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
        map.entry(group).or_default().push(RoomEntry {
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
        .route(
            "/api/room-tree/ancestors/{id}",
            get(get_room_tree_ancestors),
        )
        .route("/api/room-tree/search", post(search_room_tree))
}

/// 不经 HTTP 层的核心逻辑：查询指定节点的子节点。
///
/// 目的：便于 example/集成脚本复用相同逻辑，不必依赖 axum Router 测试工具链。
pub async fn room_tree_children_core(id: &str, limit: usize) -> anyhow::Result<ChildrenResponse> {
    let limit = limit.clamp(1, 20000);

    if id == ROOM_ROOT_ID {
        let map = query_arch_room_groups()
            .await
            .map_err(|e| anyhow!("query_arch_room_groups failed: {e}"))?;

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

        return Ok(ChildrenResponse {
            success: true,
            parent_id: RoomTreeNodeId::Str(ROOM_ROOT_ID.to_string()),
            children,
            truncated,
            error_message: None,
        });
    }

    if let Some(group) = parse_group_name(id) {
        let map = query_arch_room_groups()
            .await
            .map_err(|e| anyhow!("query_arch_room_groups failed: {e}"))?;

        let rooms = map.get(group).cloned().unwrap_or_default();

        let mut children = Vec::with_capacity(rooms.len());
        for room in rooms {
            children.push(RoomTreeNodeDto {
                id: RoomTreeNodeId::Refno(room.refno),
                name: room.display_name,
                noun: "ROOM".to_string(),
                owner: Some(RoomTreeNodeId::Str(group_node_id(group))),
                // ponytail: exact counts are expensive here; room expansion computes real groups.
                children_count: Some(1),
            });
        }

        let truncated = children.len() > limit;
        if children.len() > limit {
            children.truncate(limit);
        }

        return Ok(ChildrenResponse {
            success: true,
            parent_id: RoomTreeNodeId::Str(id.to_string()),
            children,
            truncated,
            error_message: None,
        });
    }

    // ── 展开 COMP_GROUP（构件分组虚拟节点）→ 返回该组下的构件列表 ──
    if let Some((room_refno, group_key)) = parse_comp_group(id) {
        let components = query_room_components(&room_refno)
            .await
            .map_err(|e| anyhow!("query_room_components failed: {e}"))?;

        let group_components: Vec<_> = components
            .into_iter()
            .filter(|c| c.group_key == group_key)
            .collect();
        let mut children: Vec<RoomTreeNodeDto> = Vec::with_capacity(group_components.len());
        for c in group_components {
            let children_count = model_children_count(c.refno).await;
            children.push(RoomTreeNodeDto {
                id: RoomTreeNodeId::Str(room_item_node_id(&room_refno, &c.refno)),
                name: c.display_name,
                noun: c.noun,
                owner: Some(RoomTreeNodeId::Str(id.to_string())),
                children_count: Some(children_count),
            });
        }

        let truncated = children.len() > limit;
        if children.len() > limit {
            children.truncate(limit);
        }

        return Ok(ChildrenResponse {
            success: true,
            parent_id: RoomTreeNodeId::Str(id.to_string()),
            children,
            truncated,
            error_message: None,
        });
    }

    // ── 展开 ROOM_ITEM（房间内的 E3D 节点包装）→ 返回真实 E3D 子节点的房间包装节点 ──
    if let Some((room_refno, item_refno)) = parse_room_item(id) {
        let mut children = query_room_item_children(room_refno, item_refno, id).await?;

        let truncated = children.len() > limit;
        if children.len() > limit {
            children.truncate(limit);
        }

        return Ok(ChildrenResponse {
            success: true,
            parent_id: RoomTreeNodeId::Str(id.to_string()),
            children,
            truncated,
            error_message: None,
        });
    }

    // ── 展开 ROOM → 返回 COMP_GROUP 分组节点 ──
    let target = RefnoEnum::from(id);
    if target.is_valid() {
        // 查询构件列表并按 group_key 统计
        let components = query_room_components(&target)
            .await
            .map_err(|e| anyhow!("query_room_components failed: {e}"))?;

        let mut group_counts: BTreeMap<String, usize> = BTreeMap::new();
        for c in &components {
            *group_counts.entry(c.group_key.clone()).or_default() += 1;
        }

        let mut children: Vec<RoomTreeNodeDto> = group_counts
            .into_iter()
            .map(|(gk, cnt)| RoomTreeNodeDto {
                id: RoomTreeNodeId::Str(comp_group_node_id(&target, &gk)),
                name: gk.clone(),
                noun: "COMP_GROUP".to_string(),
                owner: Some(RoomTreeNodeId::Refno(target)),
                children_count: Some(cnt as i32),
            })
            .collect();

        let truncated = children.len() > limit;
        if children.len() > limit {
            children.truncate(limit);
        }

        return Ok(ChildrenResponse {
            success: true,
            parent_id: RoomTreeNodeId::Refno(target),
            children,
            truncated,
            error_message: None,
        });
    }

    Err(anyhow!("unknown node id: {id}"))
}

/// 不经 HTTP 层的核心逻辑：查询指定节点的祖先链。
pub async fn room_tree_ancestors_core(id: &str) -> anyhow::Result<AncestorsResponse> {
    if id == ROOM_ROOT_ID {
        return Ok(AncestorsResponse {
            success: true,
            ids: vec![RoomTreeNodeId::Str(ROOM_ROOT_ID.to_string())],
            error_message: None,
        });
    }

    if parse_group_name(id).is_some() {
        return Ok(AncestorsResponse {
            success: true,
            ids: vec![
                RoomTreeNodeId::Str(id.to_string()),
                RoomTreeNodeId::Str(ROOM_ROOT_ID.to_string()),
            ],
            error_message: None,
        });
    }

    if let Some((room_refno, item_refno)) = parse_room_item(id) {
        return Ok(AncestorsResponse {
            success: true,
            ids: room_item_ancestor_ids(room_refno, item_refno).await?,
            error_message: None,
        });
    }

    // treat as room refno or component refno
    let target = RefnoEnum::from(id);
    if !target.is_valid() {
        return Err(anyhow!("invalid refno: {id}"));
    }

    let map = query_arch_room_groups()
        .await
        .map_err(|e| anyhow!("query_arch_room_groups failed: {e}"))?;

    // 先检查是否为 ROOM refno
    for (group, rooms) in &map {
        if rooms.iter().any(|r| r.refno == target) {
            return Ok(AncestorsResponse {
                success: true,
                ids: vec![
                    RoomTreeNodeId::Refno(target),
                    RoomTreeNodeId::Str(group_node_id(group)),
                    RoomTreeNodeId::Str(ROOM_ROOT_ID.to_string()),
                ],
                error_message: None,
            });
        }
    }

    // P0: 检查是否为构件 refno（room_relate 的 out 端）
    // 查询该构件所属的 room_num 和 owner noun，反查房间 refno、group 和 comp_group
    #[derive(Debug, Deserialize, SurrealValue)]
    struct RoomNumRow {
        room_num: String,
        in_refno: Option<String>,
        in_o1_refno: Option<String>,
        in_o2_refno: Option<String>,
        in_o3_refno: Option<String>,
        refno: String,
        noun: Option<String>,
        name: Option<String>,
        o1_refno: Option<String>,
        o1_noun: Option<String>,
        o1_name: Option<String>,
        o2_refno: Option<String>,
        o2_noun: Option<String>,
        o2_name: Option<String>,
        o3_refno: Option<String>,
        o3_noun: Option<String>,
        o3_name: Option<String>,
        o4_refno: Option<String>,
        o4_noun: Option<String>,
        o4_name: Option<String>,
        o5_refno: Option<String>,
        o5_noun: Option<String>,
        o5_name: Option<String>,
    }
    let sql = format!(
        "SELECT room_num, record::id(in) AS in_refno, record::id(in.owner) AS in_o1_refno, record::id(in.owner.owner) AS in_o2_refno, record::id(in.owner.owner.owner) AS in_o3_refno, \
         record::id(out) AS refno, out.noun AS noun, fn::default_full_name(out) AS name, \
         record::id(out.owner) AS o1_refno, out.owner.noun AS o1_noun, fn::default_full_name(out.owner) AS o1_name, \
         record::id(out.owner.owner) AS o2_refno, out.owner.owner.noun AS o2_noun, fn::default_full_name(out.owner.owner) AS o2_name, \
         record::id(out.owner.owner.owner) AS o3_refno, out.owner.owner.owner.noun AS o3_noun, fn::default_full_name(out.owner.owner.owner) AS o3_name, \
         record::id(out.owner.owner.owner.owner) AS o4_refno, out.owner.owner.owner.owner.noun AS o4_noun, fn::default_full_name(out.owner.owner.owner.owner) AS o4_name, \
         record::id(out.owner.owner.owner.owner.owner) AS o5_refno, out.owner.owner.owner.owner.owner.noun AS o5_noun, fn::default_full_name(out.owner.owner.owner.owner.owner) AS o5_name \
         FROM room_relate \
         WHERE out = pe:⟨{}⟩ \
            OR out.owner = pe:⟨{}⟩ \
            OR out.owner.owner = pe:⟨{}⟩ \
            OR out.owner.owner.owner = pe:⟨{}⟩ \
            OR out.owner.owner.owner.owner = pe:⟨{}⟩ \
            OR out.owner.owner.owner.owner.owner = pe:⟨{}⟩ \
         LIMIT 1",
        target, target, target, target, target, target
    );
    let rows: Vec<RoomNumRow> = project_primary_db()
        .query_take(&sql, 0)
        .await
        .unwrap_or_default();

    if let Some(row) = rows.first() {
        let candidates = [
            delivery_candidate(Some(row.refno.clone()), row.noun.clone(), row.name.clone()),
            delivery_candidate(
                row.o1_refno.clone(),
                row.o1_noun.clone(),
                row.o1_name.clone(),
            ),
            delivery_candidate(
                row.o2_refno.clone(),
                row.o2_noun.clone(),
                row.o2_name.clone(),
            ),
            delivery_candidate(
                row.o3_refno.clone(),
                row.o3_noun.clone(),
                row.o3_name.clone(),
            ),
            delivery_candidate(
                row.o4_refno.clone(),
                row.o4_noun.clone(),
                row.o4_name.clone(),
            ),
            delivery_candidate(
                row.o5_refno.clone(),
                row.o5_noun.clone(),
                row.o5_name.clone(),
            ),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        let Some(component) = build_room_component(candidates) else {
            return Err(anyhow!("refno not found in room tree: {id}"));
        };
        let room_candidates = [
            row.in_refno.as_deref(),
            row.in_o1_refno.as_deref(),
            row.in_o2_refno.as_deref(),
            row.in_o3_refno.as_deref(),
        ]
        .into_iter()
        .flatten()
        .map(RefnoEnum::from)
        .filter(|r| r.is_valid())
        .collect::<Vec<_>>();

        // 从 room_relate.in 的 owner 链中找到房间树节点，避免同名房间串线。
        for (group, rooms) in &map {
            if let Some(room_entry) = rooms.iter().find(|r| {
                r.display_name == row.room_num && room_candidates.iter().any(|c| *c == r.refno)
            }) {
                let mut ids = model_self_and_ancestors(target)
                    .await
                    .into_iter()
                    .take_while(|refno| *refno != component.refno)
                    .map(|refno| RoomTreeNodeId::Str(room_item_node_id(&room_entry.refno, &refno)))
                    .collect::<Vec<_>>();
                ids.push(RoomTreeNodeId::Str(room_item_node_id(
                    &room_entry.refno,
                    &component.refno,
                )));
                ids.push(RoomTreeNodeId::Str(comp_group_node_id(
                    &room_entry.refno,
                    &component.group_key,
                )));
                ids.push(RoomTreeNodeId::Refno(room_entry.refno));
                ids.push(RoomTreeNodeId::Str(group_node_id(group)));
                ids.push(RoomTreeNodeId::Str(ROOM_ROOT_ID.to_string()));

                return Ok(AncestorsResponse {
                    success: true,
                    ids,
                    error_message: None,
                });
            }
        }
    }

    Err(anyhow!("refno not found in room tree: {id}"))
}

/// 不经 HTTP 层的核心逻辑：按 keyword 搜索房间树（仅返回 ROOM 节点）。
pub async fn room_tree_search_core(keyword: &str, limit: usize) -> anyhow::Result<SearchResponse> {
    let keyword = keyword.trim();
    if keyword.is_empty() {
        return Ok(SearchResponse {
            success: true,
            items: vec![],
            error_message: None,
        });
    }

    let limit = limit.clamp(1, 200) as usize;
    let q = keyword.to_lowercase();

    let map = query_arch_room_groups()
        .await
        .map_err(|e| anyhow!("query_arch_room_groups failed: {e}"))?;

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
                    children_count: Some(1),
                });
            }
        }
    }

    Ok(SearchResponse {
        success: true,
        items: out,
        error_message: None,
    })
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

    match room_tree_children_core(&id, limit).await {
        Ok(resp) => Ok(Json(resp)),
        Err(e) => Ok(Json(ChildrenResponse {
            success: false,
            parent_id: RoomTreeNodeId::Str(id),
            children: vec![],
            truncated: false,
            error_message: Some(e.to_string()),
        })),
    }
}

async fn get_room_tree_ancestors(
    Path(id): Path<String>,
) -> Result<Json<AncestorsResponse>, StatusCode> {
    match room_tree_ancestors_core(&id).await {
        Ok(resp) => Ok(Json(resp)),
        Err(e) => Ok(Json(AncestorsResponse {
            success: false,
            ids: vec![],
            error_message: Some(e.to_string()),
        })),
    }
}

async fn search_room_tree(
    Json(request): Json<SearchRequest>,
) -> Result<Json<SearchResponse>, StatusCode> {
    let keyword = request.keyword;
    let limit = request.limit.unwrap_or(50).clamp(1, 200) as usize;

    match room_tree_search_core(&keyword, limit).await {
        Ok(resp) => Ok(Json(resp)),
        Err(e) => Ok(Json(SearchResponse {
            success: false,
            items: vec![],
            error_message: Some(e.to_string()),
        })),
    }
}
