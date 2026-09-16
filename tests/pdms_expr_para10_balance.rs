//! 解析回归用例：验证 PDMS 解析出来的表达式括号是闭合的（不应出现 `'( ATTRIB PARA[10 ] / 2'` 这类缺右括号串）。
//!
//! 该用例依赖本机 E3D 项目文件（较大，不进入仓库），因此默认 `#[ignore]`。
//! 运行方式：
//! - cargo test --test pdms_expr_para10_balance -- --ignored --nocapture

use std::path::Path;

use aios_core::RefU64;
use pdms_io::io::PdmsIO;

fn paren_balance(s: &str) -> i32 {
    let mut b = 0i32;
    for ch in s.chars() {
        if ch == '(' {
            b += 1;
        } else if ch == ')' {
            b -= 1;
        }
    }
    b
}

#[tokio::test]
#[ignore]
async fn test_pdms_expr_contains_para10_inner_group_is_closed() {
    // 由 SurrealDB 可查到：pe:`13244_61945` 属于 dbnum=5052，文件名通常为 ams5052_0001。
    // 该文件在默认 E3D 项目下的位置如下（以你的实际安装目录为准）。
    let file_path = "D:/AVEVA/Projects/E3D2.1/AvevaMarineSample/ams000/ams5052_0001";
    if !Path::new(file_path).exists() {
        eprintln!("[ignore] missing pdms db file: {file_path}");
        return;
    }

    // 该 SEXT 元件的 PX 含有 `TAN( ( ATTRIB PARA[10 ] / 2 ) )`，曾在 7999 的 BRAN 生成中触发“缺右括号”报错。
    let refno: RefU64 = "13244/61945".into();

    let mut io = PdmsIO::new("ams", file_path, true);
    io.open().expect("open pdms db");

    let ele = io.auto_get_element(refno).await.expect("auto_get_element");
    let att = ele.att_map();

    let px = att
        .get_as_string("PX")
        .expect("PX should exist as string expression");

    // 关键断言：内层 `( ATTRIB PARA[10 ] / 2 )` 必须闭合。
    assert!(
        px.contains("( ATTRIB PARA[10 ] / 2 )"),
        "PX does not contain expected inner group, PX={px}"
    );

    // 整体也应括号平衡（该表达式一般是多层括号嵌套）。
    let bal = paren_balance(&px);
    assert_eq!(bal, 0, "PX parentheses not balanced: bal={bal}, PX={px}");
}
