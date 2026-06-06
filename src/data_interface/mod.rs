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

pub mod increment_manager;

pub mod increment_record;

pub mod sesno_increment;

pub mod tidb_manager;

pub use db_meta_manager::{DbMetaManager, db_meta, get_dbnum, ref0s_to_dbnums};

// #[cfg(test)]
// mod tests;
