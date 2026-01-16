/// 测试程序：分析 PDMS 数据库文件的 sesno 值
///
/// 用于诊断为什么某些数据库文件的 sesno <= 0

use pdms_io::PdmsIO;

fn main() -> anyhow::Result<()> {
    // 从配置文件读取项目路径
    let project_path = "D:/AVEVA/Projects/E3D2.1/AvevaMarineSample";
    let db_dir = format!("{}/ams000", project_path);

    println!("🔍 开始分析数据库文件的 sesno 值");
    println!("📂 数据库目录: {}\n", db_dir);

    // 读取目录中的所有文件
    let entries = std::fs::read_dir(&db_dir)?;

    let mut files_analyzed = 0;
    let mut files_with_zero_sesno = 0;
    let mut files_with_valid_sesno = 0;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        // 跳过目录和带扩展名的文件
        if path.is_dir() || path.extension().is_some() {
            continue;
        }

        let file_name = path.file_name().unwrap().to_str().unwrap();

        // 跳过带点的文件
        if file_name.contains(".") {
            continue;
        }

        files_analyzed += 1;

        // 尝试打开文件并读取 sesno
        let mut io = PdmsIO::new("AvevaMarineSample", path.clone(), true);

        match io.open() {
            Ok(_) => {
                match io.get_latest_sesno() {
                    Ok(sesno) => {
                        if sesno == 0 {
                            files_with_zero_sesno += 1;
                            println!("❌ {}: sesno = 0", file_name);

                            // 尝试读取更多信息来诊断问题
                            if let Ok(header) = io.read_pdms_header() {
                                println!("   📋 Header info:");
                                println!("      - db_num: {}", header.db_num);
                                println!("      - latest_ses_pgno: {}", header.latest_ses_pgno);
                                println!("      - page_size: {}", header.page_size);

                                // 尝试读取会话页数据
                                if header.latest_ses_pgno > 0 {
                                    match io.read_ses_data(header.latest_ses_pgno) {
                                        Ok(ses_data) => {
                                            println!("      - session sesno: {}", ses_data.sesno);
                                            println!("      - session page_type: {}", ses_data.page_type);
                                        }
                                        Err(e) => {
                                            println!("      ⚠️  读取会话页失败: {}", e);
                                        }
                                    }
                                } else {
                                    println!("      ⚠️  latest_ses_pgno = 0，这是一个空数据库");
                                }
                            }
                            println!();
                        } else {
                            files_with_valid_sesno += 1;
                            println!("✅ {}: sesno = {}", file_name, sesno);
                        }
                    }
                    Err(e) => {
                        println!("⚠️  {}: 无法读取 sesno - {}", file_name, e);
                    }
                }
            }
            Err(e) => {
                println!("⚠️  {}: 无法打开文件 - {}", file_name, e);
            }
        }
    }

    println!("\n📊 统计结果:");
    println!("   - 分析的文件总数: {}", files_analyzed);
    println!("   - sesno = 0 的文件: {}", files_with_zero_sesno);
    println!("   - sesno > 0 的文件: {}", files_with_valid_sesno);

    if files_with_zero_sesno > 0 {
        println!("\n💡 结论:");
        println!("   发现 {} 个文件的 sesno = 0", files_with_zero_sesno);
        println!("   这些文件可能是:");
        println!("   1. 新创建但从未保存过的数据库");
        println!("   2. 空数据库文件");
        println!("   3. 损坏的数据库文件");
        println!("   4. 仅包含初始结构但无实际数据的数据库");
    }

    Ok(())
}
