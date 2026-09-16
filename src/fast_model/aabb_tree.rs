/// 手动更新所有 inst_relate 的 AABB 包围盒
///
/// 已废弃：`update_inst_relate_aabbs_by_refnos` 已从 rs-core 移除，
/// AABB 更新由 scene_node 等机制替代。
///
/// # 参数
///
/// * `_replace_exist` - 是否替换已存在的包围盒数据（已忽略）
///
/// # 返回值
///
/// 返回 `anyhow::Result<()>`，当前会返回错误表示功能已移除
pub async fn manual_update_aabbs(_replace_exist: bool) -> anyhow::Result<()> {
    anyhow::bail!(
        "manual_update_aabbs 已废弃：update_inst_relate_aabbs_by_refnos 已移除，AABB 由 scene_node 等替代"
    )
}
