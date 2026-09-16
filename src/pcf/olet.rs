//! OLET（支管台，weldolet / sockolet 一类）元件的 PCF 记录生成。
//!
//! 支管台是在主管上开孔焊接、引出一路支管的元件，所以它和三通一样是
//! **分叉点**：除了自身中心点，还必须输出**支管点**（branch point），
//! 下游才知道支路从哪里岔出去。支管点的算法与三通共用，因此直接复用了
//! [`crate::pcf::tee::create_tee_branch_point_data`]。
//!
//! 字段组合：中心点 + 支管点 + SKEY + 材料码 + refno。
//! 不写焊接规格——支管台的焊接形式已由 SKEY 表达。

use aios_core::AttrMap;
use aios_core::pdms_types::*;
use sqlx::{MySql, Pool};
use crate::data_interface::tidb_manager::AiosDBManager;
use crate::pcf::bran::gen_item_code_data_attr_val;
use crate::pcf::pcf_api::{create_center_point_data, create_refno_data, create_s_key_data};
use crate::pcf::tee::create_tee_branch_point_data;

/// 生成一个 OLET 元件的 PCF 字段块。
pub async fn gen_olet_data(aios_mgr: &AiosDBManager, attr: &AttrMap, pool: &Pool<MySql>, materials: &mut Vec<(RefU64, String)>) -> Vec<u8> {
    let mut data = vec![];
    let refno = attr.get_refno();
    if refno.is_none() { return vec![]; }
    let refno = refno.unwrap();
    data.append(&mut create_center_point_data(refno, aios_mgr).await);
    // 支管点必须紧跟中心点输出，PCF 读取端按出现顺序区分主管与支路。
    data.append(&mut create_tee_branch_point_data(aios_mgr, attr, pool).await);
    data.append(&mut create_s_key_data(attr, aios_mgr).await);
    let spre = attr.get_val("SPRE");
    data.append(&mut gen_item_code_data_attr_val(spre, aios_mgr, materials).await);
    data.append(&mut create_refno_data(attr));
    data
}
