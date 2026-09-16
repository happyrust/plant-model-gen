//! CAP（管帽）元件的 PCF 记录生成。
//!
//! 管帽封住管路末端，因此它在 PCF 里还有一层**结构含义**：
//! [`crate::pcf::pcf_api::gen_node_basic_data`] 遇到 CAP 会返回 `true`，
//! 通知调用方这条 BRAN 到此为止，后面的节点不用再遍历了。
//!
//! 字段很少——管帽没有角度、没有分支点，只需要 SKEY（元件规格代号）
//! 和 refno（回溯到源库元素的引用号）。

use aios_core::AttrMap;
use aios_core::pdms_types::*;
use sqlx::{MySql, Pool};
use crate::data_interface::tidb_manager::AiosDBManager;
use crate::pcf::bran::gen_item_code_data_attr_val;
use crate::pcf::pcf_api::{create_refno_data, create_s_key_data};

/// 生成一个 CAP 元件的 PCF 字段块。
///
/// 返回空 `Vec` 表示「这个元件不产出记录」，调用方直接跳过即可，不视为错误。
pub async fn gen_cap_data(aios_mgr:&AiosDBManager, attr: &AttrMap, pool:&Pool<MySql>) -> Vec<u8> {
    let mut data = vec![];
    // 没有 refno 的元素无法回溯来源，直接放弃：宁可少一条记录，
    // 也不要往 PCF 里写一条没法对账的元件。
    let refno = attr.get_refno();
    if refno.is_none() { return vec![]; }
    // SKEY 要顺着 SPRE → DETR 两跳才查得到，细节见 create_s_key_data。
    data.append(&mut create_s_key_data(attr,aios_mgr).await);
    data.append(&mut create_refno_data(attr));
    data
}
