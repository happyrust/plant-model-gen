use aios_core::RefnoEnum;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::RwLock;

/// Full Noun 模式统计信息
/// 
/// 用于 dry_run 模式下收集和输出生成流程的统计数据
#[derive(Debug, Default)]
pub struct FullNounStatistics {
    /// 按 Noun 类型分组的 refno 计数
    noun_counts: RwLock<HashMap<String, NounStats>>,
    
    /// 总计数
    total_collected: AtomicUsize,
    total_would_process: AtomicUsize,
    total_skipped: AtomicUsize,
}

/// 单个 Noun 类型的统计信息
#[derive(Debug, Clone, Default)]
pub struct NounStats {
    /// 收集到的 refno 数量
    pub collected: usize,
    /// 将会被处理的数量（去重后）
    pub would_process: usize,
    /// 被跳过的数量（已被 BRAN/HANG 处理）
    pub skipped_by_bran: usize,
    /// 被排除的数量（配置排除）
    pub excluded: usize,
    /// 收集到的 refno 列表（可选，用于详细分析）
    pub refnos: Vec<RefnoEnum>,
}

/// 分类统计摘要
#[derive(Debug, Clone, Default)]
pub struct CategorySummary {
    pub loop_count: usize,
    pub prim_count: usize,
    pub cate_count: usize,
    pub bran_hang_count: usize,
}

impl FullNounStatistics {
    pub fn new() -> Self {
        Self::default()
    }

    /// 记录收集到的 Noun 类型及其 refnos
    pub fn record_collected(&self, noun: &str, refnos: &[RefnoEnum]) {
        let mut map = self.noun_counts.write().unwrap();
        let entry = map.entry(noun.to_uppercase()).or_default();
        entry.collected += refnos.len();
        entry.refnos.extend(refnos.iter().copied());
        self.total_collected.fetch_add(refnos.len(), Ordering::Relaxed);
    }

    /// 记录将会被处理的 refnos
    pub fn record_would_process(&self, noun: &str, count: usize) {
        let mut map = self.noun_counts.write().unwrap();
        let entry = map.entry(noun.to_uppercase()).or_default();
        entry.would_process += count;
        self.total_would_process.fetch_add(count, Ordering::Relaxed);
    }

    /// 记录被 BRAN/HANG 跳过的 refnos
    pub fn record_skipped_by_bran(&self, noun: &str, count: usize) {
        let mut map = self.noun_counts.write().unwrap();
        let entry = map.entry(noun.to_uppercase()).or_default();
        entry.skipped_by_bran += count;
        self.total_skipped.fetch_add(count, Ordering::Relaxed);
    }

    /// 记录被配置排除的 refnos
    pub fn record_excluded(&self, noun: &str, count: usize) {
        let mut map = self.noun_counts.write().unwrap();
        let entry = map.entry(noun.to_uppercase()).or_default();
        entry.excluded += count;
    }

    /// 获取分类摘要
    pub fn get_category_summary(&self) -> CategorySummary {
        use aios_core::pdms_types::{
            GNERAL_LOOP_OWNER_NOUN_NAMES, GNERAL_PRIM_NOUN_NAMES, USE_CATE_NOUN_NAMES,
        };

        let map = self.noun_counts.read().unwrap();
        let mut summary = CategorySummary::default();

        for (noun, stats) in map.iter() {
            let noun_str: &str = noun.as_str();
            if noun_str == "BRAN" || noun_str == "HANG" {
                summary.bran_hang_count += stats.would_process;
            } else if GNERAL_LOOP_OWNER_NOUN_NAMES.contains(&noun_str) {
                summary.loop_count += stats.would_process;
            } else if GNERAL_PRIM_NOUN_NAMES.contains(&noun_str) {
                summary.prim_count += stats.would_process;
            } else if USE_CATE_NOUN_NAMES.contains(&noun_str) {
                summary.cate_count += stats.would_process;
            }
        }

        summary
    }

