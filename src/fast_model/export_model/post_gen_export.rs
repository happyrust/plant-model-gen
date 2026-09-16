//! 模型生成后置 Parquet 导出。
//!
//! ## 这一层在链路里的位置
//!
//! 「模型生成运行」跑完几何之后，产物还只躺在库里；本模块负责把它们落成**导出物**
//! （`parquet/` 目录下的列存文件），供下游空间检索与外部消费使用。它是生成流程的
//! **尾巴**，不参与几何计算，只做「选库 → 校验前置数据 → 逐库导出 → 统计上报」。
//!
//! ## 三个关键设计点
//!
//! 1. **开关分两层**：运行期开关 `export_parquet_after_gen`，编译期开关 `parquet-export`
//!    feature（默认特性**不含**它，见 AGENTS.md 的编译加速一节）。两层任一未开都不导出，
//!    但走的是不同分支、给出不同 `skipped_reason`，便于事后区分「没配」还是「没编」。
//! 2. **dbnum 来源有优先级**：调用方 hint → 手工配置 → db_meta 按模块 → 从 `inst_relate`
//!    兜底发现。最终用哪一档会记进 `dbnum_source`，因为兜底档可能混入非 DESI 库，
//!    出问题时这一个字段就能定位。
//! 3. **pe_transform 用探测代替无条件刷新**：生成阶段已按实例落库，整库 BFS 刷新退化为
//!    仅对未覆盖库的兜底，不再是常规路径（详见下方 `uncovered_dbnums` 处的说明）。
//!
//! ## 失败语义
//!
//! 「选不出库」不是错误，返回带 `skipped_reason` 的报告；而**导出本身失败是错误**，
//! 直接 `Err` 中断整轮，不做部分成功——半套导出物比没有更危险。
//!
//! ## 调用方契约
//!
//! - 必须在几何生成**成功之后**调用；本模块不校验生成水位，拿到什么就导什么。
//! - 返回 `Ok` 只保证「没有库导失败」，不保证「导出了东西」——要判断有没有产物，
//!   看 `exported_dbnums` 是否为空，别只看 `Ok`。
//! - 本模块不推进模型生成水位，也不写任何锚点；水位由上层在整轮成功后统一发布。

use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::options::DbOptionExt;

/// 一次后置 Parquet 导出的结果报告。
///
/// 无论导出是否真的发生，本结构都会被返回，用「字段组合」而不是「Option 化整个报告」
/// 来表达三种结局，方便调用方与日志统一处理：
///
/// - **未启用**：`enabled=false`，`skipped_reason` 写明是哪个开关关着；
/// - **启用但跳过**：`enabled=true` 且 `exported_dbnums` 为空，`skipped_reason` 说明原因；
/// - **正常导出**：`enabled=true`、`exported_dbnums` 非空、`skipped_reason=None`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostGenerationParquetExportReport {
    /// 运行期开关是否打开（注意：打开不等于真的导出了，还要看 `exported_dbnums`）。
    pub enabled: bool,
    /// 本轮实际导出的 dbnum 列表，已去重并升序。
    pub exported_dbnums: Vec<u32>,
    /// 导出物根目录（`<项目输出目录>/parquet`）；跳过时为 `None`。
    pub output_dir: Option<PathBuf>,
    /// dbnum 是从哪一档来源选出来的，取值见 `export_parquet_after_generation_impl` 内的
    /// 优先级链；兜底档（`inst_relate_fallback`）意味着结果可能不精确，排查时先看这里。
    pub dbnum_source: Option<String>,
    /// 跳过原因；仅在没有产生导出物时为 `Some`。
    pub skipped_reason: Option<String>,
}

impl PostGenerationParquetExportReport {
    /// 运行期开关 `export_parquet_after_gen=false` 时的报告。
    ///
    /// 这是「用户主动没开」，属于正常路径，不打 warn 日志。
    fn disabled() -> Self {
        Self {
            enabled: false,
            exported_dbnums: Vec::new(),
            output_dir: None,
            dbnum_source: None,
            skipped_reason: Some("export_parquet_after_gen=false".to_string()),
        }
    }

    /// 开关已开、但因为别的原因没导出时的报告（如 feature 未编译）。
    ///
    /// 与 [`Self::disabled`] 的区别在 `enabled`：这里是 `true`，代表「本该导出却没导成」，
    /// 调用方通常需要给出告警。
    fn skipped(reason: impl Into<String>) -> Self {
        Self {
            enabled: true,
            exported_dbnums: Vec::new(),
            output_dir: None,
            dbnum_source: None,
            skipped_reason: Some(reason.into()),
        }
    }
}

