//! 房间树 + ZONE 快速测试闭环（缓存生成路径）
//!
//! 目标：
//! 1) 仅生成指定 ZONE（默认 24381/146882）下的模型（走 foyer cache，不写 SurrealDB）
//! 2) 执行房间计算（写入 room_relate）
//! 3) 调用 room-tree 核心逻辑验证 / 打印输出
//!
//! 运行示例：
//! - 确保 output/spatial_index.sqlite 已存在（房间计算粗筛依赖）
//! - cargo run --example room_tree_zone_cache_demo --features "gen_model sqlite-index web_server" -- --nocapture
//!
//! 可选环境变量：
//! - DBOPTION_PATH：配置文件路径前缀（默认 "DbOption"）
//! - ZONE_REFNO：目标 ZONE refno（默认 "24381/146882"）
//! - ROOM_RELATION_CONCURRENCY / ROOM_RELATION_CANDIDATE_LIMIT：房间计算性能调参

use anyhow::{Context, Result};
use aios_core::RefnoEnum;
use std::env;

const DEFAULT_ZONE_REFNO: &str = "24381/146882";

#[tokio::main]
async fn main() -> Result<()> {
    // 1) 配置加载
    let dbopt_path = env::var("DBOPTION_PATH").unwrap_or_else(|_| "db_options/DbOption".to_string());
    let mut db_option_ext = aios_database::options::get_db_option_ext_from_path(&dbopt_path)
        .with_context(|| format!("加载 DbOption 失败: {}", dbopt_path))?;

    // aios_core 的 init_surreal 使用 DB_OPTION_FILE 选择配置文件；
    // 示例里 DBOPTION_PATH 仅用于加载 DbOptionExt，因此这里同步设置以避免配置不一致。
    unsafe {
        std::env::set_var("DB_OPTION_FILE", &dbopt_path);
    }

    // 2) 初始化 SurrealDB（房间计算与 room-tree 查询都需要访问 DB）
    aios_core::init_surreal()
        .await
        .context("初始化 SurrealDB 失败")?;

    // 3) 强制切换到“缓存方式生成模型”
    db_option_ext.use_cache = true;
    db_option_ext.use_surrealdb = false;
    db_option_ext.inner.gen_model = true;
    db_option_ext.inner.gen_mesh = true;
    db_option_ext.inner.replace_mesh = Some(true);
    db_option_ext.inner.apply_boolean_operation = false;

    // 4) 检查 SQLite 空间索引是否存在（房间计算粗筛依赖）
    let idx_path = aios_database::spatial_index::SqliteSpatialIndex::default_path();

    // 5) 仅生成目标 ZONE 下模型
    let zone_str = env::var("ZONE_REFNO").unwrap_or_else(|_| DEFAULT_ZONE_REFNO.to_string());
    let zone_refno = RefnoEnum::from(zone_str.as_str());
    if !zone_refno.is_valid() {
        anyhow::bail!("无效的 ZONE_REFNO: {}", zone_str);
    }

    println!("🏗️  目标 ZONE: {}", zone_refno);
    println!(
        "🧭  生成路径: use_cache={}, use_surrealdb={}, meshes_path={}",
        db_option_ext.use_cache,
        db_option_ext.use_surrealdb,
        db_option_ext.inner.get_meshes_path().display()
    );

    aios_database::fast_model::gen_all_geos_data(
        vec![zone_refno],
        &db_option_ext,
        None,
        db_option_ext.target_sesno,
    )
    .await
    .context("模型生成失败")?;

    // 6) 如果缺少空间索引，尝试从缓存导出 instances_{dbnum}.json + aabb.json 并自动导入生成索引
    if !idx_path.exists() {
        // 注意：Refno 的 ref0（如 24381）不一定等同于 SurrealDB 的 dbnum。
        // cache 按 dbnum 分批存储，这里从 db_meta_info.json 推导实际 dbnum。
        let db_meta = aios_database::data_interface::db_meta_manager::db_meta();
        db_meta.ensure_loaded().context("加载 db_meta_info.json 失败")?;
        let cache_dbnum: u32 = db_meta
            .get_dbnum_by_refno(zone_refno)
            .context("无法从 db_meta_info.json 推导 cache dbnum")?;

        println!(
            "🧩 未发现空间索引 {:?}，将尝试从缓存生成（cache dbnum={}）",
            idx_path, cache_dbnum
        );

        let export_dir = std::path::PathBuf::from("output/instances_cache_for_index");
        let cache_dir = db_option_ext.get_foyer_cache_dir();
        let mesh_dir = db_option_ext.inner.get_meshes_path();
        let mesh_lod_tag = format!("{:?}", db_option_ext.inner.mesh_precision.default_lod);

        aios_database::fast_model::export_model::export_prepack_lod::export_dbnum_instances_json_from_cache(
            cache_dbnum,
            &export_dir,
            &cache_dir,
            Some(&mesh_dir),
            Some(mesh_lod_tag.as_str()),
            true,
            None,
        )
        .await
        .context("缓存导出 instances_{dbnum}.json 失败")?;

        let instances_path = export_dir.join(format!("instances_{}.json", cache_dbnum));
        if !instances_path.exists() {
            anyhow::bail!(
                "导出完成但未找到 instances 文件: {}",
                instances_path.display()
            );
        }

        // 复用 CLI 逻辑：删除旧索引并重建
        if idx_path.exists() {
            std::fs::remove_file(&idx_path).ok();
        } else if let Some(parent) = idx_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        let idx = aios_database::sqlite_index::SqliteAabbIndex::open(&idx_path)
            .context("打开 SQLite 索引失败")?;
        idx.init_schema().context("初始化 SQLite 索引 schema 失败")?;
        let import_stats = idx
            .import_from_instances_json(&instances_path, &aios_database::sqlite_index::ImportConfig::default())
            .context("导入 instances.json 到 SQLite 空间索引失败")?;

        println!(
            "✅ 空间索引生成完成: unique_count={}, total_inserted={} (equi={}, children={}, tubings={})",
            import_stats.unique_count,
            import_stats.total_inserted,
            import_stats.equi_count,
            import_stats.children_count,
            import_stats.tubings_count
        );
    }

    // 6) 房间计算（落库 room_relate）
    let stats = aios_database::fast_model::build_room_relations(&db_option_ext.inner)
        .await
        .context("房间计算失败")?;
    println!(
        "✅ 房间计算完成: rooms={}, panels={}, components={}, build_time_ms={}",
        stats.total_rooms, stats.total_panels, stats.total_components, stats.build_time_ms
    );

    // 7) 检查 room_relate 是否产出（否则 room-tree 会回退 noun_hierarchy）
    let rooms = aios_core::room::algorithm::query_rooms_from_room_relate()
        .await
        .context("query_rooms_from_room_relate 失败")?;
    println!("📌 room_relate rooms_from_relate: {}", rooms.len());

    // 8) 调用 room-tree 核心逻辑做最小验证
    use aios_database::web_api::room_tree_api::{
        RoomTreeNodeId, room_tree_ancestors_core, room_tree_children_core, room_tree_search_core,
    };

    let root = room_tree_children_core("room-root", 2000)
        .await
        .context("room_tree_children_core(root) 失败")?;
    println!("🌳 room-tree groups: {}", root.children.len());
    if root.children.is_empty() {
        anyhow::bail!("room-tree root children 为空：请确认 room_relate 或 noun_hierarchy 有数据");
    }

    let first_group = &root.children[0];
    let group_id = match &first_group.id {
        RoomTreeNodeId::Str(s) => s.clone(),
        RoomTreeNodeId::Refno(_) => {
            anyhow::bail!("ROOM_GROUP 的 id 期望为字符串，但得到 refno")
        }
    };
    println!("📂 sample group: {}", first_group.name);

    let group_children = room_tree_children_core(&group_id, 2000)
        .await
        .context("room_tree_children_core(group) 失败")?;
    println!("🏠 rooms in sample group: {}", group_children.children.len());

    // 祖先链验证：优先取该 group 下的第一个 room
    let mut sample_room_refno: Option<RefnoEnum> = None;
    for node in &group_children.children {
        if let RoomTreeNodeId::Refno(r) = node.id {
            sample_room_refno = Some(r);
            break;
        }
    }

    if let Some(room_refno) = sample_room_refno {
        let anc = room_tree_ancestors_core(&room_refno.to_string())
            .await
            .context("room_tree_ancestors_core(room) 失败")?;
        println!("🧬 ancestors ids len={}", anc.ids.len());
    } else {
        println!("⚠️ sample group 下无 room 节点（可能 room_relate 未覆盖该组），跳过 ancestors 校验");
    }

    // 搜索验证：用 group 名称作为 keyword（通常可命中）
    let keyword = first_group.name.clone();
    let search = room_tree_search_core(&keyword, 50)
        .await
        .context("room_tree_search_core 失败")?;
    println!("🔎 search({:?}) hits={}", keyword, search.items.len());

    Ok(())
}
