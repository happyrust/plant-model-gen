-- Read-only diagnosis for BRAN 24381/145018 and its valve catalog closure.
SELECT * FROM replica_apply_watermark:global;
SELECT authoritative_snapshot_id, history_start_snapshot, manifest_hash, status FROM replica_snapshot_binding;
SELECT dbnum, sesno, commit_fingerprint FROM generation_replica_manifest ORDER BY dbnum;

SELECT count() AS n FROM generation_replica_element GROUP ALL;
SELECT count() AS n FROM generation_replica_reference GROUP ALL;
SELECT count() AS n FROM generation_replica_hierarchy GROUP ALL;
SELECT * FROM generation_replica_element LIMIT 2;

SELECT refno, owner_refno, noun, name
FROM generation_replica_element
WHERE refno IN ["24381_145035", "13246_465874", "13246_243978", "13246_243868", "13246_244032"];

SELECT source_refno, attribute_name, target_refno, ordinal
FROM generation_replica_reference
WHERE source_refno IN ["24381_145035", "13246_465874", "13246_243978", "13246_243868", "13246_244032"]
ORDER BY source_refno, attribute_name, ordinal;

SELECT parent_refno, child_refno, ordinal
FROM generation_replica_hierarchy
WHERE parent_refno IN ["24381_145018", "13246_243978", "13246_243868", "13246_244032"]
ORDER BY parent_refno, ordinal;
