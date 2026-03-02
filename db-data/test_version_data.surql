-- ============================================================
-- SurrealKV MVCC 版本管理测试脚本
-- 前置条件: surrealkv 启动时需要 versioned=true
-- 连接: surreal sql --conn http://127.0.0.1:8030 --user root --pass root --ns test --db version_test
-- ============================================================

-- 1. 初始化命名空间和数据库
-- ============================================================
USE NS test DB version_test;

-- 2. 定义表结构
-- ============================================================
DEFINE TABLE pe TYPE NORMAL SCHEMALESS;
DEFINE TABLE pe_owner TYPE RELATION;

-- 3. 插入 PE 层级数据 (模拟 SITE → ZONE → EQUI)
-- ============================================================

-- SITE 节点
CREATE pe:site_001 CONTENT {
    refno: '10000_100001',
    owner: '0_0',
    name: '/SITE-001',
    noun: 'SITE',
    dbnum: 7997,
    sesno: 1,
    deleted: false,
    lock: false,
    status_code: 'A',
    children: [pe:zone_001]
};

-- ZONE 节点
CREATE pe:zone_001 CONTENT {
    refno: '10000_100010',
    owner: '10000_100001',
    name: '/ZONE-A',
    noun: 'ZONE',
    dbnum: 7997,
    sesno: 1,
    deleted: false,
    lock: false,
    status_code: 'A',
    children: [pe:equi_001, pe:equi_002],
    attrs: {
        PURP: 'General',
        LOCK: false
    }
};

-- EQUI 节点 1 - 后续会多次修改
CREATE pe:equi_001 CONTENT {
    refno: '10000_200001',
    owner: '10000_100010',
    name: '/EQUI-PUMP-A01',
    noun: 'EQUI',
    dbnum: 7997,
    sesno: 1,
    deleted: false,
    lock: false,
    status_code: 'A',
    children: [],
    attrs: {
        BORE: 150,
        HEIG: 3000,
        WEIG: 1200.5,
        DESC: '初始版本 - 水泵设备',
        FUNC: 'PUMP'
    }
};

-- EQUI 节点 2 - 后续会被删除
CREATE pe:equi_002 CONTENT {
    refno: '10000_200002',
    owner: '10000_100010',
    name: '/EQUI-VALVE-B01',
    noun: 'EQUI',
    dbnum: 7997,
    sesno: 1,
    deleted: false,
    lock: false,
    status_code: 'A',
    children: [],
    attrs: {
        BORE: 100,
        HEIG: 500,
        WEIG: 80.0,
        DESC: '初始版本 - 阀门',
        FUNC: 'VALVE'
    }
};

-- 建立 pe_owner 父子关系
RELATE pe:zone_001 -> pe_owner -> pe:site_001;
RELATE pe:equi_001 -> pe_owner -> pe:zone_001;
RELATE pe:equi_002 -> pe_owner -> pe:zone_001;

-- 记录 T1 时间戳
LET $t1 = time::now();
RETURN "====== T1 (初始版本创建完成) ======";
RETURN $t1;

-- 等待 2 秒确保时间戳分离
SLEEP 2s;

-- 4. 第一次修改 (v1 → v2): 修改 EQUI-001 的属性
-- ============================================================
RETURN "====== 执行第一次修改 (v2) ======";

UPDATE pe:equi_001 SET
    name = '/EQUI-PUMP-A01-REV2',
    sesno = 2,
    attrs.BORE = 200,
    attrs.HEIG = 3500,
    attrs.DESC = '第二版 - 增大口径和高度',
    attrs.MATERIAL = 'SS316L';

-- 记录 T2 时间戳
LET $t2 = time::now();
RETURN "====== T2 (第一次修改完成) ======";
RETURN $t2;

-- 等待 2 秒
SLEEP 2s;

-- 5. 第二次修改 (v2 → v3): 再次修改 EQUI-001 + 软删除 EQUI-002
-- ============================================================
RETURN "====== 执行第二次修改 (v3) ======";

