-- ============================================================
-- specs/022 T4: sesno_version_anchor 锚点链连续性审计
-- 用途：按 dbnum 升序输出全量锚点链，并列出"断链可疑项"——
--   from_sesno 存在（排除无 from_sesno 的 Legacy Anchor）
--   且 source != 'full'（全量锚点是基线重置语义，不参与衔接判定）
--   且 from_sesno != 同 dbnum 前一锚点 sesno + 1 的记录。
-- 修复口径（specs/022-versioned-pe-att-storage/ops-notes.md 第四节）：
--   历史断链只能对该 dbnum 全量重灌；锚点不可变，不做"补洞"式回填。
-- 用法：
--   powershell -File scripts/smoke/anchor_continuity_audit.ps1 -Ns <surreal_ns> -Db <project>
--   或 surreal sql --endpoint http://127.0.0.1:8030 --user root --pass root --ns <ns> --db <db> 逐条执行
--   （交互式 shell 按行解析，语句保持单行；LET 与后续 RETURN 需在同一会话）
-- 实现说明：SurrealQL 无窗口函数；且本 fork（surrealdb-3.2.0-nightly 实测）
--   $parent 关联子查询在 math::max(array::flatten([SELECT ...])) 形态下不回传
--   外层行值（恒空集），故"前一锚点"用 LET 全链快照 + 闭包 filter/map 计算：
--   prev = max(同 dbnum 且 sesno < 当前 sesno 的锚点 sesno)，前一锚点含 full
--   与 Legacy Anchor（两者都构成合法的衔接基准）。每 dbnum 首条锚点没有
--   前一锚点（prev_sesnos 为空），无从判定衔接，不列为可疑项。
-- ============================================================

RETURN "====== [1] anchor chain: all anchors ordered by (dbnum, sesno) ======";
LET $rows = (SELECT dbnum, sesno, from_sesno, source, fingerprint != NONE AS has_fingerprint, type::string(anchored_at) AS anchored_at FROM sesno_version_anchor ORDER BY dbnum ASC, sesno ASC);
RETURN $rows;

RETURN "====== [2] suspects: from_sesno set, source != 'full', from_sesno != prev anchor sesno + 1 ======";
RETURN $rows.filter(|$r| $r.from_sesno != NONE AND $r.source != 'full').map(|$r| { dbnum: $r.dbnum, sesno: $r.sesno, from_sesno: $r.from_sesno, source: $r.source, prev_sesnos: $rows.filter(|$p| $p.dbnum == $r.dbnum AND $p.sesno < $r.sesno).map(|$p| $p.sesno) }).filter(|$s| $s.prev_sesnos.len() > 0 AND $s.from_sesno != math::max($s.prev_sesnos) + 1).map(|$s| { dbnum: $s.dbnum, sesno: $s.sesno, from_sesno: $s.from_sesno, source: $s.source, prev_sesno: math::max($s.prev_sesnos), expected_from: math::max($s.prev_sesnos) + 1 });

RETURN "====== audit_anchor_continuity end ======";
