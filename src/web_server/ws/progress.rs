//! WebSocket 进度推送模块
//!
//! 提供基于 WebSocket 的实时任务进度推送功能

use crate::shared::{ProgressHub, ProgressMessage};
use crate::web_server::AppState;
use axum::{
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

/// WebSocket 错误响应
#[derive(Debug, Clone, Serialize)]
struct WsErrorMessage {
    r#type: String,
    error: String,
    task_id: Option<String>,
}

/// WebSocket 握手成功响应
#[derive(Debug, Clone, Serialize)]
struct WsHandshakeMessage {
    r#type: String,
    task_id: String,
    message: String,
    current_state: Option<ProgressMessage>,
}

/// 订阅单个任务进度的 WebSocket 端点
///
/// 路由: `/ws/progress/:task_id`
///
/// # 握手流程
///
/// 1. 客户端发起 WebSocket 连接
/// 2. 服务器检查任务是否存在
/// 3. 如果存在，发送当前状态 + 握手成功消息
/// 4. 如果不存在，发送错误消息并关闭连接
///
/// # 推送流程
///
/// 1. 订阅任务的进度广播通道
/// 2. 持续推送进度更新
/// 3. 客户端断开或任务完成时自动清理
pub async fn ws_progress_handler(
    ws: WebSocketUpgrade,
    Path(task_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    info!("WebSocket 连接请求: task_id={}", task_id);
    let hub = state.progress_hub.clone();
    ws.on_upgrade(move |socket| handle_progress_socket(socket, task_id, hub))
}

/// 处理单个任务进度的 WebSocket 连接
async fn handle_progress_socket(socket: WebSocket, task_id: String, hub: Arc<ProgressHub>) {
    let (mut sender, mut receiver) = socket.split();

    // 检查任务是否存在
    let current_state = hub.get_task_state(&task_id);

    // 发送握手消息
    let handshake_msg = if let Some(ref state) = current_state {
        WsHandshakeMessage {
            r#type: "handshake".to_string(),
            task_id: task_id.clone(),
            message: "连接成功，开始推送进度".to_string(),
            current_state: Some(state.clone()),
        }
    } else {
        warn!("任务 {} 不存在，将自动创建订阅", task_id);
        WsHandshakeMessage {
            r#type: "handshake".to_string(),
            task_id: task_id.clone(),
            message: "任务尚未开始，等待进度更新".to_string(),
            current_state: None,
        }
    };

    if let Ok(json) = serde_json::to_string(&handshake_msg) {
        if sender.send(Message::Text(json.into())).await.is_err() {
            error!("发送握手消息失败，客户端已断开");
            return;
        }
    }

    // 订阅任务进度
    let mut progress_rx = hub.subscribe(&task_id);

    debug!("开始推送任务 {} 的进度更新", task_id);

    // 使用 tokio::select! 同时处理两个异步流
    loop {
        tokio::select! {
            // 处理来自客户端的消息（心跳、关闭等）
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => {
                        info!("客户端主动关闭 WebSocket 连接: task_id={}", task_id);
                        break;
                    }
                    Some(Ok(Message::Ping(data))) => {
                        debug!("收到客户端 Ping");
                        if sender.send(Message::Pong(data)).await.is_err() {
                            warn!("发送 Pong 失败，连接可能已断开");
                            break;
                        }
                    }
                    Some(Ok(Message::Text(text))) => {
                        debug!("收到客户端消息: {}", text);
                        // 可以处理客户端命令，例如暂停/恢复推送
                    }
                    Some(Err(e)) => {
                        error!("接收 WebSocket 消息出错: {}", e);
                        break;
                    }
                    _ => {}
                }
            }

            // 处理进度更新
            progress = progress_rx.recv() => {
                match progress {
                    Ok(msg) => {
                        // 序列化并发送进度消息
                        match serde_json::to_string(&msg) {
                            Ok(json) => {
                                if sender.send(Message::Text(json.into())).await.is_err() {
                                    warn!("发送进度消息失败，客户端可能已断开");
                                    break;
                                }

                                // 如果任务已完成，发送完成消息后关闭连接
                                if matches!(msg.status, crate::shared::TaskStatus::Completed | crate::shared::TaskStatus::Failed | crate::shared::TaskStatus::Cancelled) {
                                    info!("任务 {} 已结束 (状态: {:?})，关闭连接", task_id, msg.status);
                                    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                                    let _ = sender.send(Message::Close(None)).await;
                                    break;
                                }
                            }
                            Err(e) => {
                                error!("序列化进度消息失败: {}", e);
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!("客户端处理过慢，跳过了 {} 条消息", skipped);
                        // 发送警告消息
                        let warning = serde_json::json!({
                            "type": "warning",
                            "message": format!("进度推送过快，已跳过 {} 条消息", skipped)
                        });
                        if let Ok(json) = serde_json::to_string(&warning) {
                            let _ = sender.send(Message::Text(json.into())).await;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        info!("进度广播通道已关闭 (任务可能已被清理)");
                        break;
                    }
                }
            }
        }
    }

    debug!("WebSocket 连接已关闭: task_id={}", task_id);
}

/// 所有任务进度自动推送的 WebSocket 端点
///
/// 路由: `/ws/tasks`
///
/// 前端连接后自动接收所有活跃任务的进度更新（无需发送 subscribe 命令）。
/// 也支持可选的客户端命令（list 等）。
pub async fn ws_tasks_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    info!("WebSocket 多任务连接请求");
    let hub = state.progress_hub.clone();
    let task_manager = state.task_manager.clone();
    ws.on_upgrade(move |socket| handle_tasks_socket(socket, hub, task_manager))
}

/// 客户端命令（保留兼容）
#[derive(Debug, Deserialize)]
#[serde(tag = "action")]
enum ClientCommand {
    #[serde(rename = "subscribe")]
    Subscribe { task_id: String },
    #[serde(rename = "unsubscribe")]
    Unsubscribe { task_id: String },
    #[serde(rename = "list")]
    List,
}

/// 将 ProgressMessage 包装成前端期望的 WebSocketMessage 格式
fn wrap_progress_as_ws_message(msg: &ProgressMessage) -> serde_json::Value {
    serde_json::json!({
        "type": "task_progress",
        "data": msg,
        "timestamp": msg.timestamp.to_rfc3339()
    })
}

/// 处理多任务自动推送的 WebSocket 连接
async fn handle_tasks_socket(
    socket: WebSocket,
    hub: Arc<ProgressHub>,
    task_manager: std::sync::Arc<tokio::sync::Mutex<crate::web_server::TaskManager>>,
) {
    let (mut sender, mut receiver) = socket.split();

    // 发送握手消息
    let handshake = serde_json::json!({
        "type": "handshake",
        "message": "任务监控连接成功，自动推送所有任务进度",
        "active_tasks": hub.active_tasks(),
        "timestamp": chrono::Utc::now().to_rfc3339()
    });

    if let Ok(json) = serde_json::to_string(&handshake) {
        if sender.send(Message::Text(json.into())).await.is_err() {
            error!("发送握手消息失败");
            return;
        }
    }

    // 自动订阅所有当前活跃任务
    let mut subscriptions: Vec<(String, broadcast::Receiver<ProgressMessage>)> = Vec::new();
    let mut known_task_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    // 订阅所有已知任务
    for task_id in hub.active_tasks() {
        let rx = hub.subscribe(&task_id);
        subscriptions.push((task_id.clone(), rx));
        known_task_ids.insert(task_id);
    }

    // 发送所有活跃任务的当前状态快照
    for state in hub.all_task_states() {
        let envelope = wrap_progress_as_ws_message(&state);
        if let Ok(json) = serde_json::to_string(&envelope) {
            if sender.send(Message::Text(json.into())).await.is_err() {
                warn!("发送初始状态快照失败");
                return;
            }
        }
    }

    // 定期检查新任务的计时器
    let mut discover_interval = tokio::time::interval(tokio::time::Duration::from_secs(2));
    discover_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            // 处理来自客户端的消息
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => {
                        info!("客户端关闭任务监控连接");
                        break;
                    }
                    Some(Ok(Message::Ping(data))) => {
                        if sender.send(Message::Pong(data)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Text(text))) => {
                        // 兼容旧的 subscribe/list 命令
                        if let Ok(cmd) = serde_json::from_str::<ClientCommand>(&text) {
                            match cmd {
                                ClientCommand::Subscribe { task_id } => {
                                    if !known_task_ids.contains(&task_id) {
                                        let rx = hub.subscribe(&task_id);
                                        subscriptions.push((task_id.clone(), rx));
                                        known_task_ids.insert(task_id);
                                    }
                                }
                                ClientCommand::Unsubscribe { task_id } => {
                                    subscriptions.retain(|(id, _)| id != &task_id);
                                    known_task_ids.remove(&task_id);
                                }
                                ClientCommand::List => {
                                    let tasks = hub.all_task_states();
                                    let response = serde_json::json!({
                                        "type": "task_list",
                                        "data": tasks,
                                        "timestamp": chrono::Utc::now().to_rfc3339()
                                    });
                                    if let Ok(json) = serde_json::to_string(&response) {
                                        let _ = sender.send(Message::Text(json.into())).await;
                                    }
                                }
                            }
                        }
                    }
                    Some(Err(e)) => {
                        error!("接收 WebSocket 消息出错: {}", e);
                        break;
                    }
                    _ => {}
                }
            }

            // 处理所有订阅的进度更新
            _ = async {
                for (_task_id, rx) in &mut subscriptions {
                    while let Ok(msg) = rx.try_recv() {
                        let envelope = wrap_progress_as_ws_message(&msg);
                        if let Ok(json) = serde_json::to_string(&envelope) {
                            if sender.send(Message::Text(json.into())).await.is_err() {
                                return;
                            }
                        }
                    }
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            } => {}

            // 定期发现新任务并自动订阅
            _ = discover_interval.tick() => {
                let current_tasks = hub.active_tasks();
                for task_id in &current_tasks {
                    if !known_task_ids.contains(task_id) {
                        info!("自动订阅新任务: {}", task_id);
                        let rx = hub.subscribe(task_id);
                        subscriptions.push((task_id.clone(), rx));
                        known_task_ids.insert(task_id.clone());

                        // 推送新任务的当前状态
                        if let Some(state) = hub.get_task_state(task_id) {
                            let envelope = wrap_progress_as_ws_message(&state);
                            if let Ok(json) = serde_json::to_string(&envelope) {
                                let _ = sender.send(Message::Text(json.into())).await;
                            }
                        }
                    }
                }

                // 清理已完成任务的订阅（减少内存）
                subscriptions.retain(|(id, _)| {
                    if let Some(state) = hub.get_task_state(id) {
                        !matches!(state.status,
                            crate::shared::TaskStatus::Completed |
                            crate::shared::TaskStatus::Failed |
                            crate::shared::TaskStatus::Cancelled
                        )
                    } else {
                        false
                    }
                });
            }
        }
    }

    debug!("任务监控连接已关闭");
}
