/// 分析 PDMS 数据库文件的类型分布
///
/// 用于统计各种 db_type 的文件数量

use parse_pdms_db::parse::*;

#[derive(Default)]
struct DbFileInfo {
    file_name: String,
    db_type: String,
    db_no: u32,
}

fn main() -> anyhow::Result<()> {
    let project_path = "D:/AVEVA/Projects/E3D2.1/AvevaMarineSample";
    let db_dir = format!("{}/ams000", project_path);

    println!("🔍 开始分析数据库文件的类型分布");
    println!("📂 数据库目录: {}\n", db_dir);

    // 读取目录中的所有文件
    let entries = std::fs::read_dir(&db_dir)?;

    let mut type_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut files_by_type: std::collections::HashMap<String, Vec<DbFileInfo>> = std::collections::HashMap::new();
    let mut total_files = 0;
    let mut error_count = 0;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        // 跳过目录和带扩展名的文件
        if path.is_dir() || path.extension().is_some() {
            continue;
        }

        let file_name = path.file_name().unwrap().to_str().unwrap().to_string();

        // 跳过带点的文件
        if file_name.contains(".") {
            continue;
        }

        total_files += 1;

        // 读取文件头获取 db_type
        match std::fs::read(&path) {
            Ok(data) => {
                if data.len() >= 60 {
                    // 使用 parse_file_basic_info 函数
                    let basic_info = parse_file_basic_info(&data);
                    let db_type = basic_info.db_type;
                    let db_no = basic_info.db_no;

                    // 统计类型
                    *type_counts.entry(db_type.clone()).or_insert(0) += 1;

                    // 记录文件
                    files_by_type
                        .entry(db_type.clone())
                        .or_insert_with(Vec::new)
                        .push(DbFileInfo {
                            file_name,
                            db_type: db_type.clone(),
                            db_no,
                        });
                }
            }
            Err(e) => {
                error_count += 1;
                if error_count <= 5 {
                    println!("⚠️  无法读取文件 {}: {}", path.display(), e);
                }
            }
        }
    }

    if error_count > 5 {
        println!("⚠️  ... 还有 {} 个文件读取失败", error_count - 5);
    }

    println!("\n📊 类型分布统计:");
    println!("   总文件数: {}", total_files);
    println!("   成功读取: {}", total_files - error_count);
    println!("   读取失败: {}\n", error_count);

    let mut sorted_types: Vec<_> = type_counts.iter().collect();
    sorted_types.sort_by(|a, b| b.1.cmp(a.1)); // 按数量降序排序

    for (db_type, count) in sorted_types {
        println!("   {:8}: {} 个文件", db_type, count);
    }

    println!("\n📋 目标类型文件详情:");
    let target_types = ["DICT", "SYST", "GLB", "GLOB", "DESI", "CATA"];

    for target_type in &target_types {
        if let Some(files) = files_by_type.get(*target_type) {
            println!("\n   {} ({} 个文件):", target_type, files.len());
            for file in files.iter().take(10) {
                println!("      - {} (db_no: {})", file.file_name, file.db_no);
            }
            if files.len() > 10 {
                println!("      ... 还有 {} 个文件", files.len() - 10);
            }
        } else {
            println!("\n   {} (0 个文件)", target_type);
        }
    }

    println!("\n💡 结论:");
    let target_count: usize = target_types
        .iter()
        .filter_map(|t| type_counts.get(*t))
        .sum();

    println!("   目标类型 (DICT, SYST, GLB, GLOB, DESI, CATA) 总共 {} 个文件", target_count);
    println!("   非目标类型 {} 个文件", total_files - target_count);

    Ok(())
}
