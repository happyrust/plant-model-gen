SELECT
  in as refno,
  (if type::record("inst_relate_aabb", record::id(in)).aabb_id != NONE {
    record::id(type::record("inst_relate_aabb", record::id(in)).aabb_id)
  } else { None }) as world_aabb_hash,
  (if type::record("pe_transform", record::id(in)).world_trans != NONE {
    record::id(type::record("pe_transform", record::id(in)).world_trans)
  } else { None }) as world_trans_hash,
  (SELECT record::id(trans) as trans_hash, record::id(out) as geo_hash, out.unit_flag ?? false as unit_flag
   FROM $parent.out->geo_relate
   WHERE visible
     AND out != NONE
     AND (out.meshed OR out.unit_flag OR out.id IN ['1','2','3'])
     AND (trans.d ?? NONE) != NONE
     AND geo_type IN ['Pos', 'DesiPos', 'CatePos', 'Compound']) as insts
FROM [inst_relate:[24381, 145019]]
WHERE in != NONE;
