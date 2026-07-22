use std::fs::{File, OpenOptions};
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;

use duckdb::{Connection, OptionalExt, params};
use fs2::FileExt;
use serde::{Deserialize, Serialize};

use super::DuckLakeAuthority;
use super::schema::DUCKLAKE_CATALOG_ALIAS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelUnitImpactKind {
    Mesh,
    Placement,
    Delivery,
    Noop,
    Tombstone,
}

impl ModelUnitImpactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mesh => "mesh",
            Self::Placement => "placement",
            Self::Delivery => "delivery",
            Self::Noop => "noop",
            Self::Tombstone => "tombstone",
        }
    }
}

impl FromStr for ModelUnitImpactKind {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "mesh" => Ok(Self::Mesh),
            "placement" => Ok(Self::Placement),
            "delivery" => Ok(Self::Delivery),
            "noop" => Ok(Self::Noop),
            "tombstone" => Ok(Self::Tombstone),
            other => anyhow::bail!("不支持的模型影响类型: {other}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelUnitCommit {
    pub dbnum: u32,
    pub unit_refno: String,
    pub unit_noun: String,
    pub sesno: u32,
    pub impact_kind: ModelUnitImpactKind,
    /// NoOp 提交复用先前资产；其他提交通常等于 `sesno`。
    pub artifact_sesno: u32,
    pub project_name: String,
    /// 相对于 `output/<project>/` 的 manifest 路径。
    pub manifest_path: String,
    pub generated_at: String,
}

impl ModelUnitCommit {
    pub fn manifest_url(&self) -> Option<String> {
        if self.impact_kind == ModelUnitImpactKind::Tombstone {
            return None;
        }
        Some(format!(
            "/files/output/{}/{}",
            urlencoding::encode(&self.project_name),
            self.manifest_path.replace('\\', "/")
        ))
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.dbnum > 0, "dbnum 必须大于 0");
        anyhow::ensure!(self.sesno > 0, "sesno 必须大于 0");
        anyhow::ensure!(self.artifact_sesno > 0, "artifact_sesno 必须大于 0");
        anyhow::ensure!(
            self.artifact_sesno <= self.sesno,
            "artifact_sesno 不能晚于提交 sesno"
        );
        let mut refno_parts = self.unit_refno.split('_');
        let ref0 = refno_parts.next().unwrap_or_default();
        let ref1 = refno_parts.next().unwrap_or_default();
        anyhow::ensure!(
            !ref0.is_empty()
                && !ref1.is_empty()
                && refno_parts.next().is_none()
                && ref0.parse::<u32>().is_ok_and(|value| value > 0)
                && ref1.parse::<u32>().is_ok(),
            "unit_refno 必须是下划线规范化参考号"
        );
        anyhow::ensure!(
            self.unit_noun == self.unit_noun.trim().to_ascii_uppercase()
                && crate::version_management::model_impact::is_delivery_unit_root_noun(
                    &self.unit_noun
                ),
            "unit_noun 必须是最小交付单元根 BRAN/HANG/EQUI/WALL/FLOOR"
        );
        anyhow::ensure!(
            !self.project_name.trim().is_empty(),
            "project_name 不能为空"
        );
        anyhow::ensure!(
            !self.project_name.contains(['/', '\\']),
            "project_name 不能包含路径分隔符"
        );
        if self.impact_kind == ModelUnitImpactKind::Noop {
            anyhow::ensure!(
                self.artifact_sesno < self.sesno,
                "NoOp 提交必须复用更早的 artifact_sesno"
            );
        }
        if self.impact_kind == ModelUnitImpactKind::Tombstone {
            anyhow::ensure!(
                self.artifact_sesno == self.sesno,
                "Tombstone 提交必须使用当前 sesno 作为兼容 artifact_sesno"
            );
            anyhow::ensure!(
                self.manifest_path.is_empty(),
                "Tombstone 提交不得包含 manifest_path"
            );
        } else {
            validate_relative_manifest_path(&self.manifest_path)?;
        }
        Ok(())
    }

