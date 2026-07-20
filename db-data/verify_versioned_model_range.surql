-- Spec 024 / P5 capability gate.
-- Run only against an isolated RocksDB/SurrealKV instance opened with versioned=true.
-- The script throws on any point/range VERSION regression; reaching the final RETURN is PASS.

OPTION IMPORT;
USE NS spec024 DB model_range_gate;

REMOVE TABLE IF EXISTS inst_relate;
REMOVE TABLE IF EXISTS geo_relate;
REMOVE TABLE IF EXISTS inst_info;
REMOVE TABLE IF EXISTS inst_geo;
REMOVE TABLE IF EXISTS spec024_model_range_gate;

DEFINE TABLE inst_relate TYPE NORMAL SCHEMALESS;
DEFINE TABLE geo_relate TYPE NORMAL SCHEMALESS;
DEFINE TABLE inst_info TYPE NORMAL SCHEMALESS;
DEFINE TABLE inst_geo TYPE NORMAL SCHEMALESS;

-- Generation 1: pure-refno point id + pure-refno-prefix range ids.
UPSERT inst_info:info_a CONTENT { generation: 1, label: 'info-a' };
UPSERT inst_geo:geo_a CONTENT { generation: 1, label: 'geo-a' };
UPSERT inst_relate:[24001, 100] CONTENT {
    generation: 1,
    inst_info_ids: [inst_info:info_a],
    geo_relate_ids: [geo_relate:[24001, 100, 0]]
};
UPSERT geo_relate:[24001, 100, 0] CONTENT {
    generation: 1,
    inst_geo_id: inst_geo:geo_a
};
SLEEP 200ms;
LET $t1 = time::now();
SLEEP 200ms;

-- Generation 2: replace the same point/range generation and add a second range row.
UPSERT inst_info:info_b CONTENT { generation: 2, label: 'info-b' };
UPSERT inst_geo:geo_b0 CONTENT { generation: 2, label: 'geo-b0' };
UPSERT inst_geo:geo_b1 CONTENT { generation: 2, label: 'geo-b1' };
DELETE inst_relate:[24001, 100];
DELETE geo_relate:[24001, 100, NONE]..=[24001, 100, ..];
UPSERT inst_relate:[24001, 100] CONTENT {
    generation: 2,
    inst_info_ids: [inst_info:info_b],
    geo_relate_ids: [geo_relate:[24001, 100, 0], geo_relate:[24001, 100, 1]]
};
UPSERT geo_relate:[24001, 100, 0] CONTENT {
    generation: 2,
    inst_geo_id: inst_geo:geo_b0
};
UPSERT geo_relate:[24001, 100, 1] CONTENT {
    generation: 2,
    inst_geo_id: inst_geo:geo_b1
};
SLEEP 200ms;
LET $t2 = time::now();
SLEEP 200ms;

-- Gate A: point VERSION returns the complete requested generation.
LET $point_v1 = SELECT VALUE generation FROM inst_relate:[24001, 100] VERSION $t1;
IF $point_v1 != [1] {
    THROW 'P5 gate failed: point VERSION at t1 did not return generation 1';
};
LET $point_v2 = SELECT VALUE generation FROM inst_relate:[24001, 100] VERSION $t2;
IF $point_v2 != [2] {
    THROW 'P5 gate failed: point VERSION at t2 did not return generation 2';
};

-- Gate B: array-id range VERSION must not silently return current state.
LET $range_v1 = array::sort(
    SELECT VALUE generation
    FROM geo_relate:[24001, 100, NONE]..=[24001, 100, ..]
    VERSION $t1
);
IF $range_v1 != [1] {
    THROW 'P5 gate failed: range VERSION at t1 did not return exactly generation 1';
};
LET $range_v2 = array::sort(
    SELECT VALUE generation
    FROM geo_relate:[24001, 100, NONE]..=[24001, 100, ..]
    VERSION $t2
);
IF $range_v2 != [2, 2] {
    THROW 'P5 gate failed: range VERSION at t2 did not return both generation 2 rows';
};

-- Gate C: all records referenced by the relation generation are queryable at the same timestamp.
LET $info_v1 = SELECT VALUE generation FROM inst_info:info_a VERSION $t1;
LET $geo_v1 = SELECT VALUE generation FROM inst_geo:geo_a VERSION $t1;
LET $info_v2 = SELECT VALUE generation FROM inst_info:info_b VERSION $t2;
LET $geo_v2 = array::sort([
    (SELECT VALUE generation FROM inst_geo:geo_b0 VERSION $t2)[0],
    (SELECT VALUE generation FROM inst_geo:geo_b1 VERSION $t2)[0]
]);
IF $info_v1 != [1] OR $geo_v1 != [1] OR $info_v2 != [2] OR $geo_v2 != [2, 2] {
    THROW 'P5 gate failed: referenced inst_info/inst_geo records are not coherent at anchor time';
};

-- Gate D: hard deletion keeps the complete pre-delete generation available through VERSION.
DELETE inst_relate:[24001, 100];
DELETE geo_relate:[24001, 100, NONE]..=[24001, 100, ..];
SLEEP 200ms;
LET $t3 = time::now();
LET $current_point = SELECT VALUE generation FROM inst_relate:[24001, 100];
LET $current_range = SELECT VALUE generation
    FROM geo_relate:[24001, 100, NONE]..=[24001, 100, ..];
LET $deleted_point_v2 = SELECT VALUE generation
    FROM inst_relate:[24001, 100] VERSION $t2;
LET $deleted_range_v2 = array::sort(
    SELECT VALUE generation
    FROM geo_relate:[24001, 100, NONE]..=[24001, 100, ..]
    VERSION $t2
);
IF $current_point != [] OR $current_range != [] {
    THROW 'P5 gate failed: hard-deleted model records still exist in current state';
};
IF $deleted_point_v2 != [2] OR $deleted_range_v2 != [2, 2] {
    THROW 'P5 gate failed: VERSION did not recover the complete pre-delete generation';
};

UPSERT spec024_model_range_gate:latest CONTENT {
    status: 'PASS',
    point_version: true,
    range_version: true,
    referenced_records_same_timestamp: true,
    deletion_history: true,
    t1: $t1,
    t2: $t2,
    t3: $t3
};

RETURN {
    status: 'PASS',
    point_version: true,
    range_version: true,
    referenced_records_same_timestamp: true,
    deletion_history: true,
    t1: $t1,
    t2: $t2,
    t3: $t3
};
