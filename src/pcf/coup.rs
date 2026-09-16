//! COUP（管接头）元件的 PCF 记录生成。
//!
//! 管接头用于两段管子的直连（螺纹或承插），几何上是一小段过渡件。
//! 在 PCF 里它只需要「SKEY + 材料码 + refno」三组字段：位置由所在 BRAN 的
//! 端点序列推出，不必单独给中心点；连接方式已经体现在 SKEY 里，
//! 也就不再单独写焊接规格。

use aios_core::AttrMap;
use aios_core::pdms_types::*;
use sqlx::{MySql, Pool};
use crate::data_interface::tidb_manager::AiosDBManager;
use crate::pcf::bran::gen_item_code_data_attr_val;
use crate::pcf::pcf_api::{create_refno_data, create_s_key_data};

/// 生成一个 COUP 元件的 PCF 字段块。
///
/// `materials` 为出参，登记本件用到的材料，供整篇 PCF 末尾汇总。
pub async fn gen_coup_data(aios_mgr: &AiosDBManager, attr: &AttrMap, pool: &Pool<MySql>, materials: &mut Vec<(RefU64, String)>) -> Vec<u8> {
    let mut data = vec![];
    // 无 refno 就没法回溯来源，整条记录作废。
    let refno = attr.get_refno();
    if refno.is_none() { return vec![]; }
    // 绑定局部变量是为了与其他元件保持一致的写法；本函数后续并未真正用到它。
    let refno = refno.unwrap();
    data.append(&mut create_s_key_data(attr, aios_mgr).await);
    let spre = attr.get_val("SPRE");
    data.append(&mut gen_item_code_data_attr_val(spre, aios_mgr, materials).await);
    data.append(&mut create_refno_data(attr));
    data
}
