use std::path::{Path, PathBuf};

use anyhow::Context;
use duckdb::Connection;

#[derive(Debug, Clone, PartialEq)]
pub struct PositionSimulationResult {
    pub moved_refno: String,
    pub delta_mm: [f64; 3],
}

pub fn create_position_shifted_artifact(
    source_dir: &Path,
    target_dir: &Path,
    root_refno: &str,
    source_sesno: u32,
    target_sesno: u32,
    component_refno: Option<&str>,
    delta_mm: [f64; 3],
) -> anyhow::Result<PositionSimulationResult> {
    anyhow::ensure!(
        delta_mm.iter().all(|value| value.is_finite()),
        "position simulation delta must be finite"
    );
    anyhow::ensure!(
        delta_mm.iter().any(|value| *value != 0.0),
        "position simulation delta must not be zero"
    );
    anyhow::ensure!(
        source_dir.is_dir(),
        "source artifact does not exist: {}",
        source_dir.display()
    );
    anyhow::ensure!(
        !target_dir.exists(),
        "target artifact already exists: {}",
        target_dir.display()
    );

    let root_refno = normalize_refno(root_refno);
    let source_manifest_path = source_dir.join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&source_manifest_path).with_context(|| {
            format!("read source manifest: {}", source_manifest_path.display())
        })?)
        .with_context(|| format!("parse source manifest: {}", source_manifest_path.display()))?;
    anyhow::ensure!(
        manifest
            .get("root_refno")
            .and_then(serde_json::Value::as_str)
            == Some(root_refno.as_str()),
        "source manifest root_refno does not match: expected={root_refno}"
    );

    let instances_path = required_table_path(source_dir, "instances.parquet")?;
    let transforms_path = required_table_path(source_dir, "transforms.parquet")?;
    let aabb_path = required_table_path(source_dir, "aabb.parquet")?;
    let connection = Connection::open_in_memory()?;
    let requested_component = component_refno.map(normalize_refno);
    if requested_component.as_deref() == Some(root_refno.as_str()) {
        anyhow::bail!("position simulation requires a non-root component");
    }
    let component_filter = requested_component
        .as_ref()
        .map(|value| format!("AND refno_str = '{}'", sql_string(value)))
        .unwrap_or_default();
    let component_sql = format!(
        "SELECT refno_str, trans_hash, aabb_hash
         FROM read_parquet('{}')
         WHERE refno_str <> '{}'
           AND trans_hash IS NOT NULL
           AND trans_hash <> ''
           AND aabb_hash IS NOT NULL
           AND aabb_hash <> ''
           {component_filter}
         ORDER BY refno_str
         LIMIT 1",
        sql_path(&instances_path),
        sql_string(&root_refno),
    );
    let (moved_refno, old_transform_hash, old_aabb_hash): (String, String, String) = connection
        .query_row(&component_sql, [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .with_context(|| match requested_component {
            Some(ref value) => format!("component {value} is not movable in source artifact"),
            None => "source artifact has no movable non-root component".to_string(),
        })?;

    let new_transform_hash = format!("sim-{target_sesno}-{moved_refno}-transform");
    let new_aabb_hash = format!("sim-{target_sesno}-{moved_refno}-aabb");
    ensure_single_source_row(
        &connection,
        &transforms_path,
        "trans_hash",
        &old_transform_hash,
    )?;
    ensure_single_source_row(&connection, &aabb_path, "aabb_hash", &old_aabb_hash)?;

    let target_parent = target_dir
        .parent()
        .ok_or_else(|| anyhow::anyhow!("target artifact has no parent directory"))?;
    std::fs::create_dir_all(target_parent)?;
    let target_name = target_dir
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| anyhow::anyhow!("target artifact has an invalid directory name"))?;
    let staging_dir =
        target_parent.join(format!(".{target_name}.simulating-{}", std::process::id()));
    anyhow::ensure!(
        !staging_dir.exists(),
        "simulation staging directory already exists: {}",
        staging_dir.display()
    );
    std::fs::create_dir(&staging_dir)?;

    let result = write_shifted_artifact(
        &connection,
        source_dir,
        &staging_dir,
        &instances_path,
        &transforms_path,
        &aabb_path,
        source_sesno,
        target_sesno,
        &moved_refno,
        &old_transform_hash,
        &old_aabb_hash,
        &new_transform_hash,
        &new_aabb_hash,
        delta_mm,
        &mut manifest,
    );
    if let Err(error) = result {
        let _ = std::fs::remove_dir_all(&staging_dir);
        return Err(error);
    }
    std::fs::rename(&staging_dir, target_dir).with_context(|| {
        format!(
            "publish simulated artifact: {} -> {}",
            staging_dir.display(),
            target_dir.display()
        )
    })?;

    Ok(PositionSimulationResult {
        moved_refno,
        delta_mm,
    })
}

