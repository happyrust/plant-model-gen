/// 测试 included_db_files 过滤逻辑
///
/// 模拟 database.rs 中的过滤逻辑，验证是否存在问题


#[derive(Clone)]
struct TestDbOption {
    total_sync: bool,
    included_db_files: Option<Vec<String>>,
}

fn main() -> anyhow::Result<()> {
    let project_path = "D:/AVEVA/Projects/E3D2.1/AvevaMarineSample";
    let db_dir = format!("{}/ams000", project_path);

    println!("🔍 测试 included_db_files 过滤逻辑\n");
    println!("📂 数据库目录: {}\n", db_dir);

    // 测试场景 1: 当前配置 - total_sync = true, included_db_files = ["ams1112_0001"]
    let option1 = TestDbOption {
        total_sync: true,
        included_db_files: Some(vec!["ams1112_0001".to_string()]),
    };

    // 测试场景 2: total_sync = false, included_db_files = ["ams1112_0001"]
    let option2 = TestDbOption {
        total_sync: false,
        included_db_files: Some(vec!["ams1112_0001".to_string()]),
    };

    // 测试场景 3: total_sync = true, included_db_files = None
    let option3 = TestDbOption {
        total_sync: true,
        included_db_files: None,
    };

    let entries = std::fs::read_dir(&db_dir)?;
    let mut all_files: Vec<String> = Vec::new();

    for entry in entries {
        let path = entry?.path();
        if path.is_dir() || path.extension().is_some() {
            continue;
        }
        let file_name = path.file_name().unwrap().to_str().unwrap().to_string();
        if file_name.contains(".") {
            continue;
        }
        all_files.push(file_name);
    }

    all_files.sort();
    let total_count = all_files.len();

    println!("📊 总文件数: {}\n", total_count);

    // 测试当前错误的逻辑（database.rs:1407-1415）
    println!("❌ 当前错误的逻辑 (逻辑或 ||):");
    println!("   代码: if (is_parse_sys && is_total_sync)");
    println!("           || included_db_files.is_none()");
    println!("           || condition3");
    println!("           || included_db_files.contains(&file_name)\n");

    let is_parse_sys = true; // 模拟

    for (i, option) in [option1.clone(), option2.clone(), option3.clone()].iter().enumerate() {
        let mut processed_count = 0;

        for file_name in &all_files {
            let condition1 = is_parse_sys && option.total_sync;
            let condition2 = option.included_db_files.is_none();
            let condition3 = option.included_db_files.as_ref()
                .map(|v| v.is_empty())
                .unwrap_or(false);

            if condition1
                || condition2
                || condition3
                || option.included_db_files.as_ref()
                    .unwrap()
                    .contains(file_name)
            {
                processed_count += 1;
            }
        }

        println!("   场景 {}: total_sync={}, included_db_files={:?}",
            i + 1,
            option.total_sync,
            option.included_db_files
        );
        println!("   ✗ 处理了 {} / {} 个文件 ({:.1}%)\n",
            processed_count,
            total_count,
            (processed_count as f32 / total_count as f32) * 100.0
        );
    }

    println!("{}", "─".repeat(60));
    println!("\n✅ 正确的逻辑 (逻辑与 &&):");
    println!("   代码: let should_parse = is_parse_sys && is_total_sync;");
    println!("         let should_parse = should_parse && (");
    println!("             included_db_files.is_none()");
    println!("             || included_db_files.is_empty()");
    println!("             || included_db_files.contains(&file_name)");
    println!("         );\n");

    for (i, option) in [option1, option2, option3].iter().enumerate() {
        let mut processed_count = 0;

        for file_name in &all_files {
            let should_parse = is_parse_sys && option.total_sync;
            let should_parse = should_parse && (
                option.included_db_files.is_none()
                || option.included_db_files.as_ref().map(|v| v.is_empty()).unwrap_or(false)
                || option.included_db_files.as_ref().unwrap().contains(file_name)
            );

            if should_parse {
                processed_count += 1;
            }
        }

        println!("   场景 {}: total_sync={}, included_db_files={:?}",
            i + 1,
            option.total_sync,
            option.included_db_files
        );
        println!("   ✓ 处理了 {} / {} 个文件 ({:.1}%)\n",
            processed_count,
            total_count,
            (processed_count as f32 / total_count as f32) * 100.0
        );
    }

    println!("{}", "─".repeat(60));
    println!("\n💡 结论:");
    println!("   当 total_sync = true 且 included_db_files = [\"ams1112_0001\"] 时:");
    println!("   - 错误逻辑 (||): 处理所有 448 个文件 ❌");
    println!("   - 正确逻辑 (&&): 只处理 1 个文件 ✅");

    Ok(())
}
