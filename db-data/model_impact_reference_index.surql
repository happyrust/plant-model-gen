-- ADR-0011 P1 step-1：目录引用反向索引 cata_ref_index 的 SQL 形态验证。
-- 与 src/versioned_db/cata_ref_index.rs 一一对应，验证 Rust 侧发出的每种语句在引擎上成立：
--   schema DDL            = ensure_cata_ref_index_schema()
--   INSERT INTO [{id:...}] = insert_edges_sql() / edge_insert_object()（数组 record id + 显式 id）
--   DELETE WHERE source    = delete_sources_sql()（replace-by-source 先删段）
--   SELECT WHERE target_refno IN [..] (AND attribute IN [..]) = load_inbound_references()（反查核心）
--   SELECT WHERE source_dbnum AND source_refno IN [..]        = load_outbound_references()
--   SELECT VALUE count() GROUP ALL                            = count_index_rows()
--   UPSERT cata_ref_index_state / SELECT ...                  = write_state()/read_state()
--
-- 夹具语义（设计实例→目录定义 的 as-written 语法边）：
--   100_1 (A) --CATR--> 200_1 (SCOM S) , --SPRE--> 200_2 (P)
--   100_2 (B) --CATR--> 200_1 (SCOM S)
--   100_3 (C) --PRTREF-> 200_3 (T)
-- 核心诉求：改「目录定义 SCOM 200_1」时要能反查到引用它的 A、B 两个实例。
-- 运行方式：scripts/smoke/model_impact_reference_index.ps1（HTTP POST /sql，独立 ns/db，不触碰业务数据）

-- ===== 清场（可重复执行）=====
REMOVE TABLE IF EXISTS cata_ref_index;
REMOVE TABLE IF EXISTS cata_ref_index_state;

-- ===== schema（= ensure_cata_ref_index_schema）=====
DEFINE TABLE IF NOT EXISTS cata_ref_index TYPE NORMAL SCHEMALESS;
DEFINE INDEX IF NOT EXISTS idx_crx_target ON TABLE cata_ref_index FIELDS target_refno;
DEFINE INDEX IF NOT EXISTS idx_crx_source ON TABLE cata_ref_index FIELDS source_dbnum, source_refno;
DEFINE TABLE IF NOT EXISTS cata_ref_index_state TYPE NORMAL SCHEMALESS;

-- [6] 初始 replace-by-source 插入（4 条边；= insert_edges_sql 的对象形态）
INSERT INTO cata_ref_index [
    { id: cata_ref_index:[100, '100_1', 'CATR', 0],   source_dbnum: 100, source_refno: '100_1', attribute: 'CATR',   ordinal: 0, target_refno: '200_1' },
    { id: cata_ref_index:[100, '100_1', 'SPRE', 0],   source_dbnum: 100, source_refno: '100_1', attribute: 'SPRE',   ordinal: 0, target_refno: '200_2' },
    { id: cata_ref_index:[100, '100_2', 'CATR', 0],   source_dbnum: 100, source_refno: '100_2', attribute: 'CATR',   ordinal: 0, target_refno: '200_1' },
    { id: cata_ref_index:[100, '100_3', 'PRTREF', 0], source_dbnum: 100, source_refno: '100_3', attribute: 'PRTREF', ordinal: 0, target_refno: '200_3' }
];

-- [7] 行数（= count_index_rows），期望 4
SELECT count() FROM cata_ref_index WHERE source_dbnum = 100 GROUP ALL;

-- [8] 反查 SCOM 200_1 的引用者（核心）：期望 [100_1, 100_2]
SELECT VALUE source_refno FROM cata_ref_index WHERE target_refno IN ['200_1'] LIMIT 100;

-- [9] 带属性族过滤反查（families=[CATR,SPRE]）：200_3 的 PRTREF 边被滤掉，期望 [100_1, 100_2]
SELECT VALUE source_refno FROM cata_ref_index WHERE target_refno IN ['200_1', '200_3'] AND attribute IN ['CATR', 'SPRE'] LIMIT 100;

-- [10] 出边（= load_outbound_references）for 100_1：期望 [200_1, 200_2]（CATR<SPRE）
SELECT VALUE target_refno FROM cata_ref_index WHERE source_dbnum = 100 AND source_refno IN ['100_1'] ORDER BY attribute;

-- [11] replace-by-source 先删段（= delete_sources_sql）：删 100_1 出边
DELETE cata_ref_index WHERE source_dbnum = 100 AND source_refno IN ['100_1'];

-- [12] 删后行数：期望 2（B.CATR、C.PRTREF）
SELECT count() FROM cata_ref_index WHERE source_dbnum = 100 GROUP ALL;

-- [13] 重插 100_1（改引用：CATR 由 200_1 改指 200_9、丢弃 SPRE）
INSERT INTO cata_ref_index [
    { id: cata_ref_index:[100, '100_1', 'CATR', 0], source_dbnum: 100, source_refno: '100_1', attribute: 'CATR', ordinal: 0, target_refno: '200_9' }
];

-- [14] 重插后行数：期望 3
SELECT count() FROM cata_ref_index WHERE source_dbnum = 100 GROUP ALL;

-- [15] 旧目标 200_1 现在只剩 B 引用（A 已改指他处）——证明 replace-by-source 同步更新了反向索引：期望 [100_2]
SELECT VALUE source_refno FROM cata_ref_index WHERE target_refno IN ['200_1'] LIMIT 100;

-- [16] 新目标 200_9 由 A 引用：期望 [100_1]
SELECT VALUE source_refno FROM cata_ref_index WHERE target_refno IN ['200_9'] LIMIT 100;

-- [17] 水位 UPSERT（= write_state）
UPSERT cata_ref_index_state:[100] SET dbnum = 100, ready = true, row_count = 3, checksum = 'deadbeef', extractor_version = 'cata-ref-index-v1', backfilled_at = time::now();

-- [18] 水位读取（= read_state）：期望 ready=true、row_count=3
SELECT dbnum, ready, row_count, checksum, extractor_version FROM cata_ref_index_state:[100];
