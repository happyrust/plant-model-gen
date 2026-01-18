//! db_meta_cache - ref0 -> dbnum 缓存
//! 解析期生成的 db_meta_info.json 数据在此缓存，以便快速查询 refno 对应的 dbnum

use aios_core::RefnoEnum;

/// 根据 refno 获取 dbnum（使用 aios_core 的实现）
pub fn get_dbnum_for_refno(refno: RefnoEnum) -> Option<u32> {
    aios_core::get_dbnum_by_refno(refno.refno())
}
