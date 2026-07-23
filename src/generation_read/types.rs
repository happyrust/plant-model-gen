use std::collections::{BTreeMap, BTreeSet};

use aios_core::{NamedAttrMap, NamedAttrValue, RefnoEnum, Transform};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::error::{GenerationReadError, GenerationReadResult};

pub const ATTRIBUTE_CODEC_VERSION: u16 = 1;
const ATTRIBUTE_PAYLOAD_JSON_MAGIC: &[u8; 4] = b"ATJ1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GenerationReadBackendKind {
    Surreal,
}

impl GenerationReadBackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Surreal => "surreal",
        }
    }
}

/// Controls whether one generation run observes the current database state or
/// a single, caller-selected historical instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerationReadMode {
    Live,
    ReadAt,
}

impl GenerationReadMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::ReadAt => "read_at",
        }
    }
}

/// Immutable input-read contract shared by every query in one generation run.
///
/// `Live` is reserved for initialization, before a project has a stable data
/// anchor. Incremental, catch-up, and repair callers must construct `ReadAt`
/// with the one anchor timestamp and the complete observed watermark vector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationReadSpec {
    mode: GenerationReadMode,
    read_at: Option<String>,
    observed_watermarks: BTreeMap<u32, u32>,
}

impl GenerationReadSpec {
    /// Read the current state. The session factory observes watermarks while
    /// opening the session and records them in its input manifest.
    pub fn live() -> Self {
        Self {
            mode: GenerationReadMode::Live,
            read_at: None,
            observed_watermarks: BTreeMap::new(),
        }
    }

    /// Pin every read in the run to the same data-anchor timestamp.
    pub fn at(
        read_at: impl Into<String>,
        observed_watermarks: BTreeMap<u32, u32>,
    ) -> GenerationReadResult<Self> {
        let spec = Self {
            mode: GenerationReadMode::ReadAt,
            read_at: Some(read_at.into().trim().to_string()),
            observed_watermarks,
        };
        spec.validate()?;
        Ok(spec)
    }

    pub fn mode(&self) -> GenerationReadMode {
        self.mode
    }

    pub fn read_at(&self) -> Option<&str> {
        self.read_at.as_deref()
    }

    pub fn observed_watermarks(&self) -> &BTreeMap<u32, u32> {
        &self.observed_watermarks
    }

