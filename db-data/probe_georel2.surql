SELECT * FROM geo_relate LIMIT 1;
SELECT count() AS n FROM geo_relate WHERE geom_refno = pe:24381_145019 GROUP ALL;
SELECT count() AS n FROM geo_relate WHERE geom_refno INSIDE [pe:24381_145019, pe:24381_145021, pe:24381_145018] GROUP ALL;
SELECT geom_refno, geo_type, in, out FROM geo_relate WHERE geom_refno = pe:24381_145019 LIMIT 5;