// 添加到 review_api.rs 的调试接口
// 用于检查 review_form_model 表的数据

/// GET /api/review/tasks/debug/form-model/:form_id - 调试构件关联数据
pub async fn debug_form_model(Path(form_id): Path<String>) -> impl IntoResponse {
    info!("Debugging form model data for form_id: {}", form_id);

    // 检查 review_tasks 表中的数据
    let task_sql = "SELECT id, title, components FROM review_tasks WHERE form_id = $form_id AND (deleted IS NONE OR deleted = false) LIMIT 1";
    
    let task_data = match project_primary_db()
        .query(task_sql)
        .bind(("form_id", form_id.clone()))
        .await
    {
        Ok(mut response) => {
            #[derive(Debug, Serialize, Deserialize, SurrealValue)]
            struct TaskDebugRow {
                id: Option<String>,
                title: Option<String>,
                components: Option<Vec<ReviewComponent>>,
            }
            
            let rows: Vec<TaskDebugRow> = response.take(0).unwrap_or_default();
            rows.into_iter().next()
        }
        Err(e) => {
            warn!("Failed to query task: {}", e);
            None
        }
    };

    // 检查 review_form_model 表中的数据
    let model_sql = "SELECT model_refno, created_at FROM review_form_model WHERE form_id = $form_id";
    
    let model_data = match project_primary_db()
        .query(model_sql)
        .bind(("form_id", form_id.clone()))
        .await
    {
        Ok(mut response) => {
            #[derive(Debug, Serialize, Deserialize, SurrealValue)]
            struct ModelDebugRow {
                model_refno: Option<String>,
                created_at: Option<surrealdb::types::Datetime>,
            }
            
            let rows: Vec<ModelDebugRow> = response.take(0).unwrap_or_default();
            rows
        }
        Err(e) => {
            warn!("Failed to query form model: {}", e);
            vec![]
        }
    };

    let debug_response = serde_json::json!({
        "form_id": form_id,
        "task_data": task_data,
        "model_associations": model_data,
        "model_count": model_data.len(),
        "timestamp": chrono::Utc::now().to_rfc3339()
    });

    (StatusCode::OK, Json(debug_response))
}
