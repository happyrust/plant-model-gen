SELECT ->inst_relate AS r FROM pe:24381_145019;
SELECT ->inst_relate->inst_info AS info FROM pe:24381_145019;
SELECT <-inst_relate AS r FROM pe:24381_145019;
SELECT id, in, out FROM inst_relate WHERE in = pe:24381_145019;
SELECT out->geo_relate AS g FROM inst_relate WHERE in = pe:24381_145019;
SELECT record::id(out) AS geo_hash, out.meshed AS meshed, out.unit_flag AS unit_flag FROM (SELECT out FROM geo_relate WHERE in = inst_info:⟨4230367404655002616⟩ AND visible = true);
SELECT * FROM inst_geo:⟨10105853823688291601⟩;