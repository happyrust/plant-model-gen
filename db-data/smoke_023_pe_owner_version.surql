-- specs/023 FR-011 能力验证：versioned 实例上 pe_owner 边的 VERSION 查询能力
-- 运行方式：scripts/smoke/pe_owner_version_capability_smoke.ps1（HTTP POST /sql，整体一个请求，LET 变量全程有效）
-- 数据落在独立 db（默认 smoke_023_cap），不触碰业务/夹具数据。

-- ===== 清场（可重复执行）=====
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

-- ===== 状态 2：删 b、增 c、a 改名（模拟一次增量）=====
CREATE pe:c SET name = 'childC', noun = 'PIPE', owner = pe:p;
DELETE pe_owner:[pe:p, 1];
DELETE pe:b;
UPDATE pe:a SET name = 'childA_renamed';
UPDATE pe:p SET children = [pe:a, pe:c];
INSERT RELATION INTO pe_owner [
    { id: pe_owner:[pe:p, 1], in: pe:c, out: pe:p }
];

SLEEP 200ms;
LET $t2 = time::now();
SLEEP 200ms;

-- ===== 能力探测 =====

-- [C0] 基线：VERSION 点查（022 已验证的用法，必须通过）
-- 期望: childA
SELECT VALUE name FROM pe:a VERSION $t1;
-- 期望: childA_renamed
SELECT VALUE name FROM pe:a VERSION $t2;

-- [C1] VERSION + 图遍历（FROM <record><-pe_owner）
-- 期望 t1: [pe:a, pe:b]；t2: [pe:a, pe:c]
SELECT VALUE in FROM pe:p<-pe_owner VERSION $t1;
SELECT VALUE in FROM pe:p<-pe_owner VERSION $t2;

-- [C2] VERSION + 图 idiom（<-pe_owner.in on record）
-- 期望 t1: [pe:a, pe:b]；t2: [pe:a, pe:c]
SELECT VALUE <-pe_owner.in FROM ONLY pe:p VERSION $t1;
SELECT VALUE <-pe_owner.in FROM ONLY pe:p VERSION $t2;

-- [C3] VERSION + record id 区间扫（数组 id 前缀区间）
-- 期望 t1: [pe:a, pe:b]；t2: [pe:a, pe:c]
SELECT VALUE in FROM pe_owner:[pe:p, 0]..=[pe:p, 4294967295] VERSION $t1;
SELECT VALUE in FROM pe_owner:[pe:p, 0]..=[pe:p, 4294967295] VERSION $t2;

-- [C4] 保底路径：pe.children 字段 VERSION 点查
-- 期望 t1: [pe:a, pe:b]；t2: [pe:a, pe:c]
SELECT VALUE children FROM pe:p VERSION $t1;
SELECT VALUE children FROM pe:p VERSION $t2;

-- [C5] 已删除节点的历史点查（删除语义）
-- 期望 t1: childB；当前: NONE/空
SELECT VALUE name FROM pe:b VERSION $t1;
SELECT VALUE name FROM pe:b;

-- [C6] INSERT RELATION 撞已存在 id 的行为（幂等策略选型依据）
-- 观察：报错 / 忽略 / 覆盖
INSERT RELATION INTO pe_owner [
    { id: pe_owner:[pe:p, 0], in: pe:c, out: pe:p }
];
-- 撞 id 后实际保存的 in（期望仍是 pe:a → INSERT 不覆盖）
SELECT VALUE in FROM pe_owner:[pe:p, 0];

-- [C7] INSERT IGNORE 与 RELATION 组合的语法支持度由 smoke 脚本单独探测
-- （`INSERT IGNORE RELATION INTO` 在 fork 上是解析错误，会毁掉整个批次，故不放本文件）

-- [C8] 同值重插（in/out 与既有边完全一致）行为确认（tasks T001）
-- 观察：成功或报错均可接受；关键是边保持 in=pe:a 不被破坏
INSERT RELATION INTO pe_owner [
    { id: pe_owner:[pe:p, 0], in: pe:a, out: pe:p }
];
SELECT VALUE in FROM pe_owner:[pe:p, 0];
