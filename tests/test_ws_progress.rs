//! WebSocket 进度推送端到端模拟测试
//!
//! 验证 ProgressHub → WebSocket handler → WS 客户端 全链路：
//! 1. 启动最小化 axum mock server（无数据库依赖）
//! 2. WS 客户端连接 `/ws/progress/{task_id}`
//! 3. 后台模拟多步骤进度推送
//! 4. 验证客户端收到正确的进度消息序列
//!
//! 运行：`cargo test --test test_ws_progress -- --nocapture`

use aios_database::shared::{ProgressHub, ProgressMessageBuilder, TaskStatus};
use axum::{
    Router,
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::Response,
    routing::get,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::sync::Arc;
use tokio::net::TcpListener;

// ─── Mock Server State ──────────────────────────────────────────

#[derive(Clone)]
struct MockState {
    hub: Arc<ProgressHub>,
}

// ─── WS Handler（复刻 progress.rs 核心逻辑）────────────────────

async fn ws_progress_handler(
    ws: WebSocketUpgrade,
    Path(task_id): Path<String>,
    State(state): State<MockState>,
) -> Response {
    let hub = state.hub.clone();
    ws.on_upgrade(move |socket| handle_socket(socket, task_id, hub))
}

async fn handle_socket(socket: WebSocket, task_id: String, hub: Arc<ProgressHub>) {
    let (mut sender, mut receiver) = socket.split();

    // 发送握手
    let current = hub.get_task_state(&task_id);
    let handshake = serde_json::json!({
        "type": "handshake",
        "task_id": task_id,
        "message": "连接成功",
        "current_state": current,
    });
    if let Ok(json) = serde_json::to_string(&handshake) {
        let _ = sender.send(Message::Text(json.into())).await;
    }

    // 订阅进度
    let mut rx = hub.subscribe(&task_id);

    loop {
        tokio::select! {
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
            progress = rx.recv() => {
                match progress {
                    Ok(msg) => {
                        if let Ok(json) = serde_json::to_string(&msg) {
                            if sender.send(Message::Text(json.into())).await.is_err() {
                                break;
                            }
                            if matches!(msg.status, TaskStatus::Completed | TaskStatus::Failed) {
                                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                                let _ = sender.send(Message::Close(None)).await;
                                break;
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    Err(_) => {} // Lagged, skip
                }
            }
        }
    }
}

// ─── Mock 任务进度模拟 ──────────────────────────────────────────

async fn simulate_task_progress(hub: Arc<ProgressHub>, task_id: String) {
    let steps = vec![
        ("数据预检", 10.0, 100, 1000),
        ("解析数据库", 30.0, 300, 1000),
        ("生成几何体", 60.0, 600, 1000),
        ("构建空间索引", 80.0, 800, 1000),
        ("导出模型", 95.0, 950, 1000),
    ];
    let total_steps = steps.len() as u32;

    // 注册任务
    hub.register(task_id.clone());

    // 等待 WebSocket 客户端连接并订阅
    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

    for (i, (step_name, pct, processed, total)) in steps.iter().enumerate() {
        let msg = ProgressMessageBuilder::new(&task_id)
            .status(TaskStatus::Running)
            .step(*step_name, (i + 1) as u32, total_steps)
            .percentage(*pct)
            .items(*processed, *total)
            .message(format!("正在{}...", step_name))
            .build();
        let _ = hub.publish(msg);
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    }

    // 完成
    let done = ProgressMessageBuilder::new(&task_id)
        .status(TaskStatus::Completed)
        .percentage(100.0)
        .step("完成", total_steps, total_steps)
        .items(1000, 1000)
        .message("任务完成")
        .build();
    let _ = hub.publish(done);
}

// ─── GUI 端 ProgressUpdate 格式（模拟 plant-model-gui 的反序列化）──

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ProgressUpdate {
    #[serde(default)]
    task_id: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    percentage: f32,
    #[serde(default)]
    current_step: String,
    #[serde(default)]
    current_step_number: u32,
    #[serde(default)]
    total_steps: u32,
    #[serde(default)]
    processed_items: u64,
    #[serde(default)]
    total_items: u64,
    #[serde(default)]
    message: String,
}

// ─── 测试入口 ───────────────────────────────────────────────────

#[tokio::test]
async fn test_ws_progress_end_to_end() {
    let hub = Arc::new(ProgressHub::default());
    let state = MockState { hub: hub.clone() };

    let app = Router::new()
        .route("/ws/progress/{task_id}", get(ws_progress_handler))
        .with_state(state);

    // 绑定随机端口
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    println!("✅ Mock server 启动: http://{}", addr);

    // 启动 server
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let task_id = "mock-task-001";

    // 启动模拟进度（后台）
    let hub_clone = hub.clone();
    let tid = task_id.to_string();
    tokio::spawn(async move {
        simulate_task_progress(hub_clone, tid).await;
    });

    // WS 客户端连接
    let ws_url = format!("ws://{}/ws/progress/{}", addr, task_id);
    println!("🔗 连接 WebSocket: {}", ws_url);

    let (ws_stream, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("WebSocket 连接失败");

    let (mut _write, mut read) = ws_stream.split();

    let mut received_updates: Vec<ProgressUpdate> = Vec::new();
    let mut got_handshake = false;

    // 带超时地接收消息
    let timeout = tokio::time::Duration::from_secs(10);
    let result = tokio::time::timeout(timeout, async {
        while let Some(msg) = read.next().await {
            match msg {
                Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                    let text_str = text.to_string();
                    // 尝试解析为握手消息
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text_str) {
                        if val.get("type").and_then(|v| v.as_str()) == Some("handshake") {
                            got_handshake = true;
                            println!("📨 握手消息: task_id={}", val.get("task_id").unwrap_or(&serde_json::Value::Null));
                            continue;
                        }
                    }

                    // 解析为进度消息
                    match serde_json::from_str::<ProgressUpdate>(&text_str) {
                        Ok(update) => {
                            println!(
                                "📊 进度: status={:<12} step={:<16} pct={:>5.1}%  items={}/{}  msg={}",
                                update.status,
                                update.current_step,
                                update.percentage,
                                update.processed_items,
                                update.total_items,
                                update.message,
                            );

                            let is_done = update.status == "completed" || update.status == "failed";
                            received_updates.push(update);

                            if is_done {
                                break;
                            }
                        }
                        Err(e) => {
                            println!("⚠️  消息解析跳过: {} | 原文: {}...", e, &text_str[..text_str.len().min(100)]);
                        }
                    }
                }
                Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => {
                    println!("🔒 服务端关闭连接");
                    break;
                }
                Err(e) => {
                    println!("❌ WS 错误: {}", e);
                    break;
                }
                _ => {}
            }
        }
    })
    .await;

    // ─── 断言 ───────────────────────────────────────────────────

    assert!(result.is_ok(), "测试超时：未在 10 秒内完成");
    assert!(got_handshake, "未收到握手消息");
    assert!(
        received_updates.len() >= 5,
        "进度消息不足：收到 {} 条，期望 >= 5",
        received_updates.len()
    );

    // 验证进度递增
    let mut last_pct = 0.0_f32;
    for u in &received_updates {
        assert!(
            u.percentage >= last_pct,
            "进度应单调递增: {} < {}",
            u.percentage,
            last_pct
        );
        last_pct = u.percentage;
    }

    // 验证最终状态
    let last = received_updates.last().unwrap();
    assert_eq!(last.status, "completed", "最终状态应为 completed");
    assert_eq!(last.percentage, 100.0, "最终进度应为 100%");
    assert_eq!(last.processed_items, 1000, "最终处理数应为 1000");

    // 验证 task_id 一致
    for u in &received_updates {
        assert_eq!(u.task_id, task_id, "task_id 应一致");
    }

    println!("\n✅ 端到端测试通过！共收到 {} 条进度消息", received_updates.len());
    println!("   - 握手: ✓");
    println!("   - 进度递增: ✓ (0% → 100%)");
    println!("   - 最终状态: completed ✓");
    println!("   - task_id 一致: ✓");
}
