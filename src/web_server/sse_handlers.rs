// SSE (Server-Sent Events) 事件流处理器

use axum::{
    http::StatusCode,
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use std::time::SystemTime;
use tokio_stream::wrappers::BroadcastStream;

use crate::web_server::sync_control_center::SYNC_EVENT_TX;

/// SSE 事件类型
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum SyncEvent {
    Started {
        env_id: String,
        timestamp: String,
    },
    Stopped {
        env_id: String,
        timestamp: String,
    },
    Paused {
        env_id: String,
        timestamp: String,
    },
    Resumed {
        env_id: String,
        timestamp: String,
    },
    SyncStarted {
        task_id: String,
        file_path: String,
        file_size: u64,
        timestamp: String,
    },
    SyncProgress {
        task_id: String,
        progress: u8,
        timestamp: String,
    },
    SyncCompleted {
        task_id: String,
        file_path: String,
        duration_ms: u64,
        timestamp: String,
    },
    SyncFailed {
        task_id: String,
        file_path: String,
        error: String,
        timestamp: String,
    },
    MqttConnected {
        env_id: String,
        timestamp: String,
    },
    MqttDisconnected {
        env_id: String,
        reason: String,
        timestamp: String,
    },
    QueueSizeChanged {
        env_id: String,
        queue_size: u32,
        timestamp: String,
    },
    MetricsUpdated {
        env_id: String,
        metrics: serde_json::Value,
        timestamp: String,
    },
    ConnectionChanged {
        mqtt_connected: bool,
        watcher_active: bool,
        timestamp: String,
    },
    ProgressUpdate {
        total: u64,
        completed: u64,
        failed: u64,
        pending: u64,
        timestamp: String,
    },
    Alert {
        level: String,
        message: String,
        timestamp: String,
    },
    /// MQTT 订阅 / 主从角色状态变更（B4）
    ///
    /// 触发时机：
    /// - `set_as_master_node` / `set_as_client_node` 写盘成功
    /// - `start_mqtt_subscription_api` / `stop_mqtt_subscription_api` 成功
    ///
    /// 字段含义与 `GET /api/mqtt/subscription/status` 对齐，便于前端
    /// `MqttNodesView` / `LogsView` 收到事件后直接 reload 状态。
    MqttSubscriptionStatusChanged {
        is_running: bool,
        is_master_node: bool,
        location: String,
        timestamp: String,
    },
}

impl SyncEvent {
    /// 获取当前时间戳字符串
    fn now() -> String {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .to_string()
    }
}

/// SSE 事件流处理器
///
/// GET /api/sync/events
///
/// 返回 Server-Sent Events 流，实时推送同步事件。
///
/// C5 · 修 G5 BroadcastStream 漏首事件：
/// 在 broadcast subscribe 之后、live 流开始之前，**立即**用一条
/// "MQTT 订阅状态快照"作为首事件喂给前端。即使 sender 端在 listener
/// 真正进入 await 之前就 send 完导致 broadcast lag、漏掉首条事件，
/// 前端也能从这条 snapshot 拿到当前正确状态，整体仍是最终一致的。
pub async fn sync_events_handler() -> impl IntoResponse {
    let rx = SYNC_EVENT_TX.subscribe();

    let live_stream = BroadcastStream::new(rx).filter_map(|result| async move {
        match result {
            Ok(event) => match serde_json::to_string(&event) {
                Ok(json) => Some(Ok::<_, Infallible>(
                    Event::default().data(json).event("message"),
                )),
                Err(e) => {
                    eprintln!("Failed to serialize SSE event: {}", e);
                    None
                }
            },
            Err(e) => {
                eprintln!("SSE broadcast error: {}", e);
                None
            }
        }
    });

    let initial_event = build_initial_mqtt_status_snapshot()
        .await
        .and_then(|event| serde_json::to_string(&event).ok())
        .map(|json| Ok::<_, Infallible>(Event::default().data(json).event("message")));
    let initial_stream = stream::iter(initial_event.into_iter());

    let event_stream = initial_stream.chain(live_stream);
    Sse::new(event_stream).keep_alive(KeepAlive::default())
}

/// C5 · 构造"当前 MQTT 订阅状态快照"事件，复用 push_subscription_status_event
/// 的字段计算口径（is_running / is_master_node / location / timestamp）。
///
/// 失败（拿不到 location 等）时返回 None，调用方略过初始事件即可，
/// 不影响 live 流。
async fn build_initial_mqtt_status_snapshot() -> Option<SyncEvent> {
    use crate::web_server::{mqtt_monitor_handlers, remote_runtime};

    let db_option = aios_core::get_db_option();
    let location = db_option.location.trim().to_string();
    if location.is_empty() {
        return None;
    }

    let is_running = remote_runtime::REMOTE_RUNTIME.read().await.is_some();
    let db_path = mqtt_monitor_handlers::get_node_config_db_path();
    let is_master_node = mqtt_monitor_handlers::check_is_master_node(&location, &db_path);
    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_default();

    Some(SyncEvent::MqttSubscriptionStatusChanged {
        is_running,
        is_master_node,
        location,
        timestamp,
    })
}

/// 测试 SSE 连接的处理器
///
/// GET /api/sync/events/test
///
/// 发送测试事件以验证 SSE 连接
pub async fn test_sse_handler() -> impl IntoResponse {
    let test_event = SyncEvent::Started {
        env_id: "test-env".to_string(),
        timestamp: SyncEvent::now(),
    };

    // 发送测试事件
    match SYNC_EVENT_TX.send(test_event) {
        Ok(_) => (StatusCode::OK, "Test event sent"),
        Err(e) => {
            eprintln!("Failed to send test event: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to send test event",
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_event_serialization() {
        let event = SyncEvent::SyncStarted {
            task_id: "task-123".to_string(),
            file_path: "/path/to/file.cba".to_string(),
            file_size: 1024000,
            timestamp: "1234567890".to_string(),
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("SyncStarted"));
        assert!(json.contains("task-123"));
    }
}
