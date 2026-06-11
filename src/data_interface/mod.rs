pub mod db_model;
// pub mod spatial_model;
pub mod interface;
pub mod structs;

pub mod mesh_manager;

pub mod db_manager;

pub mod db_meta_manager;

/// 全库 ref0/dbnum 预扫描索引（index-only，写 SQLite）。依赖 rusqlite（sqlite-index feature）。
#[cfg(feature = "sqlite-index")]
pub mod db_index;

/// 元件库（CATA）按需解析 — refno 级引用闭包的基础原语（spec 002）。
pub mod cata_closure;

/// T008 离线校验：按需闭包站点 vs 全量基准站点的生成结果一致性对比（spec 002）。
#[cfg(feature = "surreal-save")]
pub mod cata_closure_verify;

pub mod increment_manager;

pub mod increment_record;

pub mod sesno_increment;

pub mod tidb_manager;

pub use db_meta_manager::{DbMetaManager, db_meta, get_dbnum, ref0s_to_dbnums};

// #[cfg(test)]
// mod tests;
