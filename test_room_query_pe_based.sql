-- 房间计算查询测试：panel_refno 回归 pe，字段用 pe 风格 (owner)
-- 连接: surreal sql --endpoint ws://localhost:8020 --namespace 1516 --database AvevaMarineSample -u root -p root --pretty

-- ========== 前置检查 ==========
SELECT count() FROM pe WHERE noun = 'PANE' GROUP ALL;
SELECT count() FROM FRMW WHERE NAME CONTAINS 'ROOM' GROUP ALL;

-- ========== 方式1：从 PANE 查询，owner 递归2层，REFNO 即 panel 的 pe ==========
SELECT owner.{2}.id AS frmw_id, array::last(string::split(owner.{2}.NAME, '-')) AS room_num, REFNO AS panel_refno FROM PANE WHERE owner.{2}.NAME CONTAINS 'ROOM' LIMIT 10;

-- ========== 方式2：同上，放宽条件（仅 owner.{2}.NAME 非空）==========
SELECT owner.{2}.id AS frmw_id, array::last(string::split(owner.{2}.NAME, '-')) AS room_num, REFNO AS panel_refno FROM PANE WHERE owner.{2}.NAME IS NOT NONE LIMIT 10;

-- ========== 方式3：从 pe 查询（pe.owner 须为 record，部分数据可能为 string 会报错）==========
SELECT id AS panel_refno, owner.{2}.id AS frmw_id, array::last(string::split(owner.{2}.name, '-')) AS room_num FROM pe WHERE noun = 'PANE' AND owner.{2}.name CONTAINS 'ROOM' LIMIT 10;

-- ========== 方式4：使用 LET 批量 ==========
LET $panel_pes = (SELECT VALUE id FROM pe WHERE noun = 'PANE' LIMIT 5000);
SELECT REFNO AS panel_refno, owner.{2}.id AS frmw_id, array::last(string::split(owner.{2}.NAME, '-')) AS room_num FROM PANE WHERE REFNO IN $panel_pes AND owner.{2}.NAME IS NOT NONE LIMIT 10;

-- ========== 方式5：显式 owner 链（owner.owner.owner，兼容性更好）==========
SELECT REFNO AS panel_refno, owner.owner.owner.id AS frmw_id, array::last(string::split(owner.owner.owner.NAME, '-')) AS room_num FROM PANE WHERE owner.owner.owner.NAME IS NOT NONE LIMIT 10;
