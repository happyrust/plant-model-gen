-- DEFINE FIELD refnos ON TABLE his_pe TYPE set<record<pe>>;
DEFINE TABLE IF NOT EXISTS inst_info;

-- 考虑到版本的原因，需要使用到历史祖先的记录，所以需要使用到这个函数
-- 获取节点的所有祖先节点（不包括根节点和空节点）
DEFINE FUNCTION OVERWRITE fn::ancestor($r: record) {
    return if !record::exists($r) {
        return [];
    } else {
        let $nodes = (SELECT VALUE @.{..+collect+inclusive}.owner FROM ONLY $r LIMIT 1) ?: [];
        return array::filter($nodes, |$v| $v != NONE && $v != pe:0_0);
    };
};



-- 获取节点的最新版本（如果不是数组ID则返回自身）
DEFINE FUNCTION OVERWRITE fn::newest_pe(
    $pe: record,
) {
    if $pe == none {
        return none;
    };
    let $id = record::id($pe);
    if !type::is_array($id) {
        return $pe;
    };
    let $refno = $id[0];
    return type::record('pe', $refno);
};

-- 获取节点的最新版本ID
DEFINE FUNCTION OVERWRITE fn::newest_pe_id(
    $pe: record,
) {
    if !type::is_array($pe) {
        return record::id($pe);
    };
    return record::id($pe)[0];
};


-- 获取节点在同类型兄弟节点中的序号
DEFINE FUNCTION OVERWRITE fn::order(
    $pe: record,
) {
    if $pe.owner == none {
        return none;
    };
    let $a = ($pe.owner<-pe_owner.in)[where noun=$pe.noun];
    if $a == none {
        let $b = (fn::newest_pe($pe).owner<-pe_owner.in)[where noun=$pe.noun];
        return array::find_index($b, $pe);
    } else {
        return array::find_index($a, $pe);
    };
};

-- 获取节点的默认名称（如果名称为空则自动生成）
DEFINE FUNCTION OVERWRITE fn::default_name(
    $pe: record,
) {
    return (select value (if name=='' {string::join(' ', noun, <string> (fn::order($pe)?:0 + 1) ) } else { name } ) from $pe)[0];
};

-- 批量获取节点的默认名称
DEFINE FUNCTION OVERWRITE fn::default_names(
    $pes: array<record>,
) {
    return select value (if name=='' {string::join(' ', noun,<string> (
       fn::order(id)?:0 + 1) ) } else { name } ) from $pes;
};


-- 获取节点的完整默认名称（包含所有祖先信息）
DEFINE FUNCTION OVERWRITE fn::default_full_name($r: record) {
    return if !record::exists($r) {
        return '';
    }else if $r.name != '' && $r.name != none {
        return $r.name;
    } else if $r.owner.name != '' && $r.owner.name != none {
        return string::concat(fn::default_name($r), " OF ", $r.owner.noun, ' ', $r.owner.name);
    } else {
        let $a = fn::ancestor($r);
        let $b = array::len($a) - array::find_index(array::reverse($a).name, '');
        let $i = array::slice($a, 0, $b);
        let $c = $b + 1;
        return string::concat(array::join(fn::default_names($i), " OF "), " OF ", $a[$c].noun, ' ', $a[$c].name);
    };
};


//备份修改的数据，只针对修改的处理
//如果是新增，应该是对pe_owner的备份
-- 备份数据：处理修改和删除操作的数据备份
DEFINE FUNCTION OVERWRITE fn::backup_data(
    $pes: array<record>,
    $is_del: bool,
    $sesno: number,
) {
    for $pe in $pes {
        //只操作最新的 refno
        if type::is_array(record::id($pe)) || !record::exists($pe) {
            continue;
        };
        if $pe.sesno > $sesno {
            continue;
        };
        let $old_id = [<string> record::id($pe.id), $pe.sesno];
        let $old_pe_id = type::record('pe', $old_id);

        let $a = (select * omit id from $pe)[0];
        let $b = (select * omit id from $pe.refno)[0];

        if $a != none && $b != none {
            let $old_inst_relate_id = type::record('inst_relate', $old_id);
            let $old_att_id = type::record(string::concat($a.noun, '_H'), $old_id);
            let $old_uda_att_id = type::record('ATT_UDA_H', $old_id);
            if $is_del{
                //软删除
                update $pe.id set op=2, old_pe=$old_pe_id, sesno=$sesno, deleted=true;
            } else {
                update $pe.id set op=1, old_pe=$old_pe_id, sesno = $sesno;
            };
            upsert type::record('his_pe', record::id($pe.id)) set refnos+=$old_pe_id;
            upsert $old_pe_id content $a.patch([{
                                  'op': 'replace',
                                  'path': 'refno',
                                  'value': $old_att_id
                                }]);
            //属性数据需要单独的备份方法，因为有很多引用
            upsert $old_att_id content $b.patch([{
                                  'op': 'replace',
                                  'path': 'REFNO',
                                  'value': $old_pe_id
                                }]);
            //uda 属性需要单独的备份方法，因为有很多引用
            let $uda_data = select * from only type::record('ATT_UDA', record::id($pe.id)) limit 1;
            if $uda_data != none {
                upsert $old_uda_att_id content $uda_data.patch([{
                                  'op': 'replace',
                                  'path': 'refno',
                                  'value': $old_pe_id
                                }]);
            };
            //删除属性
            if $is_del{
               delete $pe.refno;
            };

            //备份关联
            let $c = (select *, $old_inst_relate_id as id, $old_pe_id as in from $pe->inst_relate);
            if !array::is_empty($c) {
                delete select id from $pe->inst_relate;
                insert relation into inst_relate $c;
            };

            if $a.noun == "BRAN" || $a.noun == "HANG" {

                let $tubis = (select *, type::record('tubi_relate', [ $old_pe_id, record::id(id)[1] ]) as id
                                 from tubi_relate where record::id(id)[0] = $pe.id);
                //将 tubi relate 整体备份
                if !array::is_empty($tubis) {
                    //删除之前的，变成历史的
                    delete tubi_relate where record::id(id)[0] = $pe.id;
                    insert relation into tubi_relate $tubis;
                };
            };
        }
    }
};


