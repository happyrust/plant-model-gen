use aios_core::RefnoEnum;
use axum::{Router, extract::Path, http::StatusCode, response::Json, routing::get};
use serde::{Deserialize, Serialize};

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

async fn get_ui_attr(Path(refno): Path<RefnoEnum>) -> Result<Json<UiAttrResponse>, StatusCode> {
    let refno_str = refno.to_string();

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
        Err(e) => Ok(Json(UiAttrResponse {
            success: false,
            refno: refno_str,
            attrs: serde_json::Value::Object(serde_json::Map::new()),
            full_name: None,
            error_message: Some(format!("get_ui_named_attmap failed: {e}")),
        })),
    }
}