    fn same_payload(&self, other: &Self) -> bool {
        self.dbnum == other.dbnum
            && self.unit_refno == other.unit_refno
            && self.unit_noun == other.unit_noun
            && self.sesno == other.sesno
            && self.impact_kind == other.impact_kind
            && self.artifact_sesno == other.artifact_sesno
            && self.project_name == other.project_name
            && self.manifest_path == other.manifest_path
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelUnitCommitOutcome {
    pub snapshot_id: u64,
    pub commit: ModelUnitCommit,
    pub idempotent: bool,
}

impl DuckLakeAuthority {
    pub fn commit_model_unit(
        &self,
        commit: ModelUnitCommit,
    ) -> anyhow::Result<ModelUnitCommitOutcome> {
        commit.validate()?;
        // DuckLake 数据表不支持 UNIQUE/PRIMARY KEY。用 catalog 旁的 OS advisory
        // lock 串行化所有进程的 check+insert，确保三元组身份不会并发重复写入。
        let _write_lock = ModelUnitWriteLock::acquire(&self.config().metadata_catalog)?;
        let mut connection = self.lock_connection()?;
        if let Some(existing) =
            read_exact(&connection, commit.dbnum, &commit.unit_refno, commit.sesno)?
        {
            anyhow::ensure!(
                existing.same_payload(&commit),
                "模型提交键已存在但内容不同: ({}, {}, {})",
                commit.dbnum,
                commit.unit_refno,
                commit.sesno
            );
            let snapshot_id = snapshot_id_for_commit(&connection, &commit)?;
            return Ok(ModelUnitCommitOutcome {
                snapshot_id,
                commit: existing,
                idempotent: true,
            });
        }

        connection.execute_batch("BEGIN TRANSACTION;")?;
        let result = (|| -> anyhow::Result<()> {
            insert_commit(&connection, &commit)?;
            connection.execute(
                &format!(
                    "CALL {}.set_commit_message(?, ?, extra_info => ?)",
                    DUCKLAKE_CATALOG_ALIAS
                ),
                params![
                    "plant-model-gen",
                    format!(
                        "model unit commit {}/{}/{}",
                        commit.dbnum, commit.unit_refno, commit.sesno
                    ),
                    model_unit_extra_info(&commit)
                ],
            )?;
            Ok(())
        })();
        match result {
            Ok(()) => connection.execute_batch("COMMIT;")?,
            Err(error) => {
                let _ = connection.execute_batch("ROLLBACK;");
                return Err(error);
            }
        }
        let row_count: u64 = connection.query_row(
            "SELECT count(*) FROM model_unit_commit \
             WHERE dbnum = ? AND unit_refno = ? AND sesno = ?",
            params![
                i64::from(commit.dbnum),
                &commit.unit_refno,
                i64::from(commit.sesno)
            ],
            |row| row.get(0),
        )?;
        anyhow::ensure!(
            row_count == 1,
            "模型提交三元组必须唯一: ({}, {}, {}) rows={row_count}",
            commit.dbnum,
            commit.unit_refno,
            commit.sesno
        );
        let snapshot_id = snapshot_id_for_commit(&connection, &commit)?;
        Ok(ModelUnitCommitOutcome {
            snapshot_id,
            commit,
            idempotent: false,
        })
    }

    pub fn model_unit_commit(
        &self,
        dbnum: u32,
        unit_refno: &str,
        sesno: u32,
    ) -> anyhow::Result<Option<ModelUnitCommit>> {
        let connection = self.lock_connection()?;
        read_exact(&connection, dbnum, unit_refno, sesno)
    }

    pub fn latest_model_unit_commit(
        &self,
        dbnum: u32,
        unit_refno: &str,
    ) -> anyhow::Result<Option<ModelUnitCommit>> {
        let connection = self.lock_connection()?;
        read_latest(&connection, dbnum, unit_refno)
    }

    pub fn list_model_unit_commits(
        &self,
        dbnum: u32,
        unit_refno: &str,
    ) -> anyhow::Result<Vec<ModelUnitCommit>> {
        let connection = self.lock_connection()?;
        read_list(&connection, dbnum, unit_refno)
    }
}

struct ModelUnitWriteLock {
    file: File,
    #[allow(dead_code)]
    path: PathBuf,
}

impl ModelUnitWriteLock {
    fn acquire(metadata_catalog: &Path) -> anyhow::Result<Self> {
        let path = metadata_catalog.with_extension("model-unit.lock");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&path)?;
        FileExt::lock_exclusive(&file)?;
        Ok(Self { file, path })
    }
}

impl Drop for ModelUnitWriteLock {
    fn drop(&mut self) {
        if let Err(error) = FileExt::unlock(&self.file) {
            log::warn!(
                "释放模型提交写锁失败(path={}): {error}",
                self.path.display()
            );
        }
    }
}

fn validate_relative_manifest_path(value: &str) -> anyhow::Result<()> {
    anyhow::ensure!(!value.trim().is_empty(), "manifest_path 不能为空");
    let path = Path::new(value);
    anyhow::ensure!(!path.is_absolute(), "manifest_path 必须相对于项目输出目录");
    anyhow::ensure!(
        path.components()
            .all(|component| matches!(component, Component::Normal(_))),
        "manifest_path 不能包含根目录、当前目录或父目录"
    );
    anyhow::ensure!(
        value.replace('\\', "/").ends_with("/manifest.json"),
        "manifest_path 必须指向 manifest.json"
    );
    Ok(())
}

fn insert_commit(connection: &Connection, commit: &ModelUnitCommit) -> anyhow::Result<()> {
    connection.execute(
        "INSERT INTO model_unit_commit (
            dbnum, unit_refno, unit_noun, sesno, impact_kind, artifact_sesno,
            project_name, manifest_path, generated_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            i64::from(commit.dbnum),
            commit.unit_refno,
            commit.unit_noun,
            i64::from(commit.sesno),
            commit.impact_kind.as_str(),
            i64::from(commit.artifact_sesno),
            commit.project_name,
            commit.manifest_path,
            commit.generated_at,
        ],
    )?;
    Ok(())
}

