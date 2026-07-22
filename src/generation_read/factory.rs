use std::sync::Arc;

use serde::Deserialize;
use surrealdb::types::SurrealValue;

use crate::options::DbOptionExt;
use crate::version_store::SurrealReplicaStore;

use super::error::{GenerationReadError, GenerationReadResult};
use super::surreal::SurrealVersionedReadBackend;
use super::traits::{GenerationReadBackend, VersionedReadSession};
use super::types::{DataVersion, InputVersionManifest};

/// specs/027（ADR-0008）：生成读取唯一后端 = Surreal 主库。
///
/// DuckLake/compare 后端已随 ADR-0007 退役；输入版本清单降级为观测记录，
/// 由当前 Committed Watermark 构造，不再有“权威 snapshot 来源”或 fail-closed
/// 覆盖校验。
pub async fn open_generation_read_session(
    options: &DbOptionExt,
) -> GenerationReadResult<Arc<dyn VersionedReadSession>> {
    let manifest = Arc::new(resolve_input_version_manifest(options).await?);
    let surreal: Arc<dyn GenerationReadBackend> = Arc::new(SurrealVersionedReadBackend::new(
        SurrealReplicaStore::default(),
    ));
    surreal.open_session(manifest).await
}

#[derive(Debug, Deserialize, SurrealValue)]
struct WatermarkRow {
    dbnum: i64,
    #[serde(default)]
    sesno: Option<i64>,
}

/// 观测构造输入版本清单：`dbnum → Committed Watermark`。
///
/// 口径与 `versioned_db::version_commit::committed_watermark` 一致
/// （锚点优先、回退 `dbnum_info_table`）；`manual_db_nums` 存在时按其过滤。
/// 该清单只写入运行结果供复现解释（观测记录），不参与任何绑定或失败关闭。
pub async fn resolve_input_version_manifest(
    options: &DbOptionExt,
) -> GenerationReadResult<InputVersionManifest> {
    use aios_core::project_primary_db;

    let sql = "SELECT dbnum, math::max(sesno) AS sesno FROM dbnum_info_table GROUP BY dbnum;\n\
               SELECT dbnum, math::max(sesno) AS sesno FROM sesno_version_anchor \
               WHERE source IN ['full', 'incremental'] GROUP BY dbnum;";
    let backend_error = |operation: &'static str, message: String| GenerationReadError::BackendQuery {
        backend: "surreal",
        operation,
        message,
    };
    let mut response = project_primary_db()
        .query(sql)
        .await
        .map_err(|error| backend_error("watermark.query", error.to_string()))?;

    let mut take_rows = |idx: usize| -> GenerationReadResult<Vec<WatermarkRow>> {
        match response.take::<Vec<WatermarkRow>>(idx) {
            Ok(rows) => Ok(rows),
            // 表不存在（从未解析/从未锚定的站点）按无记录处理
            Err(error) if error.to_string().contains("does not exist") => Ok(Vec::new()),
            Err(error) => Err(backend_error("watermark.take", error.to_string())),
        }
    };
    let legacy_rows = take_rows(0)?;
    let anchor_rows = take_rows(1)?;

    let mut watermarks = std::collections::BTreeMap::<u32, u32>::new();
    for row in legacy_rows {
        if let (Ok(dbnum), Some(sesno)) = (
            u32::try_from(row.dbnum),
            row.sesno.and_then(|value| u32::try_from(value).ok()),
        ) {
            watermarks.insert(dbnum, sesno);
        }
    }
    for row in anchor_rows {
        if let (Ok(dbnum), Some(sesno)) = (
            u32::try_from(row.dbnum),
            row.sesno.and_then(|value| u32::try_from(value).ok()),
        ) {
            if sesno > 0 {
                watermarks.insert(dbnum, sesno);
            }
        }
    }

    if let Some(manual) = options.inner.manual_db_nums.as_ref()
        && !manual.is_empty()
    {
        let allowed: std::collections::BTreeSet<u32> = manual.iter().copied().collect();
        watermarks.retain(|dbnum, _| allowed.contains(dbnum));
    }

    if watermarks.is_empty() {
        return Err(backend_error(
            "watermark.observe",
            "没有可观测的 dbnum 水位（dbnum_info_table / sesno_version_anchor 均为空，或 manual_db_nums 过滤后为空）"
                .to_string(),
        ));
    }

    let versions = watermarks.into_iter().map(|(dbnum, sesno)| DataVersion {
        dbnum,
        sesno,
        commit_fingerprint: format!("observed-watermark:{dbnum}:{sesno}"),
    });
    InputVersionManifest::new(0, 0, versions)
}
