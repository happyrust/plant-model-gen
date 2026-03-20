-- =============================================================================
-- 17496_106028 / 17496_142306 布尔运算问题排查脚本
-- =============================================================================
-- 用法: 在 SurrealDB Web 或 CLI 中执行（NS: 1516, DB: AvevaMarineSample）
-- 执行 regen-model 后、导出前或导出后运行均可
-- =============================================================================

-- 1. 检查 17496_142306 的 inst_relate：has_cata_neg 是否为 true？
SELECT id, in, in.id as refno, has_cata_neg, solid 
FROM inst_relate 
WHERE in.id = 17496_142306 OR in.id = 17496_142305 OR in.id = 17496_106028;

-- 2. 检查 geo_relate 中 17496_142306 的几何体：是否有 cata_neg / CatePos？
SELECT in, geom_refno, geo_type, visible, cata_neg
FROM inst_relate:⟨17496_142306⟩->out->geo_relate
WHERE visible;

SELECT in, geom_refno, geo_type, visible, cata_neg
FROM inst_relate:⟨17496_142305⟩->out->geo_relate
WHERE visible;

-- 3. 全库 has_cata_neg 统计：catalog 布尔任务应来自这里
SELECT count() FROM inst_relate WHERE has_cata_neg = true;
SELECT value in.id FROM inst_relate WHERE has_cata_neg = true LIMIT 20;

-- 4. 检查 inst_relate_cata_bool（catalog 布尔结果状态）
SELECT * FROM inst_relate_cata_bool 
WHERE in = inst_relate:⟨17496_142306⟩ OR in = inst_relate:⟨17496_142305⟩;

-- 5. Cross neg 关系：17496_106028 被谁切割？(ngmr_relate / neg_relate)
SELECT in, out, pe FROM neg_relate WHERE out = pe:⟨17496_106028⟩ LIMIT 5;
SELECT in, out, pe FROM ngmr_relate WHERE out = pe:⟨17496_106028⟩ LIMIT 5;

-- 6. 实例级布尔目标来源
SELECT value out FROM neg_relate GROUP BY out;
SELECT value out FROM ngmr_relate GROUP BY out;