#[allow(clippy::too_many_arguments)]
fn write_shifted_artifact(
    connection: &Connection,
    source_dir: &Path,
    staging_dir: &Path,
    instances_path: &Path,
    transforms_path: &Path,
    aabb_path: &Path,
    source_sesno: u32,
    target_sesno: u32,
    moved_refno: &str,
    old_transform_hash: &str,
    old_aabb_hash: &str,
    new_transform_hash: &str,
    new_aabb_hash: &str,
    delta_mm: [f64; 3],
    manifest: &mut serde_json::Value,
) -> anyhow::Result<()> {
    copy_unchanged_artifact_files(source_dir, staging_dir)?;
    let [dx, dy, dz] = delta_mm.map(sql_number);
    connection.execute_batch(&format!(
        "COPY (
            SELECT * REPLACE (
                CASE WHEN refno_str = '{moved}' THEN '{new_transform}' ELSE trans_hash END AS trans_hash,
                CASE WHEN refno_str = '{moved}' THEN '{new_aabb}' ELSE aabb_hash END AS aabb_hash
            ) FROM read_parquet('{source_instances}')
         ) TO '{target_instances}' (FORMAT PARQUET);
         COPY (
            SELECT * FROM read_parquet('{source_transforms}')
            UNION ALL
            SELECT * REPLACE (
                '{new_transform}' AS trans_hash,
                m03 + {dx} AS m03,
                m13 + {dy} AS m13,
                m23 + {dz} AS m23
            ) FROM read_parquet('{source_transforms}')
            WHERE trans_hash = '{old_transform}'
         ) TO '{target_transforms}' (FORMAT PARQUET);
         COPY (
            SELECT * FROM read_parquet('{source_aabb}')
            UNION ALL
            SELECT * REPLACE (
                '{new_aabb}' AS aabb_hash,
                min_x + {dx} AS min_x,
                min_y + {dy} AS min_y,
                min_z + {dz} AS min_z,
                max_x + {dx} AS max_x,
                max_y + {dy} AS max_y,
                max_z + {dz} AS max_z
            ) FROM read_parquet('{source_aabb}')
            WHERE aabb_hash = '{old_aabb}'
         ) TO '{target_aabb}' (FORMAT PARQUET);",
        moved = sql_string(moved_refno),
        new_transform = sql_string(new_transform_hash),
        new_aabb = sql_string(new_aabb_hash),
        old_transform = sql_string(old_transform_hash),
        old_aabb = sql_string(old_aabb_hash),
        source_instances = sql_path(instances_path),
        target_instances = sql_path(&staging_dir.join("instances.parquet")),
        source_transforms = sql_path(transforms_path),
        target_transforms = sql_path(&staging_dir.join("transforms.parquet")),
        source_aabb = sql_path(aabb_path),
        target_aabb = sql_path(&staging_dir.join("aabb.parquet")),
    ))?;

    increment_manifest_rows(manifest, "transforms")?;
    increment_manifest_rows(manifest, "aabb")?;
    manifest["generated_at"] = serde_json::Value::String(chrono::Utc::now().to_rfc3339());
    manifest["simulation"] = serde_json::json!({
        "kind": "position_shift",
        "source_sesno": source_sesno,
        "source_artifact_sesno": source_dir.file_name().and_then(|value| value.to_str()).and_then(|value| value.parse::<u32>().ok()),
        "target_sesno": target_sesno,
        "moved_refno": moved_refno,
        "delta_mm": delta_mm,
    });
    manifest["total_bytes"] = serde_json::Value::from(total_file_bytes(staging_dir)?);
    std::fs::write(
        staging_dir.join("manifest.json"),
        serde_json::to_vec_pretty(manifest)?,
    )?;
    Ok(())
}

