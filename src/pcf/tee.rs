//! TEE（三通）元件的 PCF 记录生成。
//!
//! 三通是管路上的分叉点，是 PCF 里最麻烦的一类元件——除了自身位置，还得
//! 交待「支路从哪个点岔出去」，否则下游拼不出拓扑。
//!
//! ## 两条完全不同的输出分支
//!
//! 按 SKEY 分流：
//!
//! - **`TESO`（set-on tee，鞍式接管）**：支管直接骑焊在主管外壁上，没有
//!   独立的三通实体。所以类型名要改写成 `TEE-SET-ON`，**不输出端点**
//!   （它不占主管长度），支管点直接取自身世界坐标。
//! - **其余三通**：正常成品三通，输出端点 + 中心点 + 支管点 + 材料 +
//!   焊接规格 + 壁厚，是完整一档。
//!
//! ## 支管点怎么求
//!
//! 三通的 `CREF` 指向支路那条 BRAN。要判断该三通接的是支路的**头**还是**尾**：
//! 拿支路 BRAN 的 `HREF`/`TREF` 与本三通 refno 比对，命中头就取支路第一段
//! 的起点，命中尾就取最后一段的终点。见 [`create_tee_branch_point_data`]。
//!
//! 注意 [`crate::pcf::pcf_api::gen_node_basic_data`] 把 TEE 排除在「统一输出
//! 类型名与端点」那段之外，正是因为上面两条分支的输出格式不一样。

use std::task::Poll;
use aios_core::AttrMap;
use aios_core::pdms_types::*;
use aios_core::prim_geo::tubing::{TubiEdge, TubiSize};
use dashmap::DashMap;
use glam::Vec3;
use sqlx::{MySql, Pool};
use crate::api::attr::query_implicit_attr;
use crate::api::element::query_name;
use crate::aql_api::tubi::query_bran_info;
use crate::data_interface::interface::PdmsDataInterface;
use crate::data_interface::tidb_manager::AiosDBManager;
use crate::pcf::bran::{gen_endpoint_data, gen_item_code_data_attr_val, gen_type_name_data};
use crate::pcf::pcf_api::{create_center_point_data, create_thickness_data, create_pipeline_spec_data, create_refno_data, create_s_key_data, create_tee_item_code_bran_data, create_weld_spec_data, gen_s_key_data_str, get_s_key_value};

/// 生成一个 TEE 元件的 PCF 字段块。
///
/// `start_edge` / `end_edge` 是该三通在主管上的前后两段 TubiEdge，
/// 用来推端点；`thickness_map` 是外径壁厚对照表，查支路壁厚要用。
pub async fn gen_tee_data(aios_mgr: &AiosDBManager, attr: &AttrMap, bran_attr: &AttrMap,
                          pool: &Pool<MySql>, materials: &mut Vec<(RefU64, String)>,
                          start_edge: &TubiEdge, end_edge: &TubiEdge, thickness_map: &DashMap<String, DashMap<String, String>>) -> Vec<u8> {
    let mut data = vec![];
    let refno = attr.get_refno();
    if refno.is_none() { return vec![]; }
    let refno = refno.unwrap();
    let type_name = attr.get_type_str();
    // SKEY 决定走哪条分支，所以必须先取出来；取不到时按空串处理，走通用分支。
    let s_key = get_s_key_value(attr, aios_mgr, pool).await;
    let s_key = s_key.unwrap_or("".to_string());
    // TEE 的 SKEY 值为 TEST时 需要做特殊处理
    if s_key == "TESO" {
        // ---- 分支 A：鞍式接管 ----
        // 类型名换成 PCF 的专用关键字，不能沿用 PDMS 的 "TEE"。
        data.append(&mut gen_type_name_data("TEE-SET-ON"));
        data.append(&mut create_center_point_data(refno, aios_mgr).await);
        // 鞍式接管的支管点就是它自己的位置，不必去查支路 BRAN。
        data.append(&mut create_tee_set_on_branch_1_point_data(aios_mgr, refno).await);
        // SKEY 已知是 TESO，直接写字面量，省一次查询。
        data.append(&mut gen_s_key_data_str("TESO"));
        // 材料码取自**主管 BRAN**（bran_attr）而不是三通自身：
        // 鞍式接管在提料时算作主管的一部分。
        data.append(&mut create_tee_item_code_bran_data(bran_attr, pool).await);
        data.append(&mut create_refno_data(attr));
    } else {
        // ---- 分支 B：成品三通 ----
        data.append(&mut gen_type_name_data(type_name));
        // 前一段的**终点**就是本元件的入口端点。
        if let TubiSize::BoreSize(bore) = start_edge.tubi_size {
            let start_point = start_edge.end_pt;
            data.append(&mut gen_endpoint_data(start_point, bore));
        }

        // 后一段的**起点**就是本元件的出口端点。
        if let TubiSize::BoreSize(bore) = end_edge.tubi_size {
            let end_point = end_edge.start_pt;
            data.append(&mut gen_endpoint_data(end_point, bore));
        }
        data.append(&mut create_center_point_data(refno, aios_mgr).await);
        data.append(&mut create_tee_branch_point_data(aios_mgr, attr, pool).await);
        data.append(&mut gen_s_key_data_str(s_key.as_str()));
        let spre = attr.get_val("SPRE");
        data.append(&mut gen_item_code_data_attr_val(spre, aios_mgr, materials).await);
        data.append(&mut create_weld_spec_data(attr, aios_mgr).await);
        data.append(&mut create_refno_data(attr));
        // 末参数 false = 输出 BRANCH1-OD / BRANCH1-THICK（支路壁厚），
        // 传 true 才是 PIPE-OD / PIPE-THICK（主管壁厚）。
        data.append(&mut create_cref_thickness_data(attr, pool, thickness_map, false).await);
    }
    data
}

