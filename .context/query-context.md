# SigMap Query Context
Generated: 2026-05-26T01:23:34.555Z

## .worktrees\pe-transform-backends\src\web_api\review_api.rs
```
pub struct CreateTaskRequest
pub struct UpdateTaskRequest
pub struct ReviewActionRequest
pub struct SubmitToNextRequest
pub struct ReturnRequest
pub struct WorkflowStep
pub struct ReviewComponent
pub struct ReviewAttachment
pub struct ReviewTask
pub struct TaskListResponse
pub struct TaskResponse
pub struct ActionResponse
pub struct TaskListQuery
pub struct ConfirmedRecordData
pub struct ConfirmedRecordResponse
pub struct ConfirmedRecordWithMeta
pub struct AnnotationComment
pub struct CreateCommentRequest
pub struct CommentResponse
pub struct CommentQuery
```

## .worktrees\pe-transform-backends\src\web_server\sqlite_spatial_api.rs
```
pub struct SqliteSpatialQueryParams
pub struct SpatialQueryResult
pub struct SpatialQueryResultItem
pub struct AabbDto
pub struct Vec3Dto
pub struct SpatialStatsResult
pub async fn api_sqlite_spatial_query(Query(params) → Json<SpatialQueryResult>
pub async fn api_sqlite_spatial_stats() → Json<SpatialStatsResult>
```

## src\web_api\review_api.rs
```
pub struct CreateTaskRequest
pub struct UpdateTaskRequest
pub struct ReviewActionRequest
pub struct SubmitToNextRequest
pub struct ReturnRequest
pub struct WorkflowStep
pub struct ReviewComponent
pub struct ReviewAttachment
pub struct ReviewTask
pub struct TaskListResponse
pub struct TaskResponse
pub struct ActionResponse
pub struct TaskListQuery
pub struct ConfirmedRecordData
pub struct ConfirmedRecordResponse
pub struct ConfirmedRecordWithMeta
pub struct AnnotationComment
pub struct CreateCommentRequest
pub struct CommentResponse
pub struct CommentQuery
```

## .worktrees\pe-transform-backends\src\web_api\review_annotation_state.rs
```
pub struct ApplyAnnotationStateRequest
pub struct ApplyAnnotationStateResponse
pub struct AnnotationStateView
pub struct QueryAnnotationStatesRequest
pub struct QueryAnnotationStatesResponse
pub fn create_annotation_state_routes() → Router
pub async fn sync_annotation_states_from_snapshot(form_id: &str, task_id: &str, current_node: &str, operator_id: &str, operator_name: &str, operator_role: &str, annotations: &[Value], cloud_annotations: &[Value], rect_annotations: &[Value],)
pub async fn load_annotation_states_by_task(form_id: &str, task_id: &str,) → Result<Vec<AnnotationStateV...
pub async fn delete_annotation_states_by_form_id(form_id: &str) → Result<(), String>
```

## .worktrees\pe-transform-backends\src\web_api\platform_api\types.rs
```
pub struct EmbedUrlRequest
pub struct EmbedUrlResponse
pub struct EmbedUrlData
pub struct EmbedUrlQuery
pub struct EmbedLineage
pub struct ReviewFormSummary
pub struct CachePreloadRequest
pub struct CachePreloadResponse
pub struct SyncWorkflowRequest
pub struct WorkflowActor
pub struct WorkflowNextStep
pub struct WorkflowVerifyNextStepDiagnostic
pub struct SyncWorkflowResponse
pub struct VerifyWorkflowResponse
pub struct VerifyWorkflowData
pub struct SyncWorkflowData
pub struct WorkflowRecord
pub struct WorkflowAnnotationComment
pub struct WorkflowAttachment
pub struct DeleteReviewRequest
```
