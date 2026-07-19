# Plant Model Versioning

This context names the business concepts that make PE/ATT source-row history trustworthy across E3D session updates.

## Language

**Version Commit**:
The complete PE/ATT source-row state for one dbnum at a target sesno that has passed verification and is eligible for historical reads.
_Avoid_: Incremental batch, save result

**Version Anchor**:
An immutable mapping from a committed `(dbnum, sesno)` to its storage time, making that Version Commit visible to historical reads.
_Avoid_: Mutable checkpoint, latest timestamp

**Commit Fingerprint**:
The deterministic identity of a Version Commit, derived from its normalized changes, sesno range, and source observation.
_Avoid_: Row count, file timestamp

**Commit Pending**:
A candidate Version Commit whose writes may have started but whose verification and Version Anchor publication are incomplete. A Commit Pending blocks later sesno commits for the same dbnum.
_Avoid_: Partial success, warning-only success

**Legacy Anchor**:
A pre-fingerprint Version Anchor retained for read compatibility but never rewritten or treated as proof of a reproducible Version Commit.
_Avoid_: Backfilled commit

**Committed Watermark**:
The highest sesno for a dbnum whose Version Anchor is published; the only resume point an incremental collection may start from. Falls back to `dbnum_info_table` max sesno only for dbnums that predate anchoring.
_Avoid_: dbnum_info_table max sesno, file latest sesno, cached header sesno

**Element Change**:
One classified per-element operation (add / modify / delete, with noun, owner, and model category) observed while collecting a sesno range from a source db file. Exists only inside a collection outcome and its reports; it is not an independently queryable store.
_Avoid_: element_changes table row, increment record, IncrementInfo
