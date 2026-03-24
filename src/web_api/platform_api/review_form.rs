//! Review form (review_forms table) lifecycle management.
//!
//! SurrealQL 风格对齐 `.cursor/skills/plant-surrealdb` 中的通用约定：
//! - 只取标量列表时优先 `SELECT VALUE`；
//! - 逻辑删除用显式 `(deleted IS NONE OR deleted = false)`（旧行无 `deleted` 视为未删）；
//! - 能用单次 `UPDATE … WHERE` 完成的不要先 `SELECT` 再写（减少往返）。

use aios_core::project_primary_db;
use surrealdb::types::{self as surrealdb_types, SurrealValue};

use crate::web_api::review_api::{ReviewAttachment, ReviewComponent, ReviewTask, WorkflowStep};
use super::types::{
    ReviewForm, ReviewFormRow, derive_review_form_status_from_task_status, review_form_from_row,
};

/// `review_tasks` 未软删过滤片段（可选 `bool` 字段：勿单独用 `deleted = false` 排除「字段缺失」旧数据）
pub const REVIEW_TASK_ACTIVE_SQL: &str = "(deleted IS NONE OR deleted = false)";

// ============================================================================
// Schema
// ============================================================================

async fn ensure_review_forms_schema() -> anyhow::Result<()> {
    project_primary_db()
        .query(
            r#"
            DEFINE TABLE IF NOT EXISTS review_forms SCHEMAFULL;
            DEFINE FIELD IF NOT EXISTS form_id ON TABLE review_forms TYPE string;
            DEFINE FIELD IF NOT EXISTS project_id ON TABLE review_forms TYPE string;
            DEFINE FIELD IF NOT EXISTS user_id ON TABLE review_forms TYPE string;
            DEFINE FIELD IF NOT EXISTS role ON TABLE review_forms TYPE none | string;
            DEFINE FIELD IF NOT EXISTS requester_id ON TABLE review_forms TYPE string;
            DEFINE FIELD IF NOT EXISTS source ON TABLE review_forms TYPE string;
            DEFINE FIELD IF NOT EXISTS status ON TABLE review_forms TYPE string;
            DEFINE FIELD IF NOT EXISTS task_created ON TABLE review_forms TYPE bool DEFAULT false;
            DEFINE FIELD IF NOT EXISTS deleted ON TABLE review_forms TYPE bool DEFAULT false;
            DEFINE FIELD IF NOT EXISTS created_at ON TABLE review_forms TYPE datetime;
            DEFINE FIELD IF NOT EXISTS updated_at ON TABLE review_forms TYPE datetime DEFAULT time::now();
            DEFINE FIELD IF NOT EXISTS deleted_at ON TABLE review_forms TYPE option<datetime>;
            DEFINE INDEX IF NOT EXISTS idx_form_id ON TABLE review_forms FIELDS form_id UNIQUE;
            "#,
        )
        .await?;
    Ok(())
}

// ============================================================================
// CRUD
// ============================================================================

pub async fn get_review_form_by_form_id(form_id: &str) -> anyhow::Result<Option<ReviewForm>> {
    ensure_review_forms_schema().await?;

    let mut response = project_primary_db()
        .query(
            r#"
            SELECT * FROM review_forms
            WHERE form_id = $form_id
            ORDER BY updated_at DESC, created_at DESC
            LIMIT 1
            "#,
        )
        .bind(("form_id", form_id.to_string()))
        .await?;

    let rows: Vec<ReviewFormRow> = response.take(0)?;
    Ok(rows.into_iter().next().map(review_form_from_row))
}

