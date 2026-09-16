//! 管道外径壁厚表的读取。
//!
//! PCF 要输出 PIPE-OD（外径）和 PIPE-THICK（壁厚），但这两个值**不在
//! PDMS/E3D 源库里**——源库只给公称直径（DN）和一个等级代号。真正的
//! 外径与壁厚要查一张工程约定的对照表，也就是随仓库分发的
//! `resource/管道外径壁厚表.xlsx`。
//!
//! ## 表结构与索引方式
//!
//! Excel 每行是一个 DN，列 `H` / `I` / `L` 是三个等级各自的「外径x壁厚」
//! 字符串（如 `219.1x6.35`）。读进来后做成两级映射：
//!
//! ```text
//! DN → { "H" → "外径x壁厚", "I" → …, "L" → … }
//! ```
//!
//! 查表时的 key 从管线名里切出来（第 4 段是 DN、第 5 段首字母是等级），
//! 拆分与拼装逻辑在 [`crate::pcf::pcf_api::create_thickness_data`]。
//!
//! 用 `DashMap` 而不是 `HashMap`：这张表一次读入、之后被多条管路并发查询，
//! 免去外层再包一层锁。

use anyhow::anyhow;

use calamine::{open_workbook, RangeDeserializerBuilder, Reader, Xlsx};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use crate::pcf::pcf_api::create_thickness_data;
use crate::ssc::SiteExcelData;

/// Excel 一行的反序列化目标。
///
/// 四个字段都是 `Option`：表里有合并单元格和空行，缺列是常态，
/// 由 [`Self::is_null`] 统一判定该行要不要丢弃。
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct PipeThicknessTable {
    dn: Option<String>,
    h: Option<String>,
    i: Option<String>,
    l: Option<String>,
}

impl PipeThicknessTable {
    /// 只要有任一格缺失就算无效行。
    ///
    /// 要求四格俱全是刻意从严：少一个等级就意味着后面 `unwrap` 会炸，
    /// 与其在装表时 panic，不如在这里整行跳过。
    fn is_null(&self) -> bool {
        if self.dn.is_none() || self.i.is_none() || self.l.is_none() || self.h.is_none() { return true; }
        false
    }
}

/// 获取管道外径壁厚表
pub fn get_pipe_thickness_table() -> anyhow::Result<DashMap<String, DashMap<String, String>>> {
    let mut map = DashMap::new();
    // 相对路径：要求进程工作目录是仓库根，部署包里 resource/ 必须一起带上。
    let mut workbook: Xlsx<_> = open_workbook("resource/管道外径壁厚表.xlsx")?;
    // 两个 `?`：外层解 Option（找不到工作表），内层解 Result（读取失败）。
    let range = workbook.worksheet_range("Sheet1")
        .ok_or(anyhow::anyhow!("Cannot find 'Sheet1'"))??;
    let mut iter = RangeDeserializerBuilder::new().from_range(&range)?;
    while let Some(result) = iter.next() {
        let v: PipeThicknessTable = result?;
        if !v.is_null() {
            // 三条 entry 都用 or_insert：**先到先得，重复 DN 不覆盖**。
            // 表里若有重复行，以靠前那行为准。
            map.entry(v.dn.clone().unwrap()).or_insert_with(DashMap::new).entry("H".to_string()).or_insert(v.h.unwrap());
            map.entry(v.dn.clone().unwrap()).or_insert_with(DashMap::new).entry("I".to_string()).or_insert(v.i.unwrap());
            // 最后一次不再 clone：dn 到这里可以直接移动出去。
            map.entry(v.dn.unwrap()).or_insert_with(DashMap::new).entry("L".to_string()).or_insert(v.l.unwrap());
        }
    }
    Ok(map)
}

/// 冒烟用例：拿一个真实管线名走一遍「读表 → 切 key → 生成字段」。
/// 只验证不 panic，不断言具体数值——数值随 resource 表更新而变。
#[test]
fn test_get_pipe_thickness_table() -> anyhow::Result<()> {
    let pipe_name = "2ACAS-A6-806-6-LJ6";
    let map = get_pipe_thickness_table()?;
    let name = create_thickness_data(pipe_name, &map,true);
    Ok(())
}
