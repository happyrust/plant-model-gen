-- Repair pe_owner edges for BRAN 24381_145018 from pe.children
DELETE pe_owner WHERE out = pe:`24381_145018`;
LET $kids = (SELECT VALUE children FROM ONLY pe:`24381_145018`) ?? [];
FOR $i IN 0..(array::len($kids)) {
        LET $c = $kids[$i];
        IF $c != NONE {
                RELATE $c->pe_owner->pe:`24381_145018` CONTENT {
                        id: [record::id($c), $i]
                };
        };
};
SELECT count() AS c FROM pe_owner WHERE out = pe:`24381_145018` GROUP ALL;
SELECT in.noun AS noun, record::id(in) AS id FROM pe_owner WHERE out = pe:`24381_145018`;
