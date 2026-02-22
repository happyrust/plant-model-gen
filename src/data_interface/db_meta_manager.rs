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
    ///
    /// 只使用项目目录 output/{project_name}/scene_tree/db_meta_info.json
    /// 若不存在，自动触发解析生成
    pub fn try_load_default(&self) -> Result<()> {
        // 尝试从 DbOption 获取 project_name
        let project_paths = self.get_project_based_paths();

        // 尝试项目目录
        for path in &project_paths {
            if Path::new(path).exists() {
                return self.load(path);
            }
        }

        // 文件不存在时，自动触发解析生成（不再兼容旧目录结构 output/scene_tree）
        println!("📂 检测到 indextree 文件缺失，正在自动生成...");
        self.auto_generate_indextree()?;

        // 重新尝试加载
        for path in &project_paths {
            if Path::new(path).exists() {
                return self.load(path);
            }
        }

        anyhow::bail!("自动生成后仍未找到 db_meta_info.json，尝试路径: {:?}", project_paths)
    }

    /// 获取基于项目名称的路径列表
    /// 
    /// 从配置文件读取 project_name，构建 output/{project_name}/scene_tree/ 路径
    /// 优先使用 DB_OPTION_FILE 环境变量指定的配置文件
    fn get_project_based_paths(&self) -> Vec<String> {
        let mut paths = Vec::new();
        
        // 优先使用环境变量指定的配置文件，否则回退到默认
        let config_name = std::env::var("DB_OPTION_FILE").unwrap_or_else(|_| "db_options/DbOption".to_string());
        let config_file = format!("{}.toml", config_name);
        
        if let Ok(content) = std::fs::read_to_string(&config_file) {
            // 简单解析 project_name = "xxx"
            for line in content.lines() {
                let line = line.trim();
                if line.starts_with("project_name") {
                    if let Some(value) = line.split('=').nth(1) {
                        let name = value.trim().trim_matches('"').trim_matches('\'');
                        if !name.is_empty() {
                            let path = format!("output/{}/scene_tree/db_meta_info.json", name);
                            paths.push(path);
                            break;
                        }
                    }
                }
            }
        }
        
        paths
    }

    /// 自动生成 indextree 文件（使用解析方式，只处理 DESI 类型）
    fn auto_generate_indextree(&self) -> Result<()> {
        use crate::versioned_db::database::sync_total_async_threaded;
        use aios_core::options::DbOption;
        use dashmap::DashSet;
        use std::sync::Arc;

        // 从配置文件读取配置
        let config_name = std::env::var("DB_OPTION_FILE").unwrap_or_else(|_| "db_options/DbOption".to_string());
        let config_path = format!("{}.toml", config_name);
        if !std::path::Path::new(&config_path).exists() {
            anyhow::bail!("未找到配置文件 {}", config_path);
        }

        let content = std::fs::read_to_string(&config_path)?;
        let mut db_option: DbOption = toml::from_str(&content)
            .map_err(|e| anyhow::anyhow!("解析 {} 失败: {}", config_path, e))?;

        // 设置为仅生成树结构模式
        db_option.gen_tree_only = true;
        db_option.total_sync = true;
        db_option.save_db = Some(false);

        // 只生成 project_name 对应的 indextree
        let project_name = db_option.project_name.clone();

        println!("🔄 正在通过 PDMS 解析生成 indextree (gen_tree_only 模式, 项目: {}, 类型: DESI)...", project_name);

        // 使用 tokio runtime 执行异步生成
        let result = match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                tokio::task::block_in_place(|| {
                    handle.block_on(async {
                        let cur_dbno_set = Arc::new(DashSet::new());
                        // 【关键】只处理 DESI 类型的 db 文件
                        sync_total_async_threaded(
                            &db_option,
                            &project_name,
                            cur_dbno_set,
                            &["DESI"],
                            100,
                        ).await
                    })
                })
            }
            Err(_) => {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                rt.block_on(async {
                    let cur_dbno_set = Arc::new(DashSet::new());
                    // 【关键】只处理 DESI 类型的 db 文件
                    sync_total_async_threaded(
                        &db_option,
                        &project_name,
                        cur_dbno_set,
                        &["DESI"],
                        100,
                    ).await
                })
            }
        };

        match result {
            Ok(_) => {
                println!("✅ indextree 生成完成");
                Ok(())
            }
            Err(e) => {
                anyhow::bail!("indextree 生成失败: {}", e)
            }
        }
    }

    /// 从指定项目目录加载
    pub fn load_from_project(&self, project_name: &str) -> Result<()> {
        let path = format!("output/{}/scene_tree/db_meta_info.json", project_name);
        if Path::new(&path).exists() {
            self.load(&path)
        } else {
            anyhow::bail!("项目 {} 的 db_meta_info.json 不存在: {}", project_name, path)
        }
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

/// 生成所有 DESI 类型的 indextree 文件
pub fn generate_desi_indextree() -> anyhow::Result<()> {
    use crate::versioned_db::database::sync_total_async_threaded;
    use aios_core::options::DbOption;
    use dashmap::DashSet;
    use std::sync::Arc;

    // 优先使用环境变量指定的配置文件，否则回退到默认
    let config_name = std::env::var("DB_OPTION_FILE").unwrap_or_else(|_| "db_options/DbOption".to_string());
    let config_path = format!("{}.toml", config_name);
    if !std::path::Path::new(&config_path).exists() {
        anyhow::bail!("未找到配置文件 {}", config_path);
    }

    let content = std::fs::read_to_string(&config_path)?;
    let mut db_option: DbOption = toml::from_str(&content)
        .map_err(|e| anyhow::anyhow!("解析 {} 失败: {}", config_path, e))?;

    // 设置为仅生成树结构模式
    db_option.gen_tree_only = true;
    db_option.total_sync = true;
    db_option.save_db = Some(false);

    let project_name = db_option.project_name.clone();
    println!("🔄 正在生成 DESI 类型 indextree (项目: {})...", project_name);

    // 使用 tokio runtime 执行异步生成
    let result = match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            tokio::task::block_in_place(|| {
                handle.block_on(async {
                    let cur_dbno_set = Arc::new(DashSet::new());
                    sync_total_async_threaded(
                        &db_option,
                        &project_name,
                        cur_dbno_set,
                        &["DESI"],
                        100,
                    ).await
                })
            })
        }
        Err(_) => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            rt.block_on(async {
                let cur_dbno_set = Arc::new(DashSet::new());
                sync_total_async_threaded(
                    &db_option,
                    &project_name,
                    cur_dbno_set,
                    &["DESI"],
                    100,
                ).await
            })
        }
    };

    result.map_err(|e| anyhow::anyhow!("indextree 生成失败: {}", e))
}

