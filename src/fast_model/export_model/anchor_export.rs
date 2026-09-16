use anyhow::Context;
use serde::{Deserialize, Serialize};

/// 已解析且可安全用于历史导出的 `model_gen` 锚点上下文。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnchorExportContext {
    pub dbnum: u32,
    pub requested_sesno: u32,
    pub resolved_sesno: u32,
    pub exact: bool,
    pub source: String,
    pub anchored_at: String,
}

impl AnchorExportContext {
    pub async fn resolve(dbnum: u32, requested_sesno: u32) -> anyhow::Result<Self> {
        let hit = aios_core::resolve_model_anchor(dbnum, requested_sesno)
            .await
            .context("resolve model_gen anchor for export")?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "未找到 dbnum={dbnum} sesno<={requested_sesno} 的 model_gen anchor；拒绝用当前态冒充历史导出"
                )
            })?;
        let source = hit.source.unwrap_or_default();
        if source != "model_gen" {
            anyhow::bail!(
                "导出锚点 source={}，期望 model_gen；拒绝跨 source 导出",
                source
            );
        }
        Ok(Self {
            dbnum,
            requested_sesno,
            resolved_sesno: hit.sesno,
            exact: hit.exact,
            source,
            anchored_at: hit.anchored_at,
        })
    }

    pub fn version_clause(&self) -> String {
        format!(" VERSION d'{}'", self.anchored_at)
    }
}
