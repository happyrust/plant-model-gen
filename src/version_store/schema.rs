pub const VERSION_STORE_SCHEMA_VERSION: u16 = 2;
pub const DUCKLAKE_CATALOG_ALIAS: &str = "generation_store";

pub const CREATE_SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS store_metadata (
    key VARCHAR NOT NULL,
    value VARCHAR NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT current_timestamp
);

CREATE TABLE IF NOT EXISTS data_version_state (
    dbnum UINTEGER NOT NULL,
    sesno UINTEGER NOT NULL,
    from_sesno UINTEGER NOT NULL,
    commit_fingerprint VARCHAR NOT NULL,
    source VARCHAR NOT NULL,
    source_hash VARCHAR,
    committed_at TIMESTAMPTZ NOT NULL DEFAULT current_timestamp
);

CREATE TABLE IF NOT EXISTS version_manifest (
    manifest_fingerprint VARCHAR NOT NULL,
    dbnum UINTEGER NOT NULL,
    sesno UINTEGER NOT NULL,
    commit_fingerprint VARCHAR NOT NULL
);

CREATE TABLE IF NOT EXISTS db_catalog (
    dbnum UINTEGER NOT NULL,
    ref0 UINTEGER,
    db_type VARCHAR NOT NULL,
    project VARCHAR NOT NULL
);

CREATE TABLE IF NOT EXISTS element (
    dbnum UINTEGER NOT NULL,
    refno VARCHAR NOT NULL,
    owner_refno VARCHAR NOT NULL,
    noun VARCHAR NOT NULL,
    name VARCHAR NOT NULL,
    has_children BOOLEAN NOT NULL,
    attr_codec_version USMALLINT NOT NULL,
    attr_payload BLOB NOT NULL,
    attr_hash VARCHAR NOT NULL
);

CREATE TABLE IF NOT EXISTS element_tombstone (
    dbnum UINTEGER NOT NULL,
    refno VARCHAR NOT NULL,
    sesno UINTEGER NOT NULL,
    commit_fingerprint VARCHAR NOT NULL
);

CREATE TABLE IF NOT EXISTS hierarchy_edge (
    dbnum UINTEGER NOT NULL,
    parent_refno VARCHAR NOT NULL,
    child_refno VARCHAR NOT NULL,
    ordinal UINTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS reference_edge (
    dbnum UINTEGER NOT NULL,
    source_refno VARCHAR NOT NULL,
    attribute_name VARCHAR NOT NULL,
    target_refno VARCHAR NOT NULL,
    ordinal UINTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS transform (
    dbnum UINTEGER NOT NULL,
    refno VARCHAR NOT NULL,
    local_transform BLOB,
    world_transform BLOB NOT NULL,
    transform_hash VARCHAR NOT NULL
);

CREATE TABLE IF NOT EXISTS model_unit_commit (
    dbnum UINTEGER NOT NULL,
    unit_refno VARCHAR NOT NULL,
    unit_noun VARCHAR NOT NULL,
    sesno UINTEGER NOT NULL,
    impact_kind VARCHAR NOT NULL,
    artifact_sesno UINTEGER NOT NULL,
    project_name VARCHAR NOT NULL,
    manifest_path VARCHAR NOT NULL,
    generated_at VARCHAR NOT NULL
);
"#;

pub const PARTITION_SCHEMA_SQL: &str = r#"
ALTER TABLE data_version_state SET PARTITIONED BY (dbnum);
ALTER TABLE version_manifest SET PARTITIONED BY (dbnum);
ALTER TABLE db_catalog SET PARTITIONED BY (dbnum);
ALTER TABLE element SET PARTITIONED BY (dbnum);
ALTER TABLE element_tombstone SET PARTITIONED BY (dbnum);
ALTER TABLE hierarchy_edge SET PARTITIONED BY (dbnum);
ALTER TABLE reference_edge SET PARTITIONED BY (dbnum);
ALTER TABLE transform SET PARTITIONED BY (dbnum);
ALTER TABLE model_unit_commit SET PARTITIONED BY (dbnum, unit_refno);
"#;

pub const MIGRATE_V1_TO_V2_SQL: &str = r#"
ALTER TABLE model_unit_commit SET PARTITIONED BY (dbnum, unit_refno);
UPDATE store_metadata SET value = '2', updated_at = current_timestamp
WHERE key = 'schema_version';
"#;
