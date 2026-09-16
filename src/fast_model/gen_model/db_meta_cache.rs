//! db_meta_cache - ref0 -> dbnum 缓存
//! 解析期生成的 db_meta_info.json 数据在此缓存，以便快速查询 refno 对应的 dbnum

use aios_core::RefnoEnum;

/// 根据 refno 获取 dbnum。
///
/// 注意：refno 的高 32 位是 ref0，不是 dbnum，禁止回退到“直接拿 ref0 当 dbnum”的猜测逻辑。
pub fn get_dbnum_for_refno(refno: RefnoEnum) -> Option<u32> {
    let db_meta = crate::data_interface::db_meta_manager::db_meta();
    let _ = db_meta.ensure_loaded();
    db_meta.get_dbnum_by_refno(refno)
}