pub async fn ensure_review_form_stub(
    form_id: &str,
    project_id: &str,
    requester_id: &str,
    source: &str,
) -> anyhow::Result<ReviewForm> {
    ensure_review_forms_schema().await?;

    if let Some(existing) = get_review_form_by_form_id(form_id).await? {
        if existing.status == "deleted" {
            anyhow::bail!("form_id={} 对应主单据已删除，禁止重新打开", form_id);
        }

        project_primary_db()
            .query(
                r#"
                UPDATE review_forms
                SET
                    project_id = IF string::len(string::trim($project_id)) > 0 THEN $project_id ELSE project_id END,
                    user_id = IF string::len(string::trim($requester_id)) > 0 THEN $requester_id ELSE user_id END,
                    requester_id = IF string::len(string::trim($requester_id)) > 0 THEN $requester_id ELSE requester_id END,
                    source = $source,
                    deleted = false,
                    updated_at = time::now()
                WHERE form_id = $form_id
                "#,
            )
            .bind(("form_id", form_id.to_string()))
            .bind(("project_id", project_id.trim().to_string()))
            .bind(("requester_id", requester_id.trim().to_string()))
            .bind(("source", source.trim().to_string()))
            .await?;

        return get_review_form_by_form_id(form_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("form_id={} 主单据更新后读取失败", form_id));
    }

    project_primary_db()
        .query(
            r#"
            CREATE review_forms CONTENT {
                form_id: $form_id,
                project_id: $project_id,
                user_id: $requester_id,
                requester_id: $requester_id,
                role: NONE,
                source: $source,
                status: 'blank',
                task_created: false,
                deleted: false,
                created_at: time::now(),
                updated_at: time::now(),
                deleted_at: NONE
            }
            "#,
        )
        .bind(("form_id", form_id.to_string()))
        .bind(("project_id", project_id.trim().to_string()))
        .bind(("requester_id", requester_id.trim().to_string()))
        .bind(("source", source.trim().to_string()))
        .await?;

    get_review_form_by_form_id(form_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("form_id={} 主单据创建后读取失败", form_id))
}

pub async fn sync_review_form_with_task_status(
    form_id: &str,
    project_id: Option<&str>,
    requester_id: Option<&str>,
    source: &str,
    task_status: &str,
) -> anyhow::Result<()> {
    let _ = ensure_review_form_stub(
        form_id,
        project_id.unwrap_or_default(),
        requester_id.unwrap_or_default(),
        source,
    )
    .await?;

    let form_status = derive_review_form_status_from_task_status(task_status);
    project_primary_db()
        .query(
            r#"
            UPDATE review_forms
            SET
                project_id = IF string::len(string::trim($project_id)) > 0 THEN $project_id ELSE project_id END,
                user_id = IF string::len(string::trim($requester_id)) > 0 THEN $requester_id ELSE user_id END,
                requester_id = IF string::len(string::trim($requester_id)) > 0 THEN $requester_id ELSE requester_id END,
                source = $source,
                task_created = true,
                status = $status,
                deleted = false,
                deleted_at = NONE,
                updated_at = time::now()
            WHERE form_id = $form_id
            "#,
        )
        .bind(("form_id", form_id.to_string()))
        .bind(("project_id", project_id.unwrap_or_default().trim().to_string()))
        .bind(("requester_id", requester_id.unwrap_or_default().trim().to_string()))
        .bind(("source", source.trim().to_string()))
        .bind(("status", form_status))
        .await?;

    Ok(())
}

pub async fn mark_review_form_deleted(form_id: &str) -> anyhow::Result<()> {
    ensure_review_forms_schema().await?;

    project_primary_db()
        .query(
            r#"
            UPDATE review_forms
            SET
                status = 'deleted',
                task_created = false,
                deleted = true,
                deleted_at = time::now(),
                updated_at = time::now()
            WHERE form_id = $form_id
            "#,
        )
        .bind(("form_id", form_id.to_string()))
        .await?;

    Ok(())
}

/// PMS 入站删除：主单软删 + 该 `form_id` 下任务软删；不物理删除子表与附件文件。
pub async fn soft_delete_review_bundle(form_id: &str) -> anyhow::Result<()> {
    ensure_review_forms_schema().await?;

    project_primary_db()
        .query(
            r#"
            UPDATE review_tasks SET
                deleted = true,
                deleted_at = time::now(),
                updated_at = time::now(),
                status = 'deleted'
            WHERE form_id = $form_id
            "#,
        )
        .bind(("form_id", form_id.to_string()))
        .await?;

    mark_review_form_deleted(form_id).await?;
    Ok(())
}

