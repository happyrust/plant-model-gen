//! 模型生成预检查协调器
//!
//! 在模型生成开始前，统一检查并生成必要的预处理数据：
//! - Tree 索引文件（{dbnum}.tree）
//! - pe_transform（按需：跳过 / 子树 / 整库）
//! - db_meta_info.json（数据库元信息）
//!
//! # pe_transform 策略（按需）
//!
//! - **Skip (L0)**：GenPipeline 已从 VersionedReadSession 加载 transforms，不刷 pe_transform
//! - **ScopeRoots (L1)**：仅刷新给定 roots 子树（`refresh_pe_transform_for_root_refnos`）
//! - **FullDbnum (L2)**：整库覆盖探测 + 全量刷新（显式运维/全量 regen，可能极慢）
//!
//! 增量路径另见 `invalidate_pe_transform_for_root_refnos`：owner/POS 变更后只清受影响子树，
//! 再靠 L0 session 或 lazy miss 回写，禁止「任一缺口 → 整库」。

use crate::data_interface::db_meta_manager::db_meta;
use crate::options::DbOptionExt;
use aios_core::RefnoEnum;
use anyhow::{Context, Result};
use std::collections::HashSet;

/// pe_transform 预检查模式（按需生成）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PeTransformPrecheckMode {
    /// L0：不检查、不刷新（session 已提供 transforms）
    #[default]
    Skip,
    /// L1：只刷新 `pe_transform_roots` 子树
    ScopeRoots,
    /// L2：按 dbnum 全库覆盖探测并刷新
    FullDbnum,
}

