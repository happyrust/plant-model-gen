//! MQTT 仅承载源 db 文件分发，不参与增量提交。
//!
//! 增量检测和落库统一由 `version_management::watch_incremental` 轮询完成；本模块
//! 独立维护文件名索引、CBA 压缩、MQTT 收发与远端 clone。
//!
//! 收发两端的台账写 SQLite（`sync_ledger`），不碰 SurrealDB；收包端 clone 之后对着消息里的
//! `file_hashes` / `file_sesnos` 做 hash 与 e3d-io 校验，结果同样进台账。

use std::sync::Arc;

use once_cell::sync::Lazy;
use pdms_io::watch::PdmsWatcher;
use tokio::sync::Mutex;

/// MQTT 连接状态，供 web 状态页读取。
pub static MQTT_CONNECT_STATUS: Lazy<Mutex<Option<bool>>> = Lazy::new(|| Mutex::new(None));

/// 为 MQTT clone 建立文件名到本地路径的索引。
pub async fn initialize_file_index(watcher: &PdmsWatcher) -> anyhow::Result<()> {
    watcher.init_local_watcher().await
}

/// 一次广播里的一个源文件，以及随它进台账的信息。
///
/// 完整站点（`watch_incremental`）只填 `path` + `sesno`；中继轮询（P3）还会带上广播前水位、
/// e3d-io 读到的 sesno 与会话 diff。
#[cfg(feature = "mqtt")]
#[derive(Debug, Clone)]
pub struct PublishSourceFile {
    pub path: std::path::PathBuf,
    /// 该文件此刻的 latest sesno；写进 `SyncE3dFileMsg.file_sesnos`（None → 0 = 未声明）。
    pub sesno: Option<u32>,
    /// 广播前水位（台账 `sesno_from`）。
    pub sesno_from: Option<u32>,
    /// e3d-io 实际读到的 latest sesno（台账 `sesno_seen`）。
    pub sesno_seen: Option<u32>,
    /// e3d-io 会话 diff 计数；None 且 `diff_status` 为 None 时台账记 `diff_status = skipped`。
    pub diff: Option<super::sync_ledger::DiffTally>,
    pub diff_status: Option<super::sync_ledger::DiffStatus>,
    /// 变更清单（RefNo 级），进 `e3d_sync_changes`。
    pub changes: Vec<super::sync_ledger::ChangeEntry>,
    /// 台账 `verify_detail`：中继轮询用它记 diff 判不了的原因、清单截断（`changes_truncated:<总数>`）等。
    pub verify_detail: Option<String>,
}

#[cfg(feature = "mqtt")]
impl PublishSourceFile {
    pub fn new(path: std::path::PathBuf, sesno: Option<u32>) -> Self {
        Self {
            path,
            sesno,
            sesno_from: None,
            sesno_seen: None,
            diff: None,
            diff_status: None,
            changes: Vec::new(),
            verify_detail: None,
        }
    }
}

#[cfg(feature = "mqtt")]
pub struct MqttFilePublisher {
    client: rumqttc::AsyncClient,
    event_loop: tokio::task::JoinHandle<()>,
}

#[cfg(feature = "mqtt")]
impl MqttFilePublisher {
    pub fn start() -> Self {
        use crate::mqtt_service::new_mqtt_inst;

        let db_option = aios_core::get_db_option();
        let mut mqtt = new_mqtt_inst(&format!(
            "{}-{}-file-pub",
            db_option.location, db_option.project_code
        ));
        let client = mqtt.client.clone();
        let event_loop = tokio::spawn(async move {
            loop {
                match mqtt.el.poll().await {
                    Ok(rumqttc::Event::Incoming(rumqttc::Packet::ConnAck(_))) => {
                        *MQTT_CONNECT_STATUS.lock().await = Some(true);
                    }
                    Ok(_) => {}
                    Err(error) => {
                        *MQTT_CONNECT_STATUS.lock().await = Some(false);
                        log::error!("MQTT 文件发布连接异常: {error}");
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    }
                }
            }
        });
        Self { client, event_loop }
    }

