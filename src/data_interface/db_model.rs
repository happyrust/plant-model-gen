use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use aios_core::accel_tree::acceleration_tree::{AccelerationTree, RStarBoundingBox};
use aios_core::file_helper::collect_db_dirs;
use aios_core::get_db_option;
use aios_core::options::DbOption;
use aios_core::pdms_types::*;
use dashmap::DashMap;
use glam::Vec3;
use itertools::Itertools;
use once_cell::sync::Lazy;
use parry3d::bounding_volume::{Aabb, BoundingVolume};
use parry3d::math::Vector;
use parry3d::query::{Ray, RayCast};
use pdms_io::watch::PdmsWatcher;
use rayon::prelude::*;
#[cfg(feature = "sql")]
use sqlx::pool::PoolOptions;
#[cfg(feature = "sql")]
use sqlx::{Executor, MySql, MySqlPool, Pool, Row};

use crate::consts::*;
use crate::data_interface::interface::PdmsDataInterface;
use crate::data_interface::tidb_manager::AiosDBManager;
use crate::defines::CACHED_MDB_SITE_MAP;

// PDMS/RVM treats sub-2 mm endpoint residue as a coincident connection rather
// than a physical tubing segment. Keeping the same tolerance avoids exporting
// tiny branch-end cylinders caused by rounded arrive/leave coordinates.
pub const TUBI_TOL: f32 = 2.0f32;

// project + mdb + module
pub static GLOBAL_MDB_WORLD_MAP: Lazy<DashMap<String, PdmsElement>> = Lazy::new(DashMap::new);

impl AiosDBManager {
    /// 从默认配置文件初始化
    pub async fn init_form_config() -> anyhow::Result<Self> {
        let db_option = get_db_option();
        let mgr = Self::init(&db_option).await?;
        Ok(mgr)
    }

    ///快速获得table名称
    // 已废弃: cache 模块已移除
    pub fn get_table_name(&self, refno: RefU64) -> String {
        "UNSET".to_string()
    }

    ///获得默认的连接字符串
    #[inline]
    pub fn get_default_conn_str(d: &DbOption) -> String {
        let user = d.user.as_str();
        let pwd = urlencoding::encode(d.password.as_str());
        let ip = d.ip.as_str();
        let port = d.port.as_str();
        format!("mysql://{user}:{pwd}@{ip}:{port}")
    }

    #[cfg(feature = "sql")]
    #[inline]
    pub async fn get_global_pool(&self) -> anyhow::Result<Pool<MySql>> {
        let connection_str = self.default_conn_str();
        let url = &format!("{connection_str}/{}", GLOBAL_DATABASE);
        PoolOptions::new()
            .max_connections(500)
            .acquire_timeout(Duration::from_secs(10 * 60))
            .connect(url)
            .await
            .map_err({ |x| anyhow::anyhow!(x.to_string()) })
    }

    ///获得默认的连接字符串
    #[inline]
    pub fn default_conn_str(&self) -> String {
        let d = &self.db_option;
        let user = d.user.as_str();
        let pwd = urlencoding::encode(&d.password);
        let ip = d.ip.as_str();
        let port = d.port.as_str();
        format!("mysql://{user}:{pwd}@{ip}:{port}")
    }
    /// 获得pool
    #[cfg(feature = "sql")]
    #[inline]
    pub async fn get_db_pool(connection_str: &str, project: &str) -> anyhow::Result<Pool<MySql>> {
        let url = &format!("{connection_str}/{}", project);
        PoolOptions::new()
            .max_connections(500)
            .acquire_timeout(Duration::from_secs(10 * 60))
            .connect(url)
            .await
            .map_err({ |x| anyhow::anyhow!(x.to_string()) })
    }

    #[inline]
    pub fn puhua_conn_str(&self) -> String {
        let d = &self.db_option;
        let user = d.puhua_database_user.as_str();
        let pwd = d.puhua_database_password.as_str();
        let ip = d.puhua_database_ip.as_str();
        format!("mysql://{user}:{pwd}@{ip}")
    }

    ///获取普华mysql数据库的连接pool
    #[cfg(feature = "sql")]
    #[inline]
    pub async fn get_puhua_pool(&self) -> anyhow::Result<Pool<MySql>> {
        let conn = self.puhua_conn_str();
        let url = &format!("{conn}/{}", PUHUA_MATERIAL_DATABASE);
        PoolOptions::new()
            .max_connections(500)
            .acquire_timeout(Duration::from_secs(10 * 60))
            .connect(url)
            .await
            .map_err({ |x| anyhow::anyhow!(x.to_string()) })
    }

    ///获取mysql数据库模糊查询的连接pool
    #[cfg(feature = "sql")]
    #[inline]
    pub async fn get_fuzzy_query_pool(&self) -> anyhow::Result<Pool<MySql>> {
        let connection_str = self.default_conn_str();
        let url = &format!("{connection_str}/{}", FUZZY_QUERT);
        PoolOptions::new()
            .max_connections(500)
            .acquire_timeout(Duration::from_secs(10 * 60))
            .connect(url)
            .await
            .map_err({ |x| anyhow::anyhow!(x.to_string()) })
    }

