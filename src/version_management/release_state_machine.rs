use crate::version_management::ducklake_store::ModelVersionDuckLakeStore;
use crate::version_management::model_release::write_release_sidecar;
use crate::version_management::types::{
    ModelReleaseLifecycle, ModelReleaseQuality, ModelReleaseReconcileReport, ModelReleaseStatus,
    ModelReleaseStatusEvent, ModelVersionDuckLakeConfig,
};
use anyhow::Context;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelReleaseStateMachineAction {
    Review,
    PublishIfReady,
    FailIfUnusable,
}

impl ModelReleaseStateMachineAction {
    pub fn from_str(raw: &str) -> anyhow::Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "review" => Ok(Self::Review),
            "publish_if_ready" | "publish-if-ready" | "publish" => Ok(Self::PublishIfReady),
            "fail_if_unusable" | "fail-if-unusable" | "fail" => Ok(Self::FailIfUnusable),
            _ => anyhow::bail!(
                "invalid release state-machine action '{}'; expected review, publish_if_ready, or fail_if_unusable",
                raw
            ),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Review => "review",
            Self::PublishIfReady => "publish_if_ready",
            Self::FailIfUnusable => "fail_if_unusable",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ModelReleaseStateMachineRequest {
    pub ducklake: ModelVersionDuckLakeConfig,
    pub release_id: String,
    pub action: ModelReleaseStateMachineAction,
    pub reason: Option<String>,
    pub require_generation_job_id: bool,
    pub require_baseline_state: bool,
    pub require_asset_manifest: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelReleaseStateMachineReport {
    pub release_id: String,
    pub action: ModelReleaseStateMachineAction,
    pub previous_status: ModelReleaseStatus,
    pub previous_lifecycle: ModelReleaseLifecycle,
    pub current_status: ModelReleaseStatus,
    pub current_lifecycle: ModelReleaseLifecycle,
    pub transition_allowed: bool,
    pub applied: bool,
    pub action_taken: String,
    pub recommended_action: String,
    #[serde(default)]
    pub blockers: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
    pub reconcile: ModelReleaseReconcileReport,
    #[serde(default)]
    pub events: Vec<ModelReleaseStatusEvent>,
}

pub fn run_model_release_state_machine(
    request: ModelReleaseStateMachineRequest,
) -> anyhow::Result<ModelReleaseStateMachineReport> {
    if request.release_id.trim().is_empty() {
        anyhow::bail!("release_id is required");
    }

    let store = ModelVersionDuckLakeStore::open_writer(request.ducklake)?;
    let reconcile = store
        .reconcile_release(&request.release_id, false, false)
        .with_context(|| {
            format!(
                "review release '{}' before state transition",
                request.release_id
            )
        })?;
    let previous_status = reconcile.current_status.clone();
    let previous_lifecycle = reconcile.current_lifecycle.clone();
    let mut blockers = production_publish_blockers(
        &reconcile,
        request.require_generation_job_id,
        request.require_baseline_state,
        request.require_asset_manifest,
    );
    let mut warnings = reconcile.warnings.clone();

    let transition_allowed = match request.action {
        ModelReleaseStateMachineAction::Review => blockers.is_empty(),
        ModelReleaseStateMachineAction::PublishIfReady => blockers.is_empty(),
        ModelReleaseStateMachineAction::FailIfUnusable => !blockers.is_empty(),
    };

    let mut applied = false;
    let mut action_taken = "none".to_string();
    let mut recommended_action = recommended_action_for(
        &request.action,
        transition_allowed,
        &blockers,
        &reconcile.recommended_action,
    );

    match request.action {
        ModelReleaseStateMachineAction::Review => {}
        ModelReleaseStateMachineAction::PublishIfReady => {
            if transition_allowed {
                if previous_lifecycle == ModelReleaseLifecycle::Published {
                    action_taken = "already_published".to_string();
                    recommended_action =
                        "release is already published and production evidence is complete"
                            .to_string();
                } else {
                    store.update_release_status(
                        &request.release_id,
                        ModelReleaseStatus::Published,
                        request
                            .reason
                            .as_deref()
                            .or(Some("state machine production evidence passed")),
                    )?;
                    applied = true;
                    action_taken = "published".to_string();
                    recommended_action =
                        "release was marked published after state-machine gates passed".to_string();
                }
            }
        }
        ModelReleaseStateMachineAction::FailIfUnusable => {
            if transition_allowed {
                if previous_lifecycle == ModelReleaseLifecycle::Failed {
                    action_taken = "already_failed".to_string();
                    recommended_action =
                        "release is already failed and still has blocking evidence".to_string();
                } else {
                    let reason = request.reason.unwrap_or_else(|| {
                        format!(
                            "state machine marked release unusable: {}",
                            blockers.join("; ")
                        )
                    });
                    store.update_release_status(
                        &request.release_id,
                        ModelReleaseStatus::Failed,
                        Some(&reason),
                    )?;
                    applied = true;
                    action_taken = "failed".to_string();
                    recommended_action =
                        "release was marked failed because blocking evidence remains".to_string();
                }
            } else {
                warnings.push(
                    "fail_if_unusable was not applied because no blockers were found".to_string(),
                );
            }
        }
    }

    let events = store.release_events(&request.release_id)?.events;
    let current = store.get_release(&request.release_id)?;
    if applied {
        write_release_sidecar(&current)?;
    }
    if matches!(
        request.action,
        ModelReleaseStateMachineAction::PublishIfReady
    ) && !transition_allowed
        && blockers.is_empty()
    {
        blockers.push("publish_if_ready was not allowed by state-machine policy".to_string());
    }

    Ok(ModelReleaseStateMachineReport {
        release_id: request.release_id,
        action: request.action,
        previous_status,
        previous_lifecycle,
        current_status: current.release_status,
        current_lifecycle: current.release_lifecycle,
        transition_allowed,
        applied,
        action_taken,
        recommended_action,
        blockers,
        warnings,
        reconcile,
        events,
    })
}

fn production_publish_blockers(
    reconcile: &ModelReleaseReconcileReport,
    require_generation_job_id: bool,
    require_baseline_state: bool,
    require_asset_manifest: bool,
) -> Vec<String> {
    let release = &reconcile.release;
    let mut blockers = reconcile.problems.clone();

    if reconcile.current_lifecycle == ModelReleaseLifecycle::Failed {
        blockers.push("release lifecycle is failed; repair or register a new release".to_string());
    }
    if release.release_quality != ModelReleaseQuality::CompleteVisual {
        blockers.push(format!(
            "release quality is {}, expected complete_visual for production publication",
            release.release_quality.as_str()
        ));
    }
    if require_baseline_state
        && (release.baseline_state_manifest_path.is_none()
            || release.baseline_state_manifest_hash.is_none())
    {
        blockers.push(
            "baseline state manifest path/hash evidence is required for production publication"
                .to_string(),
        );
    }
    if require_generation_job_id && release.generation_job_id.is_none() {
        blockers
            .push("generation_job_id evidence is required for production publication".to_string());
    }
    let visual_rows = release
        .rows_by_table
        .get("geo_instances")
        .copied()
        .unwrap_or_default();
    if require_asset_manifest
        && visual_rows > 0
        && (release.asset_manifest_path.is_none() || release.asset_manifest_hash.is_none())
    {
        blockers.push(
            "release-local mesh asset manifest path/hash evidence is required for visual production publication"
                .to_string(),
        );
    }

    blockers.sort();
    blockers.dedup();
    blockers
}

fn recommended_action_for(
    action: &ModelReleaseStateMachineAction,
    transition_allowed: bool,
    blockers: &[String],
    reconcile_recommendation: &str,
) -> String {
    match (action, transition_allowed) {
        (ModelReleaseStateMachineAction::Review, true) => {
            "release evidence currently satisfies production publication gates".to_string()
        }
        (ModelReleaseStateMachineAction::Review, false) => format!(
            "release is not production-publishable yet; resolve blockers: {}",
            blockers.join("; ")
        ),
        (ModelReleaseStateMachineAction::PublishIfReady, true) => {
            "release can be marked published by the state machine".to_string()
        }
        (ModelReleaseStateMachineAction::PublishIfReady, false) => format!(
            "publish_if_ready was denied; resolve blockers: {}",
            blockers.join("; ")
        ),
        (ModelReleaseStateMachineAction::FailIfUnusable, true) => {
            "release has blockers and can be marked failed".to_string()
        }
        (ModelReleaseStateMachineAction::FailIfUnusable, false) => {
            format!(
                "release has no blocking evidence problems; leaving status unchanged. Reconcile says: {}",
                reconcile_recommendation
            )
        }
    }
}
