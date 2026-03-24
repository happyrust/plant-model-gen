//! Outbound fire-and-forget notifications to external systems (PMS).

use tracing::{info, warn};

use super::auth::sha256_hex;
use super::config::EXTERNAL_REVIEW_CONFIG;

/// 异步通知外部系统删除校审数据（fire-and-forget）
pub fn notify_workflow_delete_async(task_id: String, operator_id: String) {
    if EXTERNAL_REVIEW_CONFIG.is_mock() {
        info!(
            "[WORKFLOW_DELETE] Mock模式跳过 - task_id={}, operator_id={}",
            task_id, operator_id
        );
        return;
    }

    let spawn_task_id = task_id.clone();
    let spawn_operator_id = operator_id.clone();

    tokio::spawn(async move {
        let result = notify_workflow_delete(&spawn_task_id, &spawn_operator_id).await;
        match result {
            Ok(_) => info!(
                "[WORKFLOW_DELETE] 删除通知成功 - task_id={}",
                spawn_task_id
            ),
            Err(e) => warn!(
                "[WORKFLOW_DELETE] 删除通知失败 - task_id={}, error={}",
                spawn_task_id, e
            ),
        }
    });
}

async fn notify_workflow_delete(task_id: &str, operator_id: &str) -> anyhow::Result<()> {
    let config = &*EXTERNAL_REVIEW_CONFIG;
    let url = format!(
        "{}{}",
        config.base_url.trim_end_matches('/'),
        config.workflow_delete_path
    );

    let token = sha256_hex(&format!(
        "{}:{}:{}",
        config.auth_secret, task_id, operator_id
    ));

    let body = serde_json::json!({
        "task_id": task_id,
        "operator_id": operator_id,
        "token": token,
    });

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(config.timeout_seconds))
        .build()?;

    let resp = client.post(&url).json(&body).send().await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("外部系统返回错误 {}: {}", status, text);
    }

    Ok(())
}
