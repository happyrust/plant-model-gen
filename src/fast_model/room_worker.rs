//! Room Worker 模块
//!
//! 后台任务处理器，用于异步执行房间计算任务。
//! 支持任务队列、取消机制和进度报告。

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, broadcast};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use aios_core::options::DbOption;

use super::room_model::{RoomBuildStats, build_room_relations_with_cancel};

/// Worker 配置
#[derive(Clone, Debug)]
pub struct RoomWorkerConfig {
    /// 最大并发任务数（建议 1-2，房间计算是 CPU/内存密集型）
    pub max_concurrent_tasks: usize,
    /// 单任务超时（秒）
    pub task_timeout_secs: u64,
    /// 进度报告间隔（毫秒）
    pub progress_report_interval_ms: u64,
}

impl Default for RoomWorkerConfig {
    fn default() -> Self {
        Self {
            max_concurrent_tasks: 1,
            task_timeout_secs: 3600, // 1 小时
            progress_report_interval_ms: 1000,
        }
    }
}

/// 房间任务类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RoomTaskType {
    /// 重建所有房间关系
    RebuildAll,
    /// 重建指定房间关系
    RebuildByRoomNumbers(Vec<String>),
    /// 增量更新
    IncrementalUpdate,
}

/// 任务状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RoomWorkerTaskStatus {
    /// 排队等待中
    Queued,
    /// 运行中
    Running {
        progress: f32,
        stage: String,
    },
    /// 已完成
    Completed {
        stats: RoomBuildStats,
    },
    /// 失败
    Failed {
        error: String,
    },
    /// 已取消
    Cancelled,
}

impl RoomWorkerTaskStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            RoomWorkerTaskStatus::Completed { .. }
                | RoomWorkerTaskStatus::Failed { .. }
                | RoomWorkerTaskStatus::Cancelled
        )
    }
}

/// Room Worker 任务
#[derive(Debug, Clone)]
pub struct RoomWorkerTask {
    /// 任务 ID
    pub id: String,
    /// 任务类型
    pub task_type: RoomTaskType,
    /// 数据库配置
    pub db_option: DbOption,
    /// 任务状态
    pub status: RoomWorkerTaskStatus,
    /// 创建时间
    pub created_at: DateTime<Utc>,
    /// 更新时间
    pub updated_at: DateTime<Utc>,
    /// 优先级（0 = 最高）
    pub priority: u8,
}

impl RoomWorkerTask {
    pub fn new(id: String, task_type: RoomTaskType, db_option: DbOption) -> Self {
        let now = Utc::now();
        Self {
            id,
            task_type,
            db_option,
            status: RoomWorkerTaskStatus::Queued,
            created_at: now,
            updated_at: now,
            priority: 100, // 默认普通优先级
        }
    }

    pub fn with_priority(mut self, priority: u8) -> Self {
        self.priority = priority;
        self
    }
}

/// 进度事件
#[derive(Debug, Clone, Serialize)]
pub struct ProgressEvent {
    pub task_id: String,
    pub progress: f32,
    pub stage: String,
    pub timestamp: DateTime<Utc>,
}

/// Room Worker 主结构
pub struct RoomWorker {
    config: RoomWorkerConfig,
    /// 任务队列
    task_queue: Arc<RwLock<VecDeque<RoomWorkerTask>>>,
    /// 活跃任务（正在执行的）
    active_tasks: Arc<DashMap<String, RoomWorkerTask>>,
    /// 已完成任务（历史记录）
    completed_tasks: Arc<RwLock<Vec<RoomWorkerTask>>>,
    /// 取消令牌
    cancel_tokens: Arc<DashMap<String, CancellationToken>>,
    /// 进度广播发送端
    progress_tx: broadcast::Sender<ProgressEvent>,
    /// Worker 运行标志
    running: Arc<std::sync::atomic::AtomicBool>,
}

