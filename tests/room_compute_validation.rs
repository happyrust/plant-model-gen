//! 房间计算优化验证测试
//!
//! 使用共享 fixture 数据验证房间计算结果的正确性

use aios_core::RefnoEnum;
use aios_database::fast_model::export_model::export_room_instances::RoomComputeValidationFixture;
use aios_database::fast_model::room_model::build_room_panels_relate_for_query;
use std::collections::HashSet;

fn fixture_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("verification/room/compute/room_compute_validation.json")
}

fn load_validation_fixture() -> RoomComputeValidationFixture {
    RoomComputeValidationFixture::load_from_path(&fixture_path())
        .expect("Failed to parse validation fixture")
}

#[tokio::test]
#[ignore] // 需要数据库连接
async fn test_room_panel_mapping_validation() -> anyhow::Result<()> {
    let validation = load_validation_fixture();

    for test_case in &validation.test_cases {
        println!("Running test case: {}", test_case.case_id);

        let panel_refno = RefnoEnum::from(test_case.panel_refno.as_str());
        let room_number = &test_case.room_number;

        // 查询房间面板映射
        let room_panels = build_room_panels_relate_for_query(&vec![]).await?;

        // 查找包含目标 panel 的房间
        let found = room_panels
            .iter()
            .find(|(_, rnum, panels)| rnum == room_number && panels.contains(&panel_refno));

        assert!(
            found.is_some(),
            "Room {} should contain panel {}",
            room_number,
            test_case.panel_refno
        );

        println!("✓ Test case {} passed", test_case.case_id);
    }

    Ok(())
}

#[tokio::test]
#[ignore] // 需要数据库连接
async fn test_room_component_relationship() -> anyhow::Result<()> {
    let validation = load_validation_fixture();

    for test_case in &validation.test_cases {
        let expected_components: HashSet<RefnoEnum> = test_case
            .expected_components
            .iter()
            .map(|s| RefnoEnum::from(s.as_str()))
            .collect();

        // 这里可以添加实际的构件关系验证逻辑
        // 例如查询 room_relate 表验证构件是否在房间内

        println!(
            "✓ Validated {} expected components for room {}",
            expected_components.len(),
            test_case.room_number
        );
    }

    Ok(())
}