//备份修改的数据，只针对修改的处理
//如果是新增，应该是对pe_owner的备份
-- 备份所有者关系：处理父子关系的备份
DEFINE FUNCTION OVERWRITE fn::backup_owner_relate(
    $pes: array<record>,
    $delete_old: bool,
) {
    for $pe in $pes {
        //只操作最新的 refno
        // if type::is_array(record::id($pe)) || !record::exists($pe) {
        //      continue;
        // };
        let $old_id = [<string> record::id($pe.id), $pe.sesno];
        let $old_pe_id = type::record('pe', $old_id);
        //fix 层级不连续的问题，需要更新到之前的 in->owner 关系
        //let $old_owners_id = select value od from $pe->pe_owner where type::is_array(record::id(out));
        -- return $old_owners_id;
        //如果是旧节点，需要更新到旧的引用节点
        //for $o in $old_owners_id {
            //需要删除了，重新创建这个单独的id
            //update $o set in=$old_pe_id;
        //};
        //如果子节点的sesno >= 父节点的sesno 了，则需要使用 old_pe
        //反之则不需要动
        let $c = select *, type::record('pe_owner', [ $old_pe_id, record::id(id)[1] ]) as id,
                    if $pe.sesno>in.sesno { in } else { in.old_pe?:in } as in, $old_pe_id as out
                    from $pe<-pe_owner;
        -- return $c;
        if !array::is_empty($c) {
            if $delete_old{
                delete select id from $pe<-pe_owner;
            };
            insert relation into pe_owner $c;
        }
    }
};

// let $pe = pe:17496_171606;
// fn::backup_owner_relate([$pe], false);

-- 获取节点在指定会话号或数据库号下的最新会话号
DEFINE FUNCTION OVERWRITE fn::latest_pe_sesno($pe: record, $s: option<number>, $db: option<number>){
    let $sesno = $pe.sesno?:0;
    let $r = return if $s != none and $db != none {
        #找到对应的时间
        let $t = ses:[$db, $s].date;
        return select value sesno from only pe_ses_h:[record::id($pe), 0]..[record::id($pe)] where ses.date<=$t ORDER BY sesno DESC limit 1;
    } else if $s != none {
        return if $s >= $sesno {
            return $sesno;
        } else{
            return select value sesno from only pe_ses_h:[record::id($pe), 0]..[record::id($pe)] where sesno<=$s ORDER BY sesno DESC limit 1;
        };
    } else {
        return $sesno
    };

    return [$r?:0, $sesno];
};

-- 获取节点在指定会话号或数据库号下的最新版本
DEFINE FUNCTION OVERWRITE fn::latest_pe($pe: record, $s: option<number>, $db: option<number>){
    let $s = fn::latest_pe_sesno($pe, $s, $db);
    return if $s[1] == $s[0]  {
        return $pe;
    } else{
        let $id = record::id($pe);
    return type::record('pe', [$id, $s[0]]);
    }
};


-- 根据日期时间查找节点
DEFINE FUNCTION OVERWRITE fn::find_pe_by_datetime($pe: record, $dt: datetime){
    let $his_pe = type::record('his_pe', fn::newest_pe_id($pe));
    if !record::exists($his_pe) {
        return $pe;
    };
    let $a = select value id from only array::reverse(array::flatten([$his_pe.refnos?:[], $pe])) where <datetime>fn::ses_date(id) <= $dt limit 1;
    return $a?:$pe;
};


-- fn::latest_pe_sesno(pe:17496_171715, 880, 1112);
-- fn::latest_pe_sesno(pe:17496_171715, 880);
-- fn::latest_pe_sesno(pe:17496_171715);

