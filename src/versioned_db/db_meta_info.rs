//! db_meta_info - 数据库元信息管理
//! 用于 refno(ref_0) -> dbnum 的快速映射，以及记录 db 文件头的关键信息

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// 旧版默认目录（兼容）
pub const DEFAULT_TREE_DIR: &str = "output/scene_tree";
pub const DEFAULT_OUTPUT_ROOT: &str = "output";

fn current_config_path() -> PathBuf {
    let config_name =
        std::env::var("DB_OPTION_FILE").unwrap_or_else(|_| "db_options/DbOption".to_string());
    let path = PathBuf::from(&config_name);
    if path.extension().and_then(|ext| ext.to_str()) == Some("toml") {
        path
    } else {
        PathBuf::from(format!("{config_name}.toml"))
    }
}

fn output_root_from_toml(content: &str) -> Option<PathBuf> {
    let value = toml::from_str::<toml::Value>(content).ok()?;
    value
        .get("output_root")
        .and_then(|entry| entry.as_str())
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn project_name_from_toml(content: &str) -> Option<String> {
    let value = toml::from_str::<toml::Value>(content).ok()?;
    value
        .get("project_name")
        .and_then(|entry| entry.as_str())
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn output_root_from_config(path: &Path) -> Option<PathBuf> {
    let content = std::fs::read_to_string(path).ok()?;
    output_root_from_toml(&content)
}

fn project_name_from_config(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    project_name_from_toml(&content)
}

/// 获取当前配置的输出根目录，默认 output。
pub fn get_output_root() -> PathBuf {
    output_root_from_config(&current_config_path())
        .unwrap_or_else(|| PathBuf::from(DEFAULT_OUTPUT_ROOT))
}

/// 获取当前 DB_OPTION_FILE 指向配置中的 project_name。
pub fn get_current_project_name() -> Option<String> {
    project_name_from_config(&current_config_path())
}

/// 获取基于项目名称的输出目录。
pub fn get_project_output_dir(project_name: &str) -> PathBuf {
    get_output_root().join(project_name)
}

/// 获取基于项目名称的 scene_tree 目录
pub fn get_project_tree_dir(project_name: &str) -> std::path::PathBuf {
    get_project_output_dir(project_name).join("scene_tree")
}

/// 获取当前配置对应的 scene_tree 目录。
pub fn get_current_project_tree_dir() -> Option<PathBuf> {
    get_current_project_name().map(|project_name| get_project_tree_dir(&project_name))
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
pub fn update_db_meta_info_json(output_dir: &Path, update: DbFileMetaUpdate) -> anyhow::Result<()> {
    use serde_json::{Map, Value, json};
    use std::fs;

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

    // 将 sesno_timestamp (i64 秒级时间戳) 转为 RFC3339 字符串
    let updated_at_str = update
        .sesno_timestamp
        .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default();

    // 更新 db_files
    if let Some(db_files) = meta.get_mut("db_files") {
        if let Some(obj) = db_files.as_object_mut() {
            obj.insert(
                update.dbnum.to_string(),
                json!({
                    "dbnum": update.dbnum,
                    "db_type": update.db_type,
                    "file_name": update.file_name,
                    "file_path": update.file_path.to_string_lossy(),
                    "updated_at": updated_at_str,
                    "header_hex_60": update.header_hex_60,
                    "header_debug": update.header_debug,
                    "latest_sesno": update.latest_sesno,
                    "ref0s": update.ref0s.iter().collect::<Vec<_>>()
                }),
            );
        }
    }

    // ref0_to_dbnum 必须只从 db_files[*].ref0s 推导，避免历史/手工脏映射残留。
    // 这样可以保证：映射来源始终是“扫描解析数据库文件”的结果。
    let mut rebuilt_ref0_map = Map::new();
    if let Some(db_files_obj) = meta.get("db_files").and_then(|v| v.as_object()) {
        for (dbnum_str, info) in db_files_obj {
            let Ok(dbnum) = dbnum_str.parse::<u32>() else {
                continue;
            };
            let Some(ref0s) = info.get("ref0s").and_then(|v| v.as_array()) else {
                continue;
            };
            for ref0 in ref0s {
                if let Some(ref0_u64) = ref0.as_u64() {
                    let key = ref0_u64.to_string();
                    if let Some(previous) = rebuilt_ref0_map.insert(key.clone(), json!(dbnum)) {
                        anyhow::ensure!(
                            previous.as_u64() == Some(dbnum as u64),
                            "ref0={key} 同时归属于 dbnum={} 和 dbnum={dbnum}",
                            previous
                        );
                    }
                }
            }
        }
    }
    meta["ref0_to_dbnum"] = Value::Object(rebuilt_ref0_map);

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

#[cfg(test)]
mod tests {
    use super::{DEFAULT_OUTPUT_ROOT, get_project_output_dir, output_root_from_toml};
    use std::path::PathBuf;

    #[test]
    fn output_root_from_toml_reads_non_empty_value() {
        let root = output_root_from_toml(
            r#"
project_name = "demo"
output_root = "runtime/admin_sites/site-8080/output"
"#,
        );

        assert_eq!(
            root,
            Some(PathBuf::from("runtime/admin_sites/site-8080/output"))
        );
    }

    #[test]
    fn output_root_from_toml_ignores_empty_value() {
        assert_eq!(output_root_from_toml(r#"output_root = "   ""#), None);
    }

    #[test]
    fn project_output_dir_uses_default_root_when_config_missing() {
        assert_eq!(
            get_project_output_dir("demo"),
            PathBuf::from(DEFAULT_OUTPUT_ROOT).join("demo")
        );
    }
}
