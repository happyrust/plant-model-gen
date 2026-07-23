use std::collections::BTreeMap;
use std::sync::Arc;

use serde::Deserialize;
use surrealdb::types::SurrealValue;

use super::error::{GenerationReadError, GenerationReadResult};
use super::surreal::SurrealVersionedReadBackend;
use super::traits::{GenerationReadBackend, VersionedReadSession};
use super::types::{DataVersion, GenerationReadMode, GenerationReadSpec, InputVersionManifest};
use crate::options::DbOptionExt;

/// specs/027（ADR-0008）：生成读取唯一后端 = Surreal 主库。
///
/// 兼容入口只用于 initialization/live 读取。需要绑定数据锚点的调用方必须使用
/// [`open_generation_read_session_with_spec`] 显式传入读取契约。
pub async fn open_generation_read_session(
    options: &DbOptionExt,
) -> GenerationReadResult<Arc<dyn VersionedReadSession>> {
    let spec = GenerationReadSpec::live();
    open_generation_read_session_with_spec(options, &spec).await
}

/// 按一次生成 run 的不可变读取契约打开 session。
///
pub async fn open_generation_read_session_with_spec(
    options: &DbOptionExt,
    spec: &GenerationReadSpec,
) -> GenerationReadResult<Arc<dyn VersionedReadSession>> {
    spec.validate()?;
    let manifest = Arc::new(resolve_input_version_manifest_for_spec(options, spec).await?);
    let read_at = match spec.mode() {
        GenerationReadMode::Live => None,
        GenerationReadMode::ReadAt => Some(
            spec.read_at()
                .ok_or_else(|| {
                    GenerationReadError::InvalidReadSpec("read_at 模式必须提供锚点时间".to_string())
                })?
                .to_string(),
        ),
    };
    let surreal: Arc<dyn GenerationReadBackend> =
        Arc::new(SurrealVersionedReadBackend::new(read_at));
    surreal.open_session(manifest).await
}

async fn resolve_input_version_manifest_for_spec(
    options: &DbOptionExt,
    spec: &GenerationReadSpec,
) -> GenerationReadResult<InputVersionManifest> {
    if spec.mode() == GenerationReadMode::Live {
        return resolve_input_version_manifest(options).await;
    }
    let versions = spec
        .observed_watermarks()
        .iter()
        .map(|(&dbnum, &sesno)| DataVersion {
            dbnum,
            sesno,
            commit_fingerprint: format!("anchored-watermark:{dbnum}:{sesno}"),
        });
    InputVersionManifest::new(0, 0, versions)
}

#[derive(Debug, Deserialize, SurrealValue)]
struct WatermarkRow {
    dbnum: i64,
    #[serde(default)]
    sesno: Option<i64>,
}

#[derive(Debug, Deserialize, SurrealValue)]
struct AnchorReadRow {
    dbnum: i64,
    sesno: i64,
    anchored_at: String,
}

/// Resolve the one immutable main-table read instant for a generation attempt.
///
/// The selected time is the newest current data anchor among the active dbnums.
/// Because data/model mutation is serialized by the project lock, querying all
/// main tables at that instant observes every selected dbnum at its recorded
/// watermark and cannot include a concurrent future commit.
pub async fn resolve_anchored_generation_read_spec(
    options: &DbOptionExt,
) -> GenerationReadResult<GenerationReadSpec> {
    let rows = load_anchor_rows(
        "source IN ['full', 'incremental_baseline', 'incremental']",
        "generation_anchor.query",
    )
    .await?;
    let selected_dbnums = options
        .inner
        .manual_db_nums
        .as_ref()
        .filter(|values| !values.is_empty());
    let mut selected = BTreeMap::<u32, (u32, String)>::new();
    for row in rows {
        let (Ok(dbnum), Ok(sesno)) = (u32::try_from(row.dbnum), u32::try_from(row.sesno)) else {
            continue;
        };
        if dbnum == 0
            || sesno == 0
            || selected_dbnums.is_some_and(|values| !values.contains(&dbnum))
        {
            continue;
        }
        match selected.get(&dbnum) {
            Some((current, _)) if *current >= sesno => {}
            _ => {
                selected.insert(dbnum, (sesno, row.anchored_at));
            }
        }
    }
    let read_at = selected
        .values()
        .map(|(_, anchored_at)| anchored_at)
        .max()
        .cloned()
        .ok_or_else(|| {
            GenerationReadError::InvalidReadSpec(
                "no committed data anchor is available for generation".to_string(),
            )
        })?;
    GenerationReadSpec::at(
        read_at,
        selected
            .into_iter()
            .map(|(dbnum, (sesno, _))| (dbnum, sesno))
            .collect(),
    )
}

/// Resolve the old hierarchy slice represented by the currently published
/// model watermark. Delete-only cleanup must use this slice because the target
/// data slice no longer contains the deleted PE.
pub async fn resolve_cleanup_read_spec(
    dbnum: u32,
    model_generation_watermark: u32,
) -> GenerationReadResult<Option<GenerationReadSpec>> {
    if model_generation_watermark == 0 {
        return Ok(None);
    }
    let rows = load_anchor_rows(
        &format!(
            "source = 'model_gen' AND dbnum = {dbnum} AND sesno = {model_generation_watermark}"
        ),
        "cleanup_anchor.query",
    )
    .await?;
    let read_at = rows
        .into_iter()
        .map(|row| row.anchored_at)
        .max()
        .ok_or_else(|| {
            GenerationReadError::InvalidReadSpec(format!(
                "model watermark has no anchor: dbnum={dbnum} sesno={model_generation_watermark}"
            ))
        })?;
    Ok(Some(GenerationReadSpec::at(
        read_at,
        [(dbnum, model_generation_watermark)].into_iter().collect(),
    )?))
}

async fn load_anchor_rows(
    predicate: &str,
    operation: &'static str,
) -> GenerationReadResult<Vec<AnchorReadRow>> {
    use aios_core::project_primary_db;

    let sql = format!(
        "SELECT dbnum, sesno, type::string(anchored_at) AS anchored_at \
         FROM sesno_version_anchor WHERE {predicate} \
         ORDER BY dbnum, sesno DESC, anchored_at DESC;"
    );
    let mut response = project_primary_db().query(sql).await.map_err(|error| {
        GenerationReadError::BackendQuery {
            backend: "surreal",
            operation,
            message: error.to_string(),
        }
    })?;
    response
        .take(0)
        .map_err(|error| GenerationReadError::BackendQuery {
            backend: "surreal",
            operation,
            message: error.to_string(),
        })
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
               WHERE source IN ['full', 'incremental_baseline', 'incremental'] GROUP BY dbnum;";
    let backend_error =
        |operation: &'static str, message: String| GenerationReadError::BackendQuery {
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
            if dbnum > 0 && sesno > 0 {
                watermarks.insert(dbnum, sesno);
            }
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
    if let Some(dbnums) = options
        .inner
        .manual_db_nums
        .as_ref()
        .filter(|values| !values.is_empty())
    {
        watermarks.retain(|dbnum, _| dbnums.contains(dbnum));
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