-- 获取节点的会话数据
DEFINE FUNCTION OVERWRITE fn::ses_data($pe: record){
    let $id = type::record('ses', [$pe.dbnum, $pe.sesno]);
    return select * from only $id limit 1;
};

-- 获取节点的会话日期
DEFINE FUNCTION OVERWRITE fn::ses_date($pe: record){
    return if record::exists($pe) {
        let $id = type::record('ses', [$pe.dbnum, $pe.sesno]);
        return select value date from only $id limit 1;
    } else {
        return none;
    };
};

-- 判断下一个节点是否为指定类型
DEFINE FUNCTION OVERWRITE fn::next_is_type($pe:record,$type:string) {
    return fn::next_type($pe)==$type;
};

-- 获取第一个子节点
DEFINE FUNCTION OVERWRITE fn::first_child($pe:record) {
    return select value (<-pe_owner[0].in) from only $pe limit 1;
};

-- 获取第一个非指定类型的子节点
DEFINE FUNCTION OVERWRITE fn::first_child_exclude_type($pe:record, $exclude_type: string) {
    return select value in from only $pe<-pe_owner where in.noun != $exclude_type limit 1;
};

-- 获取最后一个子节点
DEFINE FUNCTION OVERWRITE fn::last_child($pe:record) {
    return array::last((select value (<-pe_owner.in) from only $pe limit 1)?:[]);
};

-- 获取最后一个非指定类型的子节点
DEFINE FUNCTION OVERWRITE fn::last_child_exclude_type($pe:record, $exclude_type: string) {
    return (select * from only $pe<-pe_owner where in.noun != $exclude_type order by id desc limit 1).in;
};

-- 获取前一个兄弟节点
DEFINE FUNCTION OVERWRITE fn::prev_pe($pe:record) {
    if $pe == none || !record::exists($pe) {
        return none;
    };
    let $a = (record::id($pe->pe_owner.id[0])[1]) - 1;
    return select value (out<-pe_owner[$a].in) from only $pe->pe_owner limit 1;
};

-- 获取前一个非指定类型的兄弟节点
DEFINE FUNCTION OVERWRITE fn::prev_pe_exclude_type($pe:record, $exclude_type: string) {
    if $pe == none || !record::exists($pe) {
            return none;
    };
    let $a = (record::id($pe->pe_owner.id[0])[1]);
    return (select value in from ($pe->pe_owner->pe<-pe_owner)[..$a] where in.noun != $exclude_type).last();
};


-- 获取下一个兄弟节点
DEFINE FUNCTION OVERWRITE fn::next_pe($pe:record) {
    if $pe == none || !record::exists($pe) {
        return none;
    };
    let $a = (record::id($pe->pe_owner.id[0])[1]) + 1;
    return select value (out<-pe_owner[$a].in) from only $pe->pe_owner limit 1;
};

-- 获取下一个非指定类型的兄弟节点
DEFINE FUNCTION OVERWRITE fn::next_pe_exclude_type($pe:record, $exclude_type: string) {
    if $pe == none || !record::exists($pe) {
            return none;
        };
    let $a = (record::id($pe->pe_owner.id[0])[1]) + 1;
    return select value in from only ($pe->pe_owner->pe<-pe_owner)[$a..] where in.noun != $exclude_type  limit 1;
};

-- 获取前一个连接节点（跳过ATTA类型）
DEFINE FUNCTION OVERWRITE fn::prev_connect_pe($pe:record) {
    if $pe == none {
        return none;
    };
    let $p = fn::prev_pe_exclude_type($pe, 'ATTA');
    return if $p != none {
        return $p;
    } else {
        let $connect_bran = $pe.owner.refno.HREF;
        return if $connect_bran != none {
            return fn::last_child($connect_bran)?:$connect_bran;
        } else {
            return none;
        };
    };
};

-- 获取前一个连接节点的详细信息
DEFINE FUNCTION OVERWRITE fn::prev_connect_pe_data($pe:record) {
    let $p = fn::prev_connect_pe($pe);
    //如果有到达这个元件的直段，则返回这个直段的leave
    let $tmp_tubi_refno = fn::query_tubi_from($pe).leave;
    let $tubi_refno = if $tmp_tubi_refno != none {
        $tmp_tubi_refno
    } else if fn::query_tubi_to($pe.owner) != none {   //判断bran的最尾部是不是TUBI
        // 返回是否有bran开启的直段
        fn::query_tubi_to($pe.owner)
    } else {
       none
    };
    let $tubi_name = return if $tubi_refno == none {
        none
    } else { string::concat('ileave tube of ',  fn::default_full_name($tubi_refno)) };
    return if $p != none {   //如果存在上一个节点
        //如果tubi_refno是branch的最后一个节点，则需要判断关联的branch有tubi
        let $tubi_spref = return if $tubi_refno == none {
            none
        } else if $tubi_refno.noun == 'BRAN'{
            $tubi_refno.refno.HSTU.name
        } else {
            $tubi_refno.refno.LSTU.name
        };
        return {
            id: $pe,
            prev_full_name: fn::default_full_name($p),
            prev: $p,
            tubi_name: $tubi_name,
            tubi_refno: $tubi_refno,
            tubi_spref: $tubi_spref
        };
    } else {  //如果不存在上一个节点
        return if $tubi_refno != none {
            return {
                id: $pe,
                prev_full_name: fn::default_full_name($pe.owner),
                prev: $pe.owner,
                tubi_name: $tubi_name,
                tubi_refno: $pe.owner,
                tubi_spref: $pe.owner.refno.HSTU.name
            };
        } else {
            none
        };
    };
};

