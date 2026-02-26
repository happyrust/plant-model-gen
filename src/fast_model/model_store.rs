//! [foyer-removal] 桩模块：model_store 已移除，请直接使用 SUL_DB.query_take / SUL_DB.query_response。

use aios_core::{SUL_DB, SurrealQueryExt};
use surrealdb::IndexedResults as Response;
use surrealdb::opt::QueryResult as SurrealQueryResult;
use surrealdb::types::SurrealValue;

pub async fn model_query_response<S: AsRef<str>>(sql: S) -> anyhow::Result<Response> {
    SUL_DB.query_response(sql).await.map_err(Into::into)
}

pub async fn model_query_take<T, S: AsRef<str>>(sql: S, idx: usize) -> anyhow::Result<T>
where
    T: SurrealValue,
    usize: SurrealQueryResult<T>,
{
    SUL_DB.query_take(sql, idx).await.map_err(Into::into)
}
