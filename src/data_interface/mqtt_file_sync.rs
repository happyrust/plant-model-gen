//! MQTT 仅承载源 db 文件分发，不参与增量提交。
//!
//! 增量检测和落库统一由 `version_management::watch_incremental` 轮询完成；本模块
//! 独立维护文件名索引、CBA 压缩、MQTT 收发与远端 clone。

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

    /// 压缩并发布本轮已成功提交的源 db 文件。
    pub async fn publish_source_files(
        &self,
        source_files: &[std::path::PathBuf],
    ) -> anyhow::Result<()> {
        use aios_core::project_primary_db;
        use pdms_io::sync::compress::{CompressOptions, execute_compress};
        use rumqttc::QoS;

        if source_files.is_empty() {
            return Ok(());
        }
        tokio::fs::create_dir_all("assets/archives").await?;
        tokio::fs::create_dir_all("assets/temp").await?;

        let mut file_names = Vec::new();
        let mut file_hashes = Vec::new();
        for source_file in source_files {
            let Some(file_name) = source_file.file_stem().and_then(|value| value.to_str()) else {
                continue;
            };
            let output = std::path::PathBuf::from(format!("assets/archives/{file_name}.cba"));
            let hash = execute_compress(CompressOptions::new(
                source_file.clone(),
                output,
                "assets/temp",
            ))
            .await?
            .to_string();
            file_names.push(file_name.to_string());
            file_hashes.push(hash);
        }
        if file_names.is_empty() {
            return Ok(());
        }

        let payload = crate::mqtt_service::SyncE3dFileMsg::new(file_names, file_hashes);
        let mut response = project_primary_db()
            .query(format!(
                "INSERT INTO e3d_sync {};",
                serde_json::to_string(&payload)?
            ))
            .await?;
        response.check()?;
        self.client
            .publish("Sync/E3d", QoS::ExactlyOnce, true, payload)
            .await?;
        Ok(())
    }
}

#[cfg(feature = "mqtt")]
impl Drop for MqttFilePublisher {
    fn drop(&mut self) {
        self.event_loop.abort();
    }
}

/// 将远端 CBA 增量 clone 到本地源 db 文件。
#[cfg(feature = "mqtt")]
pub async fn exec_delta_clone_remotes(
    watcher: &PdmsWatcher,
    sync_msg: crate::mqtt_service::SyncE3dFileMsg,
) -> anyhow::Result<bool> {
    use pdms_io::sync::clone::{CloneOptions, execute_clone};

    if sync_msg.file_names.is_empty() {
        return Ok(false);
    }
    let db_option = aios_core::get_db_option();
    let remote_url = sync_msg.file_server_host.as_str();
    for file_name in &sync_msg.file_names {
        let url = format!("{remote_url}/{file_name}.cba");
        let Some(path) = watcher
            .file_name_full_path_map
            .get(file_name)
            .map(|entry| entry.value().clone())
        else {
            log::warn!("MQTT 文件同步跳过未知本地 db 文件: {file_name}");
            continue;
        };
        if let Some(dbnum) = watcher.get_dbno(&path)
            && db_option
                .location_dbs
                .as_ref()
                .is_some_and(|dbnums| dbnums.contains(&dbnum))
        {
            continue;
        }
        let started = std::time::Instant::now();
        let updated = execute_clone(CloneOptions::new_remote(&url, path)).await?;
        log::info!(
            "MQTT clone {} updated={} cost={:.3}s",
            file_name,
            updated,
            started.elapsed().as_secs_f64()
        );
    }
    Ok(true)
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
                    let sync_message = SyncE3dFileMsg::from(message.payload.to_vec());
                    if sync_message.location != location {
                        if let Ok(mut response) = aios_core::project_primary_db()
                            .query(format!(
                                "INSERT INTO e3d_sync {};",
                                serde_json::to_string(&sync_message).unwrap_or_default()
                            ))
                            .await
                        {
                            let _ = response.check();
                        }
                        if let Err(error) = exec_delta_clone_remotes(&watcher, sync_message).await {
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