    ///获得默认的pool
    #[cfg(feature = "sql")]
    #[inline]
    pub async fn get_default_pool(conn_str: &str) -> anyhow::Result<Pool<MySql>> {
        MySqlPool::connect(conn_str)
            .await
            .map_err(|x| anyhow::anyhow!(x.to_string()))
    }

    /// 初始化mdb
    pub async fn init_mdb(&mut self, project: &str, mdb: &str, module: &str) -> anyhow::Result<()> {
        Ok(())
    }

    ///初始化db manager
    pub async fn init(db_option: &DbOption) -> anyhow::Result<Self> {
        let dir = db_option.project_path.to_string();
        #[cfg(feature = "sql")]
        let mut project_map = DashMap::new();
        let default_conn = AiosDBManager::get_default_conn_str(&db_option);
        let projects = db_option.get_project_dir_names().clone();

        let mut db_paths =
            collect_db_dirs(&db_option.project_path, projects.iter().map(|x| x.as_ref()))
                .unwrap_or_default();
        // 临时修复：如果 db_paths 为空，直接手动添加 project_path
        if db_paths.is_empty() {
            db_paths.push(db_option.project_path.clone().into());
        }
        dbg!(&db_paths); // 调试输出：看看收集到的目录路径
        let watcher = PdmsWatcher::new(db_paths);
        #[cfg(feature = "debug_watch")]
        {
            dbg!(&db_paths);
            dbg!(watcher.headers.len());
            dbg!(watcher.file_name_full_path_map.len());
        }
        let mgr = AiosDBManager {
            #[cfg(feature = "sql")]
            project_map,
            projects,
            needed_parse_files: None,
            project_path: dir,
            db_option: db_option.clone(),
            watcher: Arc::new(watcher),
            rtree: None,
        };
        Ok(mgr)
    }

    /// 根据project获取连接池
    #[cfg(feature = "sql")]
    #[inline]
    pub fn get_project_pool(&self, project: &str) -> Option<Pool<MySql>> {
        self.project_map.get(project).map(|x| x.value().clone())
    }

    /// 根据project获取连接池
    #[cfg(feature = "sql")]
    #[inline]
    pub fn get_cur_project_pool(&self) -> Option<Pool<MySql>> {
        self.project_map
            .get(self.get_cur_project())
            .map(|x| x.value().clone())
    }

    ///获得project 的db
    #[cfg(feature = "sql")]
    #[inline]
    pub async fn get_project_pool_by_refno(&self, refno: RefU64) -> Option<(String, Pool<MySql>)> {
        // if let Some(projects) = self.ref0_projects.get(&refno.get_0()) {
        //     ///只有一个的时候
        //     if projects.len() == 1 {
        //         let project = projects.value().iter().next().as_ref().unwrap().clone();
        //         if let Some(project_pool) = self.project_map.get(project) {
        //             return Some((project.clone(), project_pool.value().clone()));
        //         }
        //     } else {
        //         for project in &self.db_option.included_projects {
        //             if let Some(pool) = self.get_project_pool(project) {
        //                 // if check_exist_refno(refno, &pool, &self.mdb_dbnums)
        //                 //     .await
        //                 //     .ok()?
        //                 // {
        //                     return Some((project.clone(), pool.clone()));
        //                 // }
        //             }
        //         }
        //     }
        // }
        None
    }

    fn match_stype(input: i32) -> String {
        match input {
            1 => "DESI".to_string(),
            2 => "CATA".to_string(),
            4 => "PROP".to_string(),
            6 => "ISOD".to_string(),
            7 => "PADD".to_string(),
            8 => "DICT".to_string(),
            9 => "ENGI".to_string(),
            14 => "SCHE".to_string(),
            _ => "".to_string(),
        }
    }

    ///获得当前mdb下的site参考号
    pub async fn get_site_refnos(&self) -> anyhow::Result<Vec<RefU64>> {
        // let world_refno = self.get_desi_world().await?.refno;
        // let r = self
        //     .get_cached_site_nodes(world_refno)
        //     .await?
        //     .unwrap_or_default()
        //     .iter()
        //     .map(|x| x.refno)
        //     .collect();
        Ok(vec![])
    }
}

#[tokio::test]
async fn test_get_attr() -> anyhow::Result<()> {
    // let mut mgr = AiosDBManager::init_form_config().await?;
    // let refno: RefU64 = RefI32Tuple((23584, 8)).into();
    // let v = mgr.get_attr(refno).await?;
    // println!("v={:?}", v.to_string_hashmap());

    // mgr.cache_geos_data("Sample", "SAMPLE").await?;

    Ok(())
}

#[test]
fn test_compute_distance() {
    let x = Vec3::new(19373.929, -2923.338, 15286.0);
    let y = Vec3::new(19381.39, -2894.83, 15286.0);
    let arrive = x.distance(y);
    let z = Vec3::new(19381.39, -2865.362, 15286.0);
    let leave = z.distance(y);
    let inst_a = Vec3::new(28.508010864257812, 7.4603271484375, 0.0);
    let inst_b = Vec3::new(0.0, 0.0, 0.0);
    let inst_dis = inst_a.distance(inst_b);
    dbg!(&inst_dis);
    dbg!(&arrive);
    dbg!(&leave);
}
