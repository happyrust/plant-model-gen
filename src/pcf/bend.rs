//! BEND（弯管）元件的 PCF 记录生成。
//!
//! 弯管与弯头（ELBO）的区别：弯头是买来的成品配件，弯管是把直管**煨弯**
//! 而成。所以弯管要额外交待「弯成什么样」——半径和角度两个参数缺一不可，
//! 预制车间照着它下料弯管。
//!
//! 字段组合：中心点 + SKEY + 弯曲半径 + 角度 + 材料码 + 焊接规格 + refno。
//! 半径和角度的读取原语复用弯头模块（[`crate::pcf::elbo::create_radius_data`]），
//! 两者在源库里用的是同一组属性。

use aios_core::AttrMap;
use aios_core::pdms_types::*;
use sqlx::{MySql, Pool};
use crate::data_interface::tidb_manager::AiosDBManager;
use crate::pcf::bran::gen_item_code_data_attr_val;
use crate::pcf::elbo::{create_radius_data, get_catr_para_data_from_spre};
use crate::pcf::pcf_api::{create_angl_data, create_center_point_data, create_refno_data, create_s_key_data, create_weld_spec_data};
use crate::pcf::tee::create_tee_branch_point_data;

/// 生成一个 BEND 元件的 PCF 字段块。
pub async fn gen_bend_data(aios_mgr: &AiosDBManager, attr: &AttrMap, pool: &Pool<MySql>, materials: &mut Vec<(RefU64, String)>) -> Vec<u8> {
    let mut data = vec![];
    let refno = attr.get_refno();
    if refno.is_none() { return vec![]; }
    let refno = refno.unwrap();
    let spre = attr.get_val("SPRE");
    data.append(&mut create_center_point_data(refno, aios_mgr).await);
    data.append(&mut create_s_key_data(attr, aios_mgr).await);
    // 半径改为直接读元素属性，不再绕 SPRE 去查元件库详图参数：
    // 煨弯半径是设计现场定的，元件库里那份未必与实际一致。
    // data.append(&mut get_catr_para_data_from_spre(spre, &aios_mgr, pool).await); // BEND-RADIUS
    data.append(&mut create_radius_data(attr));
    data.append(&mut create_angl_data(attr));
    data.append(&mut gen_item_code_data_attr_val(spre, aios_mgr, materials).await);
    data.append(&mut create_weld_spec_data(attr, aios_mgr).await);
    data.append(&mut create_refno_data(attr));
    data
}
