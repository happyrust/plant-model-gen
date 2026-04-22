use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Json,
};
use chrono::{DateTime, Utc};
use config;
use once_cell::sync::Lazy;
use rusqlite;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::web_server::AppState;

/// MQTT 节点状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MqttNodeStatus {
    /// 节点位置标识（如 "bj", "sjz"）
    pub location: String,
    /// 节点名称
    pub node_name: String,
    /// 是否在线
    pub is_online: bool,
    /// 最后心跳时间
    pub last_heartbeat: DateTime<Utc>,
    /// 订阅主题列表
    pub subscribed_topics: Vec<String>,
    /// 接收消息总数
    pub messages_received: u64,
    /// 最后接收消息时间
    pub last_message_time: Option<DateTime<Utc>>,
    /// 连接时间
    pub connected_at: DateTime<Utc>,
    /// MQTT Broker 连接状态（发布客户端）
    pub broker_connected_pub: Option<bool>,
    /// MQTT Broker 连接状态（订阅客户端）
    pub broker_connected_sub: Option<bool>,
}

/// MQTT 消息接收记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageDeliveryStatus {
    /// 消息 ID（基于 timestamp + location）
    pub message_id: String,
    /// 发送者位置
    pub sender_location: String,
    /// 发送时间
    pub sent_at: DateTime<Utc>,
    /// 会话范围
    pub session_range: Option<String>,
    /// 接收者列表
    pub receivers: Vec<ReceiverStatus>,
    /// 文件数量
    pub file_count: usize,
}

/// 接收者状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiverStatus {
    /// 接收者位置
    pub location: String,
    /// 是否已接收
    pub received: bool,
    /// 接收时间
    pub received_at: Option<DateTime<Utc>>,
    /// 处理状态
    pub status: ReceiverProcessStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReceiverProcessStatus {
    Pending,    // 等待接收
    Received,   // 已接收
    Processing, // 处理中
    Completed,  // 已完成
    Failed,     // 失败
}

/// 全局 MQTT 节点状态缓存
pub static MQTT_NODES: Lazy<Arc<RwLock<HashMap<String, MqttNodeStatus>>>> =
    Lazy::new(|| Arc::new(RwLock::new(HashMap::new())));

/// 全局 MQTT 消息投递状态缓存
pub static MQTT_MESSAGE_DELIVERY: Lazy<Arc<RwLock<HashMap<String, MessageDeliveryStatus>>>> =
    Lazy::new(|| Arc::new(RwLock::new(HashMap::new())));

/// HTTP 健康检查结果缓存（缓存 30 秒）
#[derive(Debug, Clone)]
struct HealthCheckCache {
    is_online: bool,
    checked_at: DateTime<Utc>,
}

static HTTP_HEALTH_CACHE: Lazy<Arc<RwLock<HashMap<String, HealthCheckCache>>>> =
    Lazy::new(|| Arc::new(RwLock::new(HashMap::new())));

/// HTTP 健康检查缓存有效期（秒）
const HEALTH_CHECK_CACHE_TTL: i64 = 30;

/// 更新节点心跳（由 MQTT 客户端定期调用）
pub async fn update_node_heartbeat(
    location: String,
    node_name: String,
    subscribed_topics: Vec<String>,
) {
    let mut nodes = MQTT_NODES.write().await;

    if let Some(node) = nodes.get_mut(&location) {
        // 更新已有节点
        node.last_heartbeat = Utc::now();
        node.is_online = true;
        node.subscribed_topics = subscribed_topics;
        node.broker_connected_sub = Some(true);
    } else {
        // 新增节点
        nodes.insert(
            location.clone(),
            MqttNodeStatus {
                location,
                node_name,
                is_online: true,
                last_heartbeat: Utc::now(),
                subscribed_topics,
                messages_received: 0,
                last_message_time: None,
                connected_at: Utc::now(),
                // 新节点时默认发布端为断开态，前端可通过 P 红色对应
                broker_connected_pub: Some(false),
                broker_connected_sub: Some(true), // 订阅客户端已连接，因为有心跳
            },
        );
    }
}