-- 获取下一个连接节点（跳过ATTA类型）
DEFINE FUNCTION OVERWRITE fn::next_connect_pe($pe:record) {
    if $pe == none {
        return none;
    };
    let $p = fn::next_pe_exclude_type($pe, 'ATTA');
    return if $p != none {
        return $p;
    } else {
        let $connect_bran = $pe.owner.refno.TREF;
        return if $connect_bran != none {
            return fn::first_child($connect_bran)?:$connect_bran;
        } else {
            return none;
        };
    };
};

-- 获取下一个连接节点的详细信息
DEFINE FUNCTION OVERWRITE fn::next_connect_pe_data($pe:record) {
    let $p = fn::next_connect_pe($pe);
    let $tubi_refno = if fn::query_tubi_to($pe) != none {
        $pe
    } else if fn::query_tubi_to($pe.owner.refno.TREF) != none {
        // 返回是否有bran开启的直段
      $pe.owner.refno.TREF
    } else {
       none
    };
    let $tubi_name = return if $tubi_refno == none {
       none
   } else { string::concat('ileave tube of ',  fn::default_full_name($pe)) };
    return if $p != none {
        let $tubi_spref = return if $tubi_refno == none {
            none
        } else if $tubi_refno.noun == 'BRAN'{
            $tubi_refno.refno.HSTU.name
        } else {
            $tubi_refno.refno.LSTU.name
        };
        return {
            id: $pe,
            next_full_name: fn::default_full_name($p),
            next: $p,
            tubi_refno: $tubi_refno,
            tubi_name: $tubi_name,
            tubi_spref: $tubi_spref
        };
    } else {
        return if $tubi_refno != none {
        return {
                id: $pe,
                next_full_name: fn::default_full_name($pe),
                next: $pe,
                tubi_refno: $tubi_refno,
                tubi_name: $tubi_name,
                tubi_spref: $pe.refno.LSTU.name
            };
        } else {
            none
        };
    };
};

-- 查询连接到指定节点的直段
DEFINE FUNCTION OVERWRITE fn::query_tubi_to($pe:record) {
    return select out from only $pe->tubi_relate limit 1;
};

-- 查询从指定节点出发的直段
DEFINE FUNCTION OVERWRITE fn::query_tubi_from($pe:record) {
    return select in from only $pe<-tubi_relate limit 1;
};

-- 查询分支的第一个直段
DEFINE FUNCTION OVERWRITE fn::query_bran_first_tubi($pe:record) {
    if $pe.noun != 'BRAN' {
        return none;
    };
    return tubi_relate:[$pe, 0].id;
};

-- 获取前一个节点的类型
DEFINE FUNCTION OVERWRITE fn::prev_type($pe:record) {
    return fn::prev_pe($pe).noun;
};

-- 获取下一个节点的类型
DEFINE FUNCTION OVERWRITE fn::next_type($pe:record) {
    return fn::next_pe($pe).noun;
};


-- 获取所有子节点（去重且排除已删除节点）
DEFINE FUNCTION OVERWRITE fn::children($pe:record) {
    if !record::exists($pe) {
        return [];
    };
    let $kids = $pe.children ?: [];
    return array::distinct(array::filter($kids, |$c: any|
        $c != NONE
        && record::exists($c)
        && !$c.deleted
    ));
};


// 获取ipara
// 获取保温层参数
-- 获取保温层参数
-- 根据 TEMP（温度）筛选 SELE，再根据 HBOR（孔径）筛选 SPCO
DEFINE FUNCTION OVERWRITE fn::get_ipara($pe:record) {
    // 获取父节点的 ISPE 引用
    let $ispe = $pe.refno.ISPE?:$pe.owner.refno.ISPE;
    return if $ispe == none {
        // 如果没有 ISPE 引用，返回默认参数 [0]
        return [0.0];
    } else {
        // 温度在 BRAN 上，也就是它的 owner
        let $temp = $pe.owner.refno.TEMP;
        // 孔径在 BRAN 上
        let $hbor = $pe.owner.refno.HBOR;

        // 1. 根据 TEMP 筛选 SELE 节点
        let $sele = (select * from only $ispe.{2}.children.refno.{children: REFNO.children, MAXA, ANSW, TYPE}
             where TYPE = 'SELE' and $temp >= ANSW and $temp <= MAXA
             limit 1);

        // 2. 在 SELE 的 children (SPCO) 中根据 HBOR 筛选
        let $spco = array::filter($sele.children, |$c| $hbor >= $c.refno.ANSW and $hbor <= $c.refno.MAXA)[0];

        // 3. 返回匹配 SPCO 的 CATR.PARA，如果没有匹配则返回第一个 SPCO 的值
        return if $spco != none {
            return $spco.refno.CATR.refno.PARA ?? [0.0];
        } else if $sele.children[0] != none {
            return $sele.children[0].refno.CATR.refno.PARA ?? [0.0];
        } else {
            return [0.0];
        };
    };
};

