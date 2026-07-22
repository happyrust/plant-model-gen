SELECT * FROM geo_relate WHERE in = inst_info:⟨4230367404655002616⟩ LIMIT 10;
SELECT count() AS n FROM geo_relate WHERE in = inst_info:⟨4230367404655002616⟩ GROUP ALL;
SELECT * FROM (SELECT VALUE id FROM [inst_info:⟨4230367404655002616⟩]->geo_relate) LIMIT 10;
SELECT id, geom_refno, out, geo_type FROM geo_relate WHERE out INSIDE [inst_geo:⟨1⟩, inst_geo:⟨2⟩, inst_geo:⟨3⟩] AND geom_refno INSIDE [pe:24381_145019, pe:24381_145021, pe:24381_145018] LIMIT 20;
SELECT count() AS n FROM inst_geo WHERE id INSIDE [inst_geo:⟨4097664205771118610⟩] GROUP ALL;