/// 预检查配置
#[derive(Debug, Clone)]
pub struct PrecheckConfig {
    /// 是否启用预检查
    pub enabled: bool,
    /// 是否检查 Tree 文件
    pub check_tree: bool,
    /// pe_transform 策略
    pub pe_transform_mode: PeTransformPrecheckMode,
    /// L1：子树刷新 roots（ScopeRoots 时生效）
    pub pe_transform_roots: Vec<RefnoEnum>,
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
            pe_transform_mode: PeTransformPrecheckMode::Skip,
            pe_transform_roots: Vec::new(),
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
    /// pe_transform 探测/刷新涉及的范围计数（dbnum 或 roots）
    pub pe_transform_checked: usize,
    /// 刷新写入的 pe_transform 节点数
    pub pe_transform_refreshed: usize,
    /// pe_transform 实际采用的模式
    pub pe_transform_mode: Option<PeTransformPrecheckMode>,
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
            from_meta = db_meta().get_dbnums_by_type(&db_option.inner.module);
            if from_meta.is_empty() && db_option.inner.module.eq_ignore_ascii_case("DESI") {
                println!(
                    "[precheck] ⚠️ db_meta_info.json 中未发现 DESI 数据库，回退检查所有 dbnum"
                );
                from_meta = db_meta().get_all_dbnums();
            }
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

fn pe_transform_status_label(stats: &PrecheckStats) -> &'static str {
    match stats.pe_transform_mode {
        Some(PeTransformPrecheckMode::Skip) => "⏭ 跳过(L0 session)",
        Some(PeTransformPrecheckMode::ScopeRoots) if stats.pe_transform_refreshed > 0 => {
            "✅ 子树(L1)"
        }
        Some(PeTransformPrecheckMode::ScopeRoots) => "⏭ 子树空 roots(L1)",
        Some(PeTransformPrecheckMode::FullDbnum) if stats.pe_transform_refreshed > 0 => {
            "✅ 整库(L2)"
        }
        Some(PeTransformPrecheckMode::FullDbnum) => "✅ 整库已覆盖(L2)",
        None => "—",
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
    println!("║  pe_transform: {}", pe_transform_status_label(stats));
    if stats.pe_transform_refreshed > 0 {
        println!("║    - 刷新节点: {}", stats.pe_transform_refreshed);
    }
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

/// 层级数据来自 pe 快照（SurrealDB），不依赖 .tree 文件。
async fn check_tree_files(
    dbnums: &[u32],
    _output_dir: &str,
    stats: &mut PrecheckStats,
) -> Result<()> {
    println!("[precheck] 🌲 层级数据源=pe_owner（快照），跳过 .tree 文件检查");
    stats.tree_checked = dbnums.len();
    stats.tree_generated = 0;
    stats.tree_failed = 0;
    Ok(())
}

/// L1：按 roots 子树刷新 pe_transform（不探测整库覆盖）。
async fn refresh_pe_transform_scoped(
    db_option: &DbOptionExt,
    roots: &[RefnoEnum],
    stats: &mut PrecheckStats,
) -> Result<()> {
    crate::fast_model::transform_cache::init_global_transform_cache();
    stats.pe_transform_mode = Some(PeTransformPrecheckMode::ScopeRoots);
    stats.pe_transform_checked = roots.len();

    if roots.is_empty() {
        println!("[precheck] ⏭ pe_transform L1：roots 为空，跳过刷新");
        return Ok(());
    }

    println!(
        "[precheck] 🔄 pe_transform L1：按 {} 个 roots 刷新子树...",
        roots.len()
    );
    let refreshed =
        crate::pe_transform_refresh::refresh_pe_transform_for_root_refnos(roots, db_option)
            .await
            .with_context(|| {
                format!(
                    "precheck L1 刷新 pe_transform 失败: roots_len={}",
                    roots.len()
                )
            })?;
    stats.pe_transform_refreshed = refreshed;
    println!(
        "[precheck] ✅ pe_transform L1 完成: {} 个节点（roots={}）",
        refreshed,
        roots.len()
    );
    Ok(())
}

/// L2：整库覆盖探测；未覆盖则全量刷新（可能极慢，仅显式启用）。
async fn refresh_pe_transform_full_dbnums(
    db_option: &DbOptionExt,
    dbnums: &[u32],
    stats: &mut PrecheckStats,
) -> Result<()> {
    println!("[precheck] 🔄 pe_transform L2：整库覆盖检查 dbnums={dbnums:?}...");
    println!("[precheck] ⚠️  L2 可能对大库耗时极长；scoped/session 生成应使用 L0/L1");

    if dbnums.is_empty() {
        println!("[precheck] ⚠️  没有需要检查的数据库");
        return Ok(());
    }

    crate::fast_model::transform_cache::init_global_transform_cache();
    stats.pe_transform_mode = Some(PeTransformPrecheckMode::FullDbnum);
    stats.pe_transform_checked = dbnums.len();
    stats.pe_transform_refreshed = 0;

    let mut uncovered: Vec<u32> = Vec::new();
    for &dbnum in dbnums {
        match crate::pe_transform_refresh::pe_transform_covers_dbnum(dbnum).await {
            Ok(true) => {}
            Ok(false) => uncovered.push(dbnum),
            Err(err) => {
                println!(
                    "[precheck] ⚠️  探测 pe_transform 覆盖失败（按未覆盖处理）: dbnum={dbnum} err={err}"
                );
                uncovered.push(dbnum);
            }
        }
    }

    if uncovered.is_empty() {
        println!(
            "[precheck] ✅ pe_transform L2 覆盖完好（{} 个 dbnum），transform_cache 已初始化",
            dbnums.len()
        );
        return Ok(());
    }

    println!(
        "[precheck] 🔄 pe_transform L2 未覆盖 dbnums={:?}，开始整库刷新...",
        uncovered
    );
    let refreshed =
        crate::pe_transform_refresh::refresh_pe_transform_for_dbnums(&uncovered, db_option)
            .await
            .with_context(|| format!("precheck L2 刷新 pe_transform 失败: dbnums={uncovered:?}"))?;
    stats.pe_transform_refreshed = refreshed;
    println!("[precheck] ✅ pe_transform L2 完成: {} 个节点", refreshed);
    Ok(())
}

async fn check_pe_transform(
    db_option: &DbOptionExt,
    dbnums: &[u32],
    config: &PrecheckConfig,
    stats: &mut PrecheckStats,
) -> Result<()> {
    match config.pe_transform_mode {
        PeTransformPrecheckMode::Skip => {
            crate::fast_model::transform_cache::init_global_transform_cache();
            stats.pe_transform_mode = Some(PeTransformPrecheckMode::Skip);
            stats.pe_transform_checked = 0;
            println!(
                "[precheck] ⏭ pe_transform L0：跳过（VersionedReadSession / generation_read 已提供 transforms）"
            );
            Ok(())
        }
        PeTransformPrecheckMode::ScopeRoots => {
            refresh_pe_transform_scoped(db_option, &config.pe_transform_roots, stats).await
        }
        PeTransformPrecheckMode::FullDbnum => {
            refresh_pe_transform_full_dbnums(db_option, dbnums, stats).await
        }
    }
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
        // 仍允许 L0/L1（不依赖 dbnums）
    } else {
        println!("[precheck] 📋 检查范围: {} 个数据库", dbnums.len());
        println!("[precheck] 数据库编号: {:?}", dbnums);
        println!();
    }

    // 2. 检查 db_meta_info.json
    if config.check_db_meta {
        check_db_meta_info(&mut stats)?;
    }

    // 3. 检查 Tree 文件
    if config.check_tree {
        check_tree_files(&dbnums, &config.tree_output_dir, &mut stats).await?;
    }

    // 4. pe_transform：按 L0/L1/L2 策略
    check_pe_transform(db_option, &dbnums, &config, &mut stats).await?;

    // 5. 输出统计信息
    print_precheck_summary(&stats);

    Ok(stats)
}
