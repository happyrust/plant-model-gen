//! T008 离线校验：按需闭包站点 vs 全量基准站点的生成结果一致性对比（spec 002）。
//!
//! 用法（CLI `verify-cata-closure`）：当前 `-c` 配置指向**按需站点**（被校验方），
//! `--baseline-*` 参数指向**基准站点**（通常为整库解析的站点）。对给定设计根
//! refno（如 BRAN）逐项对比：
//!
//! 1. **成员完整性**：基准库设计子树成员 pe 在按需库必须全部存在；
//! 2. **几何指纹**：按需库 `inst_relate(in→out=inst_info:<cata_hash>)` 与基准库
//!    `inst_relate`（缺失时回退解析期 `ele_reuse_relate`）逐 refno 一致 ——
//!    cata_hash 由 SPRE/DESP 派生，两条独立路径一致即生成输入等价；
//! 3. **TUBI**：两库 tubi_relate 段数对比（基准缺数据时仅告警，不判失败）；
//! 4. **裁剪率**：按 manifest 覆盖的 CATA dbnum 对比两库 pe 数（报告用）。
//!
//! 基准侧走 SurrealDB **HTTP `/sql` 接口**（不经 WS SDK）：基准站点的服务端
//! 版本不受控（如 3.1.0-alpha 与 SDK fork 握手挂起），HTTP 文本接口无此问题。
//!
//! 报告写 `output/<project>/cata_closure_verify.json`；`passed=false` 时 CLI
//! 以非零码退出，可直接接 CI / 灰度门禁。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use aios_core::{RefU64, SurrealQueryExt};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// 基准站点连接参数。
#[derive(Debug, Clone)]
pub struct BaselineEndpoint {
    /// `host:port`（可带 `ws://` / `http://` 前缀，连接时统一为 http）。
    pub address: String,
    pub ns: String,
    pub db: String,
    pub user: String,
    pub pass: String,
}

/// 基准站点 HTTP `/sql` 客户端。
struct BaselineHttp {
    client: reqwest::Client,
    url: String,
    ep: BaselineEndpoint,
}

impl BaselineHttp {
    fn new(ep: &BaselineEndpoint) -> Result<Self> {
        let host = ep
            .address
            .trim_start_matches("ws://")
            .trim_start_matches("wss://")
            .trim_start_matches("http://")
            .trim_start_matches("https://");
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .context("构建基准站点 HTTP 客户端失败")?;
        Ok(Self {
            client,
            url: format!("http://{host}/sql"),
            ep: ep.clone(),
        })
    }

    /// 执行单条语句，返回其 result 数组。
    async fn sql(&self, sql: &str) -> Result<Vec<serde_json::Value>> {
        let resp = self
            .client
            .post(&self.url)
            .basic_auth(&self.ep.user, Some(&self.ep.pass))
            .header("Accept", "application/json")
            // 3.x 与 2.x 的 ns/db header 名不同，两套都带上。
            .header("surreal-ns", &self.ep.ns)
            .header("surreal-db", &self.ep.db)
            .header("NS", &self.ep.ns)
            .header("DB", &self.ep.db)
            .body(sql.to_string())
            .send()
            .await
            .with_context(|| format!("基准站点 HTTP 请求失败: {}", self.url))?;
        let status = resp.status();
        let body: serde_json::Value = resp.json().await.context("基准站点响应非 JSON")?;
        anyhow::ensure!(status.is_success(), "基准站点 HTTP {}: {}", status, body);

        // 响应形如 [{status:"OK", result:[...]}, ...]（单语句也可能直接为对象）。
        let first = match &body {
            serde_json::Value::Array(items) => items.first().cloned().unwrap_or_default(),
            other => other.clone(),
        };
        if let Some(err) = first.get("status").and_then(|s| s.as_str()) {
            anyhow::ensure!(err == "OK", "基准站点语句失败: {}", first);
        }
        Ok(first
            .get("result")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default())
    }

    async fn count(&self, sql: &str) -> Result<i64> {
        let rows = self.sql(sql).await?;
        Ok(rows
            .first()
            .and_then(|row| row.get("count"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0))
    }
}

/// 单个 refno 的几何指纹不一致项。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HashMismatch {
    pub refno: String,
    pub ondemand: Option<String>,
    pub baseline: String,
}

