SELECT * FROM geo_relate WHERE out = pe:24381_145019 OR in = pe:24381_145019 LIMIT 5;
SELECT id, in, out FROM geo_relate WHERE in = pe:24381_145019 LIMIT 5;
SELECT ->geo_relate->geo FROM pe:24381_145019;
SELECT <-geo_relate<-geo FROM pe:24381_145019;
SELECT id FROM geo WHERE id INSIDE (SELECT VALUE out FROM geo_relate WHERE in = pe:24381_145019) LIMIT 5;
INFO FOR TABLE geo_relate;