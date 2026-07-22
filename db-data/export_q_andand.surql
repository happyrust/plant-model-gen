SELECT
  in as refno,
  in.owner ?? in as owner,
  (if type::record("inst_relate_aabb", record::id(in)).aabb_id != NONE {
    record::id(type::record("inst_relate_aabb", record::id(in)).aabb_id)
  } else { None }) as world_aabb_hash,
  (if type::record("pe_transform", record::id(in)).world_trans != NONE {
    record::id(type::record("pe_transform", record::id(in)).world_trans)
  } else { None }) as world_trans_hash,
  (SELECT record::id(trans) as trans_hash, record::id(out) as geo_hash, out.unit_flag ?? false as unit_flag
   FROM $parent.out->geo_relate
   WHERE visible
     && out != NONE
     && (out.meshed || out.unit_flag || out.id IN ['1','2','3'])
     && (trans.d ?? NONE) != NONE
     && geo_type IN ['Pos', 'CatePos', 'Compound']) as insts,
  false as has_neg
FROM [inst_relate:[24381, 145019], inst_relate:`24381_145019`]
WHERE in != NONE;
