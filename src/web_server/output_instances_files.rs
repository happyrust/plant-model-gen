use axum::extract::Path;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use std::path::{Path as StdPath, PathBuf};

fn is_safe_segment(seg: &str) -> bool {
    !seg.is_empty() && !seg.contains("..") && !seg.contains('/') && !seg.contains('\\')
}

fn detect_content_type(file_name: &str) -> &'static str {
    if file_name.ends_with(".json") {
        "application/json; charset=utf-8"
    } else if file_name.ends_with(".parquet") {
        "application/octet-stream"
    } else {
        "application/octet-stream"
    }
}

async fn read_file_response(path: &StdPath, file_name: &str) -> Response {
    match tokio::fs::read(path).await {
        Ok(bytes) => (
            [(header::CONTENT_TYPE, detect_content_type(file_name))],
            bytes,
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

pub async fn get_project_instances_file(Path((project, file)): Path<(String, String)>) -> Response {
    if !is_safe_segment(&project) || !is_safe_segment(&file) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let path = PathBuf::from("output")
        .join(project)
        .join("instances")
        .join(&file);
    read_file_response(&path, &file).await
}

pub async fn get_root_instances_file(Path(file): Path<String>) -> Response {
    if !is_safe_segment(&file) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let path = PathBuf::from("output").join("instances").join(&file);
    read_file_response(&path, &file).await
}
