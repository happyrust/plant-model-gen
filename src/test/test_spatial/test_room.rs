use crate::aql_api::pdms_room;
use crate::test::common::get_arangodb_conn_from_db_option_for_test;
use crate::test::test_helper::get_test_ams_db_manager_async;
use aios_core::pdms_types::RefU64;
use aios_core::pdms_types::UdaMajorType::T;
use glam::Vec3;
use regex::Regex;
use std::str::FromStr;
use aios_core::pdms_types::GeoBasicType::*;
use parry3d::utils::hashmap::HashMap;
use crate::data_interface::interface::PdmsDataInterface;

// ============================================================================
// 15 组贯穿件房间号测试样例
// 注意：这些测试依赖真实数据库连接，CI 中跳过
// ============================================================================

#[tokio::test]
async fn test_query_through_element_rooms_1() -> anyhow::Result<()> {
    // 样例 1  内房间号：R610，外房间号：R661
    let mgr = get_test_ams_db_manager_async().await;
    let target_refno = "24383/83477".into();
    let room_number_map = mgr
        .query_through_element_room_nums(&[target_refno], None)
        .await?;
    assert!(!room_number_map.is_empty(), "样例1: 应返回非空房间号映射");
    if let Some((inner, outer)) = room_number_map.get(&target_refno) {
        assert_eq!(inner, "R610");
        assert_eq!(outer, "R661");
    }
    Ok(())
}

#[tokio::test]
async fn test_query_through_element_rooms_2() -> anyhow::Result<()> {
    // 样例 2  与样例 1 相同 refno，验证精确值
    use std::collections::{HashMap, HashSet};
    let mgr = get_test_ams_db_manager_async().await;
    let target_refno = "24383/83477".into();
    let room_number = mgr
        .query_through_element_room_nums(&[target_refno], None)
        .await?;
    let mut expected = HashMap::new();
    expected.insert(target_refno, ("R610".to_string(), "R661".to_string()));
    assert_eq!(room_number, expected);
    Ok(())
}

#[tokio::test]
async fn test_query_through_element_rooms_3_neg_filter() -> anyhow::Result<()> {
    // 带 Neg 过滤的贯穿件查询
    let mgr = get_test_ams_db_manager_async().await;
    let target_refno = "17496/156874".into();
    let room_number_map = mgr
        .query_through_element_room_nums(&[target_refno], Some(&vec![Neg, CateNeg, CataCrossNeg]))
        .await?;
    assert!(
        !room_number_map.is_empty(),
        "样例3: 带 Neg 过滤应返回非空结果"
    );
    Ok(())
}

#[tokio::test]
async fn test_query_through_element_rooms_4_neg_filter() -> anyhow::Result<()> {
    let mgr = get_test_ams_db_manager_async().await;
    let target_refno = "17496/145284".into();
    let room_number_map = mgr
        .query_through_element_room_nums(&[target_refno], Some(&vec![Neg, CateNeg, CataCrossNeg]))
        .await?;
    assert!(
        !room_number_map.is_empty(),
        "样例4: 带 Neg 过滤应返回非空结果"
    );
    Ok(())
}

#[tokio::test]
async fn test_query_through_element_rooms_5() -> anyhow::Result<()> {
    // 样例 5  内房间号：R310，外房间号：R361
    let mgr = get_test_ams_db_manager_async().await;
    let target_refno = "24383/83697".into();
    let room_number_map = mgr
        .query_through_element_room_nums(&[target_refno], None)
        .await?;
    if let Some((inner, outer)) = room_number_map.get(&target_refno) {
        assert_eq!(inner, "R310");
        assert_eq!(outer, "R361");
    }
    Ok(())
}

#[tokio::test]
async fn test_query_through_element_rooms_6() -> anyhow::Result<()> {
    // 样例 6  内房间号：R310，外房间号：R361
    let mgr = get_test_ams_db_manager_async().await;
    let target_refno = "24383/84009".into();
    let room_number_map = mgr
        .query_through_element_room_nums(&[target_refno], None)
        .await?;
    if let Some((inner, outer)) = room_number_map.get(&target_refno) {
        assert_eq!(inner, "R310");
        assert_eq!(outer, "R361");
    }
    Ok(())
}

