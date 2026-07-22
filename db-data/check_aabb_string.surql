SELECT id, aabb_id FROM inst_relate_aabb:`24381_145019`;
SELECT id, aabb_id FROM inst_relate_aabb:[24381, 145019];
SELECT type::record("inst_relate_aabb", record::id(in)).aabb_id.d AS world_aabb FROM inst_relate:[24381, 145019];
