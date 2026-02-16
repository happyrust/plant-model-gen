//! Scene Tree 初始化逻辑
//!
//! 从 WORLD 节点开始构建整棵场景树

use aios_core::pdms_types::{
    BRAN_COMPONENT_NOUN_NAMES, GNERAL_LOOP_OWNER_NOUN_NAMES, GNERAL_PRIM_NOUN_NAMES,
    USE_CATE_NOUN_NAMES,
};
use aios_core::tool::db_tool::db1_dehash;
use aios_core::tree_query::{TreeQuery, TreeQueryFilter};
use aios_core::{RefnoEnum, RefU64, SUL_DB, SurrealQueryExt};
use anyhow::Result;
use std::collections::VecDeque;

use crate::fast_model::gen_model::tree_index_manager::TreeIndexManager;

/// 从 DbOption.toml 读取 project_name
fn get_project_name_from_config() -> String {
    let content = std::fs::read_to_string("DbOption.toml").unwrap_or_default();
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("project_name") {
            if let Some(value) = line.split('=').nth(1) {
                return value.trim().trim_matches('"').trim_matches('\'').to_string();
            }
        }
    }
    panic!("DbOption.toml 中未找到 project_name 配置");
}

/// 初始化结果
#[derive(Debug)]
pub struct SceneTreeInitResult {
    pub node_count: usize,
    pub relation_count: usize,
    pub duration_ms: u128,
}

/// Scene Node 数据
#[derive(Debug)]
struct SceneNodeData {
    id: i64,
    parent: Option<i64>,
    has_geo: bool,
    is_leaf: bool,
    dbnum: i16,
    geo_type: Option<String>,  // 几何类型：Pos, Neg, CataNeg, CataCrossNeg 等
}

/// 判断是否为几何节点
pub(crate) fn is_geo_noun(noun: &str) -> bool {
    let noun_upper = noun.to_uppercase();
    let noun_str = noun_upper.as_str();

    USE_CATE_NOUN_NAMES.contains(&noun_str)
        || GNERAL_LOOP_OWNER_NOUN_NAMES.contains(&noun_str)
        || GNERAL_PRIM_NOUN_NAMES.contains(&noun_str)
        || BRAN_COMPONENT_NOUN_NAMES.contains(&noun_str)
        || noun_str == "BRAN"
        || noun_str == "HANG"
}

/// 初始化 Scene Tree（从 WORLD 开始）
pub async fn init_scene_tree(mdb_name: &str, force_rebuild: bool) -> Result<SceneTreeInitResult> {
    let start = std::time::Instant::now();

    // 1. 初始化 Schema
    super::schema::init_schema().await?;

    // 2. 可选重建：清理旧数据
    if force_rebuild {
        cleanup_existing_scene_tree().await?;
    }

    // 3. 获取 WORLD 节点
    let world = aios_core::mdb::get_world_refno(mdb_name.to_string()).await?;
    let world_refno = world.refno();

    println!("[scene_tree] 从 WORLD {} 开始构建", world_refno);

    // 4. 从 WORLD 开始构建整棵树
    let (nodes, relations) = build_tree_from_world(world_refno.into()).await?;

    // 5. 批量写入
    let node_count = nodes.len();
    let relation_count = relations.len();

    batch_insert_nodes(&nodes).await?;
    batch_insert_relations(&relations).await?;

    let duration_ms = start.elapsed().as_millis();
    println!(
        "[scene_tree] 初始化完成: {} 节点, {} 关系, {} ms",
        node_count, relation_count, duration_ms
    );

    Ok(SceneTreeInitResult {
        node_count,
        relation_count,
        duration_ms,
    })
}