/// 更新订阅连接是否成功（用于 ConnAck 或异常时标记为未连接）
pub async fn update_subscription_status(location: String, node_name: String, connected: bool) {
    let mut nodes = MQTT_NODES.write().await;

    if let Some(node) = nodes.get_mut(&location) {
        node.broker_connected_sub = Some(connected);
        node.is_online = connected;
        if connected {
            node.last_heartbeat = Utc::now();
        }
    } else {
        nodes.insert(
            location.clone(),
            MqttNodeStatus {
                location,
                node_name,
                is_online: connected,
                last_heartbeat: Utc::now(),
                subscribed_topics: Vec::new(),
                messages_received: 0,
                last_message_time: None,
                connected_at: Utc::now(),
                broker_connected_pub: Some(false),
                broker_connected_sub: Some(connected),
            },
        );
    }
}

/// 记录节点接收消息
pub async fn record_message_received(location: String, message_id: String) {
    // 更新节点统计
    let mut nodes = MQTT_NODES.write().await;
    if let Some(node) = nodes.get_mut(&location) {
        node.messages_received += 1;
        node.last_message_time = Some(Utc::now());
    }
    drop(nodes);

    // 更新消息投递状态
    let mut delivery = MQTT_MESSAGE_DELIVERY.write().await;
    if let Some(msg) = delivery.get_mut(&message_id) {
        if let Some(receiver) = msg.receivers.iter_mut().find(|r| r.location == location) {
            receiver.received = true;
            receiver.received_at = Some(Utc::now());
            receiver.status = ReceiverProcessStatus::Received;
        }
    }
}

/// 记录新消息发送（由发送方调用）
pub async fn record_message_sent(
    message_id: String,
    sender_location: String,
    session_range: Option<String>,
    file_count: usize,
    expected_receivers: Vec<String>,
) {
    let mut delivery = MQTT_MESSAGE_DELIVERY.write().await;

    let receivers = expected_receivers
        .into_iter()
        .map(|loc| ReceiverStatus {
            location: loc,
            received: false,
            received_at: None,
            status: ReceiverProcessStatus::Pending,
        })
        .collect();

    delivery.insert(
        message_id.clone(),
        MessageDeliveryStatus {
            message_id,
            sender_location,
            sent_at: Utc::now(),
            session_range,
            receivers,
            file_count,
        },
    );
}

/// 检查并标记离线节点（定期调用）
pub async fn check_offline_nodes(timeout_secs: i64) {
    let mut nodes = MQTT_NODES.write().await;
    let now = Utc::now();

    for node in nodes.values_mut() {
        let elapsed = now.signed_duration_since(node.last_heartbeat).num_seconds();
        if elapsed > timeout_secs {
            node.is_online = false;
        }
    }
}

