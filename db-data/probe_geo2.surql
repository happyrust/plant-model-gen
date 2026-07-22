SELECT id, out FROM inst_relate WHERE owner_refno = pe:24381_145018 LIMIT 5;
SELECT * FROM inst_info:4230367404655002616 LIMIT 1;
SELECT count() AS n FROM geo_relate GROUP ALL;
SELECT count() AS n FROM inst_relate WHERE owner_refno = pe:24381_145018 GROUP ALL;
SELECT count() AS n FROM tubi_relate WHERE out = pe:24381_145018 OR in = pe:24381_145018 GROUP ALL;