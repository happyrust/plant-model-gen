use thiserror::Error;

/// GenPipeline特定的错误类型
///
/// 提供类型安全和清晰的错误信息，替代通用的 anyhow::Error
#[derive(Error, Debug)]
pub enum GenPipelineError {
    /// SJUS map 为空，可能导致几何体生成错误
    #[error("Empty SJUS map detected - geometry generation may produce incorrect results")]
    EmptySjusMap,

    /// 并发配置值无效
    #[error("Invalid concurrency value: {0}, must be between {1} and {2}")]
    InvalidConcurrency(usize, usize, usize),

    /// 批次大小无效
    #[error("Invalid batch size: {0}, must be greater than 0")]
    InvalidBatchSize(usize),

    /// 数据库查询失败
    #[error("Database query failed: {0}")]
    DatabaseError(String),

    /// 几何体生成失败
    #[error("Geometry generation failed for {0}: {1}")]
    GeometryGenerationFailed(String, String),

    /// 包装其他错误
    #[error("Internal error: {0}")]
    Other(#[from] anyhow::Error),
}

/// Result 类型别名
pub type Result<T> = std::result::Result<T, GenPipelineError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invalid_concurrency() {
        let err = GenPipelineError::InvalidConcurrency(10, 2, 8);
        assert!(err.to_string().contains("10"));
        assert!(err.to_string().contains("2"));
        assert!(err.to_string().contains("8"));
    }
}
