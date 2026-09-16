use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 子孙 refno 列表 fixture
#[derive(Serialize, Deserialize, Debug)]
pub struct DescendantRefnos {
    pub root: String,
    pub count: usize,
    pub descendants: Vec<String>,
}

/// 几何实例 fixture（与 capture_model_baseline 输出一致）
#[derive(Serialize, Deserialize, Debug)]
pub struct GeomInstanceFixture {
    pub refno: String,
    pub owner: String,
    pub has_neg: bool,
    pub world_trans: serde_json::Value,
    pub insts: Vec<GeomInstEntry>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct GeomInstEntry {
    pub geo_hash: String,
    pub geo_transform: serde_json::Value,
    pub is_tubi: bool,
    pub unit_flag: bool,
}

/// 导出统计 fixture
#[derive(Serialize, Deserialize, Debug)]
pub struct ExportSummary {
    pub component_count: usize,
    pub tubing_count: usize,
    pub total_instances: usize,
    pub geo_hash_set: Vec<String>,
}

/// OBJ 文件统计 fixture
#[derive(Serialize, Deserialize, Debug)]
pub struct ObjStats {
    pub vertex_count: usize,
    pub face_count: usize,
    pub group_count: usize,
    pub file_size_bytes: u64,
}

fn fixture_dir(refno: &str) -> PathBuf {
    PathBuf::from(format!("test_data/model_regression/{}", refno))
}

fn load_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

pub fn load_descendant_refnos(refno: &str) -> Option<DescendantRefnos> {
    load_json(&fixture_dir(refno).join("descendant_refnos.json"))
}

pub fn load_geom_instances(refno: &str) -> Option<Vec<GeomInstanceFixture>> {
    load_json(&fixture_dir(refno).join("geom_instances.json"))
}

pub fn load_export_summary(refno: &str) -> Option<ExportSummary> {
    load_json(&fixture_dir(refno).join("export_summary.json"))
}

pub fn load_obj_stats(refno: &str) -> Option<ObjStats> {
    load_json(&fixture_dir(refno).join("expected_obj_stats.json"))
}

pub fn expected_obj_path(refno: &str) -> PathBuf {
    fixture_dir(refno).join("expected.obj")
}

/// 比较两个 world_transform JSON 值，返回最大分量差
pub fn max_transform_diff(a: &serde_json::Value, b: &serde_json::Value) -> f64 {
    fn extract_floats(v: &serde_json::Value) -> Vec<f64> {
        match v {
            serde_json::Value::Number(n) => vec![n.as_f64().unwrap_or(0.0)],
            serde_json::Value::Array(arr) => arr.iter().flat_map(extract_floats).collect(),
            serde_json::Value::Object(map) => {
                let mut floats = Vec::new();
                // 按 key 排序确保顺序一致
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                for key in keys {
                    floats.extend(extract_floats(&map[key]));
                }
                floats
            }
            _ => vec![],
        }
    }

    let fa = extract_floats(a);
    let fb = extract_floats(b);

    if fa.len() != fb.len() {
        return f64::INFINITY;
    }

    fa.iter()
        .zip(fb.iter())
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f64, f64::max)
}
