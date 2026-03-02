//! PDMS/SurrealDB 模型查询辅助 API
//!
//! 目的：给前端提供“BRAN/HANG 规则”所需的最小查询能力，避免前端直连 SurrealDB（WS 版本不匹配/跨域等）。
//!
//! 当前提供：
//! - `/api/pdms/type-info?refno=...`：返回 noun / owner_noun
//! - `/api/pdms/children?refno=...`：返回 pe->owns 的子节点（按 order_index）

use aios_core::{RefnoEnum, project_primary_db, SurrealQueryExt};
use axum::{
    Router,
    extract::Query,
    http::{HeaderValue, header::CONTENT_TYPE, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json,
};
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;

pub fn create_pdms_model_query_routes() -> Router {
    Router::new()
        .route("/api/pdms/type-info", get(get_type_info))
        .route("/api/pdms/children", get(get_children))
}

fn json_utf8<T: Serialize>(value: T) -> Response {
    let mut res = Json(value).into_response();
    res.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    res
}

#[derive(Debug, Deserialize)]
pub struct RefnoQuery {
    pub refno: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TypeInfoResponse {
    pub success: bool,
    pub refno: String,
    pub noun: Option<String>,
    pub owner_refno: Option<String>,
    pub owner_noun: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChildrenResponse {
    pub success: bool,
    pub refno: String,
    pub children: Vec<String>,
    pub error_message: Option<String>,
}

fn normalize_refno_key_like(raw: &str) -> String {
    let s = raw.trim();
    if s.is_empty() {
        return String::new();
    }

    // 去掉 Surreal record 包装：pe:⟨...⟩ / pe:<...> / ⟨...⟩ / <...>
    let mut core = s;
    if let Some(i) = core.find('⟨').zip(core.find('⟩')).map(|(l, r)| (l, r)) {
        if i.0 + 1 < i.1 {
            core = &core[(i.0 + 1)..i.1];
        }
    } else if let Some(i) = core.find('<').zip(core.find('>')).map(|(l, r)| (l, r)) {
        if i.0 + 1 < i.1 {
            core = &core[(i.0 + 1)..i.1];
        }
    }

    core = core.strip_prefix("pe:").unwrap_or(core);
    core = core.strip_prefix("PE:").unwrap_or(core);
    core = core.strip_prefix('=').unwrap_or(core);

    core.replace(['/', ','], "_")
}

fn extract_refno_key(record_id: &str) -> String {
    let s = record_id.trim();
    if let Some((l, r)) = s.find('⟨').zip(s.find('⟩')) {
        if l + 1 < r {
            return s[(l + 1)..r].to_string();
        }
    }
    if let Some((l, r)) = s.find('<').zip(s.find('>')) {
        if l + 1 < r {
            return s[(l + 1)..r].to_string();
        }
    }
    if let Some(idx) = s.find(':') {
        return s[(idx + 1)..].to_string();
    }
    s.to_string()
}

async fn get_type_info(Query(query): Query<RefnoQuery>) -> Result<Response, StatusCode> {
    let refno_key = normalize_refno_key_like(&query.refno);
    if refno_key.is_empty() {
        return Ok(json_utf8(TypeInfoResponse {
            success: false,
            refno: "".to_string(),
            noun: None,
            owner_refno: None,
            owner_noun: None,
            error_message: Some("缺少 refno".to_string()),
        }));
    }

    let refno_enum = RefnoEnum::from(refno_key.as_str());
    if !refno_enum.is_valid() {
        return Ok(json_utf8(TypeInfoResponse {
            success: false,
            refno: refno_key,
            noun: None,
            owner_refno: None,
            owner_noun: None,
            error_message: Some("无效 refno".to_string()),
        }));
    }

    let pe = match aios_core::get_pe(refno_enum).await {
        Ok(Some(pe)) => pe,
        Ok(None) => {
            return Ok(json_utf8(TypeInfoResponse {
                success: false,
                refno: refno_key,
                noun: None,
                owner_refno: None,
                owner_noun: None,
                error_message: Some("pe not found".to_string()),
            }))
        }
        Err(e) => {
            return Ok(json_utf8(TypeInfoResponse {
                success: false,
                refno: refno_key,
                noun: None,
                owner_refno: None,
                owner_noun: None,
                error_message: Some(format!("db error: {e}")),
            }))
        }
    };

    let noun = pe.noun.trim().to_string();
    let owner_refno = pe.owner;
    let owner_noun = if owner_refno != RefnoEnum::default() {
        match aios_core::get_pe(owner_refno).await {
            Ok(Some(owner_pe)) => Some(owner_pe.noun.trim().to_string()),
            _ => None,
        }
    } else {
        None
    };

    Ok(json_utf8(TypeInfoResponse {
        success: true,
        refno: normalize_refno_key_like(&pe.refno.to_string()),
        noun: if noun.is_empty() { None } else { Some(noun) },
        owner_refno: if owner_refno != RefnoEnum::default() {
            Some(normalize_refno_key_like(&owner_refno.to_string()))
        } else {
            None
        },
        owner_noun,
        error_message: None,
    }))
}

#[derive(Debug, Deserialize, SurrealValue)]
struct ChildRow {
    pub child: String,
}

async fn get_children(Query(query): Query<RefnoQuery>) -> Result<Response, StatusCode> {
    let refno_key = normalize_refno_key_like(&query.refno);
    if refno_key.is_empty() {
        return Ok(json_utf8(ChildrenResponse {
            success: false,
            refno: "".to_string(),
            children: vec![],
            error_message: Some("缺少 refno".to_string()),
        }));
    }

    // BRAN/HANG：children 不是 pe->owns，而是按 tubi_relate 顺序返回“管段 refno”（leave_refno）
    let refno_enum = RefnoEnum::from(refno_key.as_str());
    if refno_enum.is_valid() {
        if let Ok(Some(pe)) = aios_core::get_pe(refno_enum.clone()).await {
            let noun = pe.noun.trim().to_uppercase();
            if noun == "BRAN" || noun == "HANG" {
                // 复用 mbd_pipe_api 的 SurrealQL 口径：tubi_relate 的 in 即 leave_refno，id[1] 为顺序
                aios_core::init_surreal()
                    .await
                    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

                #[derive(Debug, Deserialize, SurrealValue)]
                struct TubiRow {
                    pub leave_refno: RefnoEnum,
                    pub index: Option<i64>,
                }

                let pe_key = refno_enum.to_pe_key();
                let sql = format!(
                    r#"
                    SELECT
                        in as leave_refno,
                        id[1] as index
                    FROM tubi_relate:[{pe_key}, 0]..[{pe_key}, ..];
                    "#
                );

                let mut rows: Vec<TubiRow> = match project_primary_db().query_take(&sql, 0).await {
                    Ok(v) => v,
                    Err(e) => {
                        return Ok(json_utf8(ChildrenResponse {
                            success: false,
                            refno: refno_key,
                            children: vec![],
                            error_message: Some(format!("db error: {e}")),
                        }))
                    }
                };

                rows.sort_by_key(|r| r.index.unwrap_or(i64::MAX));

                let mut children: Vec<String> = Vec::with_capacity(rows.len());
                let mut seen = std::collections::HashSet::<String>::new();
                for row in rows {
                    let k = normalize_refno_key_like(&row.leave_refno.to_string());
                    if k.is_empty() || seen.contains(&k) {
                        continue;
                    }
                    seen.insert(k.clone());
                    children.push(k);
                }

                return Ok(json_utf8(ChildrenResponse {
                    success: true,
                    refno: refno_key,
                    children,
                    error_message: None,
                }));
            }
        }
    }

    // 说明：children 顺序按 order_index；与前端 useSurrealModelQuery.queryChildren 保持一致。
    // 兼容部分 SurrealDB 版本的解析器：ORDER BY 的字段需要出现在 SELECT 列表里
    let sql = format!(
        "SELECT record::id(out) as child, order_index FROM pe:⟨{refno_key}⟩->owns ORDER BY order_index"
    );

    let rows: Vec<ChildRow> = match project_primary_db().query_take(&sql, 0).await {
        Ok(v) => v,
        Err(e) => {
            return Ok(json_utf8(ChildrenResponse {
                success: false,
                refno: refno_key,
                children: vec![],
                error_message: Some(format!("db error: {e}")),
            }))
        }
    };

    let mut children: Vec<String> = Vec::with_capacity(rows.len());
    for row in rows {
        let key = normalize_refno_key_like(&extract_refno_key(&row.child));
        if !key.is_empty() {
            children.push(key);
        }
    }

    Ok(json_utf8(ChildrenResponse {
        success: true,
        refno: refno_key,
        children,
        error_message: None,
    }))
}