/// 单个 CATA dbnum 的解析规模对比。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CataPeCount {
    pub dbnum: u32,
    pub ondemand: i64,
    pub baseline: i64,
}

/// 校验报告（落 JSON + CI 门禁依据）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CataClosureVerifyReport {
    pub generated_at: String,
    pub roots: Vec<String>,
    pub baseline_address: String,
    /// 基准库子树成员总数（含根）。
    pub member_total: usize,
    /// 按需库缺失的成员 pe。
    pub member_missing: Vec<String>,
    /// 几何指纹对比总数（基准侧有指纹的 refno 数）。
    pub hash_checked: usize,
    pub hash_matched: usize,
    /// 基准侧指纹来源：inst_relate / ele_reuse_relate。
    pub hash_baseline_source: String,
    pub hash_mismatched: Vec<HashMismatch>,
    pub tubi_ondemand: usize,
    pub tubi_baseline: usize,
    /// 基准站点 tubi_relate 为空（历史缺陷），该项跳过判定。
    pub tubi_baseline_missing: bool,
    pub cata_pe_counts: Vec<CataPeCount>,
    pub passed: bool,
}

fn pe_key(refno: &RefU64) -> String {
    format!("pe:`{}_{}`", refno.get_0(), refno.get_1())
}

/// `SELECT VALUE [<string>in, <string>out] ...` 结果 → refno→hash 映射。
fn pairs_to_map(rows: Vec<serde_json::Value>) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for row in rows {
        if let Some(arr) = row.as_array() {
            if let (Some(k), Some(v)) = (
                arr.first().and_then(|v| v.as_str()),
                arr.get(1).and_then(|v| v.as_str()),
            ) {
                map.insert(k.to_string(), v.to_string());
            }
        }
    }
    map
}

/// 按需侧（全局连接）查询。
async fn ondemand_json(sql: &str) -> Result<Vec<serde_json::Value>> {
    let mut resp = aios_core::project_primary_db()
        .query_response(sql)
        .await
        .with_context(|| format!("按需站点查询失败: {sql}"))?;
    let rows: Vec<serde_json::Value> = resp.take(0).unwrap_or_default();
    Ok(rows)
}

