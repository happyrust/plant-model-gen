//! 进度回调抽象层
//!
//! 提供 `ProgressCallback` trait，使 `gen_all_geos_data` 管线可以向不同消费者上报进度：
//! - `NoopProgress`：CLI / 测试模式，零开销空实现
//! - `HubProgress`：Web Server 模式，桥接 `ProgressHub.publish()`（WebSocket 推送）
//! - `ChannelProgress`：GUI Offline 模式，通过 `flume::Sender` 传递给 GUI 线程

use std::sync::Arc;

use super::progress_hub::{ProgressHub, ProgressMessage, ProgressMessageBuilder, TaskStatus};

/// 进度回调 trait — 管线各阶段通过此接口上报进度
///
/// 设计目标：
/// - 零成本抽象：`NoopProgress` 编译优化为空
/// - `Send + Sync`：支持跨 tokio task 传递
/// - 兼容 `ProgressHub`：`HubProgress` 实现此 trait
pub trait ProgressCallback: Send + Sync + 'static {
    /// 上报进度消息
    fn report(&self, message: ProgressMessage);

    /// 便捷方法：上报阶段进度
    fn report_step(
        &self,
        task_id: &str,
        step_name: &str,
        step_number: u32,
        total_steps: u32,
        percentage: f32,
        message: &str,
    ) {
        self.report(
            ProgressMessageBuilder::new(task_id)
                .status(TaskStatus::Running)
                .step(step_name, step_number, total_steps)
                .percentage(percentage)
                .message(message)
                .build(),
        );
    }

    /// 便捷方法：上报带 items 的阶段进度
    fn report_items(
        &self,
        task_id: &str,
        step_name: &str,
        step_number: u32,
        total_steps: u32,
        percentage: f32,
        processed: u64,
        total: u64,
        message: &str,
    ) {
        self.report(
            ProgressMessageBuilder::new(task_id)
                .status(TaskStatus::Running)
                .step(step_name, step_number, total_steps)
                .percentage(percentage)
                .items(processed, total)
                .message(message)
                .build(),
        );
    }
}

// ---------------------------------------------------------------------------
// NoopProgress — CLI 模式零开销
// ---------------------------------------------------------------------------

/// 空实现，用于 CLI 模式或不需要进度回调的场景
pub struct NoopProgress;

impl ProgressCallback for NoopProgress {
    #[inline]
    fn report(&self, _message: ProgressMessage) {}
}

// ---------------------------------------------------------------------------
// HubProgress — Web Server 模式（WebSocket 推送）
// ---------------------------------------------------------------------------

/// 桥接 `ProgressHub`，将管线进度发布到 broadcast channel，
/// 由 WebSocket handler 自动推送给前端
pub struct HubProgress {
    hub: Arc<ProgressHub>,
    task_id: String,
}

impl HubProgress {
    pub fn new(hub: Arc<ProgressHub>, task_id: String) -> Self {
        Self { hub, task_id }
    }
}

impl ProgressCallback for HubProgress {
    fn report(&self, mut message: ProgressMessage) {
        message.task_id.clone_from(&self.task_id);
        let _ = self.hub.publish(message);
    }
}

// ---------------------------------------------------------------------------
// ChannelProgress — GUI Offline 模式
// ---------------------------------------------------------------------------

/// 通过 `flume::Sender` 将进度消息发送给 GUI 线程
pub struct ChannelProgress {
    sender: flume::Sender<ProgressMessage>,
}

impl ChannelProgress {
    pub fn new(sender: flume::Sender<ProgressMessage>) -> Self {
        Self { sender }
    }
}

impl ProgressCallback for ChannelProgress {
    fn report(&self, message: ProgressMessage) {
        let _ = self.sender.try_send(message);
    }
}

// ---------------------------------------------------------------------------
// 辅助函数
// ---------------------------------------------------------------------------

