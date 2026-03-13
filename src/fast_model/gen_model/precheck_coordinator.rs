//! 模型生成预检查协调器
//!
//! 在模型生成开始前，统一检查并生成必要的预处理数据：
//! - Tree 索引文件（{dbnum}.tree）
//! - model_cache transform_cache（世界坐标变换本地缓存，仅做存在性检查）
//! - db_meta_info.json（数据库元信息）
//!
//! # 设计原则
//!
//! - **最小侵入**：在现有流程前插入预检查，不改变核心生成逻辑
//! - **智能判断**：根据配置自动提取需要检查的 dbnum 列表
//! - **容错处理**：预检查失败时给出明确的警告信息，不阻断流程
//! - **性能优先**：使用并行处理提升大规模数据的处理速度

use crate::data_interface::db_meta_manager::db_meta;
use crate::fast_model::gen_model::tree_index_manager::TreeIndexManager;
use crate::options::DbOptionExt;
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::path::Path;

/// 预检查配置
#[derive(Debug, Clone)]
pub struct PrecheckConfig {
    /// 是否启用预检查
    pub enabled: bool,
    /// 是否检查 Tree 文件
    pub check_tree: bool,
    /// 是否检查 transform_cache（model cache）
    pub check_pe_transform: bool,
    /// 是否检查 db_meta_info
    pub check_db_meta: bool,
    /// Tree 文件输出目录
    pub tree_output_dir: String,
}

impl Default for PrecheckConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            check_tree: true,
            check_pe_transform: true,
            check_db_meta: true,
            tree_output_dir: "output/scene_tree".to_string(),
        }
    }
}

/// 预检查结果统计
#[derive(Debug, Default)]
pub struct PrecheckStats {
    /// 检查的 Tree 文件数量
    pub tree_checked: usize,
    /// 生成的 Tree 文件数量
    pub tree_generated: usize,
    /// 生成失败的 Tree 文件数量
    pub tree_failed: usize,
    /// 检查的 transform_cache 数量（沿用字段名以保持兼容）
    pub pe_transform_checked: usize,
    /// 刷新的 pe_transform 数量（新逻辑不再在 precheck 阶段全量刷新，通常为 0）
    pub pe_transform_refreshed: usize,
    /// db_meta_info 是否加载成功
    pub db_meta_loaded: bool,
}

/// 从配置中提取需要检查的 dbnum 列表
///
/// 优先级：
/// 1. manual_db_nums（手动指定）
/// 2. 从 db_meta_info.json 读取
/// 3. 应用 exclude_db_nums 过滤
async fn extract_target_dbnums(db_option: &DbOptionExt) -> Result<Vec<u32>> {
    let mut dbnums: Vec<u32> = if let Some(manual) = &db_option.inner.manual_db_nums {
        manual.clone()
    } else {
        let mut from_meta = Vec::new();
        if db_meta().ensure_loaded().is_ok() {
            from_meta = db_meta().get_all_dbnums();
        }
        from_meta
    };

    // 应用排除列表
    if let Some(exclude) = &db_option.inner.exclude_db_nums {
        let exclude_set: HashSet<u32> = exclude.iter().copied().collect();
        dbnums.retain(|dbnum| !exclude_set.contains(dbnum));
    }

    // 去重并排序
    let mut unique_dbnums: Vec<u32> = dbnums
        .into_iter()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    unique_dbnums.sort_unstable();

    Ok(unique_dbnums)
}

/// 检查并加载 db_meta_info.json
fn check_db_meta_info(stats: &mut PrecheckStats) -> Result<()> {
    println!("[precheck] 📄 检查 db_meta_info.json...");

    match db_meta().ensure_loaded() {
        Ok(_) => {
            let dbnum_count = db_meta().get_all_dbnums().len();
            println!(
                "[precheck] ✅ db_meta_info.json 已加载（包含 {} 个数据库）",
                dbnum_count
            );
            stats.db_meta_loaded = true;
            Ok(())
        }
        Err(e) => {
            println!("[precheck] ⚠️  db_meta_info.json 加载失败: {}", e);
            println!("[precheck]    提示：可运行以下命令生成：");
            println!("[precheck]    cargo run --example update_db_meta_info_for_dbnum");
            stats.db_meta_loaded = false;
            // 不阻断流程，仅警告
            Ok(())
        }
    }
}