async fn ondemand_count(sql: &str) -> Result<i64> {
    let rows = ondemand_json(sql).await?;
    Ok(rows
        .first()
        .and_then(|row| row.get("count"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0))
}

/// 执行校验并写报告。
pub async fn verify_cata_closure_against_baseline(
    roots: &[RefU64],
    baseline_ep: &BaselineEndpoint,
    out_path: &Path,
) -> Result<CataClosureVerifyReport> {
    // 按需侧用全局连接（当前 -c 配置站点）；基准侧 HTTP 直连。
    crate::fast_model::utils::ensure_surreal_init().await?;
    let baseline = BaselineHttp::new(baseline_ep)?;

    let mut report = CataClosureVerifyReport {
        generated_at: chrono::Utc::now().to_rfc3339(),
        roots: roots
            .iter()
            .map(|r| format!("{}_{}", r.get_0(), r.get_1()))
            .collect(),
        baseline_address: baseline_ep.address.clone(),
        ..Default::default()
    };

    // ── 1. 成员完整性 ────────────────────────────────────────────────────
    let mut member_keys: Vec<String> = Vec::new();
    for root in roots {
        let root_key = pe_key(root);
        member_keys.push(root_key.clone());
        let rows = baseline
            .sql(&format!(
                "SELECT VALUE <string>id FROM pe WHERE owner = {root_key};"
            ))
            .await?;
        member_keys.extend(
            rows.into_iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string())),
        );
    }
    report.member_total = member_keys.len();

    let member_list = member_keys.join(",");
    let missing_rows = ondemand_json(&format!(
        "RETURN array::filter([{member_list}], |$p| !record::exists($p));"
    ))
    .await?;
    report.member_missing = missing_rows
        .iter()
        .flat_map(|v| v.as_array().cloned().unwrap_or_default())
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();

    // ── 2. 几何指纹（cata_hash）一致性 ──────────────────────────────────
    let pair_sql = |table: &str| {
        format!("SELECT VALUE [<string>in, <string>out] FROM {table} WHERE in IN [{member_list}];")
    };
    let ondemand_map = pairs_to_map(ondemand_json(&pair_sql("inst_relate")).await?);

    let mut baseline_map = pairs_to_map(baseline.sql(&pair_sql("inst_relate")).await?);
    report.hash_baseline_source = "inst_relate".to_string();
    if baseline_map.is_empty() {
        // 老站点管件无 inst_relate：回退解析期写入的 ele_reuse_relate（同源 cata_hash）。
        baseline_map = pairs_to_map(baseline.sql(&pair_sql("ele_reuse_relate")).await?);
        report.hash_baseline_source = "ele_reuse_relate".to_string();
    }

    // BRAN/HANG 根自身不参与指纹对比：解析期 ele_reuse_relate 对带 TUBI 几何的
    // noun 也按自身 refno 建桥，但其几何走 tubi_relate（不生成 inst_relate），
    // 按需库无该映射是预期行为。
    let root_keys: std::collections::HashSet<String> = roots.iter().map(pe_key).collect();
    for (refno, base_hash) in &baseline_map {
        if root_keys.contains(refno) {
            continue;
        }
        report.hash_checked += 1;
        match ondemand_map.get(refno) {
            Some(hash) if hash == base_hash => report.hash_matched += 1,
            other => report.hash_mismatched.push(HashMismatch {
                refno: refno.clone(),
                ondemand: other.cloned(),
                baseline: base_hash.clone(),
            }),
        }
    }

    // ── 3. TUBI 段数 ─────────────────────────────────────────────────────
    for root in roots {
        let sql = format!(
            "SELECT count() FROM tubi_relate WHERE record::id(id)[0] = {} GROUP ALL;",
            pe_key(root)
        );
        report.tubi_ondemand += ondemand_count(&sql).await? as usize;
        report.tubi_baseline += baseline.count(&sql).await? as usize;
    }
    report.tubi_baseline_missing = report.tubi_baseline == 0;

    // ── 4. CATA 解析规模对比（manifest 覆盖的 dbnum）────────────────────
    let project = aios_core::get_db_option().project_name.clone();
    let manifest_path = super::cata_closure::default_manifest_path(&project);
    if manifest_path.exists() {
        if let Ok(manifest) = super::cata_closure::CataClosureManifest::load_json(&manifest_path) {
            for dbnum in manifest.by_dbnum.keys() {
                let sql = format!("SELECT count() FROM pe WHERE dbnum = {dbnum} GROUP ALL;");
                report.cata_pe_counts.push(CataPeCount {
                    dbnum: *dbnum,
                    ondemand: ondemand_count(&sql).await?,
                    baseline: baseline.count(&sql).await?,
                });
            }
        }
    }

    // ── 判定 + 落盘 ──────────────────────────────────────────────────────
    report.passed = report.member_missing.is_empty() && report.hash_mismatched.is_empty();

    if let Some(parent) = out_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(out_path, serde_json::to_string_pretty(&report)?)
        .with_context(|| format!("写校验报告失败: {}", out_path.display()))?;
    println!("📄 校验报告 → {}", out_path.display());
    Ok(report)
}

/// CLI 包装：解析 refno 字符串、推导默认报告路径。
pub async fn run_verify_from_cli(
    refno_strs: &[String],
    baseline_ep: &BaselineEndpoint,
    out_override: Option<PathBuf>,
) -> Result<CataClosureVerifyReport> {
    let roots: Vec<RefU64> = refno_strs
        .iter()
        .filter_map(|s| s.trim().parse::<RefU64>().ok())
        .collect();
    anyhow::ensure!(
        !roots.is_empty(),
        "--refnos 未解析出有效 refno（期望形如 24381_145018）"
    );

    let project = aios_core::get_db_option().project_name.clone();
    let out_path = out_override.unwrap_or_else(|| {
        crate::versioned_db::db_meta_info::get_project_tree_dir(&project)
            .join("cata_closure_verify.json")
    });
    verify_cata_closure_against_baseline(&roots, baseline_ep, &out_path).await
}
