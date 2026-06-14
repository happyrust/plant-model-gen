use aios_core::{RefU64, RefnoEnum};
use anyhow::{Context, anyhow};
use axum::{Router, extract::Path, http::StatusCode, response::Json, routing::get};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

pub fn create_pdms_attr_routes() -> Router {
    Router::new().route("/api/pdms/ui-attr/{refno}", get(get_ui_attr))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UiAttrResponse {
    pub success: bool,
    pub refno: String,
    pub attrs: serde_json::Value,
    /// 构件完整路径名称（层级路径，如 /SITE/ZONE/EQUI-001）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_name: Option<String>,
    pub error_message: Option<String>,
}

async fn get_ui_attr(Path(refno): Path<String>) -> Result<Json<UiAttrResponse>, StatusCode> {
    let refno_str = refno.trim().to_string();
    let refno = match RefU64::from_str(&refno_str).map(RefnoEnum::from) {
        Ok(refno) => refno,
        Err(_) => {
            return Ok(Json(UiAttrResponse {
                success: false,
                refno: refno_str.clone(),
                attrs: serde_json::Value::Object(serde_json::Map::new()),
                full_name: None,
                error_message: Some(format!("invalid refno path: {refno_str}")),
            }));
        }
    };

    match aios_core::get_ui_named_attmap(refno).await {
        Ok(attmap) => {
            let mut map = serde_json::Map::new();
            for (k, v) in attmap.map.into_iter() {
                map.insert(k, v.into());
            }
            let full_name = aios_core::get_default_full_name(refno).await.ok();
            Ok(Json(UiAttrResponse {
                success: true,
                refno: refno_str,
                attrs: serde_json::Value::Object(map),
                full_name,
                error_message: None,
            }))
        }
        Err(e) => {
            let primary_error = e.to_string();
            match query_full_named_attrs_without_uda(refno, &primary_error).await {
                Ok((attrs, full_name)) => Ok(Json(UiAttrResponse {
                    success: true,
                    refno: refno_str,
                    attrs,
                    full_name,
                    error_message: None,
                })),
                Err(full_fallback_error) => match query_rvm_relation_attrs(refno).await {
                    Ok((attrs, full_name)) => Ok(Json(UiAttrResponse {
                        success: true,
                        refno: refno_str,
                        attrs,
                        full_name,
                        error_message: Some(format!(
                            "using rvm relation-store fallback: get_ui_named_attmap failed: {e}; full named attribute fallback failed: {full_fallback_error}"
                        )),
                    })),
                    Err(rvm_fallback_error) => {
                        match query_basic_pe_attrs(refno, &primary_error).await {
                            Ok((attrs, full_name)) => Ok(Json(UiAttrResponse {
                                success: true,
                                refno: refno_str,
                                attrs,
                                full_name,
                                error_message: Some(format!(
                                    "using basic pe fallback: get_ui_named_attmap failed: {e}; full named attribute fallback failed: {full_fallback_error}; rvm relation-store fallback failed: {rvm_fallback_error}"
                                )),
                            })),
                            Err(pe_fallback_error) => Ok(Json(UiAttrResponse {
                                success: false,
                                refno: refno_str,
                                attrs: serde_json::Value::Object(serde_json::Map::new()),
                                full_name: None,
                                error_message: Some(format!(
                                    "get_ui_named_attmap failed: {e}; full named attribute fallback failed: {full_fallback_error}; rvm relation-store fallback failed: {rvm_fallback_error}; basic pe fallback failed: {pe_fallback_error}"
                                )),
                            })),
                        }
                    }
                },
            }
        }
    }
}

async fn query_full_named_attrs_without_uda(
    refno: RefnoEnum,
    primary_error: &str,
) -> anyhow::Result<(serde_json::Value, Option<String>)> {
    let mut attmap = aios_core::get_named_attmap(refno)
        .await
        .context("query named attribute record")?;
    attmap.fill_explicit_default_values();

    if attmap.map.is_empty() {
        return query_raw_attr_record_attrs(refno, primary_error).await;
    }

    let full_name = aios_core::get_default_full_name(refno)
        .await
        .ok()
        .and_then(non_empty_string);
    let mut map = serde_json::Map::new();
    for (k, v) in attmap.map.into_iter() {
        map.insert(k, v.into());
    }
    if let Some(name) = full_name.as_ref() {
        map.entry("NAME".to_string())
            .or_insert_with(|| serde_json::json!(name));
    }
    Ok((serde_json::Value::Object(map), full_name))
}

#[cfg(feature = "web_server")]
async fn query_raw_attr_record_attrs(
    refno: RefnoEnum,
    _primary_error: &str,
) -> anyhow::Result<(serde_json::Value, Option<String>)> {
    let db = crate::web_api::review_db::fresh_review_db()
        .await
        .context("open fresh surreal connection")?;
    let sql = format!(
        r#"
        SELECT fn::default_full_name(REFNO) AS NAME, *
        FROM ONLY {}.refno
        LIMIT 1
        "#,
        refno.to_pe_key()
    );
    let mut response = db.query(sql).await.context("query raw attribute record")?;
    let row_opt: Option<serde_json::Value> = response
        .take(0)
        .context("read raw attribute record result")?;
    let mut row = row_opt.ok_or_else(|| anyhow!("attribute record not found: {refno}"))?;
    let full_name = row
        .get("NAME")
        .and_then(|value| value.as_str())
        .and_then(non_empty_string);
    row.as_object_mut()
        .ok_or_else(|| anyhow!("raw attribute record did not return an object"))?;
    Ok((row, full_name))
}

#[cfg(not(feature = "web_server"))]
async fn query_raw_attr_record_attrs(
    _refno: RefnoEnum,
    _primary_error: &str,
) -> anyhow::Result<(serde_json::Value, Option<String>)> {
    Err(anyhow!(
        "raw attribute record fallback requires web_server feature"
    ))
}

fn non_empty_string(value: impl AsRef<str>) -> Option<String> {
    let value = value.as_ref().trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

async fn query_rvm_relation_attrs(
    refno: RefnoEnum,
) -> anyhow::Result<(serde_json::Value, Option<String>)> {
    let dbnum = (refno.refno().0 >> 32) as u32;
    if dbnum == 0 {
        return Err(anyhow!("refno does not encode dbnum: {refno}"));
    }

    let attrs = crate::model_relation_store::global_store()
        .query_attrs_by_refno(dbnum, refno)?
        .ok_or_else(|| anyhow!("relation-store attributes not found: {refno}"))?;
    let full_name = attrs
        .get("NAME")
        .and_then(|value| value.as_str())
        .and_then(non_empty_string);
    Ok((attrs, full_name))
}

#[cfg(feature = "web_server")]
async fn query_basic_pe_attrs(
    refno: RefnoEnum,
    primary_error: &str,
) -> anyhow::Result<(serde_json::Value, Option<String>)> {
    let db = crate::web_api::review_db::fresh_review_db()
        .await
        .context("open fresh surreal connection")?;
    let sql = format!(
        r#"
        SELECT
            record::id(id) AS refno,
            record::id(owner) AS owner,
            noun,
            name,
            dbnum,
            sesno,
            status_code,
            array::len(children ?? []) AS children_count,
            deleted,
            lock
        FROM {}
        LIMIT 1
        "#,
        refno.to_pe_key()
    );
    let mut response = db.query(sql).await.context("query pe record")?;
    let mut rows: Vec<serde_json::Value> = response.take(0).context("read pe query result")?;
    let mut row = rows
        .pop()
        .ok_or_else(|| anyhow!("pe record not found: {refno}"))?;
    let full_name = row
        .get("name")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.to_string());

    let map = row
        .as_object_mut()
        .ok_or_else(|| anyhow!("pe query did not return an object"))?;
    map.insert("_source".to_string(), serde_json::json!("pe_fallback"));
    map.insert(
        "_fallback_reason".to_string(),
        serde_json::json!(format!("get_ui_named_attmap failed: {primary_error}")),
    );

    Ok((row, full_name))
}

#[cfg(not(feature = "web_server"))]
async fn query_basic_pe_attrs(
    _refno: RefnoEnum,
    _primary_error: &str,
) -> anyhow::Result<(serde_json::Value, Option<String>)> {
    Err(anyhow!("basic pe fallback requires web_server feature"))
}
