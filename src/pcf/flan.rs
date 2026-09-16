//! FLAN（法兰）元件的 PCF 记录生成。
//!
//! 法兰是螺栓连接的接口件，下游预制软件要靠它算螺栓与垫片清单，所以除了
//! SKEY 之外还必须带上**材料码**与**焊接规格**：
//!
//! - 材料码（ITEM-CODE）从 SPRE（规格引用）查出，同时把用到的材料登记进
//!   `materials`，供整篇 PCF 末尾统一输出材料表；
//! - 焊接规格（WELD-SPEC）区分 BW（对焊）与 RF（突面），来源是元件库
//!   详图的 RTEX 文本，解析细节见 `create_weld_spec_data`。
//!
//! 法兰不需要中心点：它的位置由所在 BRAN 的端点序列决定。

use aios_core::AttrMap;
use aios_core::pdms_types::*;
use sqlx::{MySql, Pool};
use crate::data_interface::tidb_manager::AiosDBManager;
use crate::pcf::bran::gen_item_code_data_attr_val;
use crate::pcf::pcf_api::{create_refno_data, create_s_key_data, create_weld_spec_data};

/// 生成一个 FLAN 元件的 PCF 字段块。
///
/// `materials` 是**出参**：本函数会把该法兰用到的材料追加进去，
/// 调用方在整条管路走完后统一去重输出，避免同一材料重复写入。
pub async fn gen_flan_data(aios_mgr:&AiosDBManager, attr: &AttrMap, pool: &Pool<MySql>,materials:&mut Vec<(RefU64,String)>) -> Vec<u8> {
    let mut data = vec![];
    data.append(&mut create_s_key_data(attr,aios_mgr).await);
    // SPRE 指向规格库中的选型条目，材料码由它反查而来。
    let spre = attr.get_val("SPRE");
    data.append(&mut gen_item_code_data_attr_val(spre, aios_mgr,materials).await);
    data.append(&mut create_weld_spec_data(attr, aios_mgr).await);
    data.append(&mut create_refno_data(attr));
    data
}
