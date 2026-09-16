//! GRPC服务模块
//!
//! 提供数据解析进度监控、MDB管理和任务控制的GRPC接口

#[cfg(feature = "grpc")]
pub mod progress_service;

#[cfg(feature = "grpc")]
pub mod server;

#[cfg(feature = "grpc")]
pub mod error;

#[cfg(feature = "grpc")]
pub mod types;

#[cfg(feature = "grpc")]
pub mod managers;

#[cfg(feature = "grpc")]
pub mod integration;

#[cfg(feature = "grpc")]
pub mod logging;

#[cfg(feature = "grpc")]
pub mod auth;

#[cfg(feature = "grpc")]
pub mod health;

// The spatial query gRPC implementation was removed, but the `grpc` feature is
// also disabled in this workspace today. Keep it out of the module tree so
// workspace-wide tools like `cargo fmt --all` can parse successfully.

// Additional spatial gRPC modules are currently absent from `src/grpc_service/`.
// Keep these declarations disabled until their implementations are restored.

#[cfg(feature = "grpc")]
#[cfg(test)]
pub mod tests;

// 重新导出主要类型
#[cfg(feature = "grpc")]
pub use error::ServiceError;

#[cfg(feature = "grpc")]
pub use server::start_grpc_server;

#[cfg(feature = "grpc")]
pub use logging::{GrpcRequestLogger, PERFORMANCE_METRICS, TaskExecutionLogger, init_grpc_logging};

#[cfg(feature = "grpc")]
pub use auth::{AuthConfig, AuthInterceptor, AuthService, InputValidator, RateLimiter};

#[cfg(feature = "grpc")]
pub use health::{HealthChecker, HealthMonitorService, HealthStatus};