/// 初始化 Scene Tree（从指定 root refno 开始构建子树）
///
/// 用于测试/按需构建子树，避免从 WORLD 全量构建耗时过长。
pub async fn init_scene_tree_from_root(
    root_refno: RefnoEnum,
    force_rebuild: bool,
) -> Result<SceneTreeInitResult> {
    let start = std::time::Instant::now();

    super::schema::init_schema().await?;

    if force_rebuild {
        cleanup_existing_scene_tree().await?;
    }

    println!("[scene_tree] 从 ROOT {} 开始构建", root_refno);

    let (nodes, relations) = build_tree_from_world(root_refno).await?;
    let node_count = nodes.len();
    let relation_count = relations.len();

    batch_insert_nodes(&nodes).await?;
    batch_insert_relations(&relations).await?;

    let duration_ms = start.elapsed().as_millis();
    println!(
        "[scene_tree] 初始化完成(root={}): {} 节点, {} 关系, {} ms",
        root_refno, node_count, relation_count, duration_ms
    );

    // 6. 导出 Parquet 文件（使用 root 的 dbnum）
    let dbnum = TreeIndexManager::resolve_dbnum_for_refno(root_refno)?;
    let output_dir = crate::versioned_db::db_meta_info::get_project_tree_dir(&get_project_name_from_config());
    if let Err(e) = super::parquet_export::export_scene_tree_parquet(dbnum, &output_dir).await {
        eprintln!("[scene_tree] Parquet 导出失败: {}", e);
    }

    Ok(SceneTreeInitResult {
        node_count,
        relation_count,
        duration_ms,
    })
}

/// 初始化 Scene Tree（按 dbnum，WORLD 固定为 `${dbnum}_0`）
///
/// 适用场景：
/// - 测试/工具按 dbnum 初始化
/// - 数据库里缺失 MDB name→dbnum 映射时仍可构建
pub async fn init_scene_tree_by_dbno(dbnum: u32, force_rebuild: bool) -> Result<SceneTreeInitResult> {
    let start = std::time::Instant::now();

    super::schema::init_schema().await?;

    if force_rebuild {
        cleanup_scene_tree_by_dbno(dbnum).await?;
    }

    let world_refno = RefnoEnum::from(RefU64((dbnum as u64) << 32));
    println!("[scene_tree] 从 WORLD {} 开始构建 (dbnum={})", world_refno, dbnum);

    let (nodes, relations) = build_tree_from_world(world_refno).await?;
    let node_count = nodes.len();
    let relation_count = relations.len();

    batch_insert_nodes(&nodes).await?;
    batch_insert_relations(&relations).await?;

    let duration_ms = start.elapsed().as_millis();
    println!(
        "[scene_tree] 初始化完成(dbnum={}): {} 节点, {} 关系, {} ms",
        dbnum, node_count, relation_count, duration_ms
    );

    // 6. 导出 Parquet 文件
    let output_dir = crate::versioned_db::db_meta_info::get_project_tree_dir(&get_project_name_from_config());
    if let Err(e) = super::parquet_export::export_scene_tree_parquet(dbnum, &output_dir).await {
        eprintln!("[scene_tree] Parquet 导出失败: {}", e);
    }

    Ok(SceneTreeInitResult {
        node_count,
        relation_count,
        duration_ms,
    })
}

/// 清理已存在的 scene_tree 数据（用于 force_rebuild）
async fn cleanup_existing_scene_tree() -> Result<()> {
    let sql = r#"
DELETE contains;
DELETE scene_node;
"#;
    SUL_DB.query_response(sql).await?;
    println!("[scene_tree] 已清理旧的 scene_node/contains 数据");
    Ok(())
}

/// 仅清理指定 dbnum 的 scene_tree 数据（用于按库重建/测试）
async fn cleanup_scene_tree_by_dbno(dbnum: u32) -> Result<()> {
    let sql = format!(
        r#"
DELETE contains WHERE in.dbnum = {dbnum} OR out.dbnum = {dbnum};
DELETE scene_node WHERE dbnum = {dbnum};
"#
    );
    SUL_DB.query_response(&sql).await?;
    println!("[scene_tree] 已清理 dbnum={} 的 scene_node/contains 数据", dbnum);
    Ok(())
}