/// API: 获取所有 MQTT 节点状态
/// 
/// 对于从节点：只显示订阅的主节点信息，不显示自己和其他站点
/// 对于主节点：显示所有已连接的从节点
pub async fn get_mqtt_nodes_status(
    _state: State<AppState>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // 先检查离线节点（30秒超时）
    check_offline_nodes(30).await;

    // 获取当前站点配置
    use aios_core::get_db_option;
    let db_option = get_db_option();
    let current_location = db_option.location.clone();

    // 获取数据库路径
    let db_path = if std::path::Path::new("DbOption.toml").exists() {
        config::Config::builder()
            .add_source(config::File::with_name("DbOption"))
            .build()
            .ok()
            .and_then(|b| b.get_string("deployment_sites_sqlite_path").ok())
            .unwrap_or_else(|| "deployment_sites.sqlite".to_string())
    } else {
        "deployment_sites.sqlite".to_string()
    };

    // 检查当前站点是否为主节点
    let is_current_master = check_is_master_node_internal(&current_location, &db_path);

    let mut all_nodes: Vec<serde_json::Value> = Vec::new();

    if is_current_master {
        // 主节点：显示所有配置的从节点（从 remote_sync_sites 表获取）
        use crate::web_server::remote_sync_handlers;
        let all_sites = remote_sync_handlers::list_all_sites()
            .await
            .unwrap_or_default();
        
        // 获取 MQTT 节点状态（用于获取已连接节点的实时信息）
        let mqtt_nodes = MQTT_NODES.read().await;
        
        for site in &all_sites {
            let site_location = site.location.clone().unwrap_or_default();
            if site_location.is_empty() {
                continue;
            }
            
            // 检查是否为主节点
            let is_master = check_is_master_node_internal(&site_location, &db_path);
            
            // 检查是否在 MQTT_NODES 中（已连接）
            let mqtt_node = mqtt_nodes.get(&site_location);
            
            // 判断在线状态：优先使用 MQTT 连接状态，否则使用 HTTP 健康检查
            let (is_online, messages_received, last_heartbeat, has_mqtt_subscription) = 
                if let Some(node) = mqtt_node {
                    (node.is_online, node.messages_received, Some(node.last_heartbeat), true)
                } else {
                    // 通过 HTTP 健康检查判断是否在线
                    let online = check_site_http_status(site.http_host.as_deref()).await;
                    (online, 0, None, false)
                };
            
            // 标记是否为当前站点
            let is_current = site_location == current_location;
            
            all_nodes.push(json!({
                "location": site_location,
                "node_name": format!("{}", site.name),
                "is_online": is_online,
                "last_heartbeat": last_heartbeat.unwrap_or_else(chrono::Utc::now),
                "subscribed_topics": mqtt_node.map(|n| n.subscribed_topics.clone()).unwrap_or_default(),
                "messages_received": messages_received,
                "last_message_time": mqtt_node.and_then(|n| n.last_message_time),
                "connected_at": mqtt_node.map(|n| n.connected_at).unwrap_or_else(chrono::Utc::now),
                "is_master_node": is_master,
                "has_mqtt_subscription": has_mqtt_subscription,
                "broker_connected_pub": mqtt_node.and_then(|n| n.broker_connected_pub),
                "broker_connected_sub": mqtt_node.and_then(|n| n.broker_connected_sub),
                "http_host": site.http_host,
                "can_delete": !is_current,  // 当前站点不能删除
                "is_current": is_current     // 标记当前站点
            }));
        }
    } else {
        // 从节点：只显示订阅的主节点信息
        // get_subscribed_master_info 现在只返回有效的 master_location（不为 'unknown' 或 NULL）
        if let Some((master_location, master_host, master_port)) = get_subscribed_master_info(&current_location) {
            // 检查主节点是否在线
            let is_online = check_master_online(&master_location, &master_host).await;
            
            all_nodes.push(json!({
                "location": master_location.clone(),
                "node_name": format!("主节点-{}", master_location),
                "is_online": is_online,
                "last_heartbeat": chrono::Utc::now(),
                "subscribed_topics": vec!["Sync/E3d"],
                "messages_received": 0,
                "last_message_time": None::<chrono::DateTime<chrono::Utc>>,
                "connected_at": chrono::Utc::now(),
                "is_master_node": true,
                "has_mqtt_subscription": true,
                "mqtt_host": master_host,
                "mqtt_port": master_port,
                "can_delete": true,
                "is_subscribed_master": true
            }));
        }
    }

    let online_count = all_nodes
        .iter()
        .filter(|n| n["is_online"].as_bool().unwrap_or(false))
        .count();
    let total_count = all_nodes.len();

    Ok(Json(json!({
        "success": true,
        "nodes": all_nodes,
        "is_master_node": is_current_master,
        "current_location": current_location,
        "summary": {
            "total": total_count,
            "online": online_count,
            "offline": total_count - online_count,
        }
    })))
}

/// 获取从节点订阅的主节点信息
fn get_subscribed_master_info(current_location: &str) -> Option<(String, String, u16)> {
    use crate::web_server::remote_sync_handlers;
    
    let conn = remote_sync_handlers::open_sqlite().ok()?;
    
    // 查询当前站点配置的主节点信息（使用 master_location 字段）
    // 注意：如果 master_location 为 NULL，返回 None，不返回 'unknown'
    conn.query_row(
        "SELECT master_location, master_mqtt_host, COALESCE(master_mqtt_port, 1883)
         FROM remote_sync_sites 
         WHERE location = ?1 AND master_mqtt_host IS NOT NULL AND master_mqtt_port IS NOT NULL
         LIMIT 1",
        rusqlite::params![current_location],
        |row| {
            let master_location: Option<String> = row.get(0)?;
            let host: String = row.get(1)?;
            let port: i64 = row.get(2)?;
            // 如果 master_location 为 NULL 或 'unknown'，返回 None
            if let Some(loc) = master_location {
                if loc.is_empty() || loc == "unknown" {
                    return Err(rusqlite::Error::InvalidColumnType(0, "master_location".to_string(), rusqlite::types::Type::Text));
                }
                Ok((loc, host, port as u16))
            } else {
                Err(rusqlite::Error::InvalidColumnType(0, "master_location".to_string(), rusqlite::types::Type::Null))
            }
        },
    ).ok()
}