    /// Revalidates deserialized values before they cross the adapter boundary.
    pub fn validate(&self) -> GenerationReadResult<()> {
        match self.mode {
            GenerationReadMode::Live => {
                if self.read_at.is_some() || !self.observed_watermarks.is_empty() {
                    return Err(GenerationReadError::InvalidReadSpec(
                        "live 模式不能携带 read_at 或固定 observed_watermarks".to_string(),
                    ));
                }
            }
            GenerationReadMode::ReadAt => {
                let read_at = self.read_at.as_deref().ok_or_else(|| {
                    GenerationReadError::InvalidReadSpec("read_at 模式必须提供锚点时间".to_string())
                })?;
                if read_at.is_empty()
                    || read_at != read_at.trim()
                    || read_at.contains('\'')
                    || read_at.contains('\0')
                {
                    return Err(GenerationReadError::InvalidReadSpec(
                        "read_at 必须是非空且不含引号/NUL 的规范时间字符串".to_string(),
                    ));
                }
                if self.observed_watermarks.is_empty() {
                    return Err(GenerationReadError::InvalidReadSpec(
                        "read_at 模式必须提供 observed_watermarks".to_string(),
                    ));
                }
                if let Some((dbnum, sesno)) = self
                    .observed_watermarks
                    .iter()
                    .find(|(dbnum, sesno)| **dbnum == 0 || **sesno == 0)
                {
                    return Err(GenerationReadError::InvalidReadSpec(format!(
                        "observed_watermarks 必须为非零 dbnum/sesno: dbnum={dbnum} sesno={sesno}"
                    )));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataVersion {
    pub dbnum: u32,
    pub sesno: u32,
    pub commit_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputVersionManifest {
    pub authoritative_snapshot_id: u64,
    pub history_start_snapshot: u64,
    pub versions: BTreeMap<u32, DataVersion>,
    pub manifest_hash: String,
}

impl InputVersionManifest {
    pub fn new(
        authoritative_snapshot_id: u64,
        history_start_snapshot: u64,
        versions: impl IntoIterator<Item = DataVersion>,
    ) -> GenerationReadResult<Self> {
        if authoritative_snapshot_id < history_start_snapshot {
            return Err(GenerationReadError::InvalidManifest(format!(
                "snapshot_id {} 早于 history_start_snapshot {}",
                authoritative_snapshot_id, history_start_snapshot
            )));
        }

        let mut by_dbnum = BTreeMap::new();
        for version in versions {
            if version.dbnum == 0 || version.sesno == 0 {
                return Err(GenerationReadError::InvalidManifest(format!(
                    "dbnum/sesno 必须非零: dbnum={} sesno={}",
                    version.dbnum, version.sesno
                )));
            }
            if version.commit_fingerprint.trim().is_empty() {
                return Err(GenerationReadError::InvalidManifest(format!(
                    "dbnum={} 缺少 commit fingerprint",
                    version.dbnum
                )));
            }
            if by_dbnum.insert(version.dbnum, version).is_some() {
                return Err(GenerationReadError::InvalidManifest(
                    "同一 dbnum 在输入版本清单中重复".to_string(),
                ));
            }
        }
        if by_dbnum.is_empty() {
            return Err(GenerationReadError::InvalidManifest(
                "输入版本清单不能为空".to_string(),
            ));
        }

        let manifest_hash =
            hash_serializable(&(authoritative_snapshot_id, history_start_snapshot, &by_dbnum));
        Ok(Self {
            authoritative_snapshot_id,
            history_start_snapshot,
            versions: by_dbnum,
            manifest_hash,
        })
    }

    pub fn dbnums(&self) -> Vec<u32> {
        self.versions.keys().copied().collect()
    }

    pub fn verify_hash(&self) -> GenerationReadResult<()> {
        if self.authoritative_snapshot_id < self.history_start_snapshot {
            return Err(GenerationReadError::InvalidManifest(format!(
                "snapshot_id {} 早于 history_start_snapshot {}",
                self.authoritative_snapshot_id, self.history_start_snapshot
            )));
        }
        if self.versions.is_empty() {
            return Err(GenerationReadError::InvalidManifest(
                "输入版本清单不能为空".to_string(),
            ));
        }
        for (dbnum, version) in &self.versions {
            if *dbnum == 0
                || version.dbnum != *dbnum
                || version.sesno == 0
                || version.commit_fingerprint.trim().is_empty()
            {
                return Err(GenerationReadError::InvalidManifest(format!(
                    "版本项非法: map_dbnum={dbnum} value_dbnum={} sesno={} fingerprint_empty={}",
                    version.dbnum,
                    version.sesno,
                    version.commit_fingerprint.trim().is_empty()
                )));
            }
        }
        let actual = hash_serializable(&(
            self.authoritative_snapshot_id,
            self.history_start_snapshot,
            &self.versions,
        ));
        if actual == self.manifest_hash {
            Ok(())
        } else {
            Err(GenerationReadError::ManifestMismatch {
                snapshot_id: self.authoritative_snapshot_id,
                expected: self.manifest_hash.clone(),
                actual,
            })
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ElementSnapshot {
    pub refno: RefnoEnum,
    pub dbnum: u32,
    pub owner: RefnoEnum,
    pub noun: String,
    pub name: String,
    #[serde(default)]
    pub children: Vec<RefnoEnum>,
    pub has_children: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum AttributeValue {
    Invalid,
    Integer(i32),
    Long(i64),
    Float(f32),
    String(String),
    FloatArray(Vec<f32>),
    Vec3([f32; 3]),
    StringArray(Vec<String>),
    BoolArray(Vec<bool>),
    IntegerArray(Vec<i32>),
    Bool(bool),
    Element(String),
    Word(String),
    Ref(RefnoEnum),
    RefArray(Vec<RefnoEnum>),
}

impl From<&NamedAttrValue> for AttributeValue {
    fn from(value: &NamedAttrValue) -> Self {
        match value {
            NamedAttrValue::InvalidType => Self::Invalid,
            NamedAttrValue::IntegerType(value) => Self::Integer(*value),
            NamedAttrValue::StringType(value) => Self::String(value.clone()),
            NamedAttrValue::F32Type(value) => Self::Float(*value),
            NamedAttrValue::F32VecType(value) => Self::FloatArray(value.clone()),
            NamedAttrValue::Vec3Type(value) => Self::Vec3(value.to_array()),
            NamedAttrValue::StringArrayType(value) => Self::StringArray(value.clone()),
            NamedAttrValue::BoolArrayType(value) => Self::BoolArray(value.clone()),
            NamedAttrValue::IntArrayType(value) => Self::IntegerArray(value.clone()),
            NamedAttrValue::BoolType(value) => Self::Bool(*value),
            NamedAttrValue::ElementType(value) => Self::Element(value.clone()),
            NamedAttrValue::WordType(value) => Self::Word(value.clone()),
            NamedAttrValue::RefU64Type(value) => Self::Ref(RefnoEnum::from(*value)),
            NamedAttrValue::RefU64Array(value) => Self::RefArray(value.clone()),
            NamedAttrValue::LongType(value) => Self::Long(*value),
            NamedAttrValue::RefnoEnumType(value) => Self::Ref(*value),
        }
    }
}

impl From<&AttributeValue> for NamedAttrValue {
    fn from(value: &AttributeValue) -> Self {
        match value {
            AttributeValue::Invalid => Self::InvalidType,
            AttributeValue::Integer(value) => Self::IntegerType(*value),
            AttributeValue::Long(value) => Self::LongType(*value),
            AttributeValue::Float(value) => Self::F32Type(*value),
            AttributeValue::String(value) => Self::StringType(value.clone()),
            AttributeValue::FloatArray(value) => Self::F32VecType(value.clone()),
            AttributeValue::Vec3(value) => Self::Vec3Type(glam::Vec3::from_array(*value)),
            AttributeValue::StringArray(value) => Self::StringArrayType(value.clone()),
            AttributeValue::BoolArray(value) => Self::BoolArrayType(value.clone()),
            AttributeValue::IntegerArray(value) => Self::IntArrayType(value.clone()),
            AttributeValue::Bool(value) => Self::BoolType(*value),
            AttributeValue::Element(value) => Self::ElementType(value.clone()),
            AttributeValue::Word(value) => Self::WordType(value.clone()),
            AttributeValue::Ref(value) => Self::RefnoEnumType(*value),
            AttributeValue::RefArray(value) => Self::RefU64Array(value.clone()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttributeSet {
    pub refno: RefnoEnum,
    pub codec_version: u16,
    pub values: BTreeMap<String, AttributeValue>,
    pub canonical_hash: String,
}

impl AttributeSet {
    pub fn new(refno: RefnoEnum, values: BTreeMap<String, AttributeValue>) -> Self {
        let codec_version = ATTRIBUTE_CODEC_VERSION;
        let canonical_hash = hash_serializable(&(codec_version, refno, &values));
        Self {
            refno,
            codec_version,
            values,
            canonical_hash,
        }
    }

    pub fn from_named_attr_map(refno: RefnoEnum, attributes: &NamedAttrMap) -> Self {
        Self::new(
            refno,
            attributes
                .map
                .iter()
                .map(|(name, value)| (name.clone(), AttributeValue::from(value)))
                .collect(),
        )
    }

    pub fn to_named_attr_map(&self) -> NamedAttrMap {
        NamedAttrMap {
            map: self
                .values
                .iter()
                .map(|(name, value)| (name.clone(), NamedAttrValue::from(value)))
                .collect(),
        }
    }

    pub fn verify(&self) -> GenerationReadResult<()> {
        if !self.refno.is_valid() {
            return Err(GenerationReadError::PayloadCorrupt {
                refno: self.refno,
                detail: "payload refno is invalid".to_string(),
            });
        }
        if self.codec_version != ATTRIBUTE_CODEC_VERSION {
            return Err(GenerationReadError::PayloadCorrupt {
                refno: self.refno,
                detail: format!(
                    "不支持的 codec_version={} expected={}",
                    self.codec_version, ATTRIBUTE_CODEC_VERSION
                ),
            });
        }
        let actual = hash_serializable(&(self.codec_version, self.refno, &self.values));
        if actual == self.canonical_hash {
            Ok(())
        } else {
            Err(GenerationReadError::PayloadCorrupt {
                refno: self.refno,
                detail: format!(
                    "hash mismatch expected={} actual={actual}",
                    self.canonical_hash
                ),
            })
        }
    }

    pub fn reference_edges(&self, dbnum: u32) -> Vec<AttributeReference> {
        let mut out = Vec::new();
        for (name, value) in &self.values {
            match value {
                AttributeValue::Ref(target) if target.is_valid() => {
                    out.push(AttributeReference {
                        dbnum,
                        source: self.refno,
                        attribute_name: name.clone(),
                        target: *target,
                        ordinal: 0,
                    });
                }
                AttributeValue::RefArray(targets) => {
                    out.extend(targets.iter().enumerate().filter_map(|(ordinal, target)| {
                        target.is_valid().then_some(AttributeReference {
                            dbnum,
                            source: self.refno,
                            attribute_name: name.clone(),
                            target: *target,
                            ordinal: ordinal as u32,
                        })
                    }));
                }
                _ => {}
            }
        }
        out
    }
}

/// AttributeSet 的持久化 envelope。JSON payload 能正确处理 RefnoEnum 的
/// serde 表示；magic 允许读取端继续兼容已存在的无 envelope bincode 数据。
pub fn encode_attribute_set_payload(attributes: &AttributeSet) -> anyhow::Result<Vec<u8>> {
    attributes.verify()?;
    let json = serde_json::to_vec(attributes)?;
    let mut payload = Vec::with_capacity(ATTRIBUTE_PAYLOAD_JSON_MAGIC.len() + json.len());
    payload.extend_from_slice(ATTRIBUTE_PAYLOAD_JSON_MAGIC);
    payload.extend_from_slice(&json);
    Ok(payload)
}

pub fn decode_attribute_set_payload(payload: &[u8]) -> anyhow::Result<AttributeSet> {
    let attributes = if let Some(json) = payload.strip_prefix(ATTRIBUTE_PAYLOAD_JSON_MAGIC) {
        serde_json::from_slice(json)?
    } else {
        bincode::deserialize(payload)?
    };
    Ok(attributes)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttributeReference {
    pub dbnum: u32,
    pub source: RefnoEnum,
    pub attribute_name: String,
    pub target: RefnoEnum,
    pub ordinal: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HierarchyRow {
    pub dbnum: u32,
    pub parent: RefnoEnum,
    pub child: RefnoEnum,
    pub ordinal: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CatalogNode {
    pub refno: RefnoEnum,
    pub dbnum: u32,
    pub db_type: String,
    pub noun: String,
    pub owner: RefnoEnum,
    pub children: Vec<RefnoEnum>,
    pub outbound: Vec<AttributeReference>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransformSnapshot {
    pub refno: RefnoEnum,
    pub dbnum: u32,
    pub local: Option<Transform>,
    pub world: Transform,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ElementQuery {
    pub dbnums: BTreeSet<u32>,
    pub nouns: BTreeSet<String>,
    pub has_children: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatchLookup<T> {
    pub found: BTreeMap<RefnoEnum, T>,
    pub missing: Vec<RefnoEnum>,
}

impl<T> Default for BatchLookup<T> {
    fn default() -> Self {
        Self {
            found: BTreeMap::new(),
            missing: Vec::new(),
        }
    }
}

impl<T> BatchLookup<T> {
    pub fn from_found(
        requested: &[RefnoEnum],
        found: impl IntoIterator<Item = (RefnoEnum, T)>,
    ) -> Self {
        let found: BTreeMap<_, _> = found.into_iter().collect();
        let requested: BTreeSet<_> = requested.iter().copied().collect();
        let missing = requested
            .into_iter()
            .filter(|refno| !found.contains_key(refno))
            .collect();
        Self { found, missing }
    }

    pub fn require_all(
        self,
        capability: &'static str,
    ) -> GenerationReadResult<BTreeMap<RefnoEnum, T>> {
        if self.missing.is_empty() {
            Ok(self.found)
        } else {
            Err(GenerationReadError::MissingRequiredData {
                capability,
                refnos: self.missing,
            })
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionMetricsSnapshot {
    pub backend_calls: BTreeMap<String, u64>,
    pub requested_keys: BTreeMap<String, u64>,
    pub returned_rows: BTreeMap<String, u64>,
    pub elapsed_micros: BTreeMap<String, u64>,
}

impl SessionMetricsSnapshot {
    pub fn assert_call_limit(&self, capability: &str, max_calls: u64) -> GenerationReadResult<()> {
        let actual = self.backend_calls.get(capability).copied().unwrap_or(0);
        if actual <= max_calls {
            Ok(())
        } else {
            Err(GenerationReadError::PerformanceGate {
                capability: capability.to_string(),
                detail: format!("backend_calls={actual} exceeds limit={max_calls}"),
            })
        }
    }

    pub fn assert_elapsed_regression_within(
        &self,
        baseline: &Self,
        capability: &str,
        max_regression_ratio: f64,
    ) -> GenerationReadResult<()> {
        let actual = self.elapsed_micros.get(capability).copied().unwrap_or(0);
        let baseline = baseline
            .elapsed_micros
            .get(capability)
            .copied()
            .unwrap_or(0);
        if baseline == 0 {
            return Err(GenerationReadError::PerformanceGate {
                capability: capability.to_string(),
                detail: "baseline elapsed_micros is zero or missing".to_string(),
            });
        }
        let limit = baseline as f64 * (1.0 + max_regression_ratio);
        if actual as f64 <= limit {
            Ok(())
        } else {
            Err(GenerationReadError::PerformanceGate {
                capability: capability.to_string(),
                detail: format!(
                    "elapsed_micros={actual} exceeds baseline={baseline} regression_limit={max_regression_ratio:.3}"
                ),
            })
        }
    }

    pub fn assert_batch_first_hot_path(&self) -> GenerationReadResult<()> {
        // These capabilities are one-shot bulk reads. Attribute/catalog reads
        // intentionally advance a graph frontier in batches; their adapters
        // cache resolved refnos, so applying a global call_limit=1 would reject
        // legitimate multi-hop catalog closure rather than detect N+1 access.
        const CAPABILITIES: [&str; 3] = ["element.query", "hierarchy.load", "transform.load"];
        for (name, calls) in &self.backend_calls {
            if CAPABILITIES
                .iter()
                .any(|capability| name == capability || name.ends_with(&format!(".{capability}")))
                && *calls > 1
            {
                return Err(GenerationReadError::PerformanceGate {
                    capability: name.clone(),
                    detail: format!("backend_calls={calls} violates batch-first hot-path limit=1"),
                });
            }
        }
        Ok(())
    }
}

pub fn hash_serializable(value: &impl Serialize) -> String {
    let payload =
        serde_json::to_vec(value).expect("canonical generation-read value must serialize");
    hex::encode(Sha256::digest(payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refno(value: &str) -> RefnoEnum {
        RefnoEnum::from(value)
    }

    #[test]
    fn manifest_and_payload_corruption_fail_closed() {
        let mut manifest = InputVersionManifest::new(
            12,
            10,
            [DataVersion {
                dbnum: 1,
                sesno: 20,
                commit_fingerprint: "commit-20".to_string(),
            }],
        )
        .expect("manifest");
        manifest.manifest_hash = "corrupt".to_string();
        assert!(matches!(
            manifest.verify_hash(),
            Err(GenerationReadError::ManifestMismatch { .. })
        ));

        let mut structurally_invalid = InputVersionManifest::new(
            12,
            10,
            [DataVersion {
                dbnum: 1,
                sesno: 20,
                commit_fingerprint: "commit-20".to_string(),
            }],
        )
        .expect("manifest");
        structurally_invalid
            .versions
            .get_mut(&1)
            .expect("version")
            .dbnum = 2;
        structurally_invalid.manifest_hash = hash_serializable(&(
            structurally_invalid.authoritative_snapshot_id,
            structurally_invalid.history_start_snapshot,
            &structurally_invalid.versions,
        ));
        assert!(matches!(
            structurally_invalid.verify_hash(),
            Err(GenerationReadError::InvalidManifest(_))
        ));

        let mut attributes = AttributeSet::new(
            refno("1/1"),
            [("NAME".to_string(), AttributeValue::String("A".to_string()))]
                .into_iter()
                .collect(),
        );
        attributes.values.insert(
            "NAME".to_string(),
            AttributeValue::String("tampered".to_string()),
        );
        assert!(matches!(
            attributes.verify(),
            Err(GenerationReadError::PayloadCorrupt { .. })
        ));
    }

    #[test]
    fn attribute_payload_envelope_round_trips_refno_values() {
        let source = refno("10/1");
        let target = refno("10/2");
        let attributes = AttributeSet::new(
            source,
            BTreeMap::from([
                ("OWNER".to_string(), AttributeValue::Ref(target)),
                (
                    "CREFS".to_string(),
                    AttributeValue::RefArray(vec![target, source]),
                ),
            ]),
        );
        let payload = encode_attribute_set_payload(&attributes).expect("encode");
        assert!(payload.starts_with(ATTRIBUTE_PAYLOAD_JSON_MAGIC));
        let decoded = decode_attribute_set_payload(&payload).expect("decode");
        assert_eq!(decoded, attributes);
        decoded.verify().expect("verify");
    }

    #[test]
    fn batch_lookup_reports_explicit_sorted_missing_set() {
        let first = refno("1/1");
        let second = refno("1/2");
        let third = refno("1/3");
        let lookup = BatchLookup::from_found(
            &[third, first, second, third],
            [(second, "found".to_string())],
        );
        assert_eq!(lookup.missing, vec![first, third]);
        assert!(matches!(
            lookup.require_all("fixture"),
            Err(GenerationReadError::MissingRequiredData {
                capability: "fixture",
                ..
            })
        ));
    }

    #[test]
    fn performance_gate_rejects_n_plus_one_and_over_ten_percent_regression() {
        let metrics = SessionMetricsSnapshot {
            backend_calls: [("element.query".to_string(), 2)].into_iter().collect(),
            elapsed_micros: [("attribute.load".to_string(), 1_101)]
                .into_iter()
                .collect(),
            ..SessionMetricsSnapshot::default()
        };
        assert!(metrics.assert_call_limit("element.query", 1).is_err());
        assert!(metrics.assert_batch_first_hot_path().is_err());
        let baseline = SessionMetricsSnapshot {
            elapsed_micros: [("attribute.load".to_string(), 1_000)]
                .into_iter()
                .collect(),
            ..SessionMetricsSnapshot::default()
        };
        assert!(
            metrics
                .assert_elapsed_regression_within(&baseline, "attribute.load", 0.10)
                .is_err()
        );
    }
}
