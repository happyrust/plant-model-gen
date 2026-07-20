-- Spec 024 destructive reset for disposable/versioned TEST sites only.
--
-- Production migration is always:
--   new RocksDB directory -> sync_pdms full rebuild -> model regen.
-- Never run this script against a production directory and never try to convert
-- old anchor timestamps.
--
-- Safety gate (run explicitly in the selected test NS/DB before importing):
--   UPSERT spec024_migration_guard:allow_reset CONTENT { enabled: true };

OPTION IMPORT;

LET $allowed = SELECT VALUE enabled FROM spec024_migration_guard:allow_reset;
IF $allowed != [true] {
    THROW 'Spec 024 reset refused: create spec024_migration_guard:allow_reset with enabled=true in this disposable test database first';
};
DELETE spec024_migration_guard:allow_reset;

-- Pure-refno model records and their referenced model entities.
REMOVE TABLE IF EXISTS inst_relate;
REMOVE TABLE IF EXISTS geo_relate;
REMOVE TABLE IF EXISTS neg_relate;
REMOVE TABLE IF EXISTS ngmr_relate;
REMOVE TABLE IF EXISTS tubi_relate;
REMOVE TABLE IF EXISTS inst_info;
REMOVE TABLE IF EXISTS inst_geo;
REMOVE TABLE IF EXISTS inst_relate_aabb;
REMOVE TABLE IF EXISTS inst_relate_booled_aabb;
REMOVE TABLE IF EXISTS inst_relate_agg_aabb;
REMOVE TABLE IF EXISTS inst_relate_bool;
REMOVE TABLE IF EXISTS inst_relate_cata_bool;
REMOVE TABLE IF EXISTS refno_relations;
REMOVE TABLE IF EXISTS tubi_info;
REMOVE TABLE IF EXISTS pe_transform;
REMOVE TABLE IF EXISTS aabb;
REMOVE TABLE IF EXISTS trans;
REMOVE TABLE IF EXISTS pts;
REMOVE TABLE IF EXISTS scene_node;
REMOVE TABLE IF EXISTS contains;

-- Old two-part anchors and commit metadata cannot be reinterpreted safely.
REMOVE TABLE IF EXISTS sesno_version_anchor;
REMOVE TABLE IF EXISTS version_commit_state;
REMOVE TABLE IF EXISTS version_commit_lease;
REMOVE TABLE IF EXISTS pe_owner_version_meta;

RETURN {
    status: 'RESET_COMPLETE',
    test_only: true,
    next_steps: [
        'run sync_pdms to rebuild PE/ATT and full data anchors',
        'run full model generation/regen to rebuild model tables and model_gen anchors'
    ]
};
