//! INST（在线仪表）元件的 PCF 记录生成。
//!
//! 指串接在管路上的仪表件——流量计、视镜、限流孔板之类。它既是管道元件
//! （占一段长度、有材料、要焊接），又需要**独立的中心点**：仪表在 PCF 里
//! 常被单独提料和定位，只靠前后端点不足以定位它自身。
//!
//! 字段组合：中心点 + SKEY + 材料码 + 焊接规格 + refno，
//! 是 PCF 元件里比较完整的一档。

use aios_core::AttrMap;
use aios_core::pdms_types::*;
use sqlx::{MySql, Pool};
use crate::data_interface::tidb_manager::AiosDBManager;
use crate::pcf::bran::gen_item_code_data_attr_val;
use crate::pcf::pcf_api::{create_center_point_data, create_refno_data, create_s_key_data, create_weld_spec_data};

/// 生成一个 INST 元件的 PCF 字段块。
pub async fn gen_inst_data(aios_mgr: &AiosDBManager, attr: &AttrMap, pool: &Pool<MySql>, materials: &mut Vec<(RefU64, String)>) -> Vec<u8> {
    let mut data = vec![];
    let refno = attr.get_refno();
    if refno.is_none() { return vec![]; }
    // 查世界变换要用 refno，所以这里必须解包出来。
    let refno = refno.unwrap();
    data.append(&mut create_center_point_data(refno, aios_mgr).await);
    data.append(&mut create_s_key_data(attr, aios_mgr).await);
    let spre = attr.get_val("SPRE");
    data.append(&mut gen_item_code_data_attr_val(spre, aios_mgr, materials).await);
    data.append(&mut create_weld_spec_data(attr, aios_mgr).await);
    data.append(&mut create_refno_data(attr));
    data
}
