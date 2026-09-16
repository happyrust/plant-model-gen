//! GASK（垫片）元件的 PCF 记录生成。
//!
//! 垫片总是夹在两片法兰之间，本身没有独立几何尺寸可言，在 PCF 里主要起
//! **材料清单**的作用——下游要按它统计密封件用量。因此字段组合是
//! 「SKEY + 材料码 + refno」，不带端点、不带焊接规格。
//!
//! 注意它引入了 `create_center_point_data` 却没有调用：垫片的位置完全
//! 由相邻法兰决定，单独给中心点反而会和法兰的端点冲突。

use aios_core::AttrMap;
use aios_core::pdms_types::*;
use sqlx::{MySql, Pool};
use crate::data_interface::tidb_manager::AiosDBManager;
use crate::pcf::bran::gen_item_code_data_attr_val;
use crate::pcf::pcf_api::{create_center_point_data, create_refno_data, create_s_key_data};

/// 生成一个 GASK 元件的 PCF 字段块。
///
/// `materials` 为出参，用法同其他元件：登记本件用到的材料，末尾统一输出。
pub async fn gen_gask_data(aios_mgr:&AiosDBManager,attr: &AttrMap,pool:&Pool<MySql>,materials:&mut Vec<(RefU64,String)>) -> Vec<u8> {
    let mut data = vec![];
    // 无 refno 即无法回溯源库元素，放弃这条记录。
    let refno = attr.get_refno();
    if refno.is_none() { return vec![]; }
    data.append(&mut create_s_key_data(attr,aios_mgr).await);
    // 垫片的材质（石墨、金属缠绕等）就藏在 SPRE 选型里。
    let spre = attr.get_val("SPRE");
    data.append(&mut gen_item_code_data_attr_val(spre, aios_mgr,materials).await);
    data.append(&mut create_refno_data(attr));
    data
}
