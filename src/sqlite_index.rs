use std::path::{Path, PathBuf};

#[cfg(feature = "sqlite-index")]
use rusqlite::{params, Connection, Result};

// Minimal SQLite-based AABB index using the SQLite RTree virtual table.
// This module is feature-gated behind `sqlite-index` and can be integrated
// incrementally without impacting existing backends.
pub struct SqliteAabbIndex {
    path: PathBuf,
}

#[cfg(feature = "sqlite-index")]
impl SqliteAabbIndex {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let this = SqliteAabbIndex { path: path.as_ref().to_path_buf() };
        let conn = Connection::open(&this.path)?;
        Self::configure(&conn)?;
        drop(conn);
        Ok(this)
    }

    fn configure(conn: &Connection) -> Result<()> {
        // WAL for multi-reader concurrency; NORMAL sync for performance.
        conn.pragma_update(None, "journal_mode", &"WAL")?;
        conn.pragma_update(None, "synchronous", &"NORMAL")?;
        Ok(())
    }

    pub fn init_schema(&self) -> Result<()> {
        let conn = Connection::open(&self.path)?;
        Self::configure(&conn)?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS items (
                id INTEGER PRIMARY KEY,
                noun TEXT
            );
            -- 3D AABB RTree: id, [min_x, max_x], [min_y, max_y], [min_z, max_z]
            CREATE VIRTUAL TABLE IF NOT EXISTS aabb_index USING rtree(
                id, min_x, max_x, min_y, max_y, min_z, max_z
            );
            "#,
        )?;
        // 兼容旧数据库文件：如果 items 只有 id 列，这条语句会失败；忽略即可。
        let _ = conn.execute("ALTER TABLE items ADD COLUMN noun TEXT", []);
        Ok(())
    }

    // Batch insert/replace AABBs: (id, min_x, max_x, min_y, max_y, min_z, max_z)
    pub fn insert_many<I>(&self, iter: I) -> Result<usize>
    where
        I: IntoIterator<Item = (i64, f64, f64, f64, f64, f64, f64)>,
    {
        let mut conn = Connection::open(&self.path)?;
        Self::configure(&conn)?;
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO aabb_index \
                 (id, min_x, max_x, min_y, max_y, min_z, max_z) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for (id, minx, maxx, miny, maxy, minz, maxz) in iter {
                stmt.execute(params![id, minx, maxx, miny, maxy, minz, maxz])?;
            }
        } // stmt 在这里被销毁，释放对 tx 的借用
        tx.commit()?;
        Ok(1)
    }

    // AABB intersection query: returns matching ids.
    pub fn query_intersect(
        &self,
        minx: f64,
        maxx: f64,
        miny: f64,
        maxy: f64,
        minz: f64,
        maxz: f64,
    ) -> Result<Vec<i64>> {
        let conn = Connection::open(&self.path)?;
        let mut stmt = conn.prepare(
            "SELECT id FROM aabb_index \
             WHERE min_x <= ?2 AND max_x >= ?1 \
               AND min_y <= ?4 AND max_y >= ?3 \
               AND min_z <= ?6 AND max_z >= ?5",
        )?;
        let rows = stmt.query_map((minx, maxx, miny, maxy, minz, maxz), |row| row.get::<_, i64>(0))?;
        let mut ids = Vec::new();
        for r in rows {
            ids.push(r?);
        }
        Ok(ids)
    }

    // Optional: range query on X for scanning.
    pub fn query_range_x(&self, minx: f64, maxx: f64) -> Result<Vec<i64>> {
        let conn = Connection::open(&self.path)?;
        let mut stmt = conn.prepare(
            "SELECT id FROM aabb_index \
             WHERE min_x <= ?2 AND max_x >= ?1",
        )?;
        let rows = stmt.query_map((minx, maxx), |row| row.get::<_, i64>(0))?;
        let mut ids = Vec::new();
        for r in rows {
            ids.push(r?);
        }
        Ok(ids)
    }

    // Query all AABBs: returns all (id, min_x, max_x, min_y, max_y, min_z, max_z) tuples
    pub fn query_all_aabbs(&self) -> Result<Vec<(i64, f64, f64, f64, f64, f64, f64)>> {
        let conn = Connection::open(&self.path)?;
        let mut stmt = conn.prepare(
            "SELECT id, min_x, max_x, min_y, max_y, min_z, max_z FROM aabb_index"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, f64>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, f64>(4)?,
                row.get::<_, f64>(5)?,
                row.get::<_, f64>(6)?,
            ))
        })?;
        let mut aabbs = Vec::new();
        for r in rows {
            aabbs.push(r?);
        }
        Ok(aabbs)
    }

    /// 批量插入 items 表（id, noun）
    pub fn insert_items<I>(&self, iter: I) -> Result<usize>
    where
        I: IntoIterator<Item = (i64, String)>,
    {
        let mut conn = Connection::open(&self.path)?;
        Self::configure(&conn)?;
        let tx = conn.transaction()?;
        let mut count = 0;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO items (id, noun) VALUES (?1, ?2)",
            )?;
            for (id, noun) in iter {
                stmt.execute(params![id, noun])?;
                count += 1;
            }
        }
        tx.commit()?;
        Ok(count)
    }

    /// 批量插入 AABB 和 items（合并事务）
    pub fn insert_aabbs_with_items<I>(&self, iter: I) -> Result<usize>
    where
        I: IntoIterator<Item = (i64, String, f64, f64, f64, f64, f64, f64)>,
    {
        let mut conn = Connection::open(&self.path)?;
        Self::configure(&conn)?;
        let tx = conn.transaction()?;
        let mut count = 0;
        {
            let mut aabb_stmt = tx.prepare(
                "INSERT OR REPLACE INTO aabb_index \
                 (id, min_x, max_x, min_y, max_y, min_z, max_z) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            let mut item_stmt = tx.prepare(
                "INSERT OR REPLACE INTO items (id, noun) VALUES (?1, ?2)",
            )?;
            for (id, noun, minx, maxx, miny, maxy, minz, maxz) in iter {
                aabb_stmt.execute(params![id, minx, maxx, miny, maxy, minz, maxz])?;
                item_stmt.execute(params![id, noun])?;
                count += 1;
            }
        }
        tx.commit()?;
        Ok(count)
    }
}