fn read_exact(
    connection: &Connection,
    dbnum: u32,
    unit_refno: &str,
    sesno: u32,
) -> anyhow::Result<Option<ModelUnitCommit>> {
    let mut statement = connection.prepare(&format!(
        "{} WHERE dbnum = ? AND unit_refno = ? AND sesno = ?",
        select_commit_sql()
    ))?;
    let rows = statement
        .query_map(
            params![i64::from(dbnum), unit_refno, i64::from(sesno)],
            map_commit_row,
        )?
        .collect::<Result<Vec<_>, _>>()?;
    anyhow::ensure!(
        rows.len() <= 1,
        "模型提交三元组存在重复行: ({dbnum}, {unit_refno}, {sesno}) rows={}",
        rows.len()
    );
    Ok(rows.into_iter().next())
}

fn read_latest(
    connection: &Connection,
    dbnum: u32,
    unit_refno: &str,
) -> anyhow::Result<Option<ModelUnitCommit>> {
    Ok(connection
        .query_row(
            &format!(
                "{} WHERE dbnum = ? AND unit_refno = ? ORDER BY sesno DESC LIMIT 1",
                select_commit_sql()
            ),
            params![i64::from(dbnum), unit_refno],
            map_commit_row,
        )
        .optional()?)
}

fn read_list(
    connection: &Connection,
    dbnum: u32,
    unit_refno: &str,
) -> anyhow::Result<Vec<ModelUnitCommit>> {
    let mut statement = connection.prepare(&format!(
        "{} WHERE dbnum = ? AND unit_refno = ? ORDER BY sesno DESC",
        select_commit_sql()
    ))?;
    Ok(statement
        .query_map(params![i64::from(dbnum), unit_refno], map_commit_row)?
        .collect::<Result<Vec<_>, _>>()?)
}

fn select_commit_sql() -> &'static str {
    "SELECT dbnum, unit_refno, unit_noun, sesno, impact_kind, artifact_sesno,
            project_name, manifest_path, generated_at
     FROM model_unit_commit"
}

fn map_commit_row(row: &duckdb::Row<'_>) -> duckdb::Result<ModelUnitCommit> {
    let impact_kind = row
        .get::<_, String>(4)?
        .parse()
        .map_err(|error: anyhow::Error| {
            duckdb::Error::FromSqlConversionFailure(
                4,
                duckdb::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    error.to_string(),
                )),
            )
        })?;
    Ok(ModelUnitCommit {
        dbnum: row.get(0)?,
        unit_refno: row.get(1)?,
        unit_noun: row.get(2)?,
        sesno: row.get(3)?,
        impact_kind,
        artifact_sesno: row.get(5)?,
        project_name: row.get(6)?,
        manifest_path: row.get(7)?,
        generated_at: row.get(8)?,
    })
}

