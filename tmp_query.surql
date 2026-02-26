-- A: 嵌套子查询中 out 是什么？
SELECT (SELECT out FROM out->geo_relate) as test FROM inst_relate:⟨17496_106028⟩;

-- B: 直接用 inst_info record id 硬编码（绕过 out 作用域）
SELECT * FROM inst_info:⟨75144747916844⟩->geo_relate WHERE visible && geo_type IN ['Pos', 'DesiPos', 'CatePos'];

-- C: 用 $parent 引用外层的 out
SELECT in.id as refno, (SELECT record::id(out) as geo_hash FROM $parent.out->geo_relate WHERE visible && geo_type IN ['Pos', 'DesiPos', 'CatePos']) as insts FROM inst_relate:⟨17496_106028⟩;

-- D: 最简嵌套测试——子查询 FROM out->geo_relate 是否能解析
SELECT (SELECT * FROM out->geo_relate) as gr FROM inst_relate:⟨17496_106028⟩;

-- E: 直接链式访问（不用子查询）
SELECT in.id as refno, out->geo_relate.* as geo_data FROM inst_relate:⟨17496_106028⟩;