fn copy_unchanged_artifact_files(source_dir: &Path, target_dir: &Path) -> anyhow::Result<()> {
    const REGENERATED: [&str; 4] = [
        "instances.parquet",
        "transforms.parquet",
        "aabb.parquet",
        "manifest.json",
    ];
    for entry in std::fs::read_dir(source_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        if REGENERATED.iter().any(|candidate| name == *candidate) {
            continue;
        }
        std::fs::copy(entry.path(), target_dir.join(name))?;
    }
    Ok(())
}

fn required_table_path(source_dir: &Path, file_name: &str) -> anyhow::Result<PathBuf> {
    let path = source_dir.join(file_name);
    anyhow::ensure!(
        path.is_file(),
        "source artifact is missing {file_name}: {}",
        path.display()
    );
    Ok(path)
}

fn ensure_single_source_row(
    connection: &Connection,
    parquet_path: &Path,
    column: &str,
    value: &str,
) -> anyhow::Result<()> {
    let count: i64 = connection.query_row(
        &format!(
            "SELECT count(*) FROM read_parquet('{}') WHERE {column} = '{}'",
            sql_path(parquet_path),
            sql_string(value)
        ),
        [],
        |row| row.get(0),
    )?;
    anyhow::ensure!(
        count == 1,
        "expected exactly one {column}={value} row, found {count}"
    );
    Ok(())
}

fn increment_manifest_rows(manifest: &mut serde_json::Value, table: &str) -> anyhow::Result<()> {
    let pointer = format!("/tables/{table}/rows");
    let rows = manifest
        .pointer(&pointer)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| anyhow::anyhow!("source manifest is missing {pointer}"))?;
    *manifest
        .pointer_mut(&pointer)
        .ok_or_else(|| anyhow::anyhow!("source manifest is missing {pointer}"))? =
        serde_json::Value::from(rows + 1);
    Ok(())
}

fn total_file_bytes(dir: &Path) -> anyhow::Result<u64> {
    std::fs::read_dir(dir)?.try_fold(0_u64, |total, entry| {
        let entry = entry?;
        Ok(total
            + if entry.file_type()?.is_file() {
                entry.metadata()?.len()
            } else {
                0
            })
    })
}

fn normalize_refno(value: &str) -> String {
    value.trim().replace('/', "_")
}

fn sql_path(path: &Path) -> String {
    sql_string(&path.to_string_lossy().replace('\\', "/"))
}

fn sql_string(value: &str) -> String {
    value.replace('\'', "''")
}

