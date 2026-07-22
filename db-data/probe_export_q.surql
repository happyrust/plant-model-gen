SELECT
  in as refno,
  (SELECT record::id(out) as geo_hash, out.meshed as meshed, out.unit_flag as unit_flag, geo_type
   FROM $parent.out->geo_relate
   WHERE visible
     AND out != NONE
     AND (out.meshed OR out.unit_flag OR out.id IN ["1","2","3"])
     AND (trans.d ?? NONE) != NONE
     AND geo_type IN ["Pos", "CatePos", "Compound"]) as insts
FROM [inst_relate:[24381, 145019]]
WHERE in != NONE;