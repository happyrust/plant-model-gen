use crate::options::DbOptionExt;
use std::sync::Arc;

/// Full Noun / cache-first 运行的阶段标识。
///
/// 约定：
/// - Prefetch 阶段：允许访问 SurrealDB 拉取输入并写入 model cache（仅 PrefetchThenGenerate 模式）
/// - Generate 阶段：应尽量只读/写 model cache（cache-first / cache-only 离线生成）
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenStage {
    Prefetch,
    Generate,
}

impl GenStage {
    pub fn as_str(self) -> &'static str {
        match self {
            GenStage::Prefetch => "prefetch",
            GenStage::Generate => "generate",
        }
    }
}

/// Noun 处理上下文，包含所有处理过程需要的配置信息
#[derive(Clone)]
pub struct NounProcessContext {
    pub db_option: Arc<DbOptionExt>,
    pub batch_size: usize,
    pub batch_concurrency: usize,
    pub gen_stage: GenStage,
}

impl NounProcessContext {
    /// 创建新的处理上下文
    ///
    /// # Arguments
    /// * `db_option` - 数据库配置
    /// * `batch_size` - 每批次处理的数量
    /// * `batch_concurrency` - 批次处理的并发数（自动限制最小为1）
    pub fn new(db_option: Arc<DbOptionExt>, batch_size: usize, batch_concurrency: usize) -> Self {
        Self {
            db_option,
            batch_size,
            batch_concurrency: batch_concurrency.max(1),
            gen_stage: GenStage::Generate,
        }
    }

    pub fn with_stage(&self, stage: GenStage) -> Self {
        let mut cloned = self.clone();
        cloned.gen_stage = stage;
        cloned
    }

    /// 兼容旧调用点：foyer/cache-only 流程已移除，当前始终走 Direct 路径。
    #[inline]
    pub fn is_offline_generate(&self) -> bool {
        false
    }

    /// 根据总数计算分批范围
    ///
    /// 返回 (start, end) 范围列表，用于分页查询
    ///
    /// # Example
    /// ```
    /// let ctx = NounProcessContext::new(db_option, 100, 4);
    /// let ranges = ctx.bounded_chunks(350);
    /// // 返回: [(0, 100), (100, 200), (200, 300), (300, 350)]
    /// ```
    pub fn bounded_chunks(&self, total: usize) -> Vec<(usize, usize)> {
        if total == 0 {
            return vec![];
        }

        let chunk = self.batch_size.max(1);
        let mut ranges = Vec::new();
        let mut start = 0;
        while start < total {
            let end = (start + chunk).min(total);
            ranges.push((start, end));
            start = end;
        }
        ranges
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aios_core::options::DbOption;

    #[test]
    fn test_bounded_chunks() {
        let ctx = NounProcessContext {
            db_option: Arc::new(DbOptionExt::from(DbOption::default())),
            batch_size: 100,
            batch_concurrency: 4,
            gen_stage: GenStage::Generate,
        };

        // 测试正常情况
        let ranges = ctx.bounded_chunks(350);
        assert_eq!(ranges, vec![(0, 100), (100, 200), (200, 300), (300, 350)]);

        // 测试空情况
        assert_eq!(ctx.bounded_chunks(0), vec![]);

        // 测试小于batch_size的情况
        assert_eq!(ctx.bounded_chunks(50), vec![(0, 50)]);
    }

    #[test]
    fn test_batch_concurrency_minimum() {
        let ctx = NounProcessContext::new(Arc::new(DbOptionExt::from(DbOption::default())), 100, 0);
        assert_eq!(ctx.batch_concurrency, 1); // 自动修正为最小值1
    }
}
