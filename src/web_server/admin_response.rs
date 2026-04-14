use axum::{Json, http::StatusCode};
use serde::Serialize;
use serde_json::{Value, json};

pub type ApiResponse = (StatusCode, Json<Value>);

pub fn ok<T: Serialize>(message: impl Into<String>, data: T) -> ApiResponse {
    response(StatusCode::OK, true, message, Some(data))
}

pub fn accepted<T: Serialize>(message: impl Into<String>, data: T) -> ApiResponse {
    response(StatusCode::ACCEPTED, true, message, Some(data))
}

pub fn not_found(message: impl Into<String>) -> ApiResponse {
    response::<Value>(StatusCode::NOT_FOUND, false, message, None)
}

pub fn server_error(message: impl Into<String>) -> ApiResponse {
    response::<Value>(StatusCode::INTERNAL_SERVER_ERROR, false, message, None)
}

pub fn conflict(message: impl Into<String>) -> ApiResponse {
    response::<Value>(StatusCode::CONFLICT, false, message, None)
}

pub fn service_unavailable(message: impl Into<String>) -> ApiResponse {
    response::<Value>(StatusCode::SERVICE_UNAVAILABLE, false, message, None)
}

pub fn unauthorized(message: impl Into<String>) -> ApiResponse {
    response::<Value>(StatusCode::UNAUTHORIZED, false, message, None)
}

pub fn bad_request(message: impl Into<String>) -> ApiResponse {
    response::<Value>(StatusCode::BAD_REQUEST, false, message, None)
}

pub fn response<T>(
    status: StatusCode,
    success: bool,
    message: impl Into<String>,
    data: Option<T>,
) -> ApiResponse
where
    T: Serialize,
{
    (
        status,
        Json(json!({
            "success": success,
            "message": message.into(),
            "data": data
        })),
    )
}

pub fn classify_error_status(message: &str) -> StatusCode {
    if message.contains("不存在") {
        StatusCode::NOT_FOUND
    } else if message.contains("不能为空") || message.contains("必须大于") {
        StatusCode::BAD_REQUEST
    } else if message.contains("运行中")
        || message.contains("正在运行")
        || message.contains("已在运行中")
        || message.contains("不能删除")
        || message.contains("不能修改配置")
        || message.contains("已被站点")
        || message.contains("已被当前机器")
        || message.contains("已被占用")
        || message.contains("端口")
    {
        StatusCode::CONFLICT
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

pub fn managed_error(message: String) -> ApiResponse {
    response::<Value>(classify_error_status(&message), false, message, None)
}