/// 生成指定 dbnum 的 indextree 文件
pub fn generate_single_indextree(target_dbnum: u32) -> anyhow::Result<()> {
    use aios_core::options::DbOption;
    use parse_pdms_db::parse::parse_file_basic_info;
    use std::fs;
    use std::io::Read;

    // 优先使用环境变量指定配置，否则回退到默认配置
    let config_name = std::env::var("DB_OPTION_FILE").unwrap_or_else(|_| "db_options/DbOption".to_string());
    let config_path = format!("{}.toml", config_name);
    if !std::path::Path::new(&config_path).exists() {
        anyhow::bail!("未找到配置文件 {}", config_path);
    }

    let content = fs::read_to_string(&config_path)?;
    let db_option: DbOption = toml::from_str(&content)
        .map_err(|e| anyhow::anyhow!("解析 {} 失败: {}", config_path, e))?;

    let project_name = db_option.project_name.clone();
    let project_dir = db_option.get_project_path(&project_name)
        .ok_or_else(|| anyhow::anyhow!("无法获取项目路径"))?;

    println!("🔍 扫描项目目录: {}", project_dir.display());

    // 扫描项目目录下的所有文件，找到匹配的 dbnum
    let mut found_file: Option<String> = None;

    if let Ok(entries) = fs::read_dir(&project_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Ok(mut file) = fs::File::open(&path) {
                    let mut buf = [0u8; 60];
                    if file.read_exact(&mut buf).is_ok() {
                        let db_info = parse_file_basic_info(&buf);
                        if db_info.dbnum == target_dbnum {
                            found_file = Some(path.to_string_lossy().to_string());
                            println!("✅ 找到 dbnum={} 的文件: {}", target_dbnum, path.display());
                            break;
                        }
                    }
                }
            }
        }
    }

    let file_path = found_file.ok_or_else(|| {
        anyhow::anyhow!("未找到 dbnum={} 对应的 db 文件", target_dbnum)
    })?;

    println!("🔄 正在生成 dbnum={} 的 indextree...", target_dbnum);

    // 调用单文件解析函数
    let result = match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            tokio::task::block_in_place(|| {
                handle.block_on(async {
                    crate::versioned_db::database::parse_single_db_file(
                        &db_option, &project_name, &file_path, target_dbnum,
                    ).await
                })
            })
        }
        Err(_) => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            rt.block_on(async {
                crate::versioned_db::database::parse_single_db_file(
                    &db_option, &project_name, &file_path, target_dbnum,
                ).await
            })
        }
    };

    result.map_err(|e| anyhow::anyhow!("indextree 生成失败: {}", e))
}
