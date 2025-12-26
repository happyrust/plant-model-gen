use std::collections::HashMap;

/// 简化的颜色调色板，用于 Parquet 导出
pub struct SimpleColorPalette {
    colors: Vec<[f32; 4]>,
    index_map: HashMap<String, usize>,
}

impl SimpleColorPalette {
    pub fn new() -> Self {
        Self {
            colors: Vec::new(),
            index_map: HashMap::new(),
        }
    }

    pub fn index_for_noun(&mut self, noun: &str) -> i32 {
        let key = noun.to_ascii_uppercase();
        if let Some(idx) = self.index_map.get(&key) {
            return *idx as i32;
        }

        let color = self.color_for_noun(&key);
        let idx = self.colors.len();
        self.colors.push(color);
        self.index_map.insert(key, idx);
        idx as i32
    }

    pub fn into_colors(mut self) -> Vec<[f32; 4]> {
        if self.colors.is_empty() {
            self.colors.push([0.82, 0.83, 0.84, 1.0]);
        }
        self.colors
    }

    /// 内置的完整颜色映射表（与 rs-core/rs-plant3-d 保持一致）
    fn color_for_noun(&self, noun: &str) -> [f32; 4] {
        let color_u8: [u8; 4] = match noun {
            // 标准 PDMS 类型
            "UNKOWN" => [192, 192, 192, 255],
            "CE" => [0, 100, 200, 180],
            "EQUI" => [255, 190, 0, 255],
            "PIPE" => [255, 255, 0, 255],
            "HANG" => [255, 126, 0, 255],
            "STRU" => [0, 150, 255, 255],
            "SCTN" => [188, 141, 125, 255],
            "GENSEC" => [188, 141, 125, 255],
            "WALL" => [150, 150, 150, 255],
            "STWALL" => [150, 150, 150, 255],
            "CWALL" => [120, 120, 120, 255],
            "GWALL" => [173, 216, 230, 128],
            "FLOOR" => [210, 180, 140, 255],
            "CFLOOR" => [160, 130, 100, 255],
            "PANE" => [220, 220, 220, 255],
            "ROOM" => [144, 238, 144, 100],
            "AREADEF" => [221, 160, 221, 80],
            "HVAC" => [175, 238, 238, 255],
            "EXTR" => [147, 112, 219, 255],
            "REVO" => [138, 43, 226, 255],
            "HANDRA" => [255, 215, 0, 255],
            "CWBRAN" => [255, 140, 0, 255],
            "CTWALL" => [176, 196, 222, 150],
            "DEMOPA" => [255, 69, 0, 255],
            "INSURQ" => [255, 182, 193, 255],
            "STRLNG" => [0, 255, 255, 255],

            // 管道相关类型（继承 PIPE 颜色）
            "BRAN" => [255, 255, 0, 255],      // 分支 - 黄色
            "TUBI" => [255, 255, 0, 255],      // 管道段 - 黄色
            "VALV" => [255, 100, 100, 255],    // 阀门 - 浅红色
            "INST" => [100, 200, 255, 255],    // 仪表 - 浅蓝色
            "ATTA" => [200, 200, 100, 255],    // 附件 - 黄绿色

            // 变换/几何类型
            "TRNS" => [192, 192, 192, 255],    // 变换 - 灰色
            "TMPL" => [180, 180, 180, 255],    // 模板 - 灰色
            "SUBE" => [255, 190, 0, 255],      // 子设备 - 橙黄色（同 EQUI）
            "NOZZ" => [255, 160, 0, 255],      // 喷嘴 - 橙色

            // 结构相关
            "FRMW" => [0, 150, 255, 255],      // 框架 - 蓝色（同 STRU）
            "SBFR" => [0, 150, 255, 255],      // 子框架 - 蓝色
            "STSE" => [188, 141, 125, 255],    // 结构截面 - 棕色（同 SCTN）
            "JOIN" => [100, 100, 200, 255],    // 连接件 - 紫蓝色
            "SJOI" => [100, 100, 200, 255],    // 结构连接 - 紫蓝色
            "PNOD" => [150, 150, 200, 255],    // 节点 - 浅紫色

            // 电气/电缆
            "CWAY" => [255, 165, 0, 255],      // 电缆桥架 - 橙色
            "CTRAY" => [255, 165, 0, 255],     // 电缆托盘 - 橙色

            // 默认颜色（灰色）
            _ => [192, 192, 192, 255],
        };

        [
            color_u8[0] as f32 / 255.0,
            color_u8[1] as f32 / 255.0,
            color_u8[2] as f32 / 255.0,
            color_u8[3] as f32 / 255.0,
        ]
    }
}
