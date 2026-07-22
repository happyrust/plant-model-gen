use super::models::NounCategory;
use crate::data_interface::increment_record::IncrGeoUpdateLog;
use crate::generation_read::hash_serializable;
use aios_core::RefnoEnum;
use aios_core::pdms_types::{
    GNERAL_LOOP_OWNER_NOUN_NAMES, GNERAL_PRIM_NOUN_NAMES, USE_CATE_NOUN_NAMES,
};
use std::collections::HashSet;

/// A canonical, immutable description of one geometry run's work set.
///
/// All adapters normalize into this type before the executor starts, so scope
/// discovery order and incremental-log `HashSet` order cannot affect execution
/// or provenance.
#[derive(Debug, Clone)]
pub(crate) struct GenerationTargets {
    bran_hang_refnos: Vec<RefnoEnum>,
    loop_refnos: Vec<RefnoEnum>,
    cate_refnos: Vec<RefnoEnum>,
    prim_refnos: Vec<RefnoEnum>,
    delete_refnos: Vec<RefnoEnum>,
    target_hash: String,
}

impl GenerationTargets {
    pub(crate) fn new(
        bran_hang_refnos: impl IntoIterator<Item = RefnoEnum>,
        loop_refnos: impl IntoIterator<Item = RefnoEnum>,
        cate_refnos: impl IntoIterator<Item = RefnoEnum>,
        prim_refnos: impl IntoIterator<Item = RefnoEnum>,
        delete_refnos: impl IntoIterator<Item = RefnoEnum>,
    ) -> Self {
        let bran_hang_refnos = normalize_refnos(bran_hang_refnos);
        let loop_refnos = normalize_refnos(loop_refnos);
        let cate_refnos = normalize_refnos(cate_refnos);
        let prim_refnos = normalize_refnos(prim_refnos);
        let delete_refnos = normalize_refnos(delete_refnos);
        let target_hash = hash_serializable(&(
            "generation-targets/v1",
            canonical_refno_keys(&bran_hang_refnos),
            canonical_refno_keys(&loop_refnos),
            canonical_refno_keys(&cate_refnos),
            canonical_refno_keys(&prim_refnos),
            canonical_refno_keys(&delete_refnos),
        ));

        Self {
            bran_hang_refnos,
            loop_refnos,
            cate_refnos,
            prim_refnos,
            delete_refnos,
            target_hash,
        }
    }

    pub(crate) fn from_incremental(log: &IncrGeoUpdateLog) -> Self {
        Self::new(
            log.bran_hanger_refnos.iter().copied(),
            log.loop_owner_refnos.iter().copied(),
            log.basic_cata_refnos.iter().copied(),
            log.prim_refnos.iter().copied(),
            log.delete_refnos.iter().copied(),
        )
    }

    pub(crate) fn target_hash(&self) -> &str {
        &self.target_hash
    }

    pub(crate) fn bran_hang_refnos(&self) -> &[RefnoEnum] {
        &self.bran_hang_refnos
    }

    pub(crate) fn loop_refnos(&self) -> &[RefnoEnum] {
        &self.loop_refnos
    }

    pub(crate) fn cate_refnos(&self) -> &[RefnoEnum] {
        &self.cate_refnos
    }

    pub(crate) fn prim_refnos(&self) -> &[RefnoEnum] {
        &self.prim_refnos
    }

    pub(crate) fn delete_refnos(&self) -> &[RefnoEnum] {
        &self.delete_refnos
    }

    pub(crate) fn has_generation_targets(&self) -> bool {
        !self.bran_hang_refnos.is_empty()
            || !self.loop_refnos.is_empty()
            || !self.cate_refnos.is_empty()
            || !self.prim_refnos.is_empty()
    }

    pub(crate) fn is_delete_only(&self) -> bool {
        !self.delete_refnos.is_empty() && !self.has_generation_targets()
    }
}

fn normalize_refnos(values: impl IntoIterator<Item = RefnoEnum>) -> Vec<RefnoEnum> {
    let mut values: Vec<_> = values.into_iter().filter(RefnoEnum::is_valid).collect();
    values.sort_by_key(ToString::to_string);
    values.dedup();
    values
}

fn canonical_refno_keys(values: &[RefnoEnum]) -> Vec<String> {
    values.iter().map(ToString::to_string).collect()
}

/// GenPipeline下的目标类型聚合结果
#[derive(Debug, Clone)]
pub struct GenPipelineTargetCollection {
    /// 按类别分组的 Noun 列表
    pub cate_nouns: Vec<&'static str>,
    pub loop_owner_nouns: Vec<&'static str>,
    pub prim_nouns: Vec<&'static str>,
    /// 所有 Noun 的去重集合（用于快速查找）
    pub all_nouns: HashSet<&'static str>,
}

