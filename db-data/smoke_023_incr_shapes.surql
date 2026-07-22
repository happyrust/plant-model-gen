-- specs/023 T009（契约级）：精确镜像 sesno_increment T004 生成的增量语句形态，
-- 验证 graph-delete（DELETE <rec><-pe_owner / ->pe_owner）+ 先删后插 在 versioned 实例上的
-- 当前态与 VERSION 历史语义。运行：scripts/smoke/pe_owner_incr_shapes_smoke.ps1
-- （独立 ns/db，可重复执行）

-- ===== 清场 =====
REMOVE TABLE IF EXISTS pe;
REMOVE TABLE IF EXISTS pe_owner;

-- ===== 批次 1：模拟 full/首次增量（p 带子 [a, b]，完全按 T004 语句形态）=====
UPSERT pe:p CONTENT { name: 'parent', noun: 'ZONE', children: [pe:a, pe:b] };
UPSERT pe:a CONTENT { name: 'childA', noun: 'EQUI', owner: pe:p };
UPSERT pe:b CONTENT { name: 'childB', noun: 'EQUI', owner: pe:p };
-- 先删后插（对空边集 DELETE 应为 no-op 不报错）
DELETE pe:p<-pe_owner;
INSERT RELATION INTO pe_owner [
    { id: pe_owner:[pe:p, 0], in: pe:a, out: pe:p },
    { id: pe_owner:[pe:p, 1], in: pe:b, out: pe:p }
];

SLEEP 200ms;
LET $t1 = time::now();
SLEEP 200ms;

-- ===== 批次 2：模拟一次增量（sesno 顺序）：删 b、增 c、p 子列表 [a, c] =====
-- (1) Deleted b（build_increment_delete_statements + T004 边清理，语句顺序一致）
DELETE pe:b;
DELETE pe:b->pe_owner;
DELETE pe:b<-pe_owner;
-- (2) Modified p（pe UPSERT 先于边重写）
UPSERT pe:p CONTENT { name: 'parent', noun: 'ZONE', children: [pe:a, pe:c] };
DELETE pe:p<-pe_owner;
INSERT RELATION INTO pe_owner [
    { id: pe_owner:[pe:p, 0], in: pe:a, out: pe:p },
    { id: pe_owner:[pe:p, 1], in: pe:c, out: pe:p }
];
-- (3) Add c
UPSERT pe:c CONTENT { name: 'childC', noun: 'PIPE', owner: pe:p };

SLEEP 200ms;
LET $t2 = time::now();
SLEEP 200ms;

-- ===== 校验 =====

-- [V1] graph-delete 只删边不删源记录：p/a 当前均存在
-- 期望: parent
SELECT VALUE name FROM pe:p;
-- 期望: childA
SELECT VALUE name FROM pe:a;

-- [V2] 当前态 children（主查询形态 + 显式排序）
-- 注：`ORDER BY record::id(id)[1]` 是解析错误（fork 不接受 ORDER BY 内函数表达式），
--     以下两种排序写法作为候选（V2a 直接按边 id 结构序；V2b 子查询别名排序）
-- 期望: [pe:a, pe:c]
SELECT VALUE in FROM pe:p<-pe_owner ORDER BY id;
-- 期望: [pe:a, pe:c]
SELECT VALUE in FROM (SELECT in, record::id(id)[1] AS ord FROM pe:p<-pe_owner ORDER BY ord);

-- [V3] t1 历史 children（图遍历 + VERSION）
-- 期望: [pe:a, pe:b]
SELECT VALUE in FROM pe:p<-pe_owner VERSION $t1;

-- [V4] t2 历史 children
-- 期望: [pe:a, pe:c]
SELECT VALUE in FROM pe:p<-pe_owner VERSION $t2;

-- [V5] 已删除元素 b 的 t1 快照（名称）与当前态（空）
-- 期望: childB
SELECT VALUE name FROM pe:b VERSION $t1;
-- 期望: 空
SELECT VALUE name FROM pe:b;

-- [V6] 保底路径：children 字段 VERSION
-- 期望: [pe:a, pe:b]
SELECT VALUE children FROM pe:p VERSION $t1;
-- 期望: [pe:a, pe:c]
SELECT VALUE children FROM pe:p VERSION $t2;

-- [V7] 批次 1 的先删后插对空边集不报错已由批次 1 本身证明（走到这里即通过）
RETURN 'ok';
