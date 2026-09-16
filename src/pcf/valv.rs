//! VALV（阀门）元件的 PCF 记录生成。
//!
//! 阀门是 PCF 里字段最全的元件之一，因为下游既要提料、又要在轴测图上标注：
//!
//! - **中心点**：阀门有实体长度，需要独立定位；
//! - **SKEY + 材料码 + 焊接规格**：提料与排焊口用；
//! - **Name**：阀门通常有位号（如 `V-1001`），要在图上标出来，
//!   所以这里额外写一行 PDMS 侧的元素名；
//! - **ANGLE**：三通阀 / 角阀的转角，普通直通阀取不到该属性时自动省略。
//!
//! 本文件 `use crate::pcf::pcf_api::*` 是通配导入，用到的
//! `create_*` 原语都来自那里。

use aios_core::AttrMap;
use aios_core::pdms_types::*;
use sqlx::{MySql, Pool};
use crate::data_interface::tidb_manager::AiosDBManager;
use crate::pcf::bran::{gen_item_code_data_attr_val, gen_refno_data};
use crate::pcf::pcf_api::*;

/// 生成一个 VALV 元件的 PCF 字段块。
pub async fn gen_valv_data(aios_mgr: &AiosDBManager, attr: &AttrMap, pool: &Pool<MySql>, materials: &mut Vec<(RefU64, String)>) -> Vec<u8> {
    let mut data = vec![];
    let refno = attr.get_refno();
    if refno.is_none() { return vec![]; }
    let refno = refno.unwrap();
    data.append(&mut create_center_point_data(refno, aios_mgr).await);
    data.append(&mut create_s_key_data(attr, aios_mgr).await);
    let spre = attr.get_val("SPRE");
    data.append(&mut gen_item_code_data_attr_val(spre, &aios_mgr, materials).await);
    data.append(&mut create_weld_spec_data(attr, aios_mgr).await);
    // 取不到名字时回落到默认值，保证这一行始终存在——PCF 读取端不接受空位号。
    data.append(&mut gen_name_data(attr.get_name_or_default().as_str()));
    data.append(&mut create_angl_data(attr));
    data.append(&mut create_refno_data(attr));
    data
}

/// 输出阀门位号行。缩进 8 空格、`\r\n` 结尾是 PCF 的硬性格式要求，别改。
fn gen_name_data(name: &str) -> Vec<u8> { format!("        Name  {}\r\n", name).into_bytes() }