//根据类型找到父节点
-- 根据类型查找祖先节点
DEFINE FUNCTION OVERWRITE fn::find_ancestor_type($pe: record, $t: string){
    (array::flatten(select value fn::ancestor(id) from $pe)[?$self.noun=$t])[0]
};


// 返回类型集合的父节点
-- 根据类型集合查找祖先节点
DEFINE FUNCTION OVERWRITE fn::find_ancestor_types($pe: record, $t: array){
    let $r = (array::flatten(select value fn::ancestor(id) from $pe));
    return select value id from $r where noun in $t;
};

-- SITE 专业枚举初始化
DEFINE FUNCTION OVERWRITE fn::init_site_spec_value() {
    UPDATE pe SET spec_value =
        IF string::contains(name, 'PIPE') { 1 }
        ELSE IF string::contains(name, 'ELEC') { 2 }
        ELSE IF string::contains(name, 'INST') { 3 }
        ELSE IF string::contains(name, 'HVAC') { 4 }
        ELSE { 0 }
    WHERE noun = 'SITE';
};

// 返回符合类型集合的子节点ID
-- 收集符合指定类型的子孙节点ID
DEFINE FUNCTION OVERWRITE fn::collect_descendant_ids_by_types($pe:record,$t:array,$inclusive:option<bool>,$range_str:string) {
    let $infos = fn::collect_descendant_infos($pe, $t, $inclusive, $range_str);
    return array::map($infos, |$info| $info.id);
};

// 返回符合类型集合且具有 CATR 或 SPRE 属性的子节点ID
-- 收集符合指定类型的子孙节点ID，并过滤出具有 CATR 或 SPRE 属性的节点
-- 参数：
--   $root: 起始节点
--   $types: 要过滤的类型数组，空数组表示不过滤
--   $filter_inst: 是否过滤掉有 inst_relate 或 tubi_relate 的节点
--   $inclusive: 是否包含根节点本身
--   $range_str: 递归范围字符串，仅用于文档/注释
-- 返回：符合条件且 refno.CATR 或 refno.SPRE 不为空且 record exist 的节点ID数组
DEFINE FUNCTION OVERWRITE fn::collect_descendants_filter_spre(
    $root: record,
    $types: array<string>,
    $filter_inst: option<bool>,
    $inclusive: option<bool>,
    $range_str: string,
) {
    if !record::exists($root) {
        return [];
    };
    -- 使用 $inclusive 参数来控制是否包含根节点本身，默认为 true
    let $use_inclusive = $inclusive ?:true;
    let $need_filter = $filter_inst ?: false;
    
    -- 收集子孙节点时同时获取 id, noun, refno.CATR, refno.SPRE, inst_relate_id, has_tubi 信息
    let $raw_descendants = if $use_inclusive {
        (SELECT VALUE array::flatten(@.{..+collect+inclusive}.children).{ 
            id, 
            noun, 
            catr: refno.CATR, 
            spre: refno.SPRE,
            inst_relate_id,
            has_tubi
        } FROM ONLY $root LIMIT 1) ?: []
    } else {
        (SELECT VALUE array::flatten(@.{..+collect}.children).{ 
            id, 
            noun, 
            catr: refno.CATR, 
            spre: refno.SPRE,
            inst_relate_id,
            has_tubi
        } FROM ONLY $root LIMIT 1) ?: []
    };
    
    -- 第一步：过滤类型和 CATR/SPRE 条件
    let $type_filtered = array::filter($raw_descendants, |$node|
        $node != NONE
        && $node.id != NONE
        && (array::len($types) == 0 || $node.noun IN $types)
        && (
            ($node.catr != NONE && record::exists($node.catr))
            || ($node.spre != NONE && record::exists($node.spre))
        )
    );
    
    -- 第二步：如果需要，过滤掉有 inst_relate 或 tubi_relate 的节点
    let $filtered = if $need_filter {
        array::filter($type_filtered, |$node|
            !type::is_record($node.inst_relate_id)
            && !$node.has_tubi
        )
    } else {
        $type_filtered
    };
    
    return array::map($filtered, |$node| $node.id);
};

-- 收集子节点，支持按类型过滤
-- $types: 要过滤的类型数组，空数组表示不过滤
DEFINE FUNCTION OVERWRITE fn::collect_children($root: record, $types: option<array<string>>) {
    if !record::exists($root) {
        return [];
    };
    let $raw_children = (SELECT VALUE children FROM ONLY $root LIMIT 1) ?: [];
    let $children = array::flatten([$raw_children]);
    let $filtered = array::filter($children, |$child| $child != none);

    let $type_filter = $types ?: [];
    if array::len($type_filter) == 0 {
        return $filtered;
    } else {
        return array::filter($filtered, |$child| $child.noun IN $type_filter);
    };
};

