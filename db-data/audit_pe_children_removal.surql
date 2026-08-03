-- ============================================================
-- Phase 0 go/no-go 审计：去掉 pe.children、统一走 pe_owner 前的完整性闸门
-- 与 specs/023 的 audit_pe_owner_vs_children.surql 区别：
--   * 这里是**全量**逐 parent 比对（不是抽样）——删字段不可逆，必须全量确认；
--   * 额外汇总各 dbnum 的 maintained_since_sesno，并列出**没有可信边界**的 dbnum
--     （drop-now 策略下，这些 dbnum 的边界前历史层级将彻底不可回溯）。
-- 判定：
--   [2] mismatched > 0 → 该库 pe_owner 边不完整/陈旧 → 删 children 前先 `model-version rebuild-pe-owner`
--   [3] stale > 0      → childless 行残留脏边 → 同样 rebuild（先删后插会清掉）
--   [5] without_meta   → 这些 dbnum 无 pe_owner 可信起点：latest 是否安全仅由 [2] 决定，
--                        但删 children 后它们**没有任何历史层级**可查（drop-now 已接受）
-- 用法：
--   powershell -File scripts/smoke/pe_children_removal_audit.ps1 -Ns <surreal_ns> -Db <project> [-Dbnum <n>] [-Limit 0]
--   （脚本会替换下方 LET $target_dbnum 与 /*__ROWS_LIMIT__*/；直连执行时手工改即可）
-- 说明：全量 count(<-pe_owner) 是逐行图计数，大表较重；快速体检可用 -Limit 2000 走抽样。
-- ============================================================

RETURN "====== [0] target scope ======";
LET $target_dbnum = NONE;
RETURN { target_dbnum: $target_dbnum };

RETURN "====== [1] totals: parents_with_children / children_total per dbnum + edge rows ======";
RETURN (SELECT dbnum, count() AS parents_with_children, math::sum(array::len(children ?? [])) AS children_total FROM pe WHERE ($target_dbnum == NONE OR dbnum == $target_dbnum) AND array::len(children ?? []) > 0 GROUP BY dbnum);
RETURN (SELECT count() AS pe_owner_edge_rows FROM pe_owner GROUP ALL);

RETURN "====== [2] FULL parents(with children): count(<-pe_owner) vs len(children) ======";
LET $rows = (SELECT id, dbnum, array::len(children ?? []) AS child_cnt, count(<-pe_owner) AS edge_cnt FROM pe WHERE ($target_dbnum == NONE OR dbnum == $target_dbnum) AND array::len(children ?? []) > 0 /*__ROWS_LIMIT__*/);
RETURN { parents: $rows.len(), mismatched: $rows.filter(|$r| $r.child_cnt != $r.edge_cnt).len() };
RETURN array::slice($rows.filter(|$r| $r.child_cnt != $r.edge_cnt), 0, 100);

RETURN "====== [3] FULL childless rows: stale pe_owner edges ======";
LET $stale = (SELECT id, dbnum, count(<-pe_owner) AS edge_cnt FROM pe WHERE ($target_dbnum == NONE OR dbnum == $target_dbnum) AND array::len(children ?? []) == 0 /*__ROWS_LIMIT__*/).filter(|$r| $r.edge_cnt > 0);
RETURN { stale_childless: $stale.len() };
RETURN array::slice($stale, 0, 100);

RETURN "====== [4] maintained_since_sesno per dbnum (pe_owner_version_meta) ======";
RETURN (SELECT dbnum, maintained_since_sesno, source, type::string(updated_at) AS updated_at FROM pe_owner_version_meta WHERE ($target_dbnum == NONE OR dbnum == $target_dbnum) ORDER BY dbnum);

RETURN "====== [5] dbnums in pe without a pe_owner trust boundary ======";
LET $pe_dbnums = (SELECT VALUE dbnum FROM pe WHERE ($target_dbnum == NONE OR dbnum == $target_dbnum) GROUP BY dbnum);
LET $meta_dbnums = (SELECT VALUE dbnum FROM pe_owner_version_meta WHERE ($target_dbnum == NONE OR dbnum == $target_dbnum));
RETURN { pe_dbnums: $pe_dbnums, meta_dbnums: $meta_dbnums, without_meta: $pe_dbnums.filter(|$d| !array::contains($meta_dbnums, $d)) };

RETURN "====== audit_pe_children_removal end ======";