impl GenPipelineTargetCollection {
    /// 聚合并去重所有 Noun 列表
    ///
    /// 从 pdms_types 中的常量收集：
    /// - USE_CATE_NOUN_NAMES
    /// - GNERAL_LOOP_OWNER_NOUN_NAMES
    /// - GNERAL_PRIM_NOUN_NAMES
    ///
    /// 可选的 extra_nouns 用于扩展（调试或特殊场景）
    pub fn collect(extra_nouns: Option<&[&'static str]>) -> Self {
        Self::collect_with_config(extra_nouns, None)
    }

    /// 聚合并去重 Noun 列表，支持配置过滤
    ///
    /// 根据 config 过滤启用的 noun 类别和具体 noun
    pub fn collect_with_config(
        extra_nouns: Option<&[&'static str]>,
        config: Option<&super::config::GenPipelineConfig>,
    ) -> Self {
        // 收集 cate nouns（仅在类别内部去重，不做跨类别互斥）
        let mut cate_nouns = Vec::new();
        for &noun in USE_CATE_NOUN_NAMES.iter() {
            // 应用配置过滤
            if let Some(config) = config {
                if !config.should_process_noun(noun, "cate") {
                    continue;
                }
            }

            if !cate_nouns.contains(&noun) {
                cate_nouns.push(noun);
            }
        }

        // 收集 loop owner nouns
        let mut loop_owner_nouns = Vec::new();
        for &noun in GNERAL_LOOP_OWNER_NOUN_NAMES.iter() {
            // 应用配置过滤
            if let Some(config) = config {
                if !config.should_process_noun(noun, "loop") {
                    continue;
                }
            }

            if !loop_owner_nouns.contains(&noun) {
                loop_owner_nouns.push(noun);
            }
        }

        // 收集 prim nouns
        let mut prim_nouns = Vec::new();
        for &noun in GNERAL_PRIM_NOUN_NAMES.iter() {
            // 应用配置过滤
            if let Some(config) = config {
                if !config.should_process_noun(noun, "prim") {
                    continue;
                }
            }

            if !prim_nouns.contains(&noun) {
                prim_nouns.push(noun);
            }
        }

        // 添加额外的 nouns（如果提供）
        if let Some(extras) = extra_nouns {
            for &noun in extras {
                // 应用配置过滤
                if let Some(config) = config {
                    if !config.should_process_noun(noun, "cate") {
                        continue;
                    }
                }

                // 简单策略：额外的 noun 默认归入 cate 类别
                // 实际使用时可以根据需要调整
                if !cate_nouns.contains(&noun)
                    && !loop_owner_nouns.contains(&noun)
                    && !prim_nouns.contains(&noun)
                {
                    cate_nouns.push(noun);
                }
            }
        }

        // 汇总所有 noun，构建去重集合（允许同一个 noun 同时属于多个类别）
        let mut all_nouns = HashSet::new();
        for &noun in cate_nouns
            .iter()
            .chain(loop_owner_nouns.iter())
            .chain(prim_nouns.iter())
        {
            all_nouns.insert(noun);
        }
        if let Some(extras) = extra_nouns {
            for &noun in extras {
                all_nouns.insert(noun);
            }
        }

        // 如果有配置，打印过滤信息
        if let Some(config) = config {
            if !config.enabled_categories.is_empty() && !config.excluded_nouns.is_empty() {
                println!(
                    "🔍 Noun 过滤: 启用 {:?}, 排除 {:?}",
                    config.enabled_categories, config.excluded_nouns
                );
            } else if !config.enabled_categories.is_empty() {
                println!("🔍 Noun 过滤: 启用 {:?}", config.enabled_categories);
            } else if !config.excluded_nouns.is_empty() {
                println!("🔍 Noun 过滤: 排除 {:?}", config.excluded_nouns);
            }
        }

        Self {
            cate_nouns,
            loop_owner_nouns,
            prim_nouns,
            all_nouns,
        }
    }

    /// 根据 Noun 名称判断其类别
    pub fn get_category(&self, noun: &str) -> Option<NounCategory> {
        if self.cate_nouns.contains(&noun) {
            Some(NounCategory::Cate)
        } else if self.loop_owner_nouns.contains(&noun) {
            Some(NounCategory::LoopOwner)
        } else if self.prim_nouns.contains(&noun) {
            Some(NounCategory::Prim)
        } else {
            None
        }
    }

    /// 获取所有 Noun 的总数
    pub fn total_count(&self) -> usize {
        self.all_nouns.len()
    }

    /// 获取指定类别的 Noun 列表
    pub fn get_nouns_by_category(&self, category: NounCategory) -> &[&'static str] {
        match category {
            NounCategory::Cate => &self.cate_nouns,
            NounCategory::LoopOwner => &self.loop_owner_nouns,
            NounCategory::Prim => &self.prim_nouns,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collect_nouns() {
        let collection = GenPipelineTargetCollection::collect(None);

        // 所有类别中的 noun 都应该出现在 all_nouns 中
        for &noun in collection
            .cate_nouns
            .iter()
            .chain(collection.loop_owner_nouns.iter())
            .chain(collection.prim_nouns.iter())
        {
            assert!(collection.all_nouns.contains(noun));
        }

        // all_nouns 去重后的数量不大于各类别总和
        let total_in_lists = collection.cate_nouns.len()
            + collection.loop_owner_nouns.len()
            + collection.prim_nouns.len();
        assert!(collection.all_nouns.len() <= total_in_lists);
    }

    #[test]
    fn test_get_category() {
        let collection = GenPipelineTargetCollection::collect(None);

        // 测试已知的noun
        if let Some(&first_cate) = collection.cate_nouns.first() {
            assert_eq!(
                collection.get_category(first_cate),
                Some(NounCategory::Cate)
            );
        }

        // 测试不存在的noun
        assert_eq!(collection.get_category("NONEXISTENT"), None);
    }

    #[test]
    fn test_extra_nouns() {
        let extras = vec!["CUSTOM1", "CUSTOM2"];
        let collection = GenPipelineTargetCollection::collect(Some(&extras));

        assert!(collection.all_nouns.contains("CUSTOM1"));
        assert!(collection.all_nouns.contains("CUSTOM2"));
    }

    #[test]
    fn generation_targets_are_sorted_deduplicated_and_stably_hashed() {
        let one = RefnoEnum::from("1/1");
        let two = RefnoEnum::from("1/2");
        let invalid = RefnoEnum::default();

        let left = GenerationTargets::new([two, invalid, one, two], [], [two, one], [], []);
        let right = GenerationTargets::new([one, two], [], [one, two], [], []);

        assert_eq!(left.bran_hang_refnos(), [one, two]);
        assert_eq!(left.cate_refnos(), [one, two]);
        assert_eq!(left.target_hash(), right.target_hash());
    }

    #[test]
    fn incremental_target_hash_ignores_hashset_traversal_order() {
        let mut left = IncrGeoUpdateLog::default();
        left.prim_refnos.insert(RefnoEnum::from("1/2"));
        left.prim_refnos.insert(RefnoEnum::from("1/1"));

        let mut right = IncrGeoUpdateLog::default();
        right.prim_refnos.insert(RefnoEnum::from("1/1"));
        right.prim_refnos.insert(RefnoEnum::from("1/2"));

        assert_eq!(
            GenerationTargets::from_incremental(&left).target_hash(),
            GenerationTargets::from_incremental(&right).target_hash()
        );
    }

    #[test]
    fn delete_only_incremental_remains_delete_only() {
        let mut log = IncrGeoUpdateLog::default();
        log.delete_refnos.insert(RefnoEnum::from("1/9"));

        let targets = GenerationTargets::from_incremental(&log);
        assert!(targets.is_delete_only());
        assert!(!targets.has_generation_targets());
    }

    #[test]
    fn empty_incremental_has_no_targets() {
        let targets = GenerationTargets::from_incremental(&IncrGeoUpdateLog::default());
        assert!(!targets.has_generation_targets());
        assert!(!targets.is_delete_only());
        assert!(targets.delete_refnos().is_empty());
    }

    #[test]
    fn scope_adapters_with_same_targets_share_one_target_identity() {
        let bran = RefnoEnum::from("1/1");
        let cate = RefnoEnum::from("1/2");
        let prim = RefnoEnum::from("1/3");
        let full = GenerationTargets::new([bran], [], [cate], [prim], []);
        let root_scoped = GenerationTargets::new([bran, bran], [], [cate], [prim], []);
        let mut incremental_log = IncrGeoUpdateLog::default();
        incremental_log.bran_hanger_refnos.insert(bran);
        incremental_log.basic_cata_refnos.insert(cate);
        incremental_log.prim_refnos.insert(prim);
        let incremental = GenerationTargets::from_incremental(&incremental_log);

        assert_eq!(full.target_hash(), root_scoped.target_hash());
        assert_eq!(full.target_hash(), incremental.target_hash());
    }
}
