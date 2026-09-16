//! TUBI（直管段）的 PCF 记录生成。
//!
//! 直管段在 PDMS/E3D 里**不是独立的设计元素**——它是相邻两个元件之间那段
//! 被隐含出来的管子。所以本函数的签名和别的元件完全不同：它不收 `AttrMap`，
//! 而是直接收起止点与口径，由调用方（BRAN 遍历）算好后传进来。
//!
//! ## 几个要点
//!
//! - **口径决定生死**：只有 `TubiSize::BoreSize` 才产出记录。拿不到公称直径
//!   就意味着这段管子的规格未知，宁可不写，也不要输出一段没有口径的管子。
//! - **材料来自 BRAN 而非自身**：直管用的是所在管路的 HSTU（管子规格），
//!   因此材料码从 `bran_attr` 上取。
//! - **refno 借用上游元件**：直管没有自己的引用号，用 `from_refno`
//!   （前一个元件的 refno）标注来源，便于回溯是哪一段。
//! - **壁厚由调用方算好传入**：`pipe_thickness_data` 是已经成型的字节片段，
//!   这里直接拼接——壁厚要查规格表，放在循环外算一次比每段都算划算。

use aios_core::AttrMap;
use aios_core::pdms_types::*;
use aios_core::prim_geo::tubing::TubiSize;
use dashmap::DashMap;
use glam::Vec3;
use sqlx::{MySql, Pool};
use crate::data_interface::tidb_manager::AiosDBManager;
use crate::pcf::bran::{gen_endpoint_data, gen_item_code_data_attr_val, gen_refno_data, gen_refno_data_pipe};

/// 生成一段直管的 PCF 记录（含 `PIPE` 头）。
///
/// 返回空 `Vec` 表示这段管子口径未知、不产出记录。
pub async fn gen_tubi_data(start_point: Vec3,
                           end_point: Vec3,
                           tubi_size: TubiSize,
                           bran_attr: &AttrMap,
                           from_refno: Option<RefU64>,
                           materials:&mut Vec<(RefU64,String)>,
                           pipe_thickness_data:&Vec<u8>,
                           aios_mgr:&AiosDBManager) -> Vec<u8> {

    let mut pipe_data = Vec::new();

    // 非 BoreSize（口径未知）时整个 if 不进，直接返回空——这是唯一的产出条件。
    if let TubiSize::BoreSize(bore) = tubi_size {
        pipe_data.append(&mut "PIPE \r\n".to_string().into_bytes());
        // 两个端点必须按「起点在前、终点在后」输出，PCF 靠顺序判走向。
        pipe_data.append(&mut gen_endpoint_data(start_point, bore));
        pipe_data.append(&mut gen_endpoint_data(end_point, bore));
        // HSTU 是 BRAN 上的管子规格引用，直管的材料码由它决定。
        let hstu_refno = bran_attr.get_val("HSTU");
        pipe_data.append(&mut gen_item_code_data_attr_val(hstu_refno, aios_mgr,materials).await);
        if let Some(from_refno) = from_refno {
            pipe_data.append(&mut gen_refno_data_pipe(from_refno));
        }
        pipe_data.append(&mut pipe_thickness_data.clone());
    }

    pipe_data
}

