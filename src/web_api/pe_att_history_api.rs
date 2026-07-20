//! specs/022 PE/ATT 历史查询 HTTP 最小集（锚点 resolve / list + snapshot）。
//!
//! 薄封装 rs-core `version_query`；禁止绕过锚点裸查 VERSION。

use aios_core::{
    HistoryError, RefU64, RefnoEnum, format_history_error, list_data_anchors, resolve_data_anchor,
    snapshot_at,
};
use axum::{Json, Router, extract::Query, http::StatusCode, response::IntoResponse, routing::get};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::str::FromStr;

pub fn create_pe_att_history_routes() -> Router {
    Router::new()
        .route("/api/model-history/anchors", get(get_anchors))
        .route("/api/model-history/resolve-anchor", get(get_resolve_anchor))
        .route("/api/model-history/snapshot", get(get_snapshot))
}

#[derive(Debug, Deserialize)]
struct AnchorsQuery {
    dbnum: u32,
    /// 0 或不传 = 不截断
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ResolveAnchorQuery {
    dbnum: u32,
    sesno: u32,
    /// 若为 true 且只能回退命中，返回 404
    #[serde(default)]
    exact_only: bool,
}

#[derive(Debug, Deserialize)]
struct SnapshotQuery {
    dbnum: u32,
    sesno: u32,
    /// `17496/1` 或 `17496_1`
    refno: Option<String>,
    /// 夹具/覆盖 PE 记录 id，如 `pe:17496_1`
    pe_key: Option<String>,
}

#[derive(Debug, Serialize)]
struct ApiOk<T: Serialize> {
    ok: bool,
    data: T,
}

#[derive(Debug, Serialize)]
struct ApiErr {
    ok: bool,
    error: ApiErrorBody,
}

#[derive(Debug, Serialize)]
struct ApiErrorBody {
    code: &'static str,
    message: String,
}

fn ok_json<T: Serialize>(data: T) -> (StatusCode, Json<Value>) {
    (StatusCode::OK, Json(json!(ApiOk { ok: true, data })))
}

fn err_json(
    status: StatusCode,
    code: &'static str,
    message: impl Into<String>,
) -> (StatusCode, Json<Value>) {
    (
        status,
        Json(json!(ApiErr {
            ok: false,
            error: ApiErrorBody {
                code,
                message: message.into(),
            },
        })),
    )
}

fn history_err_response(err: HistoryError) -> (StatusCode, Json<Value>) {
    match &err {
        HistoryError::Expired { .. } => {
            err_json(StatusCode::GONE, "Expired", format_history_error(&err))
        }
        HistoryError::Other(_) => err_json(
            StatusCode::BAD_GATEWAY,
            "QueryFailed",
            format_history_error(&err),
        ),
    }
}

fn parse_refno(raw: &str) -> Result<RefnoEnum, String> {
    let normalized = raw.trim().trim_start_matches('/').replace('\\', "/");
    RefnoEnum::from_str(&normalized).map_err(|e| format!("invalid refno '{raw}': {e}"))
}

async fn get_anchors(Query(q): Query<AnchorsQuery>) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(0);
    match list_data_anchors(q.dbnum, limit).await {
        Ok(rows) => ok_json(json!({
            "dbnum": q.dbnum,
            "count": rows.len(),
            "anchors": rows,
        })),
        Err(e) => err_json(
            StatusCode::BAD_GATEWAY,
            "QueryFailed",
            format!("list_data_anchors failed: {e}"),
        ),
    }
}

async fn get_resolve_anchor(Query(q): Query<ResolveAnchorQuery>) -> impl IntoResponse {
    match resolve_data_anchor(q.dbnum, q.sesno).await {
        Ok(Some(hit)) => {
            if q.exact_only && !hit.exact {
                return err_json(
                    StatusCode::NOT_FOUND,
                    "AnchorMissing",
                    format!(
                        "exact-only: no exact anchor dbnum={} sesno={}; nearest sesno={}",
                        q.dbnum, q.sesno, hit.sesno
                    ),
                );
            }
            ok_json(hit)
        }
        Ok(None) => err_json(
            StatusCode::NOT_FOUND,
            "AnchorMissing",
            format!(
                "未找到 dbnum={} sesno<={} 的 sesno_version_anchor",
                q.dbnum, q.sesno
            ),
        ),
        Err(e) => err_json(
            StatusCode::BAD_GATEWAY,
            "QueryFailed",
            format!("resolve_anchor failed: {e}"),
        ),
    }
}

async fn get_snapshot(Query(q): Query<SnapshotQuery>) -> impl IntoResponse {
    let pe_key = q
        .pe_key
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let refno = if let Some(raw) = q.refno.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        match parse_refno(raw) {
            Ok(r) => r,
            Err(msg) => return err_json(StatusCode::BAD_REQUEST, "BadRequest", msg),
        }
    } else if pe_key.is_some() {
        // 夹具路径：合成 0 号 refno，真实键靠 pe_key_override
        RefnoEnum::from(RefU64(0))
    } else {
        return err_json(
            StatusCode::BAD_REQUEST,
            "BadRequest",
            "refno or pe_key is required",
        );
    };

    match snapshot_at(refno, q.sesno, Some(q.dbnum), pe_key.as_deref()).await {
        Ok(snap) => ok_json(snap),
        Err(e) => history_err_response(e),
    }
}