fn model_unit_extra_info(commit: &ModelUnitCommit) -> String {
    serde_json::json!({
        "kind": "model-unit-commit",
        "dbnum": commit.dbnum,
        "unit_refno": commit.unit_refno,
        "sesno": commit.sesno,
    })
    .to_string()
}

fn snapshot_id_for_commit(
    connection: &Connection,
    commit: &ModelUnitCommit,
) -> anyhow::Result<u64> {
    let extra_info = model_unit_extra_info(commit);
    let snapshot_ids = connection
        .prepare(&format!(
            "SELECT snapshot_id FROM {}.snapshots() WHERE commit_extra_info = ? ORDER BY snapshot_id",
            DUCKLAKE_CATALOG_ALIAS
        ))?
        .query_map(params![extra_info], |row| row.get::<_, u64>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    anyhow::ensure!(
        snapshot_ids.len() == 1,
        "模型提交必须唯一对应一个 DuckLake snapshot: ({}, {}, {}) matches={snapshot_ids:?}",
        commit.dbnum,
        commit.unit_refno,
        commit.sesno
    );
    Ok(snapshot_ids[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_schema(connection: &Connection) {
        connection
            .execute_batch(
                "CREATE TABLE model_unit_commit (
                    dbnum UINTEGER NOT NULL,
                    unit_refno VARCHAR NOT NULL,
                    unit_noun VARCHAR NOT NULL,
                    sesno UINTEGER NOT NULL,
                    impact_kind VARCHAR NOT NULL,
                    artifact_sesno UINTEGER NOT NULL,
                    project_name VARCHAR NOT NULL,
                    manifest_path VARCHAR NOT NULL,
                    generated_at VARCHAR NOT NULL
                );",
            )
            .expect("schema");
    }

    fn model_commit(sesno: u32) -> ModelUnitCommit {
        ModelUnitCommit {
            dbnum: 7997,
            unit_refno: "24381_145018".to_string(),
            unit_noun: "BRAN".to_string(),
            sesno,
            impact_kind: ModelUnitImpactKind::Mesh,
            artifact_sesno: sesno,
            project_name: "AvevaMarineSample".to_string(),
            manifest_path: format!("model_units/7997/24381_145018/{sesno}/manifest.json"),
            generated_at: "2026-07-22T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn tuple_identity_lists_newest_sesno_first() {
        let connection = Connection::open_in_memory().expect("connection");
        create_schema(&connection);
        insert_commit(&connection, &model_commit(791)).expect("first");
        insert_commit(&connection, &model_commit(897)).expect("second");

        let versions = read_list(&connection, 7997, "24381_145018").expect("list");
        assert_eq!(
            versions.iter().map(|item| item.sesno).collect::<Vec<_>>(),
            vec![897, 791]
        );
        assert_eq!(
            read_exact(&connection, 7997, "24381_145018", 791)
                .expect("exact")
                .expect("commit")
                .artifact_sesno,
            791
        );
    }

    #[test]
    fn noop_tracks_new_sesno_while_reusing_previous_artifact() {
        let mut commit = model_commit(897);
        commit.impact_kind = ModelUnitImpactKind::Noop;
        commit.artifact_sesno = 791;
        commit.manifest_path = "model_units/7997/24381_145018/791/manifest.json".to_string();
        commit.validate().expect("valid noop");

        commit.artifact_sesno = 897;
        assert!(commit.validate().is_err());
    }

    #[test]
    fn tombstone_has_no_model_artifact_url() {
        let mut commit = model_commit(898);
        commit.impact_kind = ModelUnitImpactKind::Tombstone;
        commit.manifest_path.clear();

        commit.validate().expect("valid tombstone");
        assert_eq!(commit.manifest_url(), None);

        commit.manifest_path = "model_units/7997/24381_145018/898/manifest.json".to_string();
        assert!(commit.validate().is_err());
        commit.manifest_path.clear();
        commit.artifact_sesno = 897;
        assert!(commit.validate().is_err());
    }

    #[test]
    fn rejects_manifest_path_escape_and_content_hash_is_not_part_of_contract() {
        let mut commit = model_commit(791);
        commit.manifest_path = "../other/manifest.json".to_string();
        assert!(commit.validate().is_err());
        assert!(!select_commit_sql().contains("content_hash"));

        let mut invalid_refno = model_commit(791);
        invalid_refno.unit_refno = "..\\manifest".to_string();
        assert!(invalid_refno.validate().is_err());
    }
}