/// 后置导出的唯一入口：按开关决定是否把本轮生成结果导成 Parquet。
///
/// `dbnums_hint` 是调用方（通常是刚跑完的生成流程）已知的目标库，给了就直接用，
/// 省掉后面一整条来源推断链；传 `None` 或空列表则退化为自动发现。
///
/// 本函数只做**两层开关的裁决**，真正的导出逻辑在
/// [`export_parquet_after_generation_impl`]，这样未编译 `parquet-export` 时
/// 整块重逻辑（polars/arrow 依赖）都不会进二进制。
pub async fn export_parquet_after_generation_if_enabled(
    db_option_ext: &DbOptionExt,
    dbnums_hint: Option<Vec<u32>>,
) -> Result<PostGenerationParquetExportReport> {
    // 第一层：运行期开关。没开就直接返回，连 feature 分支都不用进。
    if !db_option_ext.export_parquet_after_gen {
        return Ok(PostGenerationParquetExportReport::disabled());
    }

    // 第二层：编译期 feature。开了才有真正的实现。
    #[cfg(feature = "parquet-export")]
    {
        export_parquet_after_generation_impl(db_option_ext, dbnums_hint).await
    }

    // feature 未编译：这是配置与构建不匹配，属于「本该导出却导不了」，必须告警而不是静默。
    #[cfg(not(feature = "parquet-export"))]
    {
        // hint 在这条分支用不上，显式丢弃以免 unused 警告。
        let _ = dbnums_hint;
        log::warn!(
            "export_parquet_after_gen 已启用，但 parquet-export 特性未编译，跳过 Parquet 导出"
        );
        Ok(PostGenerationParquetExportReport::skipped(
            "parquet-export feature is disabled",
        ))
    }
}