    /// 压缩并发布本轮已成功提交（或中继轮询判定有变更）的源 db 文件。
    ///
    /// 发布成功后每个文件写一行 `outbound` 台账（SQLite，失败只 warn）；发布失败按原样上抛、不记台账。
    pub async fn publish_source_files(&self, files: &[PublishSourceFile]) -> anyhow::Result<()> {
        use super::sync_ledger::{self, DiffStatus, Direction, LedgerRow, VerifyStatus};
        use pdms_io::sync::compress::{CompressOptions, execute_compress};
        use rumqttc::QoS;

        if files.is_empty() {
            return Ok(());
        }
        tokio::fs::create_dir_all("assets/archives").await?;
        tokio::fs::create_dir_all("assets/temp").await?;

        let mut file_names = Vec::new();
        let mut file_hashes = Vec::new();
        let mut file_sesnos = Vec::new();
        let mut published: Vec<&PublishSourceFile> = Vec::new();
        for file in files {
            let Some(file_name) = file.path.file_stem().and_then(|value| value.to_str()) else {
                continue;
            };
            let output = std::path::PathBuf::from(format!("assets/archives/{file_name}.cba"));
            let hash = execute_compress(CompressOptions::new(
                file.path.clone(),
                output,
                "assets/temp",
            ))
            .await?
            .to_string();
            file_names.push(file_name.to_string());
            file_hashes.push(hash);
            file_sesnos.push(file.sesno.unwrap_or(0));
            published.push(file);
        }
        if file_names.is_empty() {
            return Ok(());
        }

        let mut payload = crate::mqtt_service::SyncE3dFileMsg::new(file_names, file_hashes);
        payload.file_sesnos = file_sesnos;
        // 显式序列化一次：发布的字节与算 msg_id 的字节必须是同一份，订阅端才能对得上。
        let bytes = serde_json::to_vec(&payload)?;
        let msg_id = sync_ledger::msg_id_from_bytes(&bytes);
        self.client
            .publish("Sync/E3d", QoS::ExactlyOnce, true, bytes)
            .await?;

        let msg_timestamp = payload.timestamp.to_rfc3339();
        let rows = published
            .iter()
            .enumerate()
            .map(|(index, file)| {
                let mut row = LedgerRow::new(
                    msg_id.clone(),
                    Direction::Outbound,
                    payload.location.clone(),
                    payload.file_names[index].clone(),
                    VerifyStatus::Ok,
                );
                row.file_hash = Some(payload.file_hashes[index].clone());
                row.sesno_from = file.sesno_from;
                row.sesno_to = file.sesno;
                row.sesno_seen = file.sesno_seen;
                row.diff = file.diff;
                row.diff_status = Some(file.diff_status.unwrap_or(DiffStatus::Skipped));
                row.verify_detail = file.verify_detail.clone();
                row.msg_timestamp = Some(msg_timestamp.clone());
                row.changes = file.changes.clone();
                row
            })
            .collect();
        sync_ledger::record(rows).await;
        Ok(())
    }
}

#[cfg(feature = "mqtt")]
impl Drop for MqttFilePublisher {
    fn drop(&mut self) {
        self.event_loop.abort();
    }
}

/// 将远端 CBA 增量 clone 到本地源 db 文件，并逐文件校验、记 `inbound` 台账。
///
/// `msg_id` 是这条消息的分组键（订阅端用收到的原始字节算 `sync_ledger::msg_id_from_bytes`；
/// 手工触发没有原始字节时用 `sync_ledger::msg_id_for`）。消息里的每个文件都会落一行台账，
/// 包括被跳过（未知本地文件 / 本站自有库）与 clone 失败的；台账写失败只 warn。
#[cfg(feature = "mqtt")]
pub async fn exec_delta_clone_remotes(
    watcher: &PdmsWatcher,
    sync_msg: crate::mqtt_service::SyncE3dFileMsg,
    msg_id: &str,
) -> anyhow::Result<bool> {
    if sync_msg.file_names.is_empty() {
        return Ok(false);
    }
    let mut rows = Vec::with_capacity(sync_msg.file_names.len());
    let result = clone_and_verify_files(watcher, &sync_msg, msg_id, &mut rows).await;
    super::sync_ledger::record(rows).await;
    result.map(|_| true)
}