    /// 打印详细统计报告
    pub fn print_report(&self) {
        let map = self.noun_counts.read().unwrap();
        let total_collected = self.total_collected.load(Ordering::Relaxed);
        let total_would_process = self.total_would_process.load(Ordering::Relaxed);
        let total_skipped = self.total_skipped.load(Ordering::Relaxed);

        println!();
        println!("╔══════════════════════════════════════════════════════════════════╗");
        println!("║              📊 Full Noun DRY RUN 统计报告                       ║");
        println!("╠══════════════════════════════════════════════════════════════════╣");
        println!("║ 总计收集: {:<10} 将处理: {:<10} 跳过: {:<10} ║",
            total_collected, total_would_process, total_skipped);
        println!("╠══════════════════════════════════════════════════════════════════╣");
        println!("║ {:^12} │ {:^8} │ {:^8} │ {:^8} │ {:^8} ║",
            "Noun", "收集", "处理", "跳过", "排除");
        println!("╠══════════════════════════════════════════════════════════════════╣");

        // 按收集数量排序
        let mut sorted: Vec<_> = map.iter().collect();
        sorted.sort_by(|a, b| b.1.collected.cmp(&a.1.collected));

        for (noun, stats) in sorted {
            if stats.collected > 0 {
                println!("║ {:^12} │ {:^8} │ {:^8} │ {:^8} │ {:^8} ║",
                    noun,
                    stats.collected,
                    stats.would_process,
                    stats.skipped_by_bran,
                    stats.excluded);
            }
        }

        println!("╠══════════════════════════════════════════════════════════════════╣");

        // 分类摘要
        let summary = self.get_category_summary();
        println!("║ 分类摘要:                                                        ║");
        println!("║   BRAN/HANG: {:<8}  LOOP: {:<8}  PRIM: {:<8}  CATE: {:<8}║",
            summary.bran_hang_count, summary.loop_count, summary.prim_count, summary.cate_count);

        println!("╚══════════════════════════════════════════════════════════════════╝");
        println!();
    }

    /// 导出为 JSON 格式（用于后续分析）
    pub fn to_json(&self) -> serde_json::Value {
        let map = self.noun_counts.read().unwrap();
        let mut noun_data = serde_json::Map::new();

        for (noun, stats) in map.iter() {
            let noun_key: String = noun.clone();
            noun_data.insert(noun_key, serde_json::json!({
                "collected": stats.collected,
                "would_process": stats.would_process,
                "skipped_by_bran": stats.skipped_by_bran,
                "excluded": stats.excluded,
            }));
        }

        let summary = self.get_category_summary();

        serde_json::json!({
            "total": {
                "collected": self.total_collected.load(Ordering::Relaxed),
                "would_process": self.total_would_process.load(Ordering::Relaxed),
                "skipped": self.total_skipped.load(Ordering::Relaxed),
            },
            "by_noun": noun_data,
            "by_category": {
                "bran_hang": summary.bran_hang_count,
                "loop": summary.loop_count,
                "prim": summary.prim_count,
                "cate": summary.cate_count,
            }
        })
    }

    /// 保存统计报告到文件
    pub fn save_to_file(&self, path: &std::path::Path) -> std::io::Result<()> {
        let json = self.to_json();
        let content = serde_json::to_string_pretty(&json)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(path, content)
    }

    /// 获取所有收集的 noun 类型
    pub fn get_noun_names(&self) -> Vec<String> {
        self.noun_counts.read().unwrap().keys().cloned().collect()
    }

    /// 获取特定 noun 的统计信息
    pub fn get_noun_stats(&self, noun: &str) -> Option<NounStats> {
        self.noun_counts.read().unwrap().get(&noun.to_uppercase()).cloned()
    }
}

/// 全局统计实例（用于 dry_run 模式）
pub static DRY_RUN_STATS: once_cell::sync::Lazy<FullNounStatistics> =
    once_cell::sync::Lazy::new(FullNounStatistics::new);

/// 重置统计信息
pub fn reset_dry_run_stats() {
    *DRY_RUN_STATS.noun_counts.write().unwrap() = HashMap::new();
    DRY_RUN_STATS.total_collected.store(0, Ordering::Relaxed);
    DRY_RUN_STATS.total_would_process.store(0, Ordering::Relaxed);
    DRY_RUN_STATS.total_skipped.store(0, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_statistics_basic() {
        let stats = FullNounStatistics::new();
        
        stats.record_collected("EQUI", &[RefnoEnum::RefU64(1), RefnoEnum::RefU64(2)]);
        stats.record_would_process("EQUI", 2);
        
        stats.record_collected("BRAN", &[RefnoEnum::RefU64(3)]);
        stats.record_would_process("BRAN", 1);
        
        let summary = stats.get_category_summary();
        assert_eq!(summary.bran_hang_count, 1);
    }

    #[test]
    fn test_statistics_json_export() {
        let stats = FullNounStatistics::new();
        stats.record_collected("PIPE", &[RefnoEnum::RefU64(1)]);
        stats.record_would_process("PIPE", 1);
        
        let json = stats.to_json();
        assert!(json["total"]["collected"].as_u64().unwrap() > 0);
    }
}
