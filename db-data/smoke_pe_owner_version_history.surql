-- Phase 0 第二项：pe_owner 图遍历在 VERSION 下是否返回“历史态”（而非静默返回当前态）
-- 背景：drop-now 删掉 pe.children 后，e3d_tree 版本分支 query_children_refnos_versioned
--   （use_pe_owner=true）的 `SELECT VALUE in FROM {parent}<-pe_owner ORDER BY id VERSION d't'`
--   成为边界内历史层级的唯一路径，且再无 pe.children 兜底。本 smoke 用生产同款查询形式，
--   在 DB 内直接算出布尔 gate，避免依赖人工核对（现有 pe_owner_version_capability_smoke 只判语法 OK）。
-- 判定（由脚本读末尾 RETURN 对象里的 gate_* 布尔）：
--   gate_children_t1_ok  : 边遍历 VERSION $t1 == [pe:a, pe:b]（历史态正确）
--   gate_children_now_ok : 边遍历 当前 == [pe:a, pe:c]
--   gate_time_travels    : t1 结果 == 历史 且 != 当前（证明确实回溯、不是巧合返回当前态）
--   三者全 true → pe_owner 图遍历可承载版本读，drop-now 可去掉 children 兜底。
-- info_* 为对照观测：id 区间扫（research C3，预期 broken）与 pe.children 兜底（预期可用，正是要放弃的）。
-- 依赖：以 versioned=true 启动的 surreal 实例（db-data/run_surrealkv_versioned.ps1）；数据落独立 ns/db，不碰业务。

REMOVE TABLE IF EXISTS pe;
REMOVE TABLE IF EXISTS pe_owner;

-- ===== 状态 1：parent p 有序子 [a, b] =====
CREATE pe:p SET name = 'parent', noun = 'ZONE', children = [pe:a, pe:b];
CREATE pe:a SET name = 'childA', noun = 'EQUI', owner = pe:p;
CREATE pe:b SET name = 'childB', noun = 'EQUI', owner = pe:p;
INSERT RELATION INTO pe_owner [
    { id: pe_owner:[pe:p, 0], in: pe:a, out: pe:p },
    { id: pe_owner:[pe:p, 1], in: pe:b, out: pe:p }
];

SLEEP 200ms;
LET $t1 = time::now();
SLEEP 200ms;

-- ===== 状态 2：删 b、增 c、a 改名、p.children 改 [a,c]（模拟一次增量）=====
CREATE pe:c SET name = 'childC', noun = 'PIPE', owner = pe:p;
DELETE pe_owner:[pe:p, 1];
DELETE pe:b;
UPDATE pe:a SET name = 'childA_renamed';
UPDATE pe:p SET children = [pe:a, pe:c];
INSERT RELATION INTO pe_owner [
    { id: pe_owner:[pe:p, 1], in: pe:c, out: pe:p }
];
SLEEP 200ms;

-- ===== 断言用取值（生产同款查询形式）=====
-- 边遍历 + ORDER BY id（与 e3d_tree_api::query_children_refnos_versioned 完全一致）
LET $edges_t1  = (SELECT VALUE in FROM pe:p<-pe_owner ORDER BY id VERSION $t1);
LET $edges_now = (SELECT VALUE in FROM pe:p<-pe_owner ORDER BY id);
-- 对照：id 区间扫（research C3，预期 VERSION 不生效、返回当前态）
LET $idrange_t1 = (SELECT VALUE in FROM pe_owner:[pe:p, 0]..=[pe:p, 4294967295] VERSION $t1);
-- 对照：pe.children 字段 VERSION 点查（正是 drop-now 要放弃的兜底）
LET $childrenfield_t1 = (SELECT VALUE children FROM ONLY pe:p VERSION $t1);

RETURN {
    edges_t1: $edges_t1,
    edges_now: $edges_now,
    idrange_t1: $idrange_t1,
    childrenfield_t1: $childrenfield_t1,
    gate_children_t1_ok:  $edges_t1 == [pe:a, pe:b],
    gate_children_now_ok: $edges_now == [pe:a, pe:c],
    gate_time_travels:    ($edges_t1 == [pe:a, pe:b]) AND ($edges_t1 != $edges_now),
    info_idrange_t1_history_ok: $idrange_t1 == [pe:a, pe:b],
    info_childrenfield_t1_ok:   $childrenfield_t1 == [pe:a, pe:b]
};