/// `exec_delta_clone_remotes` 的主体：clone → 校验 → 攒台账行。任一文件 clone 失败即返回
/// （与改造前一致），失败前已处理的文件与失败本身都留在 `rows` 里。
#[cfg(feature = "mqtt")]
async fn clone_and_verify_files(
    watcher: &PdmsWatcher,
    sync_msg: &crate::mqtt_service::SyncE3dFileMsg,
    msg_id: &str,
    rows: &mut Vec<super::sync_ledger::LedgerRow>,
) -> anyhow::Result<()> {
    use super::sync_ledger::{self, Direction, LedgerRow, VerifyStatus};
    use pdms_io::sync::clone::{CloneOptions, execute_clone};

    let db_option = aios_core::get_db_option();
    let remote_url = sync_msg.file_server_host.as_str();
    let msg_timestamp = sync_msg.timestamp.to_rfc3339();
    let new_row = |index: usize, status: VerifyStatus, detail: Option<String>| {
        let mut row = LedgerRow::new(
            msg_id,
            Direction::Inbound,
            sync_msg.location.clone(),
            sync_msg.file_names[index].clone(),
            status,
        );
        row.file_hash = sync_msg.declared_hash(index).map(str::to_string);
        row.sesno_to = sync_msg.declared_sesno(index);
        row.verify_detail = detail;
        row.msg_timestamp = Some(msg_timestamp.clone());
        row
    };

    for (index, file_name) in sync_msg.file_names.iter().enumerate() {
        let url = format!("{remote_url}/{file_name}.cba");
        let Some(path) = watcher
            .file_name_full_path_map
            .get(file_name)
            .map(|entry| entry.value().clone())
        else {
            log::warn!("MQTT 文件同步跳过未知本地 db 文件: {file_name}");
            rows.push(new_row(
                index,
                VerifyStatus::Skipped,
                Some("unknown_local_file: 本站文件索引里没有这个 db 文件".to_string()),
            ));
            continue;
        };
        let dbnum = watcher.get_dbno(&path);
        if let Some(dbnum) = dbnum
            && db_option
                .location_dbs
                .as_ref()
                .is_some_and(|dbnums| dbnums.contains(&dbnum))
        {
            rows.push(new_row(
                index,
                VerifyStatus::Skipped,
                Some(format!(
                    "own_location_db: dbnum={dbnum} 在本站 location_dbs 内，不接受远端覆盖"
                )),
            ));
            continue;
        }

        let started = std::time::Instant::now();
        let updated = match execute_clone(CloneOptions::new_remote(&url, &path)).await {
            Ok(updated) => updated,
            Err(error) => {
                rows.push(new_row(
                    index,
                    VerifyStatus::CloneFailed,
                    Some(format!("clone_failed: {error:#} (url={url})")),
                ));
                return Err(error);
            }
        };
        log::info!(
            "MQTT clone {} updated={} cost={:.3}s",
            file_name,
            updated,
            started.elapsed().as_secs_f64()
        );

        // clone 之后：hash（消息 file_hashes[i]）→ e3d-io 打开 → sesno（消息 file_sesnos[i]）
        let verdict = sync_ledger::verify_cloned_file(
            path.clone(),
            sync_msg.declared_hash(index).map(str::to_string),
            sync_msg.declared_sesno(index),
        )
        .await;
        match verdict.verify_status {
            VerifyStatus::Ok => log::info!(
                "MQTT 收包校验通过 {file_name}: sesno={}",
                verdict.sesno_seen.unwrap_or(0)
            ),
            VerifyStatus::Skipped => log::info!(
                "MQTT 收包校验部分跳过 {file_name}: {}",
                verdict.verify_detail.as_deref().unwrap_or("")
            ),
            _ => log::warn!(
                "MQTT 收包校验未通过 {file_name}: {} {}",
                verdict.verify_status.as_str(),
                verdict.verify_detail.as_deref().unwrap_or("")
            ),
        }

        // 盘上是一个能打开的库且没有硬性不一致 → 把水位推到实际 sesno，防止本站中继轮询把刚收到的文件再广播出去。
        if verdict.file_is_trustworthy()
            && let (Some(dbnum), Some(sesno_seen)) = (dbnum, verdict.sesno_seen)
        {
            sync_ledger::upsert_watermark(dbnum, file_name.clone(), sesno_seen).await;
        }

        let mut detail = verdict.verify_detail.clone();
        if !updated {
            let note = "clone reported updated=false";
            detail = Some(match detail {
                Some(existing) => format!("{existing}; {note}"),
                None => note.to_string(),
            });
        }
        let mut row = new_row(index, verdict.verify_status, detail);
        row.sesno_seen = verdict.sesno_seen;
        rows.push(row);
    }
    Ok(())
}

