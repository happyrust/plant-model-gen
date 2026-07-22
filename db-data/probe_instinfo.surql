SELECT * FROM inst_info:⟨4230367404655002616⟩;
SELECT id FROM inst_info WHERE id = inst_info:⟨4230367404655002616⟩;
SELECT count() AS n FROM inst_info GROUP ALL;
SELECT meta::id(out) AS oid, type::is_record(out) AS is_rec FROM inst_relate WHERE in = pe:24381_145019;
SELECT ->inst_info.* FROM pe:24381_145019;
SELECT id, array::len(geos) AS geos_n FROM (SELECT id, geos FROM inst_info LIMIT 1);