impl RoomWorker {
    /// 创建新的 RoomWorker
    pub fn new(config: RoomWorkerConfig) -> Self {
        let (progress_tx, _) = broadcast::channel(100);
        Self {
            config,
            task_queue: Arc::new(RwLock::new(VecDeque::new())),
            active_tasks: Arc::new(DashMap::new()),
            completed_tasks: Arc::new(RwLock::new(Vec::new())),
            cancel_tokens: Arc::new(DashMap::new()),
            progress_tx,
            running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// 启动后台 Worker
    ///
    /// 返回 JoinHandle，可用于等待 Worker 结束
    pub fn start(config: RoomWorkerConfig) -> (Arc<Self>, JoinHandle<()>) {
        let worker = Arc::new(Self::new(config));
        worker.running.store(true, std::sync::atomic::Ordering::SeqCst);

        let worker_clone = worker.clone();
        let handle = tokio::spawn(async move {
            worker_clone.worker_loop().await;
        });

        info!("🚀 RoomWorker 已启动，max_concurrent_tasks={}", worker.config.max_concurrent_tasks);

        (worker, handle)
    }

    /// 停止 Worker
    pub fn stop(&self) {
        self.running.store(false, std::sync::atomic::Ordering::SeqCst);
        info!("🛑 RoomWorker 停止信号已发送");
    }

    /// 提交任务到队列
    ///
    /// 返回任务 ID
    pub async fn submit_task(&self, task: RoomWorkerTask) -> String {
        let task_id = task.id.clone();
        
        // 按优先级插入队列
        let mut queue = self.task_queue.write().await;
        let insert_pos = queue
            .iter()
            .position(|t| t.priority > task.priority)
            .unwrap_or(queue.len());
        queue.insert(insert_pos, task);

        info!("📥 任务已提交到队列: id={}, 队列长度={}", task_id, queue.len());
        task_id
    }

    /// 取消任务
    ///
    /// 如果任务在队列中，直接移除；如果正在执行，发送取消信号
    pub async fn cancel_task(&self, task_id: &str) -> bool {
        // 1. 尝试从队列中移除
        {
            let mut queue = self.task_queue.write().await;
            if let Some(pos) = queue.iter().position(|t| t.id == task_id) {
                let mut task = queue.remove(pos).unwrap();
                task.status = RoomWorkerTaskStatus::Cancelled;
                task.updated_at = Utc::now();
                self.completed_tasks.write().await.push(task);
                info!("📤 任务已从队列中移除并取消: id={}", task_id);
                return true;
            }
        }

        // 2. 尝试取消正在执行的任务
        if let Some(token) = self.cancel_tokens.get(task_id) {
            token.cancel();
            info!("🛑 已发送取消信号给执行中的任务: id={}", task_id);
            return true;
        }

        warn!("⚠️ 未找到任务: id={}", task_id);
        false
    }

    /// 获取任务状态
    pub fn get_task_status(&self, task_id: &str) -> Option<RoomWorkerTaskStatus> {
        // 1. 检查活跃任务
        if let Some(task) = self.active_tasks.get(task_id) {
            return Some(task.status.clone());
        }

        // 2. 检查队列
        // 注意：这里用 try_read 避免阻塞
        if let Ok(queue) = self.task_queue.try_read() {
            if queue.iter().any(|t| t.id == task_id) {
                return Some(RoomWorkerTaskStatus::Queued);
            }
        }

        // 3. 检查已完成任务
        if let Ok(completed) = self.completed_tasks.try_read() {
            if let Some(task) = completed.iter().find(|t| t.id == task_id) {
                return Some(task.status.clone());
            }
        }

        None
    }

    /// 获取队列长度
    pub async fn queue_len(&self) -> usize {
        self.task_queue.read().await.len()
    }

    /// 获取活跃任务数量
    pub fn active_count(&self) -> usize {
        self.active_tasks.len()
    }

    /// 订阅进度事件
    pub fn subscribe_progress(&self) -> broadcast::Receiver<ProgressEvent> {
        self.progress_tx.subscribe()
    }

    /// Worker 主循环
    async fn worker_loop(self: Arc<Self>) {
        info!("🔄 RoomWorker 主循环启动");

        while self.running.load(std::sync::atomic::Ordering::SeqCst) {
            // 1. 检查是否有空闲槽位
            if self.active_tasks.len() >= self.config.max_concurrent_tasks {
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }

            // 2. 从队列取出任务
            let task = {
                let mut queue = self.task_queue.write().await;
                queue.pop_front()
            };

            if let Some(mut task) = task {
                // 3. 更新任务状态为运行中
                task.status = RoomWorkerTaskStatus::Running {
                    progress: 0.0,
                    stage: "初始化".to_string(),
                };
                task.updated_at = Utc::now();

                let task_id = task.id.clone();
                self.active_tasks.insert(task_id.clone(), task.clone());

                // 4. 创建取消令牌
                let cancel_token = CancellationToken::new();
                self.cancel_tokens.insert(task_id.clone(), cancel_token.clone());

                // 5. 在独立任务中执行
                let worker = self.clone();
                tokio::spawn(async move {
                    worker.execute_task(task, cancel_token).await;
                });
            } else {
                // 队列为空，等待
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }

        info!("🔄 RoomWorker 主循环结束");
    }

    /// 执行单个任务
    async fn execute_task(self: Arc<Self>, mut task: RoomWorkerTask, cancel_token: CancellationToken) {
        let task_id = task.id.clone();
        info!("▶️ 开始执行任务: id={}, type={:?}", task_id, task.task_type);

        let start_time = std::time::Instant::now();

        // 创建进度回调
        let progress_tx = self.progress_tx.clone();
        let task_id_for_callback = task_id.clone();
        let active_tasks = self.active_tasks.clone();
        let progress_callback: Box<dyn Fn(f32, &str) + Send + Sync> = Box::new(move |progress, stage| {
            // 更新活跃任务状态
            if let Some(mut entry) = active_tasks.get_mut(&task_id_for_callback) {
                entry.status = RoomWorkerTaskStatus::Running {
                    progress,
                    stage: stage.to_string(),
                };
                entry.updated_at = Utc::now();
            }

            // 广播进度事件
            let _ = progress_tx.send(ProgressEvent {
                task_id: task_id_for_callback.clone(),
                progress,
                stage: stage.to_string(),
                timestamp: Utc::now(),
            });
        });

        // 执行房间计算
        let result: anyhow::Result<RoomBuildStats> = match &task.task_type {
            RoomTaskType::RebuildAll => {
                build_room_relations_with_cancel(
                    &task.db_option,
                    Some(cancel_token.clone()),
                    Some(progress_callback),
                ).await
            }
            RoomTaskType::RebuildByRoomNumbers(room_numbers) => {
                // 调用针对特定房间的重建
                super::room_model::rebuild_room_relations_for_rooms_with_cancel(
                    room_numbers.clone(),
                    &task.db_option,
                    Some(cancel_token.clone()),
                    Some(progress_callback),
                ).await
            }
            RoomTaskType::IncrementalUpdate => {
                // 增量更新
                super::room_model::update_room_relations_incremental_with_cancel(
                    &task.db_option,
                    Some(cancel_token.clone()),
                    Some(progress_callback),
                ).await
            }
        };

        let duration = start_time.elapsed();

        // 更新任务状态
        task.updated_at = Utc::now();
        match result {
            Ok(stats) => {
                info!(
                    "✅ 任务完成: id={}, 房间={}, 面板={}, 构件={}, 耗时={:?}",
                    task_id, stats.total_rooms, stats.total_panels, stats.total_components, duration
                );
                task.status = RoomWorkerTaskStatus::Completed { stats };
            }
            Err(e) => {
                let error_msg = e.to_string();
                if error_msg.contains("取消") || error_msg.contains("cancelled") {
                    info!("🛑 任务已取消: id={}", task_id);
                    task.status = RoomWorkerTaskStatus::Cancelled;
                } else {
                    error!("❌ 任务失败: id={}, error={}", task_id, error_msg);
                    task.status = RoomWorkerTaskStatus::Failed { error: error_msg };
                }
            }
        }

        // 从活跃任务中移除，添加到已完成
        self.active_tasks.remove(&task_id);
        self.cancel_tokens.remove(&task_id);
        self.completed_tasks.write().await.push(task);

        // 清理旧的已完成任务（保留最近 100 个）
        let mut completed = self.completed_tasks.write().await;
        if completed.len() > 100 {
            let target_len = completed.len() - 100;
            completed.drain(0..target_len);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = RoomWorkerConfig::default();
        assert_eq!(config.max_concurrent_tasks, 1);
        assert_eq!(config.task_timeout_secs, 3600);
        assert_eq!(config.progress_report_interval_ms, 1000);
    }

    #[test]
    fn test_task_status_is_terminal() {
        assert!(!RoomWorkerTaskStatus::Queued.is_terminal());
        assert!(!RoomWorkerTaskStatus::Running {
            progress: 0.5,
            stage: "test".to_string()
        }.is_terminal());
        assert!(RoomWorkerTaskStatus::Completed {
            stats: RoomBuildStats {
                total_rooms: 0,
                total_panels: 0,
                total_components: 0,
                build_time_ms: 0,
                cache_hit_rate: 0.0,
                memory_usage_mb: 0.0,
            }
        }.is_terminal());
        assert!(RoomWorkerTaskStatus::Failed {
            error: "test".to_string()
        }.is_terminal());
        assert!(RoomWorkerTaskStatus::Cancelled.is_terminal());
    }
}
