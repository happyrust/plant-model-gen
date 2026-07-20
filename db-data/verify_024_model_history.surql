-- Spec 024 SC-001 / SC-003 / SC-006 isolated model-history fixture.
-- Run only on an empty RocksDB instance opened with versioned=true&retention=0.

OPTION IMPORT;
USE NS spec024 DB spec024_unified_e2e;

REMOVE TABLE IF EXISTS sesno_version_anchor;
REMOVE TABLE IF EXISTS spec024_model_history_gate;
REMOVE TABLE IF EXISTS pe;
REMOVE TABLE IF EXISTS pe_transform;
REMOVE TABLE IF EXISTS inst_relate;
REMOVE TABLE IF EXISTS geo_relate;
REMOVE TABLE IF EXISTS inst_relate_aabb;
REMOVE TABLE IF EXISTS inst_relate_bool;
REMOVE TABLE IF EXISTS inst_info;
REMOVE TABLE IF EXISTS inst_geo;
REMOVE TABLE IF EXISTS trans;
REMOVE TABLE IF EXISTS aabb;

DEFINE TABLE sesno_version_anchor SCHEMAFULL;
DEFINE FIELD dbnum ON TABLE sesno_version_anchor TYPE int;
DEFINE FIELD sesno ON TABLE sesno_version_anchor TYPE int;
DEFINE FIELD source ON TABLE sesno_version_anchor TYPE string
    ASSERT $value IN ['full', 'incremental', 'model_gen'];
DEFINE FIELD anchored_at ON TABLE sesno_version_anchor TYPE datetime;
DEFINE INDEX sesno_version_anchor_identity ON TABLE sesno_version_anchor
    FIELDS dbnum, sesno, source UNIQUE;

DEFINE TABLE pe TYPE NORMAL SCHEMALESS;
DEFINE TABLE pe_transform TYPE NORMAL SCHEMALESS;
DEFINE TABLE inst_info TYPE NORMAL SCHEMALESS;
DEFINE TABLE inst_geo TYPE NORMAL SCHEMALESS;
DEFINE TABLE trans TYPE NORMAL SCHEMALESS;
DEFINE TABLE aabb TYPE NORMAL SCHEMALESS;
DEFINE TABLE inst_relate TYPE RELATION IN pe OUT inst_info SCHEMALESS;
DEFINE TABLE geo_relate TYPE RELATION IN inst_info OUT inst_geo SCHEMALESS;
DEFINE TABLE inst_relate_aabb TYPE NORMAL SCHEMALESS;
DEFINE TABLE inst_relate_bool TYPE NORMAL SCHEMALESS;

-- Data generation 1 is ready before model generation 1.
LET $data_t1 = time::now();
UPSERT sesno_version_anchor:[24024, 100, 'incremental'] CONTENT {
    dbnum: 24024,
    sesno: 100,
    source: 'incremental',
    anchored_at: $data_t1
};
SLEEP 100ms;

UPSERT pe:⟨24001_100⟩ CONTENT { noun: 'EQUI', generation: 1 };
UPSERT pe:⟨24001_200⟩ CONTENT { noun: 'VALV', generation: 1 };

UPSERT trans:⟨world_e_v1⟩ CONTENT {
    d: { translation: [10.0, 0.0, 0.0], rotation: [0.0, 0.0, 0.0, 1.0], scale: [1.0, 1.0, 1.0] }
};
UPSERT trans:⟨geo_e_v1⟩ CONTENT {
    d: { translation: [1.0, 0.0, 0.0], rotation: [0.0, 0.0, 0.0, 1.0], scale: [1.0, 1.0, 1.0] }
};
UPSERT trans:⟨world_d_v1⟩ CONTENT {
    d: { translation: [20.0, 0.0, 0.0], rotation: [0.0, 0.0, 0.0, 1.0], scale: [1.0, 1.0, 1.0] }
};
UPSERT trans:⟨geo_d_v1⟩ CONTENT {
    d: { translation: [2.0, 0.0, 0.0], rotation: [0.0, 0.0, 0.0, 1.0], scale: [1.0, 1.0, 1.0] }
};
UPSERT aabb:⟨aabb_e_v1⟩ CONTENT { d: { mins: [0.0, 0.0, 0.0], maxs: [1.0, 1.0, 1.0] } };
UPSERT aabb:⟨aabb_d_v1⟩ CONTENT { d: { mins: [2.0, 2.0, 2.0], maxs: [3.0, 3.0, 3.0] } };

UPSERT pe_transform:[24001, 100] CONTENT { world_trans: trans:⟨world_e_v1⟩ };
UPSERT pe_transform:[24001, 200] CONTENT { world_trans: trans:⟨world_d_v1⟩ };
UPSERT inst_info:⟨info_e_v1⟩ CONTENT { generation: 1 };
UPSERT inst_info:⟨info_d_v1⟩ CONTENT { generation: 1 };
UPSERT inst_geo:⟨mesh_e_v1⟩ CONTENT { generation: 1, unit_flag: false };
UPSERT inst_geo:⟨mesh_d_v1⟩ CONTENT { generation: 1, unit_flag: false };
INSERT RELATION INTO inst_relate {
    id: inst_relate:[24001, 100],
    in: pe:⟨24001_100⟩,
    out: inst_info:⟨info_e_v1⟩,
    dbnum: 24024,
    owner_type: 'OTHER',
    spec_value: 1,
    generation: 1
};
INSERT RELATION INTO inst_relate {
    id: inst_relate:[24001, 200],
    in: pe:⟨24001_200⟩,
    out: inst_info:⟨info_d_v1⟩,
    dbnum: 24024,
    owner_type: 'OTHER',
    spec_value: 2,
    generation: 1
};
INSERT RELATION INTO geo_relate {
    id: geo_relate:[24001, 100, 0],
    in: inst_info:⟨info_e_v1⟩,
    out: inst_geo:⟨mesh_e_v1⟩,
    visible: true,
    trans: trans:⟨geo_e_v1⟩,
    geo_type: 'Pos',
    generation: 1
};
INSERT RELATION INTO geo_relate {
    id: geo_relate:[24001, 200, 0],
    in: inst_info:⟨info_d_v1⟩,
    out: inst_geo:⟨mesh_d_v1⟩,
    visible: true,
    trans: trans:⟨geo_d_v1⟩,
    geo_type: 'Pos',
    generation: 1
};
UPSERT inst_relate_aabb:[24001, 100] CONTENT { aabb_id: aabb:⟨aabb_e_v1⟩ };
UPSERT inst_relate_aabb:[24001, 200] CONTENT { aabb_id: aabb:⟨aabb_d_v1⟩ };

