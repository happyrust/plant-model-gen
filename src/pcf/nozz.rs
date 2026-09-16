//! NOZZ（管嘴）元件的 PCF 记录生成。
//!
//! 管嘴是设备（塔、罐、泵）上供管道接入的接口，因此它在 PCF 里比较特殊：
//!
//! - **要显式写类型名**：这里硬编码成 `"NOZZLE"` 而不是取 `attr.get_type()`，
//!   因为 PDMS 侧的类型名与 PCF 规定的关键字不一致，必须做这一次映射。
//! - **要写中心点**：管嘴挂在设备上、不在管路端点序列里，位置只能由自己给出。
//! - **要写连接引用**（CONNECTION-REFERENCE）：记录它连到哪台设备，
//!   下游据此把管道与设备关联起来。
//!
//! 反过来，管嘴不需要 SKEY 和材料码——它属于设备而不属于管道材料表。

use aios_core::AttrMap;
use aios_core::pdms_types::*;
use sqlx::{MySql, Pool};
use crate::data_interface::tidb_manager::AiosDBManager;
use crate::pcf::bran::{gen_item_code_data_attr_val, gen_type_name_data};
use crate::pcf::pcf_api::{create_center_point_data, create_cref_name_data, create_refno_data, create_s_key_data};

/// 生成一个 NOZZ 元件的 PCF 字段块。
pub async fn gen_nozz_data(aios_mgr: &AiosDBManager, attr: &AttrMap, pool: &Pool<MySql>) -> Vec<u8> {
    let mut data = vec![];
    let refno = attr.get_refno();
    if refno.is_none() { return vec![]; }
    // 上面已挡掉 None，这里 unwrap 安全；后续查世界坐标要用到它。
    let refno = refno.unwrap();
    data.append(&mut gen_type_name_data("NOZZLE"));
    // 中心点取的是**世界变换**后的平移量，不是局部坐标。
    data.append(&mut create_center_point_data(refno, aios_mgr).await);
    data.append(&mut create_cref_name_data(attr, pool).await);
    data.append(&mut create_refno_data(attr));
    data
}
