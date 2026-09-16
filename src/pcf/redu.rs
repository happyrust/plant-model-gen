//! REDU（异径管 / 大小头）元件的 PCF 记录生成。
//!
//! 异径管连接两种不同管径的管子。它的两端口径差异**不在这里写**——
//! 端点与口径由 [`crate::pcf::pcf_api::gen_node_basic_data`] 依据前后
//! TubiEdge 统一输出，本函数只补该元件自己的选型与连接信息。
//!
//! 字段组合：SKEY（规格代号）+ 材料码 + 焊接规格 + refno。
//! 带焊接规格是因为异径管通常对焊接入管线，下游要据此排焊口。

use aios_core::AttrMap;
use aios_core::pdms_types::*;
use sqlx::{MySql, Pool};
use crate::data_interface::tidb_manager::AiosDBManager;
use crate::pcf::bran::gen_item_code_data_attr_val;
use crate::pcf::pcf_api::{create_refno_data, create_s_key_data, create_weld_spec_data};

/// 生成一个 REDU 元件的 PCF 字段块。
///
/// `materials` 为出参：登记本件材料，供整篇 PCF 末尾统一汇总输出。
pub async fn gen_redu_data(aios_mgr: &AiosDBManager, attr: &AttrMap, pool: &Pool<MySql>, materials: &mut Vec<(RefU64, String)>) -> Vec<u8> {
    let mut data = vec![];
    // 无 refno 的元素无法对账，直接跳过而不是写半条记录。
    let refno = attr.get_refno();
    if refno.is_none() { return vec![]; }
    let refno = refno.unwrap();
    data.append(&mut create_s_key_data(attr, aios_mgr).await);
    let spre = attr.get_val("SPRE");
    data.append(&mut gen_item_code_data_attr_val(spre, aios_mgr, materials).await);
    data.append(&mut create_weld_spec_data(attr, aios_mgr).await);
    data.append(&mut create_refno_data(attr));
    data
}
