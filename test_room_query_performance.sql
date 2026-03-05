-- 房间计算查询性能测试
-- 连接命令: surreal sql --endpoint http://127.0.0.1:8009 --user root --pass root --ns 1516 --db AvevaMarineSample

-- 测试1：检查数据量
SELECT count() FROM FRMW WHERE NAME CONTAINS 'ROOM' GROUP ALL;
SELECT count() FROM SBFR GROUP ALL;
SELECT count() FROM PANE GROUP ALL;

-- 测试2：原始嵌套查询（限制10条）
SELECT VALUE [
    id,
    array::last(string::split(NAME, '-')),
    array::flatten(
        (SELECT VALUE
            (SELECT VALUE REFNO FROM PANE WHERE OWNER = $parent.REFNO)
         FROM SBFR WHERE OWNER = $parent.REFNO)
    )
] FROM FRMW
WHERE NAME IS NOT NONE AND NAME CONTAINS 'ROOM'
LIMIT 10;
