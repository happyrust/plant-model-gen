//! 测试 export_dbnum_instances_json 函数

use std::path::PathBuf;
use std::sync::Arc;

#[tokio::test]
async fn test_export_dbnum_instances_json_1112() {
    // 初始化测试数据库
    aios_core::init_surreal().await.unwrap();

    let dbnum = 1112;
    let output_dir = PathBuf::from("output/test/instances");
    let db_option_ext = crate::options::get_db_option_ext_from_path("DbOption").unwrap();
    let db_option = Arc::new(db_option_ext.inner.clone());

    // 调用导出函数
    let result: anyhow::Result<crate::fast_model::export_model::model_exporter::ExportStats> =
        crate::fast_model::export_model::export_prepack_lod::export_dbnum_instances_json(
            dbnum,
            &output_dir,
            db_option,
            false, // verbose
            None,  // 使用默认毫米单位
        )
        .await;

    assert!(result.is_ok(), "导出应该成功");

    let stats = result.unwrap();
    assert!(stats.refno_count > 0, "应该有导出的 refno");

    // 验证生成的 JSON 文件
    let json_path = output_dir.join(format!("instances_{}.json", dbnum));
    assert!(json_path.exists(), "JSON 文件应该存在");

    // 读取并验证 JSON 格式
    let json_content = std::fs::read_to_string(&json_path).unwrap();
    let json_value: serde_json::Value = serde_json::from_str(&json_content).unwrap();

    // 验证基本结构
    assert_eq!(json_value["version"], 2);
    assert!(json_value["groups"].is_array());

    // 验证没有 colors 数组
    if let Some(_colors) = json_value.get("colors") {
        panic!("不应该有 colors 数组");
    }

    // 验证 groups 结构
    if let Some(groups) = json_value["groups"].as_array() {
        if let Some(first_group) = groups.first() {
            assert!(first_group["owner_refno"].is_string());
            assert!(first_group["owner_noun"].is_string());

            // 验证 children 有 aabb 字段
            if let Some(children) = first_group["children"].as_array() {
                if let Some(first_child) = children.first() {
                    assert!(first_child["aabb"].is_object(), "child 应该有 aabb 字段");

                    // 验证没有 color_index
                    if let Some(_color_index) = first_child.get("color_index") {
                        panic!("不应该有 color_index");
                    }
                }
            }

            // 验证 tubings 有 aabb 字段
            if let Some(tubings) = first_group["tubings"].as_array() {
                if let Some(first_tubi) = tubings.first() {
                    assert!(first_tubi["aabb"].is_object(), "tubi 应该有 aabb 字段");

                    // 验证没有 name_index
                    if let Some(_name_index) = first_tubi.get("name_index") {
                        panic!("不应该有 name_index");
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn test_export_dbnum_instances_json_aabb_format() {
    // 测试 AABB 格式正确性
    aios_core::init_surreal().await.unwrap();

    let dbnum = 1112;
    let output_dir = PathBuf::from("output/test/instances_aabb");
    let db_option_ext = crate::options::get_db_option_ext_from_path("DbOption").unwrap();
    let db_option = Arc::new(db_option_ext.inner.clone());

    let _stats: crate::fast_model::export_model::model_exporter::ExportStats =
        crate::fast_model::export_model::export_prepack_lod::export_dbnum_instances_json(
            dbnum,
            &output_dir,
            db_option,
            false,
            None, // 使用默认毫米单位
        )
        .await
        .unwrap();

    let json_path = output_dir.join(format!("instances_{}.json", dbnum));
    let json_content = std::fs::read_to_string(&json_path).unwrap();
    let json_value: serde_json::Value = serde_json::from_str(&json_content).unwrap();

    // 验证 AABB 格式 { "min": [x, y, z], "max": [x, y, z] }
    if let Some(groups) = json_value["groups"].as_array() {
        for group in groups {
            // 验证 owner_aabb
            if let Some(owner_aabb) = group.get("owner_aabb").and_then(|v| v.as_object()) {
                assert!(owner_aabb.contains_key("min"));
                assert!(owner_aabb.contains_key("max"));

                if let Some(min) = owner_aabb["min"].as_array() {
                    assert_eq!(min.len(), 3, "min 应该有 3 个元素");
                }
                if let Some(max) = owner_aabb["max"].as_array() {
                    assert_eq!(max.len(), 3, "max 应该有 3 个元素");
                }
            }

            // 验证 children 的 AABB
            if let Some(children) = group["children"].as_array() {
                for child in children {
                    if let Some(aabb) = child.get("aabb").and_then(|v| v.as_object()) {
                        assert!(aabb.contains_key("min"));
                        assert!(aabb.contains_key("max"));

                        if let Some(min) = aabb["min"].as_array() {
                            assert_eq!(min.len(), 3, "min 应该有 3 个元素");
                        }
                        if let Some(max) = aabb["max"].as_array() {
                            assert_eq!(max.len(), 3, "max 应该有 3 个元素");
                        }
                    }
                }
            }

            // 验证 tubings 的 AABB
            if let Some(tubings) = group["tubings"].as_array() {
                for tubi in tubings {
                    if let Some(aabb) = tubi.get("aabb").and_then(|v| v.as_object()) {
                        assert!(aabb.contains_key("min"));
                        assert!(aabb.contains_key("max"));

                        if let Some(min) = aabb["min"].as_array() {
                            assert_eq!(min.len(), 3, "min 应该有 3 个元素");
                        }
                        if let Some(max) = aabb["max"].as_array() {
                            assert_eq!(max.len(), 3, "max 应该有 3 个元素");
                        }
                    }
                }
            }
        }
    }
}
