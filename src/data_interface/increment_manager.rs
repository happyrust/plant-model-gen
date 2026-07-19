use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use aios_core::pdms_types::*;
use aios_core::pe::SPdmsElement;
use aios_core::tool::db_tool::db1_dehash;
use aios_core::version::{backup_data, backup_owner_relate};
use aios_core::{RefU64Vec, get_db_option};
use aios_core::{clear_all_caches, project_primary_db};
use futures::StreamExt;
use indexmap::{IndexMap, IndexSet};
use itertools::Itertools;
use notify::{RecursiveMode, Watcher};
use parse_pdms_db::parse::parse_db_basic_info;
use pdms_io::defines::DbPageBasicInfo;
use pdms_io::io::{EleOperationData, EleOperationDetail, PdmsIO};
#[cfg(feature = "mqtt")]
use pdms_io::sync::compress::{CompressOptions, execute_compress};
// use pdms_io::sync::compress::{execute_compress, CompressOptions};
use pdms_io::watch::PdmsWatcher;
use petgraph::visit::Walker;
#[cfg(feature = "mqtt")]
use rumqttc::QoS;
use serde::{Deserialize, Serialize};
use tokio::fs::create_dir_all;
use walkdir::WalkDir;

use crate::data_interface::increment_record::IncrGeoUpdateLog;
use crate::data_interface::interface::PdmsDataInterface;
use crate::data_interface::sesno_increment::{
    PdmsIncrementPersistStats, PdmsSesnoCollectedOutcome,
    collect_pdms_increment_for_file_with_operations, persist_collected_pdms_increment_files,
};
use crate::data_interface::tidb_manager::AiosDBManager;
use crate::versioned_db::version_commit::committed_watermark;
#[cfg(feature = "mqtt")]
use crate::mqtt_service::SyncE3dFileMsg;
#[cfg(feature = "web_server")]
use crate::web_server::{
    remote_runtime::REMOTE_RUNTIME,
    remote_sync_handlers,
    sync_control_center::{NewSyncTaskParams, SYNC_CONTROL_CENTER},
};
use parse_pdms_db::parse::DbBasicInfo;

#[cfg(feature = "web_server")]
#[derive(Debug)]
struct GeneratedSyncArtifact {
    path: PathBuf,
    file_name: String,
    file_size: u64,
    file_hash: Option<String>,
    record_count: Option<u64>,
}

#[cfg(feature = "web_server")]
async fn enqueue_generated_sync_tasks(artifacts: Vec<GeneratedSyncArtifact>) {
    if artifacts.is_empty() {
        return;
    }

    let env_id = {
        let runtime_guard = REMOTE_RUNTIME.read().await;
        match runtime_guard.as_ref() {
            Some(state) => state.env_id.clone(),
            None => return,
        }
    };
    let env_id_for_query = env_id.clone();

    let query_result = tokio::task::spawn_blocking(
        move || -> anyhow::Result<(Option<String>, Vec<(String, Option<String>)>)> {
            let conn = remote_sync_handlers::open_sqlite()
                .map_err(|e| anyhow::anyhow!("Failed to open SQLite: {}", e))?;

            let env_name = conn
                .prepare("SELECT name FROM remote_sync_envs WHERE id = ?1 LIMIT 1")?
                .query_row([env_id_for_query.as_str()], |row| row.get::<_, String>(0))
                .ok();

            let mut stmt_sites =
                conn.prepare("SELECT id, name FROM remote_sync_sites WHERE env_id = ?1")?;
            let site_iter = stmt_sites.query_map([env_id_for_query.as_str()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })?;
            let mut sites = Vec::new();
            for item in site_iter {
                sites.push(item?);
            }

            Ok((env_name, sites))
        },
    )
    .await;

    let (env_name, site_entries) = match query_result {
        Ok(Ok(data)) => data,
        Ok(Err(err)) => {
            eprintln!("查询远程同步站点失败: {}", err);
            return;
        }
        Err(err) => {
            eprintln!("查询远程同步站点失败: {}", err);
            return;
        }
    };

    let source_env = get_db_option().location.clone();

    let targets: Vec<(Option<String>, Option<String>)> = if site_entries.is_empty() {
        vec![(None, None)]
    } else {
        site_entries
            .into_iter()
            .map(|(id, name)| (Some(id), name))
            .collect()
    };

    let mut center = SYNC_CONTROL_CENTER.write().await;
    for artifact in artifacts {
        let Some(path_str) = artifact.path.to_str().map(|s| s.to_string()) else {
            continue;
        };
        for (site_id_opt, site_name_opt) in &targets {
            center.add_task(NewSyncTaskParams {
                file_path: path_str.clone(),
                file_size: artifact.file_size,
                priority: 5,
                record_count: artifact.record_count,
                file_name: Some(artifact.file_name.clone()),
                file_hash: artifact.file_hash.clone(),
                env_id: Some(env_id.clone()),
                source_env: Some(source_env.clone()),
                target_site: site_id_opt.clone(),
                direction: Some("UPLOAD".to_string()),
                notes: site_name_opt
                    .clone()
                    .or_else(|| env_name.clone())
                    .map(|name| format!("自动同步 - {}", name)),
            });
        }
    }
}

