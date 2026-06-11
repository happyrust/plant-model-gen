//! 共享模块
//!
//! 包含跨服务共享的核心组件：
//! - progress_hub: 统一进度广播中心（服务于 gRPC 和 WebSocket）
//! - net_util: 本机网络地址推断（web_server / web_api 共用）

pub mod net_util;
pub mod progress_hub;

// 重新导出常用类型
pub use progress_hub::{ProgressHub, ProgressMessage, ProgressMessageBuilder, TaskStatus};
