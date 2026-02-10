//! LOOP/PRIM 输入缓存的 key-driven 流水线（prefetch -> write -> key -> consume）。
//!
//! 说明：
//! - 本文件提供 M1 版 runner：以 batch 为单位预取输入并写入 geom_input_cache，然后通过 key 通知 consumer。
//! - Smoke test 不依赖 SurrealDB：只验证“写入 -> 发 key -> 按 key 读回”的链路。

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_pipeline_key_driven_consume_smoke() {
        // NOTE: 这里不调用真实 fetch_*；只验证“写入->发 key->按 key 读回”的链路。
        // 期望：consumer 收到 2 个 key，并能从 cache get 到对应 batch。

        // TODO(Task3): 实现 smoke test 所需的 helper 与 ReadyBatchKey / pipeline runner。
        let _ = consume_keys_from_cache_smoke_helper().await;
    }
}