// ============================================================================
// instances.json 导入功能
// ============================================================================

/// 从 instances.json 导入空间索引的配置
#[derive(Debug, Clone)]
pub struct ImportConfig {
    /// EQUI 使用粗粒度（Owner AABB）
    pub equi_coarse: bool,
    /// BRAN/HANG 使用细粒度（Children + Tubings AABB）
    pub bran_fine: bool,
}

impl Default for ImportConfig {
    fn default() -> Self {
        Self {
            equi_coarse: true,
            bran_fine: true,
        }
    }
}

/// 将 refno 字符串（如 "17496_170764"）转换为 i64
/// 格式：(dbnum << 32) + refno
pub fn refno_str_to_i64(refno: &str) -> Option<i64> {
    // 兼容 "dbnum_refno" 与 "dbnum/refno" 两种格式
    let sep = if refno.contains('_') {
        '_'
    } else if refno.contains('/') {
        '/'
    } else {
        return None;
    };
    let parts: Vec<&str> = refno.split(sep).collect();
    if parts.len() != 2 {
        return None;
    }
    let dbnum: u32 = parts[0].parse().ok()?;
    let refno: u32 = parts[1].parse().ok()?;
    Some(((dbnum as u64) << 32 | refno as u64) as i64)
}

/// 将 i64 转换回 refno 字符串
pub fn i64_to_refno_str(id: i64) -> String {
    let id = id as u64;
    let dbnum = (id >> 32) as u32;
    let refno = (id & 0xFFFFFFFF) as u32;
    format!("{}_{}", dbnum, refno)
}