#[cfg(feature = "mqtt")]
pub async fn poll_sync_e3d_mqtt_events(watcher: Arc<PdmsWatcher>) {
    poll_sync_e3d_mqtt_events_with_backoff(watcher, 1_000, 30_000).await;
}

/// 订阅源 db 文件通知；clone 完成后由统一轮询 runner 在下一轮发现 sesno 增长。
#[cfg(feature = "mqtt")]
pub async fn poll_sync_e3d_mqtt_events_with_backoff(
    watcher: Arc<PdmsWatcher>,
    initial_backoff_ms: u64,
    max_backoff_ms: u64,
) {
    use crate::mqtt_service::{SyncE3dFileMsg, new_mqtt_inst};
    use rumqttc::{Event::Incoming, Packet, QoS};

    let db_option = aios_core::get_db_option();
    let location = db_option.location.clone();
    let mut backoff = initial_backoff_ms.max(100);
    let max_backoff = max_backoff_ms.max(backoff);
    loop {
        let mut mqtt = new_mqtt_inst(&format!(
            "{}-{}-file-sub",
            db_option.location, db_option.project_code
        ));
        let _ = mqtt.client.subscribe("Sync/E3d", QoS::ExactlyOnce).await;

        loop {
            match mqtt.el.poll().await {
                Ok(Incoming(Packet::Publish(message))) => {
                    // 坏包不再 panic 掉整个订阅循环：记日志、跳过这一条。
                    let sync_message: SyncE3dFileMsg =
                        match serde_json::from_slice(&message.payload) {
                            Ok(parsed) => parsed,
                            Err(error) => {
                                log::warn!(
                                    "MQTT Sync/E3d 消息无法解析，已跳过（{} 字节）: {error}",
                                    message.payload.len()
                                );
                                continue;
                            }
                        };
                    if sync_message.location != location {
                        // 台账写 SQLite（在 exec_delta_clone_remotes 内逐文件落行），不再写 SurrealDB。
                        let msg_id = super::sync_ledger::msg_id_from_bytes(&message.payload);
                        if let Err(error) =
                            exec_delta_clone_remotes(&watcher, sync_message, &msg_id).await
                        {
                            log::error!("MQTT 文件 clone 失败: {error:#}");
                        }
                    }
                    backoff = initial_backoff_ms.max(100);
                    *MQTT_CONNECT_STATUS.lock().await = Some(true);
                }
                Ok(_) => {}
                Err(error) => {
                    *MQTT_CONNECT_STATUS.lock().await = Some(false);
                    log::error!("MQTT 文件订阅连接异常: {error}");
                    break;
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(backoff)).await;
        backoff = backoff.saturating_mul(2).min(max_backoff);
    }
}
