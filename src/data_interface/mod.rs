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

/// MBD 部署前候选发现 — 离线读 SYST 枚举 MDB 及成员 DB 文件定位状态。
pub mod mdb_candidates;

/// T008 离线校验：按需闭包站点 vs 全量基准站点的生成结果一致性对比（spec 002）。
/// 依赖 gen_model 的 fast_model::utils 与 reqwest HTTP 基准直连；reqwest 仅由
/// web_server feature 提供，故瘦构建（sync-cli，无 web_server）不含该校验工具。
#[cfg(all(
    feature = "surreal-save",
    feature = "gen_model",
    feature = "web_server"
))]
pub mod cata_closure_verify;

pub mod increment_record;

pub mod mqtt_file_sync;

pub mod sesno_increment;

pub mod tidb_manager;

pub use db_meta_manager::{DbMetaManager, db_meta, get_dbnum, ref0s_to_dbnums};

// #[cfg(test)]
// mod tests;