#[cfg(feature = "sqlite-index")]
impl SqliteAabbIndex {
    /// 从 instances.json 文件导入空间索引
    pub fn import_from_instances_json(
        &self,
        json_path: &Path,
        config: &ImportConfig,
    ) -> anyhow::Result<ImportStats> {
        use std::fs::File;
        use std::io::BufReader;

        let file = File::open(json_path)
            .map_err(|e| anyhow::anyhow!("打开文件失败: {}: {}", json_path.display(), e))?;
        let reader = BufReader::new(file);
        let json: serde_json::Value = serde_json::from_reader(reader)
            .map_err(|e| anyhow::anyhow!("解析 JSON 失败: {}", e))?;

        self.import_from_json_value_with_path(&json, config, Some(json_path))
    }

    /// 从 JSON Value 导入空间索引
    pub fn import_from_json_value(
        &self,
        json: &serde_json::Value,
        config: &ImportConfig,
    ) -> anyhow::Result<ImportStats> {
        self.import_from_json_value_with_path(json, config, None)
    }

    fn import_from_json_value_with_path(
        &self,
        json: &serde_json::Value,
        config: &ImportConfig,
        json_path: Option<&Path>,
    ) -> anyhow::Result<ImportStats> {
        use std::collections::{HashMap, HashSet};

        let mut stats = ImportStats::default();

        let groups = json["groups"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("JSON 缺少 groups 数组"))?;

        // 旧格式：group/child 上直接携带 aabb: {min,max}
        let looks_like_inline_aabb = groups.iter().any(|g| g.get("owner_aabb").is_some())
            || groups
                .iter()
                .flat_map(|g| g.get("children").and_then(|v| v.as_array()).into_iter().flatten())
                .any(|c| c.get("aabb").is_some());

        if looks_like_inline_aabb {
            // ========================
            // 旧格式导入逻辑（min/max 直写）
            // ========================
            let mut aabb_map: HashMap<i64, (String, f64, f64, f64, f64, f64, f64)> = HashMap::new();

            for group in groups {
                let owner_noun = group["owner_noun"].as_str().unwrap_or("");

                match owner_noun {
                    "EQUI" if config.equi_coarse => {
                        if let Some(item) = Self::extract_owner_aabb(group) {
                            Self::merge_aabb(&mut aabb_map, item);
                            stats.equi_count += 1;
                        }
                    }
                    "BRAN" | "HANG" if config.bran_fine => {
                        Self::extract_children_aabbs_merged(group, &mut aabb_map, &mut stats);
                        Self::extract_tubings_aabbs_merged(group, &mut aabb_map, &mut stats);
                    }
                    _ => {}
                }
            }

            let aabb_items: Vec<_> = aabb_map
                .into_iter()
                .map(|(id, (noun, minx, maxx, miny, maxy, minz, maxz))| {
                    (id, noun, minx, maxx, miny, maxy, minz, maxz)
                })
                .collect();
            stats.unique_count = aabb_items.len();
            if !aabb_items.is_empty() {
                self.insert_aabbs_with_items(aabb_items)?;
            }
            stats.total_inserted = stats.equi_count + stats.children_count + stats.tubings_count;
            return Ok(stats);
        }

        // ========================
        // 新格式导入逻辑：AABB 去重表 aabb.json + aabb_hash 引用
        // ========================
        let aabb_table = {
            let Some(json_path) = json_path else {
                return Err(anyhow::anyhow!(
                    "instances.json 使用 aabb_hash 格式，但未提供 json_path 上下文，无法定位 aabb.json"
                ));
            };
            let base_dir = json_path.parent().ok_or_else(|| anyhow::anyhow!("无法获取 instances.json 所在目录"))?;
            let aabb_path = base_dir.join("aabb.json");
            if !aabb_path.exists() {
                return Err(anyhow::anyhow!(
                    "instances.json 使用 aabb_hash 格式，但未找到配套 aabb.json: {}",
                    aabb_path.display()
                ));
            }
            let bytes = std::fs::read(&aabb_path)
                .map_err(|e| anyhow::anyhow!("读取 aabb.json 失败: {}: {}", aabb_path.display(), e))?;
            let v: serde_json::Value = serde_json::from_slice(&bytes)
                .map_err(|e| anyhow::anyhow!("解析 aabb.json 失败: {}: {}", aabb_path.display(), e))?;
            v
        };

        fn aabb_hash_key(v: &serde_json::Value) -> Option<String> {
            if let Some(s) = v.as_str() {
                return Some(s.to_string());
            }
            if let Some(n) = v.as_u64() {
                return Some(n.to_string());
            }
            if let Some(n) = v.as_i64() {
                return Some(n.to_string());
            }
            None
        }

        fn aabb_from_table(
            aabb_table: &serde_json::Value,
            hash_value: &serde_json::Value,
        ) -> Option<(f64, f64, f64, f64, f64, f64)> {
            let key = aabb_hash_key(hash_value)?;
            let entry = aabb_table.get(&key)?;
            let min = entry.get("min")?.as_array()?;
            let max = entry.get("max")?.as_array()?;
            if min.len() < 3 || max.len() < 3 {
                return None;
            }
            Some((
                min[0].as_f64()?,
                max[0].as_f64()?,
                min[1].as_f64()?,
                max[1].as_f64()?,
                min[2].as_f64()?,
                max[2].as_f64()?,
            ))
        }

        fn merge_bounds(
            acc: &mut Option<(f64, f64, f64, f64, f64, f64)>,
            b: (f64, f64, f64, f64, f64, f64),
        ) {
            *acc = Some(match acc.take() {
                None => b,
                Some((minx, maxx, miny, maxy, minz, maxz)) => (
                    minx.min(b.0),
                    maxx.max(b.1),
                    miny.min(b.2),
                    maxy.max(b.3),
                    minz.min(b.4),
                    maxz.max(b.5),
                ),
            });
        }

        // 批量插入（避免一次性 Vec 过大）
        const CHUNK: usize = 50_000;
        let mut buf: Vec<(i64, String, f64, f64, f64, f64, f64, f64)> = Vec::with_capacity(CHUNK);
        let mut seen: HashSet<i64> = HashSet::new();

        let mut flush = |this: &SqliteAabbIndex, buf: &mut Vec<(i64, String, f64, f64, f64, f64, f64, f64)>| -> anyhow::Result<()> {
            if buf.is_empty() {
                return Ok(());
            }
            let items = std::mem::take(buf);
            this.insert_aabbs_with_items(items)?;
            Ok(())
        };

        // 1) groups：按配置导入
        for group in groups {
            let owner_noun = group.get("owner_noun").and_then(|v| v.as_str()).unwrap_or("");
            let owner_refno = group.get("owner_refno").and_then(|v| v.as_str()).unwrap_or("");

            // EQUI coarse：尝试用 children/tubings 的 AABB 合并近似 owner AABB
            if owner_noun == "EQUI" && config.equi_coarse {
                if let Some(id) = refno_str_to_i64(owner_refno) {
                    let mut merged: Option<(f64, f64, f64, f64, f64, f64)> = None;
                    if let Some(children) = group.get("children").and_then(|v| v.as_array()) {
                        for child in children {
                            if let Some(b) = child.get("aabb_hash").and_then(|h| aabb_from_table(&aabb_table, h)) {
                                merge_bounds(&mut merged, b);
                            }
                        }
                    }
                    if let Some(tubings) = group.get("tubings").and_then(|v| v.as_array()) {
                        for t in tubings {
                            if let Some(b) = t.get("aabb_hash").and_then(|h| aabb_from_table(&aabb_table, h)) {
                                merge_bounds(&mut merged, b);
                            }
                        }
                    }
                    if let Some((minx, maxx, miny, maxy, minz, maxz)) = merged {
                        if seen.insert(id) {
                            stats.unique_count += 1;
                        }
                        stats.equi_count += 1;
                        buf.push((id, owner_noun.to_string(), minx, maxx, miny, maxy, minz, maxz));
                        if buf.len() >= CHUNK {
                            flush(self, &mut buf)?;
                        }
                    }
                }
            }

            // BRAN/HANG fine：children + tubings
            if matches!(owner_noun, "BRAN" | "HANG") && config.bran_fine {
                if let Some(children) = group.get("children").and_then(|v| v.as_array()) {
                    for child in children {
                        let r = child.get("refno").and_then(|v| v.as_str()).unwrap_or("");
                        let id = match refno_str_to_i64(r) {
                            Some(v) => v,
                            None => continue,
                        };
                        let Some((minx, maxx, miny, maxy, minz, maxz)) = child
                            .get("aabb_hash")
                            .and_then(|h| aabb_from_table(&aabb_table, h))
                        else {
                            continue;
                        };
                        if seen.insert(id) {
                            stats.unique_count += 1;
                        }
                        stats.children_count += 1;
                        buf.push((id, owner_noun.to_string(), minx, maxx, miny, maxy, minz, maxz));
                        if buf.len() >= CHUNK {
                            flush(self, &mut buf)?;
                        }
                    }
                }
                if let Some(tubings) = group.get("tubings").and_then(|v| v.as_array()) {
                    for t in tubings {
                        let r = t.get("refno").and_then(|v| v.as_str()).unwrap_or("");
                        let id = match refno_str_to_i64(r) {
                            Some(v) => v,
                            None => continue,
                        };
                        let Some((minx, maxx, miny, maxy, minz, maxz)) = t
                            .get("aabb_hash")
                            .and_then(|h| aabb_from_table(&aabb_table, h))
                        else {
                            continue;
                        };
                        if seen.insert(id) {
                            stats.unique_count += 1;
                        }
                        stats.tubings_count += 1;
                        buf.push((id, "TUBI".to_string(), minx, maxx, miny, maxy, minz, maxz));
                        if buf.len() >= CHUNK {
                            flush(self, &mut buf)?;
                        }
                    }
                }
            }
        }

        // 2) instances：尽量补全（避免漏掉不在 BRAN/HANG/EQUI 分组内的构件）
        if let Some(instances) = json.get("instances").and_then(|v| v.as_array()) {
            for inst in instances {
                let r = inst.get("refno").and_then(|v| v.as_str()).unwrap_or("");
                let id = match refno_str_to_i64(r) {
                    Some(v) => v,
                    None => continue,
                };
                let Some((minx, maxx, miny, maxy, minz, maxz)) = inst
                    .get("aabb_hash")
                    .and_then(|h| aabb_from_table(&aabb_table, h))
                else {
                    continue;
                };
                let noun = inst
                    .get("noun")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or("")
                    .to_string();
                if seen.insert(id) {
                    stats.unique_count += 1;
                }
                // instances 不计入 total_inserted 的原三类统计，但对 room 计算很关键
                buf.push((id, noun, minx, maxx, miny, maxy, minz, maxz));
                if buf.len() >= CHUNK {
                    flush(self, &mut buf)?;
                }
            }
        }

        flush(self, &mut buf)?;

        stats.total_inserted = stats.equi_count + stats.children_count + stats.tubings_count;
        Ok(stats)
    }