/// 检查主节点是否在线
async fn check_master_online(master_location: &str, master_host: &str) -> bool {
    // 先检查 MQTT_NODES 中是否有（这只会在从节点连接到主节点并收到心跳时才会出现）
    let is_online = {
        let mqtt_nodes = MQTT_NODES.read().await;
        mqtt_nodes.get(master_location)
            .map(|n| n.is_online)
            .unwrap_or(false)
    };
    
    if is_online {
        return true;
    }
    
    // 尝试 HTTP 检查
    use crate::web_server::remote_sync_handlers;
    let all_sites = remote_sync_handlers::list_all_sites()
        .await
        .unwrap_or_default();
    
    // 优先使用站点配置中的 http_host
    if let Some(site) = all_sites.iter().find(|s| s.location.as_ref().map(|l| l.as_str()) == Some(master_location)) {
        if let Some(ref http_host) = site.http_host {
            return check_site_http_status(Some(http_host)).await;
        }
    }
    
    // 尝试直接从 master_host 构建 HTTP URL 检查
    // master_host 格式可能是: "198.18.0.1:1883" 或 "198.18.0.1"
    let http_url = if master_host.contains(":1883") {
        format!("http://{}", master_host.replace(":1883", ":8080"))
    } else if master_host.contains(':') {
        // 如果包含其他端口，替换为 8080
        let host_part = master_host.split(':').next().unwrap_or(master_host);
        format!("http://{}:8080", host_part)
    } else {
        format!("http://{}:8080", master_host)
    };
    
    let result = check_site_http_status(Some(&http_url)).await;
    if !result {
        log::warn!("主节点 {} HTTP 健康检查失败: {} (从 master_host {} 构建)", master_location, http_url, master_host);
    }
    result
}

/// API: 删除/移除 MQTT 节点
/// 
/// 对于从节点：取消订阅主节点并清除配置，同时通知主节点
/// 对于主节点：从监控列表和数据库中移除指定节点
pub async fn remove_mqtt_node(
    axum::extract::Path(location): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    use aios_core::get_db_option;
    use crate::web_server::remote_sync_handlers;
    
    let db_option = get_db_option();
    let current_location = db_option.location.clone();
    
    // 获取数据库路径
    let db_path = if std::path::Path::new("DbOption.toml").exists() {
        config::Config::builder()
            .add_source(config::File::with_name("DbOption"))
            .build()
            .ok()
            .and_then(|b| b.get_string("deployment_sites_sqlite_path").ok())
            .unwrap_or_else(|| "deployment_sites.sqlite".to_string())
    } else {
        "deployment_sites.sqlite".to_string()
    };
    
    let is_current_master = check_is_master_node_internal(&current_location, &db_path);
    
    if is_current_master {
        // 主节点：从 MQTT_NODES 和数据库中移除节点
        let mut nodes = MQTT_NODES.write().await;
        nodes.remove(&location);
        
        // 同时从数据库中删除站点记录
        if let Ok(conn) = remote_sync_handlers::open_sqlite() {
            let _ = conn.execute(
                "DELETE FROM remote_sync_sites WHERE location = ?1",
                rusqlite::params![location],
            );
            log::info!("主节点：已从数据库中删除站点 {}", location);
        }
        
        Ok(Json(json!({
            "status": "success",
            "message": format!("已从监控列表中移除节点: {}", location)
        })))
    } else {
        // 从节点：取消订阅并清除主节点配置
        use crate::web_server::sync_control_handlers::clear_master_config_internal;

        // 获取主节点的 HTTP 地址（用于通知）
        let master_http_host = get_master_http_host(&current_location);

        // 先停止订阅
        use crate::web_server::remote_runtime::REMOTE_RUNTIME;
        {
            let mut runtime = REMOTE_RUNTIME.write().await;
            if runtime.is_some() {
                *runtime = None;
                log::info!("已停止 MQTT 订阅");
            }
        }

        // 清除主节点配置
        if let Err(e) = clear_master_config_internal().await {
            return Ok(Json(json!({
                "status": "error",
                "message": format!("清除主节点配置失败: {}", e)
            })));
        }
        
        // 从 MQTT_NODES 中移除
        let mut nodes = MQTT_NODES.write().await;
        nodes.remove(&location);
        drop(nodes);
        
        // 通知主节点：从节点已取消订阅
        if let Some(master_host) = master_http_host {
            notify_master_unsubscribe(&master_host, &current_location).await;
        }
        
        Ok(Json(json!({
            "status": "success",
            "message": format!("已取消订阅主节点: {}", location)
        })))
    }
}

