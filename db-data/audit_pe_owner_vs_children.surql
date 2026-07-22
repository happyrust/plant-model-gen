-- ============================================================
-- specs/023 M0/T3: pe_owner 边 vs pe.children 字段 完整性审计
-- 用途：latest 树查询切换 pe_owner 前的兜底体检（D5）——
--   [1] 每 dbnum 的 parents_with_children / children_total 总量 + pe_owner 边行总数
--   [2] 抽样(默认 2000 个有子 parent)对比 count(<-pe_owner) 与 array::len(children)
--   [3] 抽样无子行（children 为空）检查是否残留脏边（delete 后未清干净）
-- 判定：
--   [2] mismatched > 0 → 该库边不完整/陈旧 → 切换前先 `model-version rebuild-pe-owner`
--   [3] stale > 0     → 残留脏边 → 同样 rebuild（先删后插会清掉）
-- 用法：
--   powershell -File scripts/smoke/pe_owner_children_audit.ps1 -Ns <surreal_ns> -Db <project> [-Dbnum <n>]
--   （脚本会把下方 LET $target_dbnum 替换为具体值；直连执行时手工改 NONE 即可）
-- 说明：抽样顺序取存储序（无 rand 全表打散——审计目标是"发现存在性"，
--   全量核对走 rebuild-pe-owner 的先删后插重建，不在读侧做）。
-- ============================================================

RETURN "====== [0] target scope ======";
LET $target_dbnum = NONE;
RETURN { target_dbnum: $target_dbnum };

RETURN "====== [1] totals: parents/children per dbnum + edge rows ======";
RETURN (SELECT dbnum, count() AS parents_with_children, math::sum(array::len(children ?? [])) AS children_total FROM pe WHERE ($target_dbnum == NONE OR dbnum == $target_dbnum) AND array::len(children ?? []) > 0 GROUP BY dbnum);
RETURN (SELECT count() AS pe_owner_edge_rows FROM pe_owner GROUP ALL);

RETURN "====== [2] sampled parents: count(<-pe_owner) vs len(children) ======";
LET $sample = (SELECT id, dbnum, array::len(children ?? []) AS child_cnt, count(<-pe_owner) AS edge_cnt FROM pe WHERE ($target_dbnum == NONE OR dbnum == $target_dbnum) AND array::len(children ?? []) > 0 LIMIT 2000);
RETURN { sampled: $sample.len(), mismatched: $sample.filter(|$r| $r.child_cnt != $r.edge_cnt).len() };
RETURN array::slice($sample.filter(|$r| $r.child_cnt != $r.edge_cnt), 0, 50);

RETURN "====== [3] sampled childless rows: stale pe_owner edges ======";
LET $stale = (SELECT id, dbnum, count(<-pe_owner) AS edge_cnt FROM pe WHERE ($target_dbnum == NONE OR dbnum == $target_dbnum) AND array::len(children ?? []) == 0 LIMIT 2000).filter(|$r| $r.edge_cnt > 0);
RETURN { stale: $stale.len() };
RETURN array::slice($stale, 0, 50);

RETURN "====== audit_pe_owner_vs_children end ======";