    /// 合并 AABB 到 map（取并集）
    fn merge_aabb(
        map: &mut std::collections::HashMap<i64, (String, f64, f64, f64, f64, f64, f64)>,
        item: (i64, String, f64, f64, f64, f64, f64, f64),
    ) {
        let (id, noun, minx, maxx, miny, maxy, minz, maxz) = item;
        map.entry(id)
            .and_modify(|e| {
                e.1 = e.1.min(minx);  // min_x
                e.2 = e.2.max(maxx);  // max_x
                e.3 = e.3.min(miny);  // min_y
                e.4 = e.4.max(maxy);  // max_y
                e.5 = e.5.min(minz);  // min_z
                e.6 = e.6.max(maxz);  // max_z
            })
            .or_insert((noun, minx, maxx, miny, maxy, minz, maxz));
    }

    fn extract_owner_aabb(group: &serde_json::Value) -> Option<(i64, String, f64, f64, f64, f64, f64, f64)> {
        let refno = group["owner_refno"].as_str()?;
        let id = refno_str_to_i64(refno)?;
        let noun = group["owner_noun"].as_str().unwrap_or("").to_string();
        let aabb = &group["owner_aabb"];

        if aabb.is_null() {
            return None;
        }

        let min = aabb["min"].as_array()?;
        let max = aabb["max"].as_array()?;

        Some((
            id,
            noun,
            min[0].as_f64()?,
            max[0].as_f64()?,
            min[1].as_f64()?,
            max[1].as_f64()?,
            min[2].as_f64()?,
            max[2].as_f64()?,
        ))
    }