/// 从 `Option` 中解包 progress callback，若为 `None` 则返回 `NoopProgress`
pub fn resolve_progress(progress: Option<Arc<dyn ProgressCallback>>) -> Arc<dyn ProgressCallback> {
    progress.unwrap_or_else(|| Arc::new(NoopProgress))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_noop_progress() {
        let progress = NoopProgress;
        progress.report_step("test", "init", 1, 5, 10.0, "testing");
        // NoopProgress should not panic
    }

    #[test]
    fn test_channel_progress() {
        let (tx, rx) = flume::bounded::<ProgressMessage>(16);
        let progress = ChannelProgress::new(tx);

        progress.report_step("task-1", "几何生成", 2, 5, 30.0, "processing");

        let msg = rx.try_recv().unwrap();
        assert_eq!(msg.task_id, "task-1");
        assert_eq!(msg.current_step, "几何生成");
        assert_eq!(msg.current_step_number, 2);
        assert_eq!(msg.total_steps, 5);
        assert_eq!(msg.percentage, 30.0);
    }

    #[test]
    fn test_channel_progress_items() {
        let (tx, rx) = flume::bounded::<ProgressMessage>(16);
        let progress = ChannelProgress::new(tx);

        progress.report_items("task-2", "布尔运算", 4, 5, 75.0, 150, 200, "boolean ops");

        let msg = rx.try_recv().unwrap();
        assert_eq!(msg.processed_items, 150);
        assert_eq!(msg.total_items, 200);
        assert_eq!(msg.percentage, 75.0);
    }

    #[test]
    fn test_resolve_progress_none() {
        let p = resolve_progress(None);
        // Should not panic
        p.report_step("t", "s", 1, 1, 0.0, "");
    }

    #[test]
    fn test_resolve_progress_some() {
        let (tx, rx) = flume::bounded::<ProgressMessage>(16);
        let p: Arc<dyn ProgressCallback> = Arc::new(ChannelProgress::new(tx));
        let p = resolve_progress(Some(p));

        p.report_step("t", "s", 1, 1, 50.0, "half");
        let msg = rx.try_recv().unwrap();
        assert_eq!(msg.percentage, 50.0);
    }

    /// 模拟 gen_all_geos_data 完整管线（按实际执行顺序），验证进度单调递增且首尾正确
    #[test]
    fn test_full_pipeline_simulation() {
        let (tx, rx) = flume::bounded::<ProgressMessage>(64);
        let progress: Arc<dyn ProgressCallback> = Arc::new(ChannelProgress::new(tx));

        // Step 0-2: 初始化阶段
        progress.report_step("gen_model", "初始化",   0, 9, 0.0, "初始化模型生成管线");
        progress.report_step("gen_model", "预检查",   1, 9, 2.0, "数据库预检查");
        progress.report_step("gen_model", "路由决策", 2, 9, 8.0, "生成策略路由");

        // Step 3: 几何体生成（stage 起始 + 5 个 chunk 细粒度）
        progress.report_step("gen_model", "几何体生成", 3, 9, 10.0, "开始几何体生成");
        for i in 1..=5u64 {
            progress.report_items(
                "gen_model", "几何体生成", 3, 9,
                10.0 + (i as f32 / 5.0) * 45.0,
                i * 20, 100,
                &format!("chunk {}/5", i),
            );
        }

        // Step 4-5: 数据写入
        progress.report_step("gen_model", "实例数据入库", 4, 9, 55.0, "写入实例数据");
        progress.report_step("gen_model", "AABB写入",     5, 9, 65.0, "写入AABB数据");

        // Step 6: 布尔运算（stage 起始 + 3 个 task 细粒度）
        progress.report_step("gen_model", "布尔运算", 6, 9, 70.0, "开始布尔运算");
        for i in 1..=3u64 {
            progress.report_items(
                "gen_model", "布尔运算", 6, 9,
                70.0 + (i as f32 / 3.0) * 20.0,
                i * 10, 30,
                &format!("布尔运算 {}/3", i),
            );
        }

        // Step 7-8: 导出
        progress.report_step("gen_model", "导出",         7, 9, 90.0, "导出模型数据");
        progress.report_step("gen_model", "Instances导出", 8, 9, 95.0, "导出实例数据");

        // 完成
        progress.report(
            ProgressMessageBuilder::new("gen_model")
                .status(TaskStatus::Completed)
                .step("完成", 9, 9)
                .percentage(100.0)
                .message("模型生成完成")
                .build(),
        );

        // 收集所有消息
        let msgs: Vec<ProgressMessage> = rx.try_iter().collect();

        // 3 初始化 + 1 几何起始 + 5 chunk + 2 写入 + 1 布尔起始 + 3 bool + 2 导出 + 1 完成 = 18
        assert_eq!(msgs.len(), 18, "应收到 18 条进度消息");

        // 验证首尾
        assert_eq!(msgs.first().unwrap().percentage, 0.0);
        assert_eq!(msgs.first().unwrap().current_step, "初始化");
        assert_eq!(msgs.last().unwrap().percentage, 100.0);
        assert_eq!(msgs.last().unwrap().status, TaskStatus::Completed);

        // 验证进度单调递增
        let mut last_pct = -1.0_f32;
        for m in &msgs {
            assert!(
                m.percentage >= last_pct,
                "进度应单调递增: {} < {} (step={})",
                m.percentage, last_pct, m.current_step
            );
            last_pct = m.percentage;
        }

        // 验证 task_id 一致
        for m in &msgs {
            assert_eq!(m.task_id, "gen_model");
        }
    }

    /// 验证 ChannelProgress 在接收端关闭后不 panic
    #[test]
    fn test_channel_progress_dropped_receiver() {
        let (tx, rx) = flume::bounded::<ProgressMessage>(1);
        let progress = ChannelProgress::new(tx);
        drop(rx);

        // 不应 panic
        progress.report_step("t", "s", 1, 1, 0.0, "should not panic");
    }

    /// 验证 Arc<dyn ProgressCallback> 可跨线程使用
    #[test]
    fn test_progress_callback_send_sync() {
        let (tx, rx) = flume::bounded::<ProgressMessage>(16);
        let progress: Arc<dyn ProgressCallback> = Arc::new(ChannelProgress::new(tx));

        let p = progress.clone();
        let handle = std::thread::spawn(move || {
            p.report_step("cross-thread", "测试", 1, 1, 50.0, "跨线程");
        });
        handle.join().unwrap();

        let msg = rx.try_recv().unwrap();
        assert_eq!(msg.task_id, "cross-thread");
        assert_eq!(msg.percentage, 50.0);
    }
}