/// 从 WORLD 开始 BFS 构建树
async fn build_tree_from_world(
    world_refno: RefnoEnum,
) -> Result<(Vec<SceneNodeData>, Vec<(i64, i64)>)> {
    let mut nodes = Vec::new();
    let mut relations = Vec::new();
    let mut queue = VecDeque::new();

    // 层级查询统一走 TreeIndex（indextree），避免依赖 SurrealDB 的 pe_owner 递归查询。
    let dbnum_u32 = TreeIndexManager::resolve_dbnum_for_refno(world_refno)?;
    let manager = TreeIndexManager::with_default_dir(vec![dbnum_u32]);
    let index = manager.load_index(dbnum_u32)?;

    queue.push_back((world_refno, None::<i64>));

    while let Some((refno, parent_id)) = queue.pop_front() {
        // 1. 获取节点信息
        let child_u64s = index
            .query_children(refno.refno(), TreeQueryFilter::default())
            .await
            .unwrap_or_default();
        let refno_i64 = refno.refno().0 as i64;
        let dbnum = dbnum_u32 as i16;

        // 2. 获取当前节点的 noun
        let noun = index
            .node_meta(refno.refno())
            .map(|m| db1_dehash(m.noun))
            .unwrap_or_default();
        let has_geo = is_geo_noun(&noun);
        let is_leaf = child_u64s.is_empty();

        // 3. 获取几何类型（仅对几何节点查询）
        let geo_type = if has_geo {
            get_geo_type_by_refno(refno).await.unwrap_or(None)
        } else {
            None
        };

        // 4. 收集节点
        nodes.push(SceneNodeData {
            id: refno_i64,
            parent: parent_id,
            has_geo,
            is_leaf,
            dbnum,
            geo_type,
        });

        // 5. 收集关系
        if let Some(pid) = parent_id {
            relations.push((pid, refno_i64));
        }

        // 5. 将子节点加入队列
        for child in child_u64s {
            queue.push_back((RefnoEnum::from(child), Some(refno_i64)));
        }
    }

    Ok((nodes, relations))
}

/// 获取节点的几何类型（从 geo_relate 表查询）
async fn get_geo_type_by_refno(refno: RefnoEnum) -> Result<Option<String>> {
    let sql = format!(
        "SELECT VALUE geo_type FROM geo_relate:{}",
        refno.refno().0
    );
    let result: Vec<String> = SUL_DB.query_take(&sql, 0).await?;
    Ok(result.into_iter().next())
}

/// 批量写入节点
async fn batch_insert_nodes(nodes: &[SceneNodeData]) -> Result<()> {
    const CHUNK_SIZE: usize = 200;

    for chunk in nodes.chunks(CHUNK_SIZE) {
        let mut sql = String::new();
        for node in chunk {
            let parent_str = match node.parent {
                Some(p) => p.to_string(),
                None => "NONE".to_string(),
            };
            let geo_type_str = match &node.geo_type {
                Some(t) => format!("'{}'", t),
                None => "NONE".to_string(),
            };
            sql.push_str(&format!(
                "INSERT INTO scene_node {{ id: scene_node:{}, parent: {}, has_geo: {}, is_leaf: {}, generated: false, dbnum: {}, geo_type: {} }};",
                node.id, parent_str, node.has_geo, node.is_leaf, node.dbnum, geo_type_str
            ));
        }
        if !sql.is_empty() {
            SUL_DB.query_response(&sql).await?;
        }
    }
    Ok(())
}

/// 批量写入关系
async fn batch_insert_relations(relations: &[(i64, i64)]) -> Result<()> {
    const CHUNK_SIZE: usize = 200;

    for chunk in relations.chunks(CHUNK_SIZE) {
        let mut sql = String::new();
        for (parent_id, child_id) in chunk {
            sql.push_str(&format!(
                "RELATE scene_node:{}->contains:[{},{}]->scene_node:{};",
                parent_id, parent_id, child_id, child_id
            ));
        }
        if !sql.is_empty() {
            SUL_DB.query_response(&sql).await?;
        }
    }
    Ok(())
}