/// 获取主节点的 HTTP 地址
fn get_master_http_host(current_location: &str) -> Option<String> {
    use crate::web_server::remote_sync_handlers;
    
    let conn = remote_sync_handlers::open_sqlite().ok()?;
    
    // 查询主节点的 http_host（通过 master_mqtt_host 关联）
    // 首先获取当前站点配置的 master_mqtt_host
    let master_mqtt_host: Option<String> = conn.query_row(
        "SELECT master_mqtt_host FROM remote_sync_sites WHERE location = ?1",
        rusqlite::params![current_location],
        |row| row.get(0),
    ).ok();
    
    if let Some(mqtt_host) = master_mqtt_host {
        // 尝试从 mqtt_host 构建 http_host（假设 HTTP 端口是 8080）
        let http_host = if mqtt_host.contains(":1883") {
            mqtt_host.replace(":1883", ":8080")
        } else if mqtt_host.contains(':') {
            // 已经有端口，替换为 8080
            let parts: Vec<&str> = mqtt_host.split(':').collect();
            format!("{}:8080", parts[0])
        } else {
            format!("{}:8080", mqtt_host)
        };
        
        // 添加 http:// 前缀
        let http_url = if http_host.starts_with("http") {
            http_host
        } else {
            format!("http://{}", http_host)
        };
        
        return Some(http_url);
    }
    
    None
}

/// 通知主节点：从节点已取消订阅
async fn notify_master_unsubscribe(master_host: &str, client_location: &str) {
    let url = format!("{}/api/mqtt/nodes/client-unsubscribed", master_host);
    
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            log::warn!("创建 HTTP 客户端失败: {}", e);
            return;
        }
    };
    
    match client.post(&url)
        .json(&json!({
            "client_location": client_location
        }))
        .send()
        .await
    {
        Ok(resp) => {
            if resp.status().is_success() {
                log::info!("已通知主节点 {} 从节点 {} 取消订阅", master_host, client_location);
            } else {
                log::warn!("通知主节点失败: HTTP {}", resp.status());
            }
        }
        Err(e) => {
            log::warn!("通知主节点失败: {}", e);
        }
    }
}

/// API: 接收从节点取消订阅的通知（主节点使用）
pub async fn client_unsubscribed(
    Json(payload): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    use crate::web_server::remote_sync_handlers;
    
    let client_location = payload.get("client_location")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    
    if client_location.is_empty() {
        return Ok(Json(json!({
            "status": "error",
            "message": "缺少 client_location 参数"
        })));
    }
    
    log::info!("收到从节点 {} 取消订阅的通知", client_location);
    
    // 从 MQTT_NODES 中移除
    let mut nodes = MQTT_NODES.write().await;
    nodes.remove(client_location);
    drop(nodes);
    
    // 从数据库中删除站点记录
    if let Ok(conn) = remote_sync_handlers::open_sqlite() {
        let deleted = conn.execute(
            "DELETE FROM remote_sync_sites WHERE location = ?1",
            rusqlite::params![client_location],
        ).unwrap_or(0);
        
        if deleted > 0 {
            log::info!("已从数据库中删除站点 {}", client_location);
        }
    }
    
    Ok(Json(json!({
        "status": "success",
        "message": format!("已处理从节点 {} 取消订阅", client_location)
    })))
}

