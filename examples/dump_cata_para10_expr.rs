//! 临时调试工具：定位 cata 元件中包含 `PARA[10 ]` 的表达式属性。
//!
//! 用法（Windows 示例）：
//! - cargo run --example dump_cata_para10_expr --features sqlite-index
//!
//! 说明：
//! - 该 example 只做“打印定位”，不作为单测/CI 断言。
//! - 若 refno / 文件路径不匹配，请按实际环境修改常量。

use aios_core::RefU64;
use pdms_io::io::PdmsIO;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 来自 7999 BRAN 日志中的 cata_refno 样本（均为 ref0=13244）。这里批量跑一遍便于定位。
    let refnos: [&str; 5] = [
        "13244/64588",
        "13244/64587",
        "13244/64800",
        "13244/65011",
        "13244/61730",
    ];

    // 通过 SurrealDB 查询可知：这些 refno 多数落在 dbnum=5052，对应 file_name=ams5052_0001
    // 默认放在 AvevaMarineSample/ams000 下（以实际环境为准）。
    let file_path = "D:/AVEVA/Projects/E3D2.1/AvevaMarineSample/ams000/ams5052_0001";

    let mut io = PdmsIO::new("ams", file_path, true);
    io.open()?;

    // 先取“完整解析结果”（包含 UDA 等处理）。若你想对比原始解析，可切回 auto_get_raw_element。
    for refno_str in refnos {
        let refno: RefU64 = refno_str.into();
        let ele = match io.auto_get_element(refno).await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("refno={refno_str} auto_get_element failed: {e}");
                continue;
            }
        };
        let att = ele.att_map();

        println!();
        println!("refno={refno} TYPE={:?} SESNO={:?}", att.get_type(), att.sesno());
        println!("---- attrs contains 'PARA' ----");
        let mut str_cnt = 0usize;
        let mut hits = 0usize;
        for (k, _v) in att.map.iter() {
            if let Some(s) = att.get_as_string(k.as_str()) {
                str_cnt += 1;
                if s.contains("PARA") {
                    hits += 1;
                    println!("attr={k} => {s}");
                }
            }
        }
        println!("string_like_attr_count={str_cnt}, contains_PARA_hits={hits}");
    }
    Ok(())
}