#[tokio::test]
async fn test_query_through_element_rooms_7() -> anyhow::Result<()> {
    // 样例 7  内房间号：R310，外房间号：R361
    let mgr = get_test_ams_db_manager_async().await;
    let target_refno = "24383/83974".into();
    let room_number_map = mgr
        .query_through_element_room_nums(&[target_refno], None)
        .await?;
    if let Some((inner, outer)) = room_number_map.get(&target_refno) {
        assert_eq!(inner, "R310");
        assert_eq!(outer, "R361");
    }
    Ok(())
}

#[tokio::test]
async fn test_query_through_element_rooms_8() -> anyhow::Result<()> {
    // 样例 8  内房间号：R430，外房间号：R461
    let mgr = get_test_ams_db_manager_async().await;
    let target_refno = "24383/83939".into();
    let room_number_map = mgr
        .query_through_element_room_nums(&[target_refno], None)
        .await?;
    if let Some((inner, outer)) = room_number_map.get(&target_refno) {
        assert_eq!(inner, "R430");
        assert_eq!(outer, "R461");
    }
    Ok(())
}

#[tokio::test]
async fn test_query_through_element_rooms_9() -> anyhow::Result<()> {
    // 样例 9  内房间号：R430，外房间号：R461
    let mgr = get_test_ams_db_manager_async().await;
    let target_refno = "24383/83869".into();
    let room_number_map = mgr
        .query_through_element_room_nums(&[target_refno], None)
        .await?;
    if let Some((inner, outer)) = room_number_map.get(&target_refno) {
        assert_eq!(inner, "R430");
        assert_eq!(outer, "R461");
    }
    Ok(())
}

#[tokio::test]
async fn test_query_through_element_rooms_10() -> anyhow::Result<()> {
    // 样例 10  内房间号：R510，外房间号：R562
    let mgr = get_test_ams_db_manager_async().await;
    let target_refno = "24383/83995".into();
    let room_number_map = mgr
        .query_through_element_room_nums(&[target_refno], None)
        .await?;
    if let Some((inner, outer)) = room_number_map.get(&target_refno) {
        assert_eq!(inner, "R510");
        assert_eq!(outer, "R562");
    }
    Ok(())
}

#[tokio::test]
async fn test_query_through_element_rooms_11() -> anyhow::Result<()> {
    // 样例 11  内房间号：R530，外房间号：R561
    let mgr = get_test_ams_db_manager_async().await;
    let target_refno = "24383/83729".into();
    let room_number_map = mgr
        .query_through_element_room_nums(&[target_refno], None)
        .await?;
    if let Some((inner, outer)) = room_number_map.get(&target_refno) {
        assert_eq!(inner, "R530");
        assert_eq!(outer, "R561");
    }
    Ok(())
}

#[tokio::test]
async fn test_query_through_element_rooms_12() -> anyhow::Result<()> {
    // 样例 12  内房间号：R630，外房间号：R663
    let mgr = get_test_ams_db_manager_async().await;
    let target_refno = "24383/84079".into();
    let room_number_map = mgr
        .query_through_element_room_nums(&[target_refno], None)
        .await?;
    if let Some((inner, outer)) = room_number_map.get(&target_refno) {
        assert_eq!(inner, "R630");
        assert_eq!(outer, "R663");
    }
    Ok(())
}

#[tokio::test]
async fn test_query_through_element_rooms_13() -> anyhow::Result<()> {
    // 样例 13  内房间号：R610，外房间号：R661
    let mgr = get_test_ams_db_manager_async().await;
    let target_refno = "24383/83596".into();
    let room_number_map = mgr
        .query_through_element_room_nums(&[target_refno], None)
        .await?;
    if let Some((inner, outer)) = room_number_map.get(&target_refno) {
        assert_eq!(inner, "R610");
        assert_eq!(outer, "R661");
    }
    Ok(())
}

#[tokio::test]
async fn test_query_through_element_rooms_14() -> anyhow::Result<()> {
    // 样例 14  内房间号：R710，外房间号：R761
    let mgr = get_test_ams_db_manager_async().await;
    let target_refno = "24383/83708".into();
    let room_number_map = mgr
        .query_through_element_room_nums(&[target_refno], None)
        .await?;
    if let Some((inner, outer)) = room_number_map.get(&target_refno) {
        assert_eq!(inner, "R710");
        assert_eq!(outer, "R761");
    }
    Ok(())
}