-- 收集子孙节点信息（包含ID和类型），支持自定义递归深度范围
-- $range_str: 递归范围字符串，仅用于文档/注释，实际在 Rust 代码中进行字符串拼接
DEFINE FUNCTION OVERWRITE fn::collect_descendant_infos(
    $root: record,
    $types: array<string>,
    $inclusive: option<bool>,
    $range_str: string,
) {
    if !record::exists($root) {
        return [];
    };
    -- 使用 $inclusive 参数来控制是否包含根节点本身，默认为 false
    let $use_inclusive = $inclusive ?:true;
    -- 注：$range_str 作为函数签名的一部分用于 Rust 代码生成 SQL
    -- Rust 代码会在调用时进行字符串拼接，生成类似以下的 SQL：
    -- (SELECT VALUE array::flatten(@.{1..5+collect}.children).{ id, noun } FROM ONLY $root LIMIT 1)
    let $raw_descendants = if $use_inclusive {
        (SELECT VALUE array::flatten(@.{..+collect+inclusive}.children).{ id, noun } FROM ONLY $root LIMIT 1) ?: []
    } else {
        (SELECT VALUE array::flatten(@.{..+collect}.children).{ id, noun } FROM ONLY $root LIMIT 1) ?: []
    };
    let $filtered = array::filter($raw_descendants, |$node|
        $node != NONE && $node.id != NONE && (array::len($types) == 0 || $node.noun IN $types)
    );
    return $filtered;
};


-- 收集子孙节点的 ID 和 has_inst 状态
-- 返回 { id, has_inst } 结构数组，可以通过 [? has_inst] 过滤出有 inst_relate 的节点
-- $range_str: 递归范围字符串，可选参数，默认为 ".."
DEFINE FUNCTION OVERWRITE fn::collect_descendant_ids_has_inst(
    $root: record,
    $types: array<string>,
    $inclusive: option<bool>,
) {
    if !record::exists($root) {
        return [];
    };
    -- 使用 $inclusive 参数来控制是否包含根节点本身，默认为 false
    let $use_inclusive = $inclusive ?:true;
    -- 注：当前固定使用 ".." 范围，未来版本支持动态范围时再使用 $range_str 参数
    let $raw_descendants = if $use_inclusive {
        (SELECT VALUE array::flatten(@.{..+collect+inclusive}.children).{ id, has_inst } FROM ONLY $root LIMIT 1) ?: []
    } else {
        (SELECT VALUE array::flatten(@.{..+collect}.children).{ id, has_inst } FROM ONLY $root LIMIT 1) ?: []
    };

    -- 先按类型过滤
    let $type_filtered = array::filter($raw_descendants, |$node|
        $node.has_inst && $node.id != NONE && (array::len($types) == 0 || $node.noun IN $types)
    );

    return array::map($type_filtered, |$x| $x.id);
};



-- 获取可见的几何子孙节点
DEFINE FUNCTION OVERWRITE fn::visible_geo_descendants($root: record, $include_self: bool, $range_str: string) {
    let $visible_types = [
        "BOX", "CYLI", "SLCY", "CONE", "DISH", "CTOR", "RTOR", "PYRA", "SNOU", "POHE", "POLYHE",
        "EXTR", "REVO", "FLOOR", "PANE", "ELCONN", "CMPF", "WALL", "GWALL", "SJOI", "FITT", "PFIT",
        "FIXING", "PJOI", "GENSEC", "RNODE", "PRTELE", "GPART", "SCREED", "PALJ", "CABLE", "BATT",
        "CMFI", "SCOJ", "SEVE", "SBFI", "STWALL", "SCTN", "NOZZ"
    ];
    let $infos = fn::collect_descendant_infos($root, $visible_types, none, $range_str);
    return array::map($infos, |$info| $info.id);
};

-- 获取负实体几何子孙节点（对应 TOTAL_NEG_NOUN_NAMES）
DEFINE FUNCTION OVERWRITE fn::negative_geo_descendants($root: record, $include_self: bool, $range_str: string) {
    let $negative_types = [
        "NBOX", "NCYL", "NLCY", "NSBO", "NCON", "NSNO", "NPYR", "NDIS", "NXTR", "NCTO", "NRTO", "NREV",
        "NSCY", "NSBO", "NSCO", "NLSN", "NSSP", "NLCY", "NSCY", "NSCT", "NSRT", "NSDS", "NSSL", "NLPY",
        "NSEX", "NSRE"
    ];
    let $infos = fn::collect_descendant_infos($root, $negative_types, none, $range_str);
    return array::map($infos, |$info| $info.id);
};