/// 后置导出的实际实现（仅在 `parquet-export` feature 下编译）。
///
/// 流程分五段，顺序不可调换：
/// 1. 按优先级确定要导哪些 dbnum；
/// 2. 应用排除名单并规范化列表；
/// 3. 确保 `pe_transform` 覆盖到位（导出依赖它算最终位姿）；
/// 4. 逐库导出，并顺带刷新 SQLite 空间索引；
/// 5. 遍历产物目录统计体积，打点上报。
#[cfg(feature = "parquet-export")]
async fn export_parquet_after_generation_impl(
    db_option_ext: &DbOptionExt,
    dbnums_hint: Option<Vec<u32>>,
) -> Result<PostGenerationParquetExportReport> {
    use std::sync::Arc;
    use std::time::Instant;

    use crate::data_interface::db_meta_manager::db_meta;
    use crate::fast_model::export_model::export_dbnum_instances_parquet::{
        export_dbnum_instances_parquet_latest, query_distinct_dbnums_from_inst_relate,
    };

    // ---- 第 1 段：确定导出目标 dbnum ------------------------------------------------
    // 空列表视同没给，避免调用方传 `Some(vec![])` 时把自动发现整条链短路掉。
    let mut dbnums = dbnums_hint
        .filter(|values| !values.is_empty())
        .unwrap_or_default();

    // 四档来源按可信度从高到低尝试，`dbnum_source` 记录最终命中哪一档。
    // 越靠后的档越不精确，出现在报告里就意味着「这次的库是猜出来的」。
    let dbnum_source = if !dbnums.is_empty() {
        // 档 1：调用方明确指定，最可信。
        "hint"
    } else if let Some(values) = db_option_ext
        .inner
        .manual_db_nums
        .clone()
        .filter(|v| !v.is_empty())
    {
        // 档 2：站点配置里手工写死的库列表。
        dbnums = values;
        "manual_db_nums"
    } else if db_meta().ensure_loaded().is_ok() {
        // 档 3：从 db_meta_info.json 按当前模块类型取库，正常部署下走这条。
        dbnums = db_meta().get_dbnums_by_type(&db_option_ext.inner.module);
        "db_meta_module"
    } else {
        // 档 4：元数据不可用时的兜底——直接扫 `inst_relate` 看实际有哪些库有数据。
        match query_distinct_dbnums_from_inst_relate().await {
            Ok(discovered) if !discovered.is_empty() => {
                // 这条路没有库类型信息，可能把非 DESI 库也捞进来，因此必须告警留痕。
                log::warn!(
                    "db_meta_info.json 不可用，临时从 inst_relate 发现 Parquet 导出 dbnum；可能包含非 DESI 库: {:?}",
                    discovered
                );
                dbnums = discovered;
                "inst_relate_fallback"
            }
            // 扫得到但是空的：库里确实没数据，属于「无事可做」而非故障。
            Ok(_) => {
                log::warn!("export_parquet_after_gen 已启用，但 inst_relate 未发现可导出的 dbnum");
                "inst_relate_empty"
            }
            // 扫失败：记 error 但不中断，让后面的空列表检查统一收口成「跳过」。
            Err(err) => {
                log::error!("自动发现 Parquet 导出 dbnum 失败: {}", err);
                "inst_relate_error"
            }
        }
    };

    // ---- 第 2 段：应用排除名单并规范化 ----------------------------------------------
    // 排除名单在所有来源之后统一生效，保证 hint 也逃不过站点级黑名单。
    if let Some(exclude_nums) = &db_option_ext.inner.exclude_db_nums {
        let exclude: std::collections::HashSet<u32> = exclude_nums.iter().copied().collect();
        dbnums.retain(|dbnum| !exclude.contains(dbnum));
    }
    // 先排序再去重：`dedup` 只能消除相邻重复，顺序也让日志和报告可对账。
    dbnums.sort_unstable();
    dbnums.dedup();

    // 选不出库不算失败，返回带原因的报告让调用方自己决定要不要告警。
    if dbnums.is_empty() {
        let reason = format!("no exportable dbnum (source={dbnum_source})");
        log::warn!("export_parquet_after_gen 已启用，但没有可导出的 dbnum ({reason})");
        return Ok(PostGenerationParquetExportReport {
            enabled: true,
            exported_dbnums: Vec::new(),
            output_dir: None,
            dbnum_source: Some(dbnum_source.to_string()),
            skipped_reason: Some(reason),
        });
    }

    log::info!(
        "📦 自动导出 Parquet: source={}, dbnums={:?}",
        dbnum_source,
        dbnums
    );
    // 打点：导出阶段的起点，配合下面的 finished 打点可算出整段耗时。
    // 耗时参数传 0 是因为计时基准 `export_started` 要到第 4 段才建立，
    // 这一拍只负责标记「阶段开始」，不报耗时。
    crate::perf_metrics::record_generate_progress(
        "post_gen_export_started",
        Some(&format!("source={dbnum_source} dbnums={dbnums:?}")),
        0,
    );

    // ---- 第 3 段：确保 pe_transform 覆盖到位 ----------------------------------------
    // 生成阶段的专门 persist_pe_transform 已按实例落库 pe_transform，因此这里改用
    // **实例级覆盖探测**：命中即跳过整库 BFS 刷新。整库 refresh 仅在未覆盖（旧库/
    // 未按新阶段生成的数据）时作为兜底运行，不再是常规路径。
    let mut uncovered_dbnums = Vec::new();
    for dbnum in &dbnums {
        match crate::pe_transform_refresh::pe_transform_covers_instances_for_dbnum(*dbnum).await {
            Ok(true) => log::info!(
                "✅ Parquet 导出前 pe_transform 实例覆盖完好（生成阶段已就地落库），dbnum={}",
                dbnum
            ),
            Ok(false) => uncovered_dbnums.push(*dbnum),
            // 探测本身失败时按「未覆盖」处理：宁可多刷一次，也不要拿缺位姿的数据去导出。
            Err(err) => {
                log::warn!(
                    "⚠️ Parquet 导出前探测 pe_transform 实例覆盖失败，按未覆盖处理: dbnum={} err={}",
                    dbnum,
                    err
                );
                uncovered_dbnums.push(*dbnum);
            }
        }
    }

    if uncovered_dbnums.is_empty() {
        // 常规路径：全部命中覆盖，整库 BFS 刷新一次都不用跑。
        log::info!(
            "✅ Parquet 导出前 pe_transform 实例覆盖完好，跳过整库刷新: dbnums={:?}",
            dbnums
        );
    } else {
        // 兜底路径：只对未覆盖的库做整库刷新，失败直接中断——位姿不全会导出错误几何。
        log::info!(
            "🔄 Parquet 导出前兜底刷新未覆盖 dbnum 的 pe_transform（旧库/未按新阶段生成）: dbnums={:?}",
            uncovered_dbnums
        );
        let refreshed = crate::pe_transform_refresh::refresh_pe_transform_for_dbnums(
            &uncovered_dbnums,
            db_option_ext,
        )
        .await
        .map_err(|e| anyhow::anyhow!("Parquet 导出前刷新 pe_transform 失败: {}", e))?;
        log::info!(
            "✅ Parquet 导出前 pe_transform 兜底刷新完成: {} 个节点",
            refreshed
        );
    }

    // ---- 第 4 段：逐库导出 ----------------------------------------------------------
    // 所有库共享一个根目录，各库在其下自建子目录，便于整体打包与统计。
    let base_output_dir = db_option_ext.get_project_output_dir().join("parquet");
    // 配置在循环内被反复传给导出器，用 Arc 避免每库克隆一份。
    let db_option = Arc::new(db_option_ext.inner.clone());
    let export_started = Instant::now();

    for (dbnum_idx, dbnum) in dbnums.iter().enumerate() {
        log::info!("📦 自动导出 dbnum={} 的 Parquet...", dbnum);
        // 逐库打点，带 `i/n` 进度，长任务时前端才能显示到哪一步了。
        crate::perf_metrics::record_generate_progress(
            "post_gen_export_dbnum_started",
            Some(&format!(
                "dbnum_index={}/{} dbnum={}",
                dbnum_idx + 1,
                dbnums.len(),
                dbnum
            )),
            export_started.elapsed().as_millis() as u64,
        );
        // 导出该库的最新态实例；任一库失败即整轮 Err，不留半套导出物。
        // 后两个位置参数含义：`true` = 同时产出伴生 json 清单；`None` = 不限定 refno 子集，
        // 导全库。返回值第一项（导出统计）此处不用，体积统计统一走第 5 段的目录遍历。
        let (_, output_dir) = export_dbnum_instances_parquet_latest(
            *dbnum,
            &base_output_dir,
            db_option.clone(),
            true,
            None,
        )
        .await
        .map_err(|e| anyhow::anyhow!("Parquet 导出 dbnum={} 失败: {}", dbnum, e))?;

        // 刚落地的 Parquet 立刻回灌空间索引，让 AABB 查询与导出物保持同步。
        // 放在库循环内而不是全部导完再统一刷，是为了让大项目能边导边可查。
        #[cfg(feature = "sqlite-index")]
        {
            use crate::spatial_index::SqliteSpatialIndex;
            use crate::sqlite_index::SqliteAabbIndex;

            let idx_path = SqliteSpatialIndex::default_path();
            // 索引文件可能落在尚不存在的目录下，先建目录再打开。
            if let Some(parent) = idx_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let idx = SqliteAabbIndex::open(&idx_path)?;
            let import_stats = idx.refresh_dbnum_from_parquet_dir(*dbnum, &output_dir)?;
            log::info!(
                "SQLite spatial index refreshed from Parquet: dbnum={}, inserted={}, path={}",
                dbnum,
                import_stats.total_inserted,
                idx_path.display()
            );
        }

        // 未编译索引特性时只提示，不影响导出本身——Parquet 已经是完整交付物。
        #[cfg(not(feature = "sqlite-index"))]
        {
            log::warn!(
                "SQLite spatial index refresh skipped because sqlite-index feature is disabled"
            );
        }
        crate::perf_metrics::record_generate_progress(
            "post_gen_export_dbnum_finished",
            Some(&format!(
                "dbnum_index={}/{} dbnum={} output_dir={}",
                dbnum_idx + 1,
                dbnums.len(),
                dbnum,
                output_dir.display()
            )),
            export_started.elapsed().as_millis() as u64,
        );
    }

    // ---- 第 5 段：统计产物体积并上报 ------------------------------------------------
    // 走一遍产物树而不是累加各库返回值：能把导出器之外写入的伴生文件也算进来，
    // 统计口径与磁盘实际占用一致。
    //
    // 注意这里统计的是**整个根目录**，包含往轮遗留的文件。这是有意为之：报告要回答
    // 「现在磁盘上有多少导出物」，而不是「本轮新写了多少」。
    let mut parquet_files = 0usize;
    let mut parquet_bytes = 0u64;
    let mut json_files = 0usize;
    let mut json_bytes = 0u64;
    for entry in walkdir::WalkDir::new(&base_output_dir)
        .into_iter()
        // 遍历过程中的读取错误直接跳过：统计不准可以接受，为此中断导出不划算。
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        // 取不到元数据时按 0 计，同样是「统计让步于主流程」。
        let len = entry.metadata().map(|m| m.len()).unwrap_or(0);
        // 只区分 parquet 与 json 两类：前者是几何/属性数据，后者是伴生清单。
        match entry.path().extension().and_then(|s| s.to_str()) {
            Some("parquet") => {
                parquet_files += 1;
                parquet_bytes += len;
            }
            Some("json") => {
                json_files += 1;
                json_bytes += len;
            }
            _ => {}
        }
    }
    crate::perf_metrics::record_export_stage(
        parquet_files,
        parquet_bytes,
        json_files,
        json_bytes,
        export_started.elapsed().as_millis() as u64,
    );
    crate::perf_metrics::record_generate_progress(
        "post_gen_export_finished",
        Some(&format!(
            "parquet_files={parquet_files} parquet_bytes={parquet_bytes} json_files={json_files} json_bytes={json_bytes}"
        )),
        export_started.elapsed().as_millis() as u64,
    );

    // 走到这里说明每个库都导成功了，`skipped_reason` 必为 None。
    Ok(PostGenerationParquetExportReport {
        enabled: true,
        exported_dbnums: dbnums,
        output_dir: Some(base_output_dir),
        dbnum_source: Some(dbnum_source.to_string()),
        skipped_reason: None,
    })
}
