//! ATTA（附件）元件的 PCF 记录生成。
//!
//! ATTA 是挂在管路上的附属件，最典型的是**管支架**。它在 PCF 里有两点特殊：
//!
//! 1. **类型名取自属性而非硬编码**：ATTA 底下还分若干子类，所以用
//!    `attr.get_type()` 原样透传，不像 NOZZ 那样映射成固定关键字。
//! 2. **用 cords 点而不是 center 点**：附件要标注的是安装坐标，
//!    与元件几何中心不是一回事。
//!
//! 另外它会输出 SUPPORT-TYPE（源属性 STEX），这是支架特有的字段，
//! 下游据此区分固定架、导向架、弹簧吊架等。
//!
//! 注意：[`crate::pcf::pcf_api::gen_node_basic_data`] 里 ATTA 与 TEE 一样
//! 被排除在「统一输出类型名与端点」那段之外，类型名由本函数自己写。

use aios_core::AttrMap;
use aios_core::pdms_types::*;
use sqlx::{MySql, Pool};
use crate::data_interface::tidb_manager::AiosDBManager;
use crate::pcf::bran::{gen_item_code_data_attr_val, gen_type_name_data};
use crate::pcf::elbo::get_catr_para_data_from_spre;
use crate::pcf::pcf_api::{create_angl_data, create_center_point_data, create_cords_point_data, create_refno_data, create_s_key_data, create_s_text_data};

/// 生成一个 ATTA 元件的 PCF 字段块。
pub async fn gen_atta_data(aios_mgr: &AiosDBManager, attr: &AttrMap, pool: &Pool<MySql>, materials: &mut Vec<(RefU64, String)>) -> Vec<u8> {
    let mut data = vec![];
    let refno = attr.get_refno();
    if refno.is_none() { return vec![]; }
    let refno = refno.unwrap();
    let spre = attr.get_val("SPRE");
    data.append(&mut gen_type_name_data(attr.get_type()));
    data.append(&mut create_cords_point_data(refno, aios_mgr).await);
    data.append(&mut create_s_key_data(attr, aios_mgr).await);
    data.append(&mut gen_item_code_data_attr_val(spre, &aios_mgr, materials).await);
    data.append(&mut create_refno_data(attr));
    // SUPPORT-TYPE 放在最后：它是可选字段，缺 STEX 时上面几项仍然成立。
    data.append(&mut create_s_text_data(attr));
    data
}