/// 求三通的支管点（BRANCH1-POINT）：支路 BRAN 的头端点或尾端点。
///
/// 判定方法是拿支路 BRAN 的 `HREF`（头连接）/ `TREF`（尾连接）跟本三通的
/// refno 比对——谁指向我，我就取那一端的坐标。
///
/// 中间任何一步查不到都返回空 `Vec`：缺支管点只是这条记录少一行，
/// 不该让整篇 PCF 失败。
pub async fn create_tee_branch_point_data(aios_mgr: &AiosDBManager, attr: &AttrMap, pool: &Pool<MySql>) -> Vec<u8> {
    let refno = attr.get_refno().unwrap(); // 在调用本方法之前已经判断过 attr 中是否存在 refno
    if let Some(cref_refno) = attr.get_refu64("CREF") {
        // 支路的分段几何存在图数据库里，要单独取连接。
        let database = aios_mgr.get_arango_db().await;
        if database.is_err() { return vec![]; }
        let database = database.unwrap();
        let bran_infos = query_bran_info(cref_refno, &database).await;
        if bran_infos.is_err() { return vec![]; }
        let bran_infos = bran_infos.unwrap();
        let cref_cache = aios_mgr.get_refno_basic(cref_refno);
        if cref_cache.is_none() { return vec![]; }
        let cref_cache = cref_cache.unwrap();
        let cref_attr = query_implicit_attr(cref_refno, cref_cache.value(), pool, Some(vec!["HREF", "TREF"])).await;
        if cref_attr.is_err() { return vec![]; }
        let cref_attr = cref_attr.unwrap();
        // 判断 bran 是 头 / 尾 连接的 tee
        if let Some(href_refno) = cref_attr.get_refu64("HREF") {
            if refno == href_refno {
                if let Some(first_tubi) = bran_infos.first() {
                    return gen_branch_1_point_data_str(first_tubi.start_pt);
                }
            }
        }
        if let Some(tref_refno) = cref_attr.get_refu64("TREF") {
            if refno == tref_refno {
                if let Some(last_tubi) = bran_infos.last() {
                    return gen_branch_1_point_data_str(last_tubi.end_pt);
                }
            }
        }
    }
    vec![]
}

/// 鞍式接管（TESO）的支管点：直接取自身的世界坐标平移量。
///
/// 与通用三通不同，这里不用去追支路 BRAN——鞍式接管本身就骑在分叉处。
async fn create_tee_set_on_branch_1_point_data(aios_mgr: &AiosDBManager, refno: RefU64) -> Vec<u8> {
    let world_transform = aios_mgr.get_world_transform(refno).await;
    if let Ok(Some(world_transform)) = world_transform {
        return gen_branch_1_point_data_str(world_transform.translation);
    }
    vec![]
}

/// cref_thickness 分为 pipe 和 bran 两种 b_pipe就代表是pipe这一种
async fn create_cref_thickness_data(attr: &AttrMap, pool: &Pool<MySql>, thickness_map: &DashMap<String, DashMap<String, String>>, b_pipe: bool) -> Vec<u8> {
    let mut data = Vec::new();
    // 壁厚要按**支路管线名**查表，所以必须先把 CREF 的名字取出来。
    let cref = attr.get_refu64("CREF");
    if cref.is_none() { return data; }
    let cref = cref.unwrap();
    let cref_name = query_name(cref, pool).await;
    if cref_name.is_err() { return data; }
    let cref_name = cref_name.unwrap();
    create_thickness_data(&cref_name, thickness_map, b_pipe)
}

/// 输出 BRANCH1-POINT 行。三个坐标分量之间是两个空格，格式固定。
fn gen_branch_1_point_data_str(point: Vec3) -> Vec<u8> {
    format!("        BRANCH1-POINT  {}  {}  {}\r\n", point.x, point.y, point.z).into_bytes()
}
