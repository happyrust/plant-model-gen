use crate::fast_model::mesh_generate::{process_meshes_bran, process_meshes_update_db_deep};
use crate::options::DbOptionExt;
use aios_core::RefnoEnum;
use aios_core::geometry::ShapeInstancesData;
use futures::stream::FuturesUnordered;
use std::sync::Arc;

/// Noun 分类枚举，用于 IndexTree 目标类型处理
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NounCategory {
    /// 使用元件库的 Noun
    Cate,
    /// Loop owner Noun
    LoopOwner,
    /// 基本体 Noun
    Prim,
}

/// 一个db生成模型里，汇总的参考号集合
#[derive(Debug, Clone, Default)]
pub struct DbModelInstRefnos {
    pub bran_hanger_refnos: Arc<Vec<RefnoEnum>>,
    pub use_cate_refnos: Arc<Vec<RefnoEnum>>,
    pub loop_owner_refnos: Arc<Vec<RefnoEnum>>,
    pub prim_refnos: Arc<Vec<RefnoEnum>>,
}

impl DbModelInstRefnos {
    pub async fn execute_gen_inst_meshes(&self, db_option_arc: Option<Arc<DbOptionExt>>) {
        if let Some(db_option) = db_option_arc {
            // BRAN 单独处理，不需要 deep 遍历和布尔运算
            if !self.bran_hanger_refnos.is_empty() {
                println!(
                    "[BRAN] 开始处理 BRAN/HANG 网格: {} 个",
                    self.bran_hanger_refnos.len()
                );
                if let Err(e) =
                    process_meshes_bran(Some(db_option.clone()), &self.bran_hanger_refnos).await
                {
                    eprintln!("process_meshes_bran failed: {:?}", e);
                }
            }

            // 其他类型使用 deep 遍历
            let mut roots = Vec::new();
            roots.extend(self.use_cate_refnos.iter().copied());
            roots.extend(self.loop_owner_refnos.iter().copied());
            roots.extend(self.prim_refnos.iter().copied());

            if roots.is_empty() {
                return;
            }

            if let Err(e) = process_meshes_update_db_deep(&db_option, &roots).await {
                eprintln!("process_meshes_update_db_deep failed: {:?}", e);
            }
        }
    }
}
