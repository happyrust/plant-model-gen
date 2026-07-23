use aios_core::RefnoEnum;
use thiserror::Error;

pub type GenerationReadResult<T> = Result<T, GenerationReadError>;

#[derive(Debug, Error)]
pub enum GenerationReadError {
    #[error("权威 snapshot 不可用: snapshot_id={snapshot_id}")]
    SnapshotUnavailable { snapshot_id: u64 },

    #[error("输入版本清单不匹配: snapshot_id={snapshot_id}, expected={expected}, actual={actual}")]
    ManifestMismatch {
        snapshot_id: u64,
        expected: String,
        actual: String,
    },

    #[error("能力 {capability} 缺少必需数据: {refnos:?}")]
    MissingRequiredData {
        capability: &'static str,
        refnos: Vec<RefnoEnum>,
    },

    #[error("属性 payload 损坏: refno={refno}, detail={detail}")]
    PayloadCorrupt { refno: RefnoEnum, detail: String },

    #[error("读取后端 {backend} 执行 {operation} 失败: {message}")]
    BackendQuery {
        backend: &'static str,
        operation: &'static str,
        message: String,
    },

    #[error(
        "生成读取历史已超出 retention 窗口: operation={operation}; \
         请改用 PDMS 源 db 文件重扫或放宽 version_retention: {message}"
    )]
    HistoryExpired {
        operation: &'static str,
        message: String,
    },

    #[error("生成读取契约非法: {0}")]
    InvalidReadSpec(String),

    #[error("读取后端 {backend} 尚不能兑现显式 read_at={read_at}；已拒绝读取，未回退到 latest")]
    UnsupportedReadAt {
        backend: &'static str,
        read_at: String,
    },

    #[error("双后端对拍不一致: capability={capability}, detail={detail}")]
    ParityMismatch {
        capability: &'static str,
        detail: String,
    },

    #[error("版本化读取性能门禁失败: capability={capability}, detail={detail}")]
    PerformanceGate { capability: String, detail: String },

    #[error("层级数据非法: {0}")]
    InvalidHierarchy(String),

    #[error("CATA 闭包非法: {0}")]
    InvalidCatalog(String),

    #[error("输入版本清单非法: {0}")]
    InvalidManifest(String),
}
