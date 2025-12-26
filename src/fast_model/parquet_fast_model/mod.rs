//! Parquet Fast Model - 完全独立于 SurrealDB 的轻量级模型生成模块
//! 
//! 直接从 parquet 文件读取数据并生成 GLB 模型，无需数据库运行。

#[cfg(feature = "duckdb-feature")]
pub mod data_source;
#[cfg(feature = "duckdb-feature")]
pub mod primitive_builder;
#[cfg(feature = "duckdb-feature")]
pub mod glb_exporter;

#[cfg(feature = "duckdb-feature")]
pub use data_source::DuckDbDataSource;
#[cfg(feature = "duckdb-feature")]
pub use primitive_builder::PrimitiveBuilder;
#[cfg(feature = "duckdb-feature")]
pub use glb_exporter::ParquetGlbExporter;
