use axum::{Router, extract::State, http::StatusCode, response::Json, routing::post};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 检索 API (兜底模式作为主模式)
///
/// 目标：根据 keyword 等过滤条件检索 noun_hierarchy
#[derive(Clone)]
pub struct SearchApiState {}

impl SearchApiState {
    pub fn from_env() -> Self {
        Self {}
    }
}

pub fn create_search_routes(state: SearchApiState) -> Router {
    Router::new()
        .route("/api/search/pdms", post(search_pdms))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
pub struct PdmsSearchRequest {
    /// name/refno 关键字；为空时仅按 nouns/site 做过滤
    pub keyword: Option<String>,
    /// noun 过滤（如 ["PIPE","EQUI"]）
    pub nouns: Option<Vec<String>>,
    /// site=dbnum 过滤（如 "17496"）
    pub site: Option<String>,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
    /// 是否返回 facet_distribution（用于前端做分组计数）
    pub facets: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PdmsSearchItem {
    pub refno: String,
    pub name: String,
    pub noun: String,
    pub site: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PdmsSearchResponse {
    pub success: bool,
    pub items: Vec<PdmsSearchItem>,
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
    pub facet_distribution: Option<HashMap<String, HashMap<String, usize>>>,
    pub error_message: Option<String>,
}

async fn search_pdms(
    State(_state): State<SearchApiState>,
    Json(req): Json<PdmsSearchRequest>,
) -> Result<Json<PdmsSearchResponse>, StatusCode> {
    let offset = req.offset.unwrap_or(0);
    // 此处简化处理：将 limit 映射到底层最大数量限制
    let limit = req.limit.unwrap_or(200).clamp(0, 2000);
    let keyword = req.keyword.unwrap_or_default();
    let keyword = keyword.trim().to_string();

    let limit_usize = limit.max(1).min(2000);
    let mut out: Vec<PdmsSearchItem> = Vec::new();

    let keyword_opt = if keyword.is_empty() {
        None
    } else {
        Some(keyword.as_str())
    };

    if let Some(nouns) = req.nouns.as_ref() {
        for noun in nouns {
            if out.len() >= limit_usize {
                break;
            }
            let noun = noun.trim();
            if noun.is_empty() {
                continue;
            }
            let rows = match aios_core::query_noun_hierarchy(noun, keyword_opt, None).await {
                Ok(v) => v,
                Err(e) => {
                    return Ok(Json(PdmsSearchResponse {
                        success: false,
                        items: vec![],
                        total: 0,
                        offset,
                        limit,
                        facet_distribution: None,
                        error_message: Some(format!("query_noun_hierarchy failed: {e}")),
                    }));
                }
            };
            for row in rows {
                if out.len() >= limit_usize {
                    break;
                }
                out.push(PdmsSearchItem {
                    refno: row.id.to_string(),
                    name: row.name,
                    noun: row.noun,
                    site: None, // 兜底模式暂时没有跨站精确数据支持
                });
            }
        }
    }

    Ok(Json(PdmsSearchResponse {
        success: true,
        total: out.len(),
        items: out,
        offset,
        limit,
        facet_distribution: None,
        error_message: None,
    }))
}