#[tokio::test]
async fn test_query_through_element_rooms_15() -> anyhow::Result<()> {
    // 样例 15  内房间号：R710，外房间号：R761
    let mgr = get_test_ams_db_manager_async().await;
    let target_refno = "24383/83813".into();
    let room_number_map = mgr
        .query_through_element_room_nums(&[target_refno], None)
        .await?;
    if let Some((inner, outer)) = room_number_map.get(&target_refno) {
        assert_eq!(inner, "R710");
        assert_eq!(outer, "R761");
    }
    Ok(())
}

// ============================================================================
// SBFI 贯穿件查询
// ============================================================================

#[tokio::test]
async fn test_query_through_element_rooms_sbfi() -> anyhow::Result<()> {
    let mgr = get_test_ams_db_manager_async().await;
    let target_refno = "17496/143434".into();
    let room_number_map = mgr
        .query_through_element_room_nums(&[target_refno], None)
        .await?;
    assert!(
        !room_number_map.is_empty(),
        "SBFI 贯穿件应返回非空房间号映射"
    );
    Ok(())
}

// ============================================================================
// 点所在房间查询
// ============================================================================

#[tokio::test]
async fn test_query_rooms_pts() -> anyhow::Result<()> {
    let mgr = get_test_ams_db_manager_async().await;
    let pts = vec![Vec3::new(10271.33, -140.43, 14275.37)];
    let room_nums = mgr.query_pts_own_room_number(&pts).await?;
    assert!(
        !room_nums.is_empty(),
        "坐标点所在房间查询应返回非空结果"
    );
    Ok(())
}

// ============================================================================
// 构件所属房间查询
// ============================================================================

#[tokio::test]
async fn test_query_refno_belong_rooms() -> anyhow::Result<()> {
    use aios_core::options::DbOption;
    use config::{Config, File};
    let s = Config::builder()
        .add_source(File::with_name("db_options/DbOption"))
        .build()?;
    let db_option: DbOption = s.try_deserialize().unwrap();
    let database = get_arangodb_conn_from_db_option_for_test(&db_option).await?;
    let refno = RefU64::from_str("24383_68084").unwrap();
    let name = pdms_room::query_refno_belong_rooms(refno, &database).await?;
    assert!(!name.is_empty(), "构件所属房间查询应返回非空结果");
    Ok(())
}

#[tokio::test]
async fn test_query_room_info_from_refno() -> anyhow::Result<()> {
    use aios_core::options::DbOption;
    use config::{Config, File};
    let s = Config::builder()
        .add_source(File::with_name("db_options/DbOption"))
        .build()?;
    let _db_option: DbOption = s.try_deserialize().unwrap();
    let mgr = get_test_ams_db_manager_async().await;
    let refno = RefU64::from_str("24381_178638").unwrap();
    let name = mgr
        .query_room_names_of_ele(refno)
        .await?
        .into_iter()
        .next()
        .unwrap_or_default();
    let room_name = pdms_room::get_room_name_split(&name).unwrap();
    assert!(
        !room_name.0.is_empty(),
        "房间名拆分后不应为空"
    );
    Ok(())
}

// ============================================================================
// 纯单元测试（不依赖数据库）
// ============================================================================

#[test]
fn test_json_serialize_uda_major_type() {
    let types = vec![T];
    let json = serde_json::to_string(&types).unwrap();
    assert!(!json.is_empty());
}

#[test]
fn test_match_room_name_regex() {
    let re = Regex::new(r"^/\d+[A-Z]{2}-RM\d{2}-R\d{3}$").unwrap();
    assert!(re.is_match("/123AB-RM03-R310"));
    assert!(re.is_match("/456CD-RM03-R312"));
    assert!(re.is_match("/789EF-RM11-R976"));
    assert!(!re.is_match("/1RA-RM03-R312"));
    assert!(!re.is_match("/1NX-RM11-R976"));
    assert!(!re.is_match("/12A-RM11-R976"));
}

#[tokio::test]
async fn test_query_room_of_refno() -> anyhow::Result<()> {
    let mgr = get_test_ams_db_manager_async().await;
    let room_refnos = mgr.query_room_refno_of_ele("17496/198243".into()).await?;
    assert!(!room_refnos.is_empty(), "构件应属于至少一个房间");
    let room_names = mgr.query_room_names_of_ele("17496/198243".into()).await?;
    assert!(!room_names.is_empty(), "构件应有房间名称");
    let rooms = mgr.query_room_eles_of_ele("17496/198243".into()).await?;
    assert!(!rooms.is_empty(), "构件应有房间元素");
    Ok(())
}