-- 查询MDB数据库编号
-- 参数：$mdb - MDB名称, $db_type - 数据库类型
-- 返回：数据库编号数组
DEFINE FUNCTION OVERWRITE fn::query_mdb_db_nums($mdb: string, $db_type: int) {
        let $dbnos = select value (select value DBNO from CURD.refno where STYP=$db_type) from only MDB where NAME=$mdb limit 1;
        return (select value dbnum from (
            select REFNO.dbnum as dbnum, array::find_index($dbnos, REFNO.dbnum) as o
            from WORL
            where REFNO.dbnum in $dbnos
            order by o
        ));
};

-- 获取法兰螺栓列表
DEFINE FUNCTION OVERWRITE fn::get_flan_bolt_list($pe:record) {
    if $pe.refno.SPRE == NONE { return NONE; };
    let $bltf = select value refno.SPRE.refno.CATR.refno.BLRF[0].name from $pe;
    if $bltf == NONE { return NONE; };
    let $bltf_split = array::last(string::split($bltf[0],'-')?:[]);
    let $blot_size = string::concat($bltf_split,'W')?:'';
    let $ancestor = fn::find_ancestor_type($pe.refno.SPRE?:pe:0_0,'SPEC').refno.BSPE;
    if $ancestor == NONE { return NONE; };
    let $spcos = fn::find_deep_children_type($ancestor,'SPCO');
    if $spcos == NONE { return NONE; };
    let $filter_spcos = select value id from $spcos where string::contains(name,$blot_size);
    if array::len($filter_spcos) == 0 { return NONE; };
    let $mat = select name , id.refno.DETR.refno.RTEX as rtext from $filter_spcos;
    return {
        'bltf':$bltf[0],
        'mat':$mat
    }
};

-- 注释掉测试函数，因为 SurrealDB 3.0 不支持 test:: 命名空间
-- REMOVE FUNCTION test::create_mock_children_tree;
-- DEFINE FUNCTION test::create_mock_children_tree() {
--     let $root = type::record('pe', '99999/1000');
--     let $box = type::record('pe', '99999/1001');
--     let $bran = type::record('pe', '99999/1002');
--     let $cable = type::record('pe', '99999/1003');
--     let $panel = type::record('pe', '99999/1004');
--
--     UPSERT $root CONTENT {
--         noun: "ROOT",
--         name: "Mock Root",
--         deleted: false,
--         children: []
--     };
--     UPSERT $box CONTENT {
--         noun: "BOX",
--         name: "Mock Box",
--         deleted: false,
--         children: []
--     };
--     UPSERT $bran CONTENT {
--         noun: "BRAN",
--         name: "Mock Bran",
--         deleted: false,
--         children: []
--     };
--     UPSERT $cable CONTENT {
--         noun: "CABLE",
--         name: "Mock Cable",
--         deleted: false,
--         children: []
--     };
--     UPSERT $panel CONTENT {
--         noun: "PANEL",
--         name: "Mock Panel",
--         deleted: true,
--         children: []
--     };
--
--     UPDATE $root SET children = [$box, $bran];
--     UPDATE $bran SET children = [$cable, $panel];
--
--     return '99999/1000';
-- };
--
-- REMOVE FUNCTION test::cleanup_mock_children_tree;
-- DEFINE FUNCTION test::cleanup_mock_children_tree() {
--     let $ids = [
--         type::record('pe', '99999/1000'),
--         type::record('pe', '99999/1001'),
--         type::record('pe', '99999/1002'),
--         type::record('pe', '99999/1003'),
--         type::record('pe', '99999/1004')
--     ];
--     for $id in $ids {
--         if record::exists($id) {
--             UPDATE $id SET children = [];
--         };
--     };
--     delete $ids;
--     return true;
-- };
--
-- REMOVE FUNCTION test::exercise_visible_descendants;
-- DEFINE FUNCTION test::exercise_visible_descendants($cleanup: bool) {
--     let $root = test::create_mock_children_tree();
--     let $collect_all = fn::collect_descendant_infos(type::record('pe', $root), [], true, "..");
--     let $visible = fn::visible_geo_descendants(type::record('pe', $root), true, "..");
--     if $cleanup {
--         test::cleanup_mock_children_tree();
--     };
--     return {
--         root: $root,
--         all_descendants: $collect_all,
--         visible_descendants: $visible
--     };
-- };

-- 获取祖先节点的属性数据，并过滤掉属性为 NONE 的节点
DEFINE FUNCTION OVERWRITE fn::ancestor_atts($r: record) {
    return if !record::exists($r) {
        return [];
    } else {
        let $ancestors = fn::ancestor($r);
        let $atts = array::map($ancestors, |$pe| $pe.refno.*);
        return array::filter($atts, |$att| $att != NONE);
    };
};

