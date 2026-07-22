-- Mirror array-id AABB rows for BRAN 24381_145019 subtree onto string ids used by export queries
LET $refs = [
  pe:`24381_145018`, pe:`24381_145019`, pe:`24381_145020`, pe:`24381_145021`, pe:`24381_145022`,
  pe:`24381_145023`, pe:`24381_145024`, pe:`24381_145025`, pe:`24381_145026`, pe:`24381_145027`,
  pe:`24381_145028`, pe:`24381_145029`, pe:`24381_145030`, pe:`24381_145031`, pe:`24381_145032`,
  pe:`24381_145033`, pe:`24381_145034`, pe:`24381_145035`
];
FOR $r IN $refs {
  LET $arr = type::record("inst_relate_aabb", [record::id($r)[0], record::id($r)[1]]);
  LET $src = SELECT * FROM ONLY $arr;
  IF $src != NONE {
    UPSERT type::record("inst_relate_aabb", record::id($r)) SET
      aabb_id = $src.aabb_id,
      aabb = $src.aabb,
      refno = $r;
  };
};
SELECT count() AS n FROM inst_relate_aabb WHERE refno INSIDE $refs AND type::is_string(record::id(id)) GROUP ALL;
SELECT id, refno FROM inst_relate_aabb WHERE refno INSIDE $refs;
