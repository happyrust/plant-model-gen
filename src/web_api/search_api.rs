use axum::{Router, extract::State, http::StatusCode, response::Json, routing::post};
use meilisearch_sdk::search::Selectors;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Meilisearch + SurrealDB(兜底) 的检索 API
///
/// 目标：给前端提供“按 noun + (name/refno) 关键字”的可分页查询，
/// 并返回可用于分组的字段（site=dbnum，noun）。
#[derive(Clone)]
pub struct SearchApiState {
    meili: Option<MeiliState>,
}

#[derive(Clone)]
struct MeiliState {
    client: meilisearch_sdk::client::Client,
    index: String,
}

impl SearchApiState {
    pub fn from_env() -> Self {
        // 先读环境变量，若未配置则回落到 DbOption.toml（由 DB_OPTION_FILE 指定）
        let opt = aios_core::get_db_option();

        let url = std::env::var("MEILI_URL")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .or_else(|| {
                opt.meili_url
                    .as_ref()
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
            });

        let key = std::env::var("MEILI_API_KEY")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .or_else(|| {
                opt.meili_api_key
                    .as_ref()
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
            });

        let index = std::env::var("MEILI_PDMS_INDEX")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .or_else(|| {
                opt.meili_pdms_index
                    .as_ref()
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
            })
            .unwrap_or_else(|| "pdms_nodes".to_string());

        let meili = match url {
            Some(u) => match meilisearch_sdk::client::Client::new(u, key) {
                Ok(client) => Some(MeiliState { client, index }),
                Err(_) => None,
            },
            None => None,
        };

        Self { meili }
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
    State(state): State<SearchApiState>,
    Json(req): Json<PdmsSearchRequest>,
) -> Result<Json<PdmsSearchResponse>, StatusCode> {
    let offset = req.offset.unwrap_or(0);
    let limit = req.limit.unwrap_or(200).clamp(0, 2000);
    let keyword = req.keyword.unwrap_or_default();
    let keyword = keyword.trim().to_string();
    let need_facets = req.facets.unwrap_or(false);

    if let Some(meili) = state.meili.as_ref() {
        let q = if keyword.is_empty() {
            None
        } else {
            Some(keyword.as_str())
        };

        let mut filters: Vec<String> = Vec::new();
        if let Some(nouns) = req.nouns.as_ref() {
            let nouns = nouns
                .iter()
                .map(|s| s.trim().to_uppercase())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>();
            if !nouns.is_empty() {
                let list = nouns
                    .iter()
                    .map(|n| format!("\"{}\"", n.replace('\"', "")))
                    .collect::<Vec<_>>()
                    .join(", ");
                filters.push(format!("noun IN [{}]", list));
            }
        }
        if let Some(site) = req.site.as_ref() {
            let site = site.trim();
            if !site.is_empty() {
                filters.push(format!("site = \"{}\"", site.replace('\"', "")));
            }
        }
        let filter_expr = if filters.is_empty() {
            None
        } else {
            Some(filters.join(" AND "))
        };

        // meilisearch-sdk 的 builder API：若编译报错，再按实际签名微调
        let index = meili.client.index(meili.index.as_str());
        let mut search = index.search();
        if let Some(q) = q {
            search.with_query(q);
        }
        search.with_offset(offset).with_limit(limit);
        if let Some(expr) = filter_expr.as_deref() {
            search.with_filter(expr);
        }
        if need_facets {
            // 前端按需分组：noun + site
            search.with_facets(Selectors::Some(&["noun", "site"]));
        }

        let result = match search.execute::<PdmsSearchItem>().await {
            Ok(v) => v,
            Err(e) => {
                return Ok(Json(PdmsSearchResponse {
                    success: false,
                    items: vec![],
                    total: 0,
                    offset,
                    limit,
                    facet_distribution: None,
                    error_message: Some(format!("meilisearch query failed: {e}")),
                }));
            }
        };

        return Ok(Json(PdmsSearchResponse {
            success: true,
            items: result.hits.into_iter().map(|h| h.result).collect(),
            total: result.estimated_total_hits.unwrap_or(0) as usize,
            offset,
            limit,
            facet_distribution: result.facet_distribution,
            error_message: None,
        }));
    }

    // 兜底：不用 Meilisearch，退回 noun_hierarchy（不保证分页/全量，仅保证功能可用）
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
                    site: None,
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
