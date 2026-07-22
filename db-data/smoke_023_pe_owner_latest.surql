-- specs/023 M0/T1 能力验证：latest（无 VERSION）pe_owner 图查询原语
-- 与 src/versioned_db/pe_owner_tree.rs 的 SQL 形态一一对应：
--   L1 children 同胞有序        SELECT VALUE in FROM <owner><-pe_owner ORDER BY id
--   L2 递归全子孙               <root>.{..256+collect}<-pe_owner<-pe（SurrealDB 3.1 递归 idiom）
--   L3 深度受限递归             <root>.{..2+collect}<-pe_owner<-pe
--   L4 祖先链（owner 链接递归）  <node>.{..256+collect}(.owner)
--   L5 children 计数 + 字段回退  SELECT VALUE { p, n: count(<-pe_owner), ch: children }
--   L6 批量 kids（边优先/字段回退）
--   L7 节点元信息批查（含 cata_hash 字符串字段）
--   L8-L11 noun 表级扫 + D3 索引（idx_pe_dbnum_noun）
-- 运行方式：scripts/smoke/pe_owner_latest_tree_smoke.ps1（HTTP POST /sql，独立 ns/db，不触碰业务数据）

-- ===== 清场（可重复执行）=====
REMOVE TABLE IF EXISTS pe;
REMOVE TABLE IF EXISTS pe_owner;
REMOVE TABLE IF EXISTS ele_reuse_relate;

-- ===== 夹具树 =====
--   w(WORL, owner 自指)
--   ├─ s1(SITE) ── z1(ZONE) ── e1(EQUI, 带 cata_hash), e2(EQUI)
--   └─ s2(SITE)
--   f(ZONE)：只有 children 字段、无 pe_owner 边（存量站点未重灌场景，验证回退）
CREATE pe:w  SET name = 'world', noun = 'WORL', dbnum = 9902, owner = pe:w,  children = [pe:s1, pe:s2];
CREATE pe:s1 SET name = 'site1', noun = 'SITE', dbnum = 9902, owner = pe:w,  children = [pe:z1];
CREATE pe:s2 SET name = 'site2', noun = 'SITE', dbnum = 9902, owner = pe:w,  children = [];
CREATE pe:z1 SET name = 'zone1', noun = 'ZONE', dbnum = 9902, owner = pe:s1, children = [pe:e1, pe:e2];
CREATE pe:e1 SET name = 'equi1', noun = 'EQUI', dbnum = 9902, owner = pe:z1, children = [], cata_hash = '12345678901234567890';
CREATE pe:e2 SET name = 'equi2', noun = 'EQUI', dbnum = 9902, owner = pe:z1, children = [];
CREATE pe:f  SET name = 'fieldonly', noun = 'ZONE', dbnum = 9902, owner = pe:s2, children = [pe:e2];
INSERT RELATION INTO pe_owner [
    { id: pe_owner:[pe:w, 0],  in: pe:s1, out: pe:w },
    { id: pe_owner:[pe:w, 1],  in: pe:s2, out: pe:w }
];
INSERT RELATION INTO pe_owner [
    { id: pe_owner:[pe:s1, 0], in: pe:z1, out: pe:s1 }
];
INSERT RELATION INTO pe_owner [
    { id: pe_owner:[pe:z1, 0], in: pe:e1, out: pe:z1 },
    { id: pe_owner:[pe:z1, 1], in: pe:e2, out: pe:z1 }
];

-- ===== 能力探测 =====

-- [L1] children 同胞有序（期望 [pe:e1, pe:e2]）
SELECT VALUE in FROM pe:z1<-pe_owner ORDER BY id;

-- [L2] 递归全子孙 +collect（期望 [pe:s1, pe:s2, pe:z1, pe:e1, pe:e2]，BFS 邻近序去重）
RETURN pe:w.{..256+collect}<-pe_owner<-pe;

-- [L3] 深度受限（期望 [pe:s1, pe:s2, pe:z1]）
RETURN pe:w.{..2+collect}<-pe_owner<-pe;

-- [L4] 祖先链 owner 链接递归（期望 [pe:z1, pe:s1, pe:w]；w owner 自指靠 +collect 去重终止）
RETURN pe:e1.{..256+collect}(.owner);

-- [L5] children 计数 + 字段回退（期望 z1: n=2；f: n=0 且 ch=[pe:e2]）
SELECT VALUE { p: id, n: count(<-pe_owner), ch: children } FROM [pe:z1, pe:f];

-- [L6] 批量 kids：边优先 / 字段回退输入（期望 z1.kids=[e1,e2]；f.kids=[] 且 f.ch=[pe:e2]）
SELECT VALUE { p: id, kids: <-pe_owner<-pe, ch: children } FROM [pe:z1, pe:f];

-- [L7] 节点元信息批查（期望 e1 带 cata_hash 字符串、z1 cata_hash 缺失）
SELECT id, noun, owner, cata_hash FROM [pe:e1, pe:z1];

-- [L8] noun 表级扫（索引前，期望 [pe:e1, pe:e2]）
SELECT VALUE id FROM pe WHERE dbnum = 9902 AND noun = 'EQUI';

-- [L9] D3 索引定义（幂等）
DEFINE INDEX IF NOT EXISTS idx_pe_dbnum_noun ON TABLE pe FIELDS dbnum, noun;

-- [L10] noun 表级扫（索引后，期望与 L8 一致）
SELECT VALUE id FROM pe WHERE dbnum = 9902 AND noun = 'EQUI';

-- [L11] noun 分组计数（期望 EQUI=2 SITE=2 ZONE=2 WORL=1）
SELECT noun, count() AS count FROM pe WHERE dbnum = 9902 GROUP BY noun;