// 辅助函数：检查是否为主节点（内部使用）
fn check_is_master_node_internal(location: &str, db_path: &str) -> bool {
    let conn = match rusqlite::Connection::open(db_path) {
        Ok(c) => c,
        Err(_) => return false,
    };

    let _ = conn.execute(
        "CREATE TABLE IF NOT EXISTS node_config (
            location TEXT PRIMARY KEY,
            is_master BOOLEAN NOT NULL DEFAULT 0,
            updated_at TEXT DEFAULT CURRENT_TIMESTAMP
        )",
        [],
    );

    conn.query_row(
        "SELECT is_master FROM node_config WHERE location = ?1",
        [location],
        |row| row.get::<_, bool>(0),
    )
    .unwrap_or(false)
}

/// 检查站点 HTTP 状态（带缓存）
/// 用于跨进程检查主节点是否在线
pub async fn check_site_http_status(http_host: Option<&str>) -> bool {
    let Some(host) = http_host else {
        return false;
    };

    let host_key = host.trim_end_matches('/').to_string();
    
    // 检查缓存
    {
        let cache = HTTP_HEALTH_CACHE.read().await;
        if let Some(cached) = cache.get(&host_key) {
            let age = Utc::now().signed_duration_since(cached.checked_at);
            if age.num_seconds() < HEALTH_CHECK_CACHE_TTL {
                return cached.is_online;
            }
        }
    }

    // 缓存过期或不存在，执行健康检查
    let health_url = format!("{}/api/health", host_key);
    let is_online = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
    {
        Ok(client) => match client.get(&health_url).send().await {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        },
        Err(_) => false,
    };

    // 更新缓存
    {
        let mut cache = HTTP_HEALTH_CACHE.write().await;
        cache.insert(host_key, HealthCheckCache {
            is_online,
            checked_at: Utc::now(),
        });
    }

    is_online
}

/// API: 获取消息投递状态
pub async fn get_message_delivery_status(
    _state: State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let limit = params
        .get("limit")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(50);

    let delivery = MQTT_MESSAGE_DELIVERY.read().await;

    // 按时间倒序排列，取最近的消息
    let mut messages: Vec<&MessageDeliveryStatus> = delivery.values().collect();
    messages.sort_by(|a, b| b.sent_at.cmp(&a.sent_at));

    let messages: Vec<&MessageDeliveryStatus> = messages.into_iter().take(limit).collect();

    // 统计信息
    let total_messages = delivery.len();
    let mut pending_deliveries = 0;
    let mut completed_deliveries = 0;

    for msg in delivery.values() {
        let all_received = msg.receivers.iter().all(|r| r.received);
        if all_received {
            completed_deliveries += 1;
        } else {
            pending_deliveries += 1;
        }
    }

    Ok(Json(json!({
        "success": true,
        "messages": messages,
        "summary": {
            "total": total_messages,
            "completed": completed_deliveries,
            "pending": pending_deliveries,
        }
    })))
}

/// API: 获取特定消息的投递详情
pub async fn get_message_delivery_detail(
    _state: State<AppState>,
    axum::extract::Path(message_id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let delivery = MQTT_MESSAGE_DELIVERY.read().await;

    if let Some(msg) = delivery.get(&message_id) {
        Ok(Json(json!({
            "success": true,
            "message": msg,
        })))
    } else {
        Ok(Json(json!({
            "success": false,
            "error": "Message not found"
        })))
    }
}

/// 定期清理旧消息（保留最近 1000 条）
pub async fn cleanup_old_messages() {
    let mut delivery = MQTT_MESSAGE_DELIVERY.write().await;

    if delivery.len() > 1000 {
        // 按时间排序，保留最新的 1000 条
        let mut messages: Vec<(String, MessageDeliveryStatus)> = delivery.drain().collect();
        messages.sort_by(|a, b| b.1.sent_at.cmp(&a.1.sent_at));

        *delivery = messages.into_iter().take(1000).collect();
    }
}
