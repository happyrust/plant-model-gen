use crate::fast_model::unit_converter::LengthUnit;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportTransformConfig {
    /// 源单位（SurrealDB 存储单位，默认 mm）
    #[serde(default = "default_mm")]
    pub source_unit: LengthUnit,
    /// 目标单位（导出单位，如 mm / m / ft）
    #[serde(default = "default_mm")]
    pub target_unit: LengthUnit,
    /// 是否在导出时做坐标系旋转（Z-up → Y-up, 绕 X 轴 -90°）
    #[serde(default)]
    pub apply_rotation: bool,
    /// 是否在导出时将 trans_hash 解析为内联矩阵
    /// - true: 矩阵内联到每个 instance（兼容 v2 前端）
    /// - false: 矩阵存入顶层 transforms 字典（v3 去重模式）
    #[serde(default)]
    pub inline_matrices: bool,
}

fn default_mm() -> LengthUnit {
    LengthUnit::Millimeter
}

impl Default for ExportTransformConfig {
    fn default() -> Self {
        Self {
            source_unit: LengthUnit::Millimeter,
            target_unit: LengthUnit::Millimeter,
            apply_rotation: false,
            inline_matrices: false,
        }
    }
}

impl ExportTransformConfig {
    pub fn needs_unit_conversion(&self) -> bool {
        self.source_unit != self.target_unit
    }

    pub fn to_manifest_json(&self) -> serde_json::Value {
        serde_json::json!({
            "source_unit": self.source_unit.name(),
            "target_unit": self.target_unit.name(),
            "rotation_applied": self.apply_rotation,
            "matrices_inlined": self.inline_matrices,
        })
    }
}