-- 查询指定正实体关联的负实体载体 refnos
-- 参数 $pe: 正实体记录，如 pe:17496_106028
-- 返回: 关联的负载体 refnos 数组（通过 neg_relate 和 ngmr_relate 关系)
-- 新结构说明：
--   neg_relate/ngmr_relate.in = geo_relate (切割几何)
--   neg_relate/ngmr_relate.pe = 负载体 pe
--   neg_relate/ngmr_relate.out = 正实体 pe
DEFINE FUNCTION OVERWRITE fn::query_negative_entities($pe: record) {
    return array::union($pe<-neg_relate.pe, $pe<-ngmr_relate.pe);
};

-- 查询点集数据
DEFINE FUNCTION OVERWRITE fn::query_ptset($refno: string) {
    let $table_name = string::concat($refno, '_inst_relate');
    let $sql = string::concat(
        '(SELECT world_trans.d AS transform, object::values(out.ptset?:{}).pt AS points FROM ',
        $table_name,
        ')[0]'
    );

    let $result = SELECT VALUE $sql FROM ONLY [];
    return $result[0];
};

-- ============================================================================
-- 性能优化函数：收集子孙节点并过滤 inst_relate 和 tubi_relate
-- ============================================================================
-- 功能：一次性完成子孙节点收集和关系过滤，避免多次往返
-- 参数：
--   $root: 起始节点
--   $types: 要筛选的节点类型数组（空数组表示不过滤类型）
--   $filter_inst: 是否过滤掉有 inst_relate 或 tubi_relate 的节点
--   $include_self: 是否包含起始节点自身
--   $skip_deleted: 是否跳过已删除的节点
-- 返回：符合条件的节点 ID 数组
-- 性能提升：相比分块查询，减少 90% 的网络往返时间
-- ============================================================================
-- 收集子孙节点并过滤实例关系和直段关系
DEFINE FUNCTION OVERWRITE fn::collect_descendants_filter_inst(
    $root: record,
    $types: array<string>,
    $filter_inst: option<bool>,
    $include_self: option<bool>,
    $skip_deleted: option<bool>,
) {
    if !record::exists($root) {
        return [];
    };

    let $need_filter = $filter_inst ?: false;
    let $with_self = $include_self ?: true;
    let $skip_deleted_nodes = $skip_deleted ?: false;

    let $raw_descendants = if $with_self {
        (SELECT VALUE array::flatten(@.{..+collect+inclusive}.children).{
            id,
            noun,
            deleted,
            inst_relate_id,
            tubi_id
        } FROM ONLY $root LIMIT 1) ?: []
    } else {
        (SELECT VALUE array::flatten(@.{..+collect}.children).{
            id,
            noun,
            deleted,
            inst_relate_id,
            tubi_id
        } FROM ONLY $root LIMIT 1) ?: []
    };

    let $type_filtered = array::filter($raw_descendants, |$node|
        $node != NONE
        && $node.id != NONE
        && (array::len($types) == 0 || $node.noun IN $types)
        && (!$skip_deleted_nodes || $node.deleted != true)
    );

    let $filtered = if $need_filter {
        array::filter($type_filtered, |$node|
            !type::is_record($node.inst_relate_id)
            && array::len(if type::is_array($node.tubi_id) { $node.tubi_id } else { [] }) == 0
        )
    } else {
        $type_filtered
    };

    return array::map($filtered, |$node| $node.id);
};

-- ============================================================================
-- inst_relate_aabb / inst_relate_booled_aabb / inst_relate_bool 表定义
-- 目的：将 AABB 和布尔结果拆分到独立表，避免频繁 update inst_relate
-- inst_relate_aabb: 原始几何 AABB
-- inst_relate_booled_aabb: 布尔运算后 AABB
-- ============================================================================
DEFINE TABLE inst_relate_aabb SCHEMALESS;
DEFINE FIELD refno ON TABLE inst_relate_aabb TYPE record<pe>;
DEFINE FIELD aabb_id ON TABLE inst_relate_aabb TYPE record<aabb>;
DEFINE INDEX idx_inst_relate_aabb_refno ON TABLE inst_relate_aabb COLUMNS refno UNIQUE;

DEFINE TABLE inst_relate_booled_aabb SCHEMALESS;
DEFINE FIELD refno ON TABLE inst_relate_booled_aabb TYPE record<pe>;
DEFINE FIELD aabb_id ON TABLE inst_relate_booled_aabb TYPE record<aabb>;
DEFINE INDEX idx_inst_relate_booled_aabb_refno ON TABLE inst_relate_booled_aabb COLUMNS refno UNIQUE;

DEFINE TABLE inst_relate_bool SCHEMALESS;
DEFINE FIELD refno ON TABLE inst_relate_bool TYPE record<pe>;
DEFINE FIELD mesh_id ON TABLE inst_relate_bool TYPE option<string>;
DEFINE FIELD status ON TABLE inst_relate_bool TYPE option<string>;
DEFINE FIELD source ON TABLE inst_relate_bool TYPE option<string>;
DEFINE FIELD updated_at ON TABLE inst_relate_bool TYPE option<datetime> DEFAULT time::now();
DEFINE INDEX idx_inst_relate_bool_refno ON TABLE inst_relate_bool COLUMNS refno UNIQUE;