    fn extract_children_aabbs_merged(
        group: &serde_json::Value,
        map: &mut std::collections::HashMap<i64, (String, f64, f64, f64, f64, f64, f64)>,
        stats: &mut ImportStats,
    ) {
        if let Some(children) = group["children"].as_array() {
            for child in children {
                if let Some(item) = Self::extract_element_aabb(child) {
                    Self::merge_aabb(map, item);
                    stats.children_count += 1;
                }
            }
        }
    }

    fn extract_tubings_aabbs_merged(
        group: &serde_json::Value,
        map: &mut std::collections::HashMap<i64, (String, f64, f64, f64, f64, f64, f64)>,
        stats: &mut ImportStats,
    ) {
        if let Some(tubings) = group["tubings"].as_array() {
            for tubi in tubings {
                if let Some(item) = Self::extract_element_aabb(tubi) {
                    Self::merge_aabb(map, item);
                    stats.tubings_count += 1;
                }
            }
        }
    }

    fn extract_children_aabbs(
        group: &serde_json::Value,
        items: &mut Vec<(i64, String, f64, f64, f64, f64, f64, f64)>,
        stats: &mut ImportStats,
    ) {
        if let Some(children) = group["children"].as_array() {
            for child in children {
                if let Some(item) = Self::extract_element_aabb(child) {
                    items.push(item);
                    stats.children_count += 1;
                }
            }
        }
    }