/// 输出预检查统计摘要
fn print_precheck_summary(stats: &PrecheckStats) {
    println!();
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║  📊 预检查完成                                               ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║  Tree 文件:                                                  ║");
    println!("║    - 检查: {} 个", stats.tree_checked);
    println!("║    - 生成: {} 个", stats.tree_generated);
    if stats.tree_failed > 0 {
        println!("║    - 失败: {} 个 ❌", stats.tree_failed);
    }
    println!(
        "║  pe_transform: {}",
        if stats.pe_transform_refreshed > 0 {
            "✅"
        } else {
            "⚠️"
        }
    );
    println!(
        "║  db_meta_info: {}",
        if stats.db_meta_loaded {
            "✅"
        } else {
            "⚠️"
        }
    );
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();
}

/// 检查并生成缺失的 Tree 文件（并行版本）
///
/// 使用 tokio::spawn 并行生成多个 Tree 文件，提升性能
async fn check_tree_files(
    dbnums: &[u32],
    output_dir: &str,
    stats: &mut PrecheckStats,
) -> Result<()> {
    println!("[precheck] 🌲 检查 Tree 索引文件...");

    let tree_dir = Path::new(output_dir);
    let manager = TreeIndexManager::new(tree_dir, dbnums.to_vec());

    let missing = manager.get_missing_tree_files();
    stats.tree_checked = dbnums.len();

    if missing.is_empty() {
        println!("[precheck] ✅ 所有 Tree 文件已存在（{} 个）", dbnums.len());
        return Ok(());
    }

    println!("[precheck] ⚠️  发现 {} 个缺失的 Tree 文件", missing.len());
    println!("[precheck] 缺失的数据库: {:?}", missing);
    println!("[precheck] 🔄 开始并行生成缺失的 Tree 文件...");

    println!("[precheck] ⚠️  缺失的 Tree 文件需要通过 PDMS 解析(--parse-db)生成");

    stats.tree_generated = 0;
    stats.tree_failed = missing.len();

    Ok(())
}

/// 检查 model_cache transform_cache 是否存在（不要求全量命中）。
///
/// 约定：precheck 只做“存在性检查”，miss 由后续模型生成阶段按需计算并回写。
async fn check_pe_transform(
    _db_option: &DbOptionExt,
    dbnums: &[u32],
    stats: &mut PrecheckStats,
) -> Result<()> {
    println!("[precheck] 🔄 检查 transform_cache（model cache）...");

    if dbnums.is_empty() {
        println!("[precheck] ⚠️  没有需要检查的数据库");
        return Ok(());
    }

    stats.pe_transform_checked = dbnums.len();
    stats.pe_transform_refreshed = 0;

    // transform_cache 已改为纯内存，无需磁盘目录
    crate::fast_model::transform_cache::init_global_transform_cache();
    println!("[precheck] ✅ transform_cache 已初始化（纯内存）");
    Ok(())
}

/// 执行模型生成前的预检查
///
/// 根据 db_option 配置，自动提取需要检查的 dbnum 列表，
/// 并确保所有必要的预处理数据就绪。
///
/// # Arguments
/// * `db_option` - 数据库配置
/// * `config` - 预检查配置（可选，使用默认配置）
///
/// # Returns
/// 返回预检查统计信息
pub async fn run_precheck(
    db_option: &DbOptionExt,
    config: Option<PrecheckConfig>,
) -> Result<PrecheckStats> {
    let config = config.unwrap_or_default();
    let mut stats = PrecheckStats::default();

    if !config.enabled {
        log::info!("[precheck] 预检查已禁用，跳过");
        return Ok(stats);
    }

    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║  🔍 模型生成预检查                                          ║");
    println!("╚══════════════════════════════════════════════════════════════╝");

    // 1. 提取需要检查的 dbnum 列表
    let dbnums = extract_target_dbnums(db_option).await?;

    if dbnums.is_empty() {
        println!("[precheck] ⚠️  未找到需要检查的数据库编号");
        return Ok(stats);
    }

    println!("[precheck] 📋 检查范围: {} 个数据库", dbnums.len());
    println!("[precheck] 数据库编号: {:?}", dbnums);
    println!();

    // 2. 检查 db_meta_info.json
    if config.check_db_meta {
        check_db_meta_info(&mut stats)?;
    }

    // 3. 检查 Tree 文件
    if config.check_tree {
        check_tree_files(&dbnums, &config.tree_output_dir, &mut stats).await?;
    }

    // 4. 检查 transform_cache（model cache）：与数据源无关，cache-only / surrealdb 都可用。
    if config.check_pe_transform {
        check_pe_transform(db_option, &dbnums, &mut stats).await?;
    }

    // 5. 输出统计信息
    print_precheck_summary(&stats);

    Ok(stats)
}
