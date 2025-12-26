// mod test_cata_expression;
// mod test_cata_hangers;
// mod test_dir;
// pub mod test_helper;
// pub mod common;
// mod test_api;
// mod test_spatial;
mod test_gen_model;
mod test_performance;
// mod test_spatial_index_1112; // 暂时注释,需要修复
// mod test_room_integration; // Removed: uses unavailable aios_core modules
// mod test_room_v2_verification; // Removed: uses unavailable aios_core modules
mod test_room_specific_refno; // 特定 refno 房间测试
// mod test_room_tee_containment; // 房间-三通包含关系测试
mod test_check_frmw_structure; // 检查 FRMW 数据库结构
// mod test_find_valid_room_data; // 查找可用于房间测试的有效数据
#[cfg(feature = "grpc")]
mod test_sctn_contact;