UPDATE pe:equi_001 SET
    name = '/EQUI-PUMP-A01-FINAL',
    sesno = 3,
    attrs.BORE = 250,
    attrs.HEIG = 4000,
    attrs.WEIG = 1500.0,
    attrs.DESC = '最终版 - 最终设计参数',
    attrs.MATERIAL = 'SS316L',
    attrs.INSULATION = 'YES';

-- 软删除 EQUI-002
UPDATE pe:equi_002 SET
    deleted = true,
    sesno = 2,
    attrs.DESC = '已删除 - 设计方案变更';

-- 更新 ZONE 的 children（移除已删除的元素）
UPDATE pe:zone_001 SET
    children = [pe:equi_001],
    sesno = 2;

-- 记录 T3 时间戳
LET $t3 = time::now();
RETURN "====== T3 (第二次修改完成) ======";
RETURN $t3;

-- ============================================================
-- 6. 版本查询验证 (Time Travel)
-- ============================================================

RETURN "==========================================";
RETURN "  开始 Time Travel 版本查询验证";
RETURN "==========================================";

-- 6.1 查询当前最新状态
RETURN ">>> [当前] 所有 PE 元素:";
SELECT id, name, noun, sesno, deleted, attrs FROM pe;

-- 6.2 查询 T1 时刻的快照 (初始版本)
RETURN ">>> [T1] 初始版本快照:";
SELECT id, name, noun, sesno, deleted, attrs FROM pe VERSION $t1;

-- 6.3 查询 T2 时刻的快照 (第一次修改后)
RETURN ">>> [T2] 第一次修改后的快照:";
SELECT id, name, noun, sesno, deleted, attrs FROM pe VERSION $t2;

-- 6.4 对比 EQUI-001 的版本变化
RETURN ">>> EQUI-001 版本对比:";

RETURN "  - T1 (初始):";
SELECT name, sesno, attrs.BORE AS bore, attrs.HEIG AS heig, attrs.DESC AS desc
    FROM pe:equi_001 VERSION $t1;

RETURN "  - T2 (第一次修改):";
SELECT name, sesno, attrs.BORE AS bore, attrs.HEIG AS heig, attrs.DESC AS desc, attrs.MATERIAL AS material
    FROM pe:equi_001 VERSION $t2;

RETURN "  - 当前 (最终):";
SELECT name, sesno, attrs.BORE AS bore, attrs.HEIG AS heig, attrs.DESC AS desc, attrs.MATERIAL AS material, attrs.INSULATION AS insulation
    FROM pe:equi_001;

-- 6.5 验证已删除元素在历史中仍可查询
RETURN ">>> EQUI-002 删除前后对比:";

RETURN "  - T1 (存在):";
SELECT name, deleted, attrs.DESC AS desc FROM pe:equi_002 VERSION $t1;

RETURN "  - 当前 (已删除):";
SELECT name, deleted, attrs.DESC AS desc FROM pe:equi_002;

-- 6.6 验证 ZONE children 的历史
RETURN ">>> ZONE children 历史:";

RETURN "  - T1 (2个子元素):";
SELECT name, children FROM pe:zone_001 VERSION $t1;

RETURN "  - 当前 (1个子元素):";
SELECT name, children FROM pe:zone_001;

-- 6.7 使用新旧对比模式
RETURN ">>> 结构化新旧对比 (EQUI-001):";
LET $old_equi = (SELECT * FROM ONLY pe:equi_001 VERSION $t1);
LET $new_equi = (SELECT * FROM ONLY pe:equi_001);
RETURN { compare: 'EQUI-001', old_name: $old_equi.name, new_name: $new_equi.name, old_bore: $old_equi.attrs.BORE, new_bore: $new_equi.attrs.BORE, old_sesno: $old_equi.sesno, new_sesno: $new_equi.sesno };

RETURN "==========================================";
RETURN "  版本管理测试完成!";
RETURN "==========================================";
