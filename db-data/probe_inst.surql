SELECT count() AS n FROM geo_relate WHERE out = pe:24381_145019 OR in = pe:24381_145019 GROUP ALL;
SELECT id, in, out, owner_refno FROM inst_relate WHERE in = pe:24381_145019 LIMIT 1;
SELECT count() AS n FROM inst_info WHERE id = pe:24381_145019 GROUP ALL;