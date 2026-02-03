//! db_meta_info - 数据库元信息管理
//! 用于 refno(ref_0) -> dbnum 的快速映射，以及记录 db 文件头的关键信息

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// 旧版默认目录（兼容）
pub const DEFAULT_TREE_DIR: &str = "output/scene_tree";

/// 获取基于项目名称的 scene_tree 目录
pub fn get_project_tree_dir(project_name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from("output").join(project_name).join("scene_tree")
}

/// 数据库文件元信息更新参数
pub struct DbFileMetaUpdate<'a> {
    pub dbnum: u32,
    pub db_type: &'a str,
    pub file_name: &'a str,
    pub file_path: &'a PathBuf,
    pub header_hex_60: Option<String>,
    pub header_debug: Option<String>,
    pub latest_sesno: Option<u32>,
    pub sesno_timestamp: Option<i64>,
    pub ref0s: BTreeSet<u32>,
}

/// 更新 db_meta_info.json 文件
pub fn update_db_meta_info_json(
    output_dir: &Path,
    update: DbFileMetaUpdate,
) -> anyhow::Result<()> {
    use std::fs;
    use serde_json::{json, Value};
    
    let meta_path = output_dir.join("db_meta_info.json");
    
    // 读取或创建新的 meta 结构
    let mut meta: Value = if meta_path.exists() {
        let content = fs::read_to_string(&meta_path)?;
        serde_json::from_str(&content)?
    } else {
        json!({
            "version": 1,
            "updated_at": chrono::Utc::now().to_rfc3339(),
            "ref0_to_dbnum": {},
            "db_files": {}
        })
    };
    
    // 更新 ref0_to_dbnum 映射
    if let Some(ref0_map) = meta.get_mut("ref0_to_dbnum") {
        if let Some(obj) = ref0_map.as_object_mut() {
            for ref0 in &update.ref0s {
                obj.insert(ref0.to_string(), json!(update.dbnum));
            }
        }
    }
    
    // 将 sesno_timestamp (i64 秒级时间戳) 转为 RFC3339 字符串
    let updated_at_str = update.sesno_timestamp
        .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default();
    
    // 更新 db_files
    if let Some(db_files) = meta.get_mut("db_files") {
        if let Some(obj) = db_files.as_object_mut() {
            obj.insert(update.dbnum.to_string(), json!({
                "dbnum": update.dbnum,
                "db_type": update.db_type,
                "file_name": update.file_name,
                "file_path": update.file_path.to_string_lossy(),
                "updated_at": updated_at_str,
                "header_hex_60": update.header_hex_60,
                "header_debug": update.header_debug,
                "latest_sesno": update.latest_sesno,
                "ref0s": update.ref0s.iter().collect::<Vec<_>>()
            }));
        }
    }
    
    // 更新 updated_at
    if let Some(updated_at) = meta.get_mut("updated_at") {
        *updated_at = json!(chrono::Utc::now().to_rfc3339());
    }
    
    // 确保目录存在
    fs::create_dir_all(output_dir)?;
    
    // 写入文件
    fs::write(&meta_path, serde_json::to_string_pretty(&meta)?)?;
    
    Ok(())
}