const JSON_CHUNK_COUNT: usize = 200;

pub const CHECK_DB_TYPES: [&'static str; 6] = ["CATA", "DESI", "DICT", "SYST", "GLB", "GLOB"];

/// 启动补增量 config 门控：环境变量 `AIOS_WATCH_STARTUP_CATCHUP` 取
/// `1`/`true`/`yes`/`on`（大小写不敏感）时开启，默认关闭。
fn startup_catchup_enabled() -> bool {
    std::env::var("AIOS_WATCH_STARTUP_CATCHUP")
        .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

impl AiosDBManager {
    /// 执行增量更新（specs/022：watch 与 CLI 共用同一 Version Commit seam）。
    ///
    /// 对每个 `(path, sesno_range)`：复用 CLI 的采集深模块收集增量操作，
    /// 然后经 `persist_collected_pdms_increment_files` → `commit_version()` 落库——
    /// fingerprint、per-dbnum lease、`sesno_version_anchor` 固化与 Commit Pending
    /// 语义全部由该 seam 提供。本函数不做源观测门（watch 无 manifest），
    /// 也永不自动恢复 pending（恢复保持人工 `incremental-sesno --recover-pending`）。
    ///
    /// # 参数
    ///
    /// * `increment_ranges_map` - 数据库文件路径 → (页面基本信息, 待提交 sesno 区间)；
    ///   区间起点必须是 Committed Watermark + 1（由调用方基于 `committed_watermark` 计算）
    ///
    /// # 返回值
    ///
    /// * `PdmsIncrementPersistStats` - 含已固化锚点 `anchors` 与按 dbnum 的
    ///   `commit_failures`；调用方据此决定哪些 dbnum 可推进 header/通知
    pub async fn execute_incr_update(
        &self,
        increment_ranges_map: IndexMap<PathBuf, (DbPageBasicInfo, RangeInclusive<i32>)>,
    ) -> anyhow::Result<PdmsIncrementPersistStats> {
        let project = get_db_option().project_name.clone();
        let mut collected = PdmsSesnoCollectedOutcome::default();
        for (path, (_basic_info, sesno_range)) in increment_ranges_map {
            println!(
                "[watch-incremental] 采集增量: path={:?}, sesno_range={:?}",
                path, &sesno_range
            );
            // collect_* 的入参是"已缓存 sesno"（= 区间起点 - 1）与目标 sesno
            let cached_sesno = (*sesno_range.start()).max(1) as u32 - 1;
            let target_sesno = (*sesno_range.end()).max(0) as u32;
            let outcome = collect_pdms_increment_for_file_with_operations(
                &project,
                &path,
                cached_sesno,
                Some(target_sesno),
                false,
            )?;
            collected.merge(outcome);
        }

        let stats =
            persist_collected_pdms_increment_files(&collected.files, None, false).await?;
        // persist-only：分类结果仅记录备用，模型生成留给 CLI/IncrementRun
        let update_log = &collected.outcome.update_log;
        if update_log.count() > 0 {
            println!(
                "[watch-incremental] 模型增量分类: prim={} loop_owner={} bran_hanger={} basic_cata={} delete={} (persist-only，不触发模型生成)",
                update_log.prim_refnos.len(),
                update_log.loop_owner_refnos.len(),
                update_log.bran_hanger_refnos.len(),
                update_log.basic_cata_refnos.len(),
                update_log.delete_refnos.len()
            );
        }
        for anchor in &stats.anchors {
            println!(
                "[watch-incremental] Version Anchor 已固化: dbnum={} sesno={} idempotent={} recovered={}",
                anchor.dbnum, anchor.sesno, anchor.idempotent, anchor.recovered
            );
        }
        for failure in &stats.commit_failures {
            eprintln!(
                "[watch-incremental] Version Commit 失败: dbnum={} sesno={}..={} error={} \
                 （保留旧水位等待下次文件事件重试；Commit Pending 需人工 incremental-sesno --recover-pending）",
                failure.dbnum, failure.from_sesno, failure.to_sesno, failure.error
            );
        }
        Ok(stats)
    }

    ///初始化监测
    /// 启动时监测数据文件夹里的文件变化，初始化 headers 与 CBA 归档。
    ///
    /// 注：启动时不做补增量（停机期间落后的区间不在这里追）。若需要
    /// "启动补增量"，应作为独立开关走与 `async_watch` 相同的
    /// `execute_incr_update`（同一 Version Commit seam），而不是旁路实现。
    pub async fn init_watcher(&self) -> anyhow::Result<()> {
        fs::create_dir_all("assets/archives")?;
        let time = Instant::now();
        dbg!(&self.watcher.watch_dirs);
        let db_option = get_db_option();
        let manual_dbnums = db_option.manual_db_nums.clone().unwrap_or_default();
        let exclude_dbnums = db_option.exclude_db_nums.clone().unwrap_or_default();

        for watch_dir in &self.watcher.watch_dirs {
            for entry in WalkDir::new(watch_dir).sort_by(|a, b| {
                let a_len = a.path().metadata().map(|m| m.len()).unwrap_or_default();
                let b_len = b.path().metadata().map(|m| m.len()).unwrap_or_default();
                b_len.cmp(&a_len)
            }) {
                let dir_entry =
                    entry.map_err(|e| anyhow::anyhow!("Failed to get directory entry: {}", e))?;
                let path = dir_entry.path();
                let file_name = path
                    .file_stem()
                    .ok_or_else(|| {
                        anyhow::anyhow!("Failed to get file stem from path: {}", path.display())
                    })?
                    .to_str()
                    .ok_or_else(|| {
                        anyhow::anyhow!("Failed to convert file stem to string: {}", path.display())
                    })?;
                if path.is_dir() {
                    continue;
                }

                let DbBasicInfo {
                    db_type,
                    ses_pgno: _,
                    dbnum,
                } = parse_db_basic_info(path.to_path_buf());
                //是否调试里有筛选
                if !manual_dbnums.is_empty() && !manual_dbnums.contains(&dbnum) {
                    continue;
                }
                //过滤掉排除的数据库编号
                if !exclude_dbnums.is_empty() && exclude_dbnums.contains(&dbnum) {
                    continue;
                }
                if !CHECK_DB_TYPES.contains(&db_type.as_str()) {
                    continue;
                }
                //TODO 这种情况，需要全新的解析
                let Ok(watermark) = committed_watermark(dbnum).await else {
                    //先暂时跳过数据库里没有的文件，todo 考虑自动追加文件全新解析
                    continue;
                };
                // watermark == 0：该 dbnum 从未全量解析过，不属于增量监听范围
                if watermark == 0 {
                    continue;
                }
                let project = get_db_option().project_name.clone();
                self.watcher
                    .file_name_full_path_map
                    .insert(file_name.to_owned(), path.to_path_buf());
                //只有开启异地同步时，才需要初始化异地更新压缩数据包
                #[cfg(feature = "mqtt")]
                {
                    // 初始化CBA的Archive文件，来保证后续增量下载, 后面是否需要加一个环境变量，来控制是否需要重新生成archive文件
                    // 是否需要完全初始化
                    let input = path.to_path_buf();
                    let output: PathBuf = format!("assets/archives/{}.cba", file_name).into();
                    // join_set.spawn(async move {
                    let compress_opt = CompressOptions::new(input, output, "assets/temp");
                    execute_compress(compress_opt)
                        .await
                        .expect("compress failed");
                    // });
                }

                // 初始化监听的headers
                {
                    let mut io = PdmsIO::new(&project, path, true);
                    io.open()?;
                    if let Ok(basic_info) = io.get_page_basic_info() {
                        self.watcher.headers.insert(path.to_path_buf(), basic_info);
                    }
                }
            }
        }

        println!("初始化增量更新耗时: {} s", time.elapsed().as_secs_f32());

        anyhow::Ok(())
    }

    /// 启动补增量：追赶服务停机期间落后的区间。
    ///
    /// config 门控（默认关闭）：仅当环境变量 `AIOS_WATCH_STARTUP_CATCHUP` 为
    /// `1`/`true`/`yes`/`on` 时执行。对每个受监听 db 文件，取 Committed Watermark
    /// 作为已提交起点，若文件最新 sesno 领先则收集 `watermark+1..=file_latest`
    /// 并经 `execute_incr_update`（与 `async_watch` 同一 Version Commit seam）落库。
    /// 安全性由该 seam 的 per-dbnum lease + Commit Pending + 锚点固化兜底：
    /// 提交失败的 dbnum 不产生锚点，下次启动/文件事件重试。
    ///
    /// 应在 `init_watcher`（已初始化 headers）之后、`async_watch` 之前调用。
    pub async fn startup_catchup(&self) -> anyhow::Result<PdmsIncrementPersistStats> {
        if !startup_catchup_enabled() {
            return Ok(PdmsIncrementPersistStats::default());
        }
        let db_option = get_db_option();
        let manual_dbnums = db_option.manual_db_nums.clone().unwrap_or_default();
        let exclude_dbnums = db_option.exclude_db_nums.clone().unwrap_or_default();
        let project = db_option.project_name.clone();

        let mut params: IndexMap<PathBuf, (DbPageBasicInfo, RangeInclusive<i32>)> = IndexMap::new();
        let mut path_by_dbnum: HashMap<u32, PathBuf> = HashMap::new();
        for watch_dir in &self.watcher.watch_dirs {
            for entry in WalkDir::new(watch_dir) {
                let dir_entry =
                    entry.map_err(|e| anyhow::anyhow!("Failed to get directory entry: {}", e))?;
                let path = dir_entry.path();
                if path.is_dir() {
                    continue;
                }
                let DbBasicInfo {
                    db_type, dbnum, ..
                } = parse_db_basic_info(path.to_path_buf());
                if !manual_dbnums.is_empty() && !manual_dbnums.contains(&dbnum) {
                    continue;
                }
                if !exclude_dbnums.is_empty() && exclude_dbnums.contains(&dbnum) {
                    continue;
                }
                if !CHECK_DB_TYPES.contains(&db_type.as_str()) {
                    continue;
                }
                let watermark = match committed_watermark(dbnum).await {
                    Ok(sesno) => sesno,
                    Err(e) => {
                        eprintln!("[startup-catchup] 查询 Committed Watermark 失败 dbnum={dbnum}: {e}");
                        continue;
                    }
                };
                // watermark==0：从未全量解析过，不属于增量追赶范围
                if watermark == 0 {
                    continue;
                }
                let mut io = PdmsIO::new(&project, path, true);
                if io.open().is_err() {
                    continue;
                }
                let file_latest_sesno = io.get_latest_sesno().unwrap_or_default();
                if file_latest_sesno <= watermark {
                    continue;
                }
                let Ok(basic_info) = io.get_page_basic_info() else {
                    continue;
                };
                println!(
                    "[startup-catchup] dbnum={dbnum} 落后: watermark={watermark} file_latest={file_latest_sesno}，追赶区间 {}..={}",
                    watermark + 1,
                    file_latest_sesno
                );
                params.insert(
                    path.to_path_buf(),
                    (basic_info, (watermark as i32 + 1)..=file_latest_sesno as i32),
                );
                path_by_dbnum.insert(dbnum, path.to_path_buf());
            }
        }

        if params.is_empty() {
            println!("[startup-catchup] 无落后 db 文件，跳过");
            return Ok(PdmsIncrementPersistStats::default());
        }
        let stats = self.execute_incr_update(params).await?;
        // 追赶成功的文件推进内存 header，使 async_watch 后续事件从正确基线计算
        for anchor in &stats.anchors {
            let Some(path) = path_by_dbnum.get(&anchor.dbnum) else {
                continue;
            };
            let mut io = PdmsIO::new(&project, path, true);
            if io.open().is_err() {
                continue;
            }
            if let Ok(basic_info) = io.get_page_basic_info() {
                self.watcher.headers.insert(path.clone(), basic_info);
            }
        }
        Ok(stats)
    }

    //开始监测数据文件夹
    pub async fn async_watch(&self) -> notify::Result<()> {
        let (mut watcher, mut rx) = PdmsWatcher::async_watcher()?;
        dbg!(&self.watcher.watch_dirs);
        self.watcher.watch_dirs.iter().for_each(|x| {
            watcher
                .watch(x.as_path(), RecursiveMode::NonRecursive)
                .expect("watch files failed");
        });

        create_dir_all("assets/archives")
            .await
            .map_err(|e| notify::Error::io(e))?;
        create_dir_all("assets/temp")
            .await
            .map_err(|e| notify::Error::io(e))?;
        while let Some(res) = rx.next().await {
            match res {
                Ok(event) => {
                    // dbg!(&event);
                    //跳过只是meta data变动的情况
                    let data_changed = matches!(
                        event.kind,
                        notify::EventKind::Modify(notify::event::ModifyKind::Data(_))
                            | notify::EventKind::Modify(notify::event::ModifyKind::Any)
                            | notify::EventKind::Create(notify::event::CreateKind::File)
                            | notify::EventKind::Remove(notify::event::RemoveKind::File)
                    );
                    if !data_changed {
                        continue;
                    }
                    //后面用派发任务的方式,不要放在这里阻塞
                    println!("changed: {:?}", &event);
                    // 添加调试信息
                    println!("开始扫描数据库头部信息，路径: {:?}", &event.paths);
                    // dbg!(&self.watcher.headers);
                    if let Ok(new_headers) = PdmsWatcher::scan_db_headers(&event.paths) {
                        println!("成功扫描到 {} 个数据库头部", new_headers.len());
                        #[cfg(feature = "web_server")]
                        let mut generated_artifacts: Vec<
                            GeneratedSyncArtifact,
                        > = Vec::new();
                        // 收集本次事件中需要通过 MQTT 推送的文件名和哈希
                        let mut notify_file_names = vec![];
                        let mut notify_file_hashes = vec![];
                        let mut params = IndexMap::new();
                        for (path, new_header) in &new_headers {
                            println!("正在处理路径: {:?}", path);
                            // dbg!(&new_header.pdms_header);
                            // dbg!(path);
                            if let Some(mut old) = self.watcher.headers.get_mut(path) {
                                dbg!(path);
                                dbg!(new_header.latest_ses_data.sesno);
                                #[cfg(feature = "web_server")]
                                let prev_sesno = old.latest_ses_data.sesno;
                                let new_sesno = new_header.latest_ses_data.sesno;

                                // specs/022：以 Committed Watermark（锚点优先）为增量起点，
                                // 而不是缓存 header 或 dbnum_info_table——Commit Pending 时
                                // 后者可能领先锚点，会静默跳过半写区间
                                let db_num = new_header.pdms_header.db_num;
                                let watermark = match committed_watermark(db_num as _).await {
                                    Ok(sesno) => sesno,
                                    Err(e) => {
                                        println!("查询 Committed Watermark 失败: {:?}", e);
                                        continue;
                                    }
                                };
                                // 从未全量解析过的 dbnum 不做增量
                                if watermark == 0 {
                                    continue;
                                }

                                // dbg!(&old.pdms_header);
                                //未发生修改，直接跳过
                                if watermark as i32 >= new_sesno {
                                    continue;
                                }
                                //比如给出准确的范围next_sesno..=end_sesno
                                params.insert(
                                    path.clone(),
                                    (new_header.clone(), (watermark as i32 + 1)..=new_sesno),
                                );
                            } else {
                                println!("watcher.headers: {:?}", self.watcher.headers);
                                println!("在 watcher.headers 中找不到路径: {:?}", path);
                                // 新增文件的处理逻辑：初始化 headers、生成 archive 并准备同步通知
                                self.watcher
                                    .headers
                                    .insert(path.clone(), new_header.clone());

                                let file_name = match path.file_stem().and_then(|s| s.to_str()) {
                                    Some(name) => name,
                                    None => {
                                        println!("无法从新文件路径中解析文件名: {:?}", path);
                                        continue;
                                    }
                                };

                                let dbnum = new_header.pdms_header.db_num as u32;

                                // 如果配置了 location_dbs，则只对本地区负责的 dbnum 发送通知
                                if let Some(location_dbs) = &get_db_option().location_dbs {
                                    if !location_dbs.contains(&dbnum) {
                                        continue;
                                    }
                                }

                                // 为新文件生成对应的 CBA 压缩包，确保远端可以通过 HTTP 下载
                                #[cfg(feature = "mqtt")]
                                let file_hash = {
                                    let output: PathBuf =
                                        format!("assets/archives/{}.cba", file_name).into();
                                    let compress_opt = CompressOptions::new(
                                        path.clone(),
                                        output.clone(),
                                        "assets/temp",
                                    );
                                    let hash = match execute_compress(compress_opt).await {
                                        Ok(h) => h.to_string(),
                                        Err(e) => {
                                            println!(
                                                "新文件压缩生成 CBA 失败: {:?}, 路径: {:?}",
                                                e, path
                                            );
                                            continue;
                                        }
                                    };

                                    #[cfg(feature = "web_server")]
                                    {
                                        let archive_size = std::fs::metadata(&output)
                                            .map(|m: std::fs::Metadata| m.len())
                                            .unwrap_or(0);
                                        generated_artifacts.push(GeneratedSyncArtifact {
                                            path: output.clone(),
                                            file_name: format!("{}.cba", file_name),
                                            file_size: archive_size,
                                            file_hash: Some(hash.clone()),
                                            record_count: None,
                                        });
                                    }

                                    hash
                                };

                                #[cfg(feature = "mqtt")]
                                {
                                    // 避免对已经同步过相同文件 hash 的记录重复发送
                                    let sql = format!(
                                        "select value <string>\
                                        id from (select * from e3d_sync where location != '{}' and '{}' in file_names and '{}' in file_hashes order by timestamp desc) ",
                                        get_db_option().location.as_str(),
                                        file_name,
                                        &file_hash
                                    );
                                    let mut response =
                                        project_primary_db().query(&sql).await.unwrap();
                                    let id = response.take::<Vec<String>>(0).unwrap();
                                    if id.is_empty() {
                                        println!("发现新增 db 文件，推送：{}", &file_name);
                                        notify_file_hashes.push(file_hash);
                                        notify_file_names.push(file_name.to_owned());
                                    }
                                }
                            }
                        }
                        // dbg!(&params);
                        if params.is_empty() {
                            continue;
                        }

                        //如果数据没有发生变化，则不需要推出变化，不需要执行增量
                        match self.execute_incr_update(params).await {
                            Ok(stats) => {
                                // specs/022：提交失败的 dbnum 不推进 header、不推送同步——
                                // Committed Watermark 未前移，下次文件事件会重试同一区间；
                                // Commit Pending 需人工 incremental-sesno --recover-pending
                                let failed_dbnums: HashSet<u32> = stats
                                    .commit_failures
                                    .iter()
                                    .map(|failure| failure.dbnum)
                                    .collect();
                                //执行没问题了，再更新当前的版本记录，headers直接存本地json
                                for (path, new_header) in new_headers {
                                    let file_name = path.file_stem().unwrap().to_str().unwrap();
                                    let dbnum = new_header.pdms_header.db_num as u32;
                                    if path.is_dir() {
                                        continue;
                                    }
                                    if failed_dbnums.contains(&dbnum) {
                                        println!(
                                            "[watch-incremental] dbnum={} 本次 Version Commit 失败，保留旧 header 等待重试",
                                            dbnum
                                        );
                                        continue;
                                    }
                                    // dbg!(&file_name);
                                    //这个地方是不是需要直接去读取文件，然后更新headers，不能太依赖json数据
                                    //或者每次启动都重新更新这个文件？
                                    if let Some(mut old) = self.watcher.headers.get_mut(&path) {
                                        // dbg!((
                                        //     old.latest_ses_data.sesno,
                                        //     new_header.latest_ses_data.sesno
                                        // ));
                                        //未发生修改，直接跳过
                                        let prev_sesno = old.latest_ses_data.sesno;
                                        let new_sesno = new_header.latest_ses_data.sesno;
                                        if old.latest_ses_data.sesno >= new_sesno {
                                            continue;
                                        }
                                        *old.value_mut() = new_header;

                                        //发生修改的文件，重新生成archive
                                        // dbg!(&path);
                                        let output: PathBuf =
                                            format!("assets/archives/{}.cba", file_name).into();
                                        // dbg!(&output);

                                        #[cfg(feature = "mqtt")]
                                        let file_hash = {
                                            let compress_opt = CompressOptions::new(
                                                path.clone(),
                                                output.clone(),
                                                "assets/temp",
                                            );
                                            execute_compress(compress_opt)
                                                .await
                                                .unwrap()
                                                .to_string()
                                        };
                                        #[cfg(not(feature = "mqtt"))]
                                        let file_hash = String::new();
                                        // dbg!(&file_hash);
                                        #[cfg(feature = "web_server")]
                                        {
                                            let archive_size = std::fs::metadata(&output)
                                                .map(|m: std::fs::Metadata| m.len())
                                                .unwrap_or(0);
                                            let delta = new_sesno.saturating_sub(prev_sesno) as u64;
                                            generated_artifacts.push(GeneratedSyncArtifact {
                                                path: output.clone(),
                                                file_name: format!("{file_name}.cba"),
                                                file_size: archive_size,
                                                file_hash: Some(file_hash.clone()),
                                                record_count: if delta > 0 {
                                                    Some(delta)
                                                } else {
                                                    None
                                                },
                                            });
                                        }

                                        //如果location_dbs为空，则不进行筛选
                                        //说明是所有地区都推送，跳过检查
                                        //必须要是地区对应的dbnos才能推送
                                        if let Some(location_dbs) = &get_db_option().location_dbs {
                                            if !location_dbs.contains(&dbnum) {
                                                continue;
                                            }
                                        }

                                        //数据库里不存在这个file hash的记录，才需要发送
                                        //是自己创建的，在记录里还没有的，才能发送消息出去
                                        //如果是别的创建的，就应该调过
                                        let sql = format!(
                                            "select value <string>\
                                            id from (select * from e3d_sync where location != '{}' and '{}' in file_names and '{}' in file_hashes order by timestamp desc) ",
                                            get_db_option().location.as_str(),
                                            file_name,
                                            &file_hash
                                        );
                                        // dbg!(&sql);
                                        // println!("sql is {}", &sql);
                                        let mut response =
                                            project_primary_db().query(&sql).await.unwrap();
                                        // dbg!(&response);
                                        let id = response.take::<Vec<String>>(0).unwrap();
                                        // dbg!(id.len());
                                        if id.is_empty() {
                                            println!("发生了增量更新，推送：{}", &file_name);
                                            notify_file_hashes.push(file_hash);
                                            notify_file_names.push(file_name.to_owned());
                                        }
                                    }
                                }
                                //now save the watch.json
                                // self.watcher.save(None).expect("save watch.json failed");
                            }
                            Err(e) => {
                                eprintln!("[watch-incremental] 增量执行失败: {:?}", e);
                            }
                        }
                        //publish notify db file updates
                        dbg!(&notify_file_names);
                        #[cfg(feature = "mqtt")]
                        if !notify_file_names.is_empty() {
                            let payload =
                                SyncE3dFileMsg::new(notify_file_names, notify_file_hashes);
                            //自己本地也要保存
                            // todo 后续还是要配置哪些dbs，哪个地方能修改，哪个地方是不能改的
                            project_primary_db()
                                .query(format!(
                                    "INSERT IGNORE INTO e3d_sync {} ",
                                    serde_json::to_string(&payload).unwrap()
                                ))
                                .await
                                .unwrap();
                            //todo 检查是否只是发生了claim page的变化，如果只是claim修改，是需要每次都同步？
                            //会导致出现循环
                            self.mqtt_client
                                .clone()
                                .publish("Sync/E3d", QoS::ExactlyOnce, true, payload)
                                .await
                                .unwrap();
                        }
                        #[cfg(feature = "web_server")]
                        enqueue_generated_sync_tasks(generated_artifacts).await;
                    } else {
                        println!("扫描数据库头部失败，错误路径: {:?}", &event.paths);
                    }
                }
                Err(e) => println!("watch error: {:?}", e),
            }
        }

        Ok(())
    }
}