SLEEP 100ms;
LET $model_t1 = time::now();
UPSERT sesno_version_anchor:[24024, 100, 'model_gen'] CONTENT {
    dbnum: 24024,
    sesno: 100,
    source: 'model_gen',
    anchored_at: $model_t1
};
SLEEP 100ms;

-- Data generation 2 is ready before model generation 2.
LET $data_t2 = time::now();
UPSERT sesno_version_anchor:[24024, 200, 'incremental'] CONTENT {
    dbnum: 24024,
    sesno: 200,
    source: 'incremental',
    anchored_at: $data_t2
};
SLEEP 100ms;

-- E changes to two geometry rows; D is hard-deleted.
DELETE inst_relate:[24001, 100];
DELETE geo_relate:[24001, 100, NONE]..=[24001, 100, ..];
DELETE inst_relate_aabb:[24001, 100];
DELETE pe_transform:[24001, 100];

DELETE inst_relate:[24001, 200];
DELETE geo_relate:[24001, 200, NONE]..=[24001, 200, ..];
DELETE inst_relate_aabb:[24001, 200];
DELETE pe_transform:[24001, 200];
DELETE pe:⟨24001_200⟩;

UPSERT pe:⟨24001_100⟩ CONTENT { noun: 'EQUI', generation: 2 };
UPSERT trans:⟨world_e_v2⟩ CONTENT {
    d: { translation: [11.0, 0.0, 0.0], rotation: [0.0, 0.0, 0.0, 1.0], scale: [1.0, 1.0, 1.0] }
};
UPSERT trans:⟨geo_e_v2_0⟩ CONTENT {
    d: { translation: [3.0, 0.0, 0.0], rotation: [0.0, 0.0, 0.0, 1.0], scale: [1.0, 1.0, 1.0] }
};
UPSERT trans:⟨geo_e_v2_1⟩ CONTENT {
    d: { translation: [4.0, 0.0, 0.0], rotation: [0.0, 0.0, 0.0, 1.0], scale: [1.0, 1.0, 1.0] }
};
UPSERT aabb:⟨aabb_e_v2⟩ CONTENT { d: { mins: [0.0, 0.0, 0.0], maxs: [2.0, 2.0, 2.0] } };
UPSERT pe_transform:[24001, 100] CONTENT { world_trans: trans:⟨world_e_v2⟩ };
UPSERT inst_info:⟨info_e_v2⟩ CONTENT { generation: 2 };
UPSERT inst_geo:⟨mesh_e_v2_0⟩ CONTENT { generation: 2, unit_flag: false };
UPSERT inst_geo:⟨mesh_e_v2_1⟩ CONTENT { generation: 2, unit_flag: false };
INSERT RELATION INTO inst_relate {
    id: inst_relate:[24001, 100],
    in: pe:⟨24001_100⟩,
    out: inst_info:⟨info_e_v2⟩,
    dbnum: 24024,
    owner_type: 'OTHER',
    spec_value: 3,
    generation: 2
};
INSERT RELATION INTO geo_relate {
    id: geo_relate:[24001, 100, 0],
    in: inst_info:⟨info_e_v2⟩,
    out: inst_geo:⟨mesh_e_v2_0⟩,
    visible: true,
    trans: trans:⟨geo_e_v2_0⟩,
    geo_type: 'Pos',
    generation: 2
};
INSERT RELATION INTO geo_relate {
    id: geo_relate:[24001, 100, 1],
    in: inst_info:⟨info_e_v2⟩,
    out: inst_geo:⟨mesh_e_v2_1⟩,
    visible: true,
    trans: trans:⟨geo_e_v2_1⟩,
    geo_type: 'Pos',
    generation: 2
};
UPSERT inst_relate_aabb:[24001, 100] CONTENT { aabb_id: aabb:⟨aabb_e_v2⟩ };

SLEEP 100ms;
LET $model_t2 = time::now();
UPSERT sesno_version_anchor:[24024, 200, 'model_gen'] CONTENT {
    dbnum: 24024,
    sesno: 200,
    source: 'model_gen',
    anchored_at: $model_t2
};

IF $model_t1 <= $data_t1 OR $model_t2 <= $data_t2 {
    THROW 'SC-001 failed: model_gen anchor is not later than its data anchor';
};

UPSERT spec024_model_history_gate:latest CONTENT {
    status: 'PASS',
    dbnum: 24024,
    refno_changed: '24001_100',
    refno_deleted: '24001_200',
    data_t1: $data_t1,
    model_t1: $model_t1,
    data_t2: $data_t2,
    model_t2: $model_t2
};

RETURN SELECT * FROM spec024_model_history_gate:latest;