// ============================================================================
// Task lookup by form_id (used by embed_url & workflow_sync)
// ============================================================================

pub async fn find_task_by_form_id(form_id: &str) -> anyhow::Result<Option<ReviewTask>> {
    #[derive(Debug, serde::Deserialize, SurrealValue)]
    struct TaskRow {
        id: surrealdb_types::RecordId,
        form_id: Option<String>,
        title: Option<String>,
        description: Option<String>,
        model_name: Option<String>,
        status: Option<String>,
        priority: Option<String>,
        requester_id: Option<String>,
        requester_name: Option<String>,
        checker_id: Option<String>,
        checker_name: Option<String>,
        approver_id: Option<String>,
        approver_name: Option<String>,
        reviewer_id: Option<String>,
        reviewer_name: Option<String>,
        components: Option<Vec<ReviewComponent>>,
        attachments: Option<Vec<ReviewAttachment>>,
        review_comment: Option<String>,
        created_at: Option<surrealdb::types::Datetime>,
        updated_at: Option<surrealdb::types::Datetime>,
        due_date: Option<surrealdb::types::Datetime>,
        current_node: Option<String>,
        workflow_history: Option<Vec<WorkflowStep>>,
        return_reason: Option<String>,
    }

    fn to_millis(value: Option<surrealdb::types::Datetime>) -> Option<i64> {
        value.map(|dt| dt.timestamp_millis())
    }

    let mut response = project_primary_db()
        .query(&format!(
            r#"
            SELECT * FROM review_tasks
            WHERE form_id = $form_id
              AND {}
            ORDER BY updated_at DESC, created_at DESC
            LIMIT 1
            "#,
            REVIEW_TASK_ACTIVE_SQL
        ))
        .bind(("form_id", form_id.to_string()))
        .await?;

    let rows: Vec<TaskRow> = response.take(0)?;
    Ok(rows.into_iter().next().map(|row| {
        let id = match row.id.key {
            surrealdb::types::RecordIdKey::String(value) => value,
            other => format!("{:?}", other),
        };
        let checker_id = row
            .checker_id
            .clone()
            .filter(|v| !v.is_empty())
            .or_else(|| row.reviewer_id.clone())
            .unwrap_or_default();
        let checker_name = row
            .checker_name
            .clone()
            .filter(|v| !v.is_empty())
            .or_else(|| row.reviewer_name.clone())
            .unwrap_or_default();

        ReviewTask {
            id,
            form_id: row.form_id.unwrap_or_default(),
            title: row.title.unwrap_or_default(),
            description: row.description.unwrap_or_default(),
            model_name: row.model_name.unwrap_or_default(),
            status: row.status.unwrap_or_else(|| "draft".to_string()),
            priority: row.priority.unwrap_or_else(|| "medium".to_string()),
            requester_id: row.requester_id.unwrap_or_default(),
            requester_name: row.requester_name.unwrap_or_default(),
            checker_id: checker_id.clone(),
            checker_name: checker_name.clone(),
            approver_id: row.approver_id.unwrap_or_default(),
            approver_name: row.approver_name.unwrap_or_default(),
            reviewer_id: row.reviewer_id.unwrap_or_else(|| checker_id),
            reviewer_name: row.reviewer_name.unwrap_or_else(|| checker_name),
            components: row.components.unwrap_or_default(),
            attachments: row.attachments,
            review_comment: row.review_comment,
            created_at: to_millis(row.created_at).unwrap_or_default(),
            updated_at: to_millis(row.updated_at).unwrap_or_default(),
            due_date: to_millis(row.due_date),
            current_node: row.current_node.unwrap_or_else(|| "sj".to_string()),
            workflow_history: row.workflow_history.unwrap_or_default(),
            return_reason: row.return_reason,
        }
    }))
}