    fn extract_tubings_aabbs(
        group: &serde_json::Value,
        items: &mut Vec<(i64, String, f64, f64, f64, f64, f64, f64)>,
        stats: &mut ImportStats,
    ) {
        if let Some(tubings) = group["tubings"].as_array() {
            for tubi in tubings {
                if let Some(item) = Self::extract_element_aabb(tubi) {
                    items.push(item);
                    stats.tubings_count += 1;
                }
            }
        }
    }

    fn extract_element_aabb(elem: &serde_json::Value) -> Option<(i64, String, f64, f64, f64, f64, f64, f64)> {
        let refno = elem["refno"].as_str()?;
        let id = refno_str_to_i64(refno)?;
        let noun = elem["noun"].as_str().unwrap_or("").to_string();
        let aabb = &elem["aabb"];

        if aabb.is_null() {
            return None;
        }

        let min = aabb["min"].as_array()?;
        let max = aabb["max"].as_array()?;

        Some((
            id,
            noun,
            min[0].as_f64()?,
            max[0].as_f64()?,
            min[1].as_f64()?,
            max[1].as_f64()?,
            min[2].as_f64()?,
            max[2].as_f64()?,
        ))
    }
}

/// 导入统计
#[derive(Debug, Default)]
pub struct ImportStats {
    pub equi_count: usize,
    pub children_count: usize,
    pub tubings_count: usize,
    pub total_inserted: usize,
    /// 去重后的唯一记录数
    pub unique_count: usize,
}

#[cfg(all(test, feature = "sqlite-index"))]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn basic_intersect() {
        let path = "test_aabb.sqlite";
        let _ = fs::remove_file(path);
        let idx = SqliteAabbIndex::open(path).unwrap();
        idx.init_schema().unwrap();

        let data = vec![
            (1, 0.0, 10.0, 0.0, 5.0, -5.0, 5.0),
            (2, 5.0, 15.0, 2.0, 8.0, -2.0, 2.0),
            (3, 20.0, 30.0, -1.0, 1.0, 0.0, 1.0),
        ];
        idx.insert_many(data).unwrap();

        let ids = idx
            .query_intersect(4.0, 6.0, 1.0, 3.0, -1.0, 1.0)
            .unwrap();
        assert!(ids.contains(&1) && ids.contains(&2) && !ids.contains(&3));

        let _ = fs::remove_file(path);
    }
}
