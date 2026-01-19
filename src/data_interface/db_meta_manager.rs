//! DbMetaManager - 数据库元信息管理模块
//! 
//! 统一管理 db_meta_info.json 的加载、缓存和查询

use anyhow::Result;
use once_cell::sync::OnceCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

static DB_META_MANAGER: OnceCell<DbMetaManager> = OnceCell::new();

/// 数据库元信息管理器
pub struct DbMetaManager {
    /// ref0 -> dbnum 映射
    ref0_to_dbnum: RwLock<HashMap<u32, u32>>,
    /// dbnum -> db_file_info 映射
    db_files: RwLock<HashMap<u32, DbFileInfo>>,
    /// 元信息文件路径
    meta_path: RwLock<Option<PathBuf>>,
}

#[derive(Debug, Clone)]
pub struct DbFileInfo {
    pub dbnum: u32,
    pub db_type: String,
    pub file_name: String,
    pub file_path: String,
    pub latest_sesno: u32,
    pub ref0s: Vec<u32>,
}

impl DbMetaManager {
    fn new() -> Self {
        Self {
            ref0_to_dbnum: RwLock::new(HashMap::new()),
            db_files: RwLock::new(HashMap::new()),
            meta_path: RwLock::new(None),
        }
    }

    /// 获取全局单例
    pub fn global() -> &'static DbMetaManager {
        DB_META_MANAGER.get_or_init(DbMetaManager::new)
    }

    /// 加载 db_meta_info.json
    pub fn load(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path)?;
        let json: serde_json::Value = serde_json::from_str(&content)?;

        // 解析 ref0_to_dbnum
        let mut ref0_map = self.ref0_to_dbnum.write().unwrap();
        ref0_map.clear();
        if let Some(obj) = json.get("ref0_to_dbnum").and_then(|v| v.as_object()) {
            for (ref0_str, dbnum_val) in obj {
                if let (Ok(ref0), Some(dbnum)) = (ref0_str.parse::<u32>(), dbnum_val.as_u64()) {
                    ref0_map.insert(ref0, dbnum as u32);
                }
            }
        }

        // 解析 db_files
        let mut db_files_map = self.db_files.write().unwrap();
        db_files_map.clear();
        if let Some(obj) = json.get("db_files").and_then(|v| v.as_object()) {
            for (dbnum_str, info) in obj {
                if let Ok(dbnum) = dbnum_str.parse::<u32>() {
                    let file_info = DbFileInfo {
                        dbnum,
                        db_type: info.get("db_type").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        file_name: info.get("file_name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        file_path: info.get("file_path").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        latest_sesno: info.get("latest_sesno").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                        ref0s: info.get("ref0s")
                            .and_then(|v| v.as_array())
                            .map(|arr| arr.iter().filter_map(|v| v.as_u64().map(|n| n as u32)).collect())
                            .unwrap_or_default(),
                    };
                    db_files_map.insert(dbnum, file_info);
                }
            }
        }

        // 同时更新 aios_core 中的缓存
        if let Err(e) = aios_core::tree_query::load_db_meta_info(path) {
            log::warn!("同步 aios_core 缓存失败: {}", e);
        }

        *self.meta_path.write().unwrap() = Some(path.to_path_buf());
        println!("✅ DbMetaManager: 已加载 {:?}, ref0 映射 {} 条, db_files {} 条", 
            path, ref0_map.len(), db_files_map.len());
        Ok(())
    }

    /// 尝试从默认路径加载
    pub fn try_load_default(&self) -> Result<()> {
        let default_paths = [
            "output/scene_tree/db_meta_info.json",
            "../output/scene_tree/db_meta_info.json",
        ];
        
        for path in &default_paths {
            if Path::new(path).exists() {
                return self.load(path);
            }
        }
        
        anyhow::bail!("未找到 db_meta_info.json，尝试路径: {:?}", default_paths)
    }

    /// 确保已加载（如未加载则尝试从默认路径加载）
    pub fn ensure_loaded(&self) -> Result<()> {
        if self.meta_path.read().unwrap().is_none() {
            self.try_load_default()
        } else {
            Ok(())
        }
    }

    /// 根据 ref0 获取 dbnum
    pub fn get_dbnum_by_ref0(&self, ref0: u32) -> Option<u32> {
        self.ref0_to_dbnum.read().unwrap().get(&ref0).copied()
    }

    /// 根据 refno 获取 dbnum
    pub fn get_dbnum_by_refno(&self, refno: aios_core::RefnoEnum) -> Option<u32> {
        let ref0 = match refno {
            aios_core::RefnoEnum::Refno(r) => r.get_0(),
            aios_core::RefnoEnum::SesRef(r) => r.refno.get_0(),
        };
        self.get_dbnum_by_ref0(ref0)
    }

    /// 获取所有 dbnum 列表
    pub fn get_all_dbnums(&self) -> Vec<u32> {
        self.db_files.read().unwrap().keys().copied().collect()
    }

    /// 根据 dbnum 获取文件信息
    pub fn get_db_file_info(&self, dbnum: u32) -> Option<DbFileInfo> {
        self.db_files.read().unwrap().get(&dbnum).cloned()
    }

    /// 将 ref0 列表转换为 dbnum 列表（去重）
    pub fn ref0s_to_dbnums(&self, ref0s: &[u32]) -> Vec<u32> {
        let dbnums: std::collections::HashSet<u32> = ref0s
            .iter()
            .filter_map(|&ref0| self.get_dbnum_by_ref0(ref0))
            .collect();
        dbnums.into_iter().collect()
    }

    /// 是否已加载
    pub fn is_loaded(&self) -> bool {
        self.meta_path.read().unwrap().is_some()
    }
}

/// 便捷函数：获取全局管理器
pub fn db_meta() -> &'static DbMetaManager {
    DbMetaManager::global()
}

/// 便捷函数：根据 ref0 获取 dbnum
pub fn get_dbnum(ref0: u32) -> Option<u32> {
    db_meta().get_dbnum_by_ref0(ref0)
}

/// 便捷函数：将 ref0 列表转换为 dbnum 列表
pub fn ref0s_to_dbnums(ref0s: &[u32]) -> Vec<u32> {
    db_meta().ref0s_to_dbnums(ref0s)
}