fn sql_number(value: f64) -> String {
    format!("{value:.17}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use duckdb::Connection;

    #[test]
    fn creates_a_new_artifact_with_one_component_position_shifted() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("791");
        let target = temp.path().join("897");
        std::fs::create_dir_all(&source).unwrap();
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(&format!(
            "COPY (SELECT * FROM (VALUES
                ('24381_145018', 1::UBIGINT, 'BRAN', NULL::VARCHAR, NULL::UBIGINT, '', NULL::VARCHAR, 'tr-root', 'bb-root', 0::BIGINT, false, false, 7997::UINTEGER),
                ('24381_145019', 2::UBIGINT, 'ELBO', '24381_145018', 1::UBIGINT, 'BRAN', NULL::VARCHAR, 'tr-child', 'bb-child', 0::BIGINT, false, false, 7997::UINTEGER)
            ) t(refno_str, refno_u64, noun, owner_refno_str, owner_refno_u64, owner_noun, cata_hash, trans_hash, aabb_hash, spec_value, spec_info_fallback, has_neg, dbnum))
            TO '{instances}' (FORMAT PARQUET);
            COPY (SELECT * FROM (VALUES
                ('tr-root', 1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 10.0,20.0,30.0,1.0),
                ('tr-child',1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 25.0,35.0,45.0,1.0)
            ) t(trans_hash,m00,m10,m20,m30,m01,m11,m21,m31,m02,m12,m22,m32,m03,m13,m23,m33))
            TO '{transforms}' (FORMAT PARQUET);
            COPY (SELECT * FROM (VALUES
                ('bb-root',0.0,0.0,0.0,10.0,10.0,10.0),
                ('bb-child',20.0,30.0,40.0,30.0,40.0,50.0)
            ) t(aabb_hash,min_x,min_y,min_z,max_x,max_y,max_z))
            TO '{aabb}' (FORMAT PARQUET);",
            instances = sql_path(&source.join("instances.parquet")),
            transforms = sql_path(&source.join("transforms.parquet")),
            aabb = sql_path(&source.join("aabb.parquet")),
        )).unwrap();
        std::fs::write(source.join("geo_instances.parquet"), b"geo").unwrap();
        std::fs::write(source.join("tubings.parquet"), b"tubi").unwrap();
        std::fs::write(
            source.join("manifest.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "version": 1,
                "format": "parquet",
                "generated_at": "2026-07-21T00:00:00Z",
                "dbnum": 7997,
                "root_refno": "24381_145018",
                "tables": {
                    "instances": {"file": "instances.parquet", "rows": 2},
                    "geo_instances": {"file": "geo_instances.parquet", "rows": 1},
                    "tubings": {"file": "tubings.parquet", "rows": 1},
                    "transforms": {"file": "transforms.parquet", "rows": 2},
                    "aabb": {"file": "aabb.parquet", "rows": 2}
                },
                "total_bytes": 0
            }))
            .unwrap(),
        )
        .unwrap();

        let result = create_position_shifted_artifact(
            &source,
            &target,
            "24381_145018",
            791,
            897,
            Some("24381_145019"),
            [100.0, -20.0, 5.0],
        )
        .unwrap();

        assert_eq!(result.moved_refno, "24381_145019");
        let moved: (String, f64, f64, f64) = connection.query_row(
            &format!("SELECT i.trans_hash, t.m03, t.m13, t.m23 FROM read_parquet('{}') i JOIN read_parquet('{}') t USING (trans_hash) WHERE i.refno_str='24381_145019'", sql_path(&target.join("instances.parquet")), sql_path(&target.join("transforms.parquet"))),
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ).unwrap();
        assert_eq!(moved.0, "sim-897-24381_145019-transform");
        assert_eq!([moved.1, moved.2, moved.3], [125.0, 15.0, 50.0]);
        let moved_min: (f64, f64, f64) = connection.query_row(
            &format!("SELECT a.min_x, a.min_y, a.min_z FROM read_parquet('{}') i JOIN read_parquet('{}') a USING (aabb_hash) WHERE i.refno_str='24381_145019'", sql_path(&target.join("instances.parquet")), sql_path(&target.join("aabb.parquet"))),
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap();
        assert_eq!(moved_min, (120.0, 10.0, 45.0));
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(target.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(
            manifest
                .pointer("/simulation/target_sesno")
                .and_then(|v| v.as_u64()),
            Some(897)
        );
        assert_eq!(
            manifest
                .pointer("/simulation/source_artifact_sesno")
                .and_then(|v| v.as_u64()),
            Some(791)
        );
        assert_eq!(
            manifest
                .pointer("/simulation/source_sesno")
                .and_then(|v| v.as_u64()),
            Some(791)
        );
        assert_eq!(
            manifest
                .pointer("/simulation/moved_refno")
                .and_then(|v| v.as_str()),
            Some("24381_145019")
        );
    }
}
