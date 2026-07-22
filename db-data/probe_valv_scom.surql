SELECT meta::id(id) AS id, noun, name FROM pe:`13246_465874`;
SELECT meta::id(id) AS id, noun, name, owner FROM pe WHERE owner = pe:`13246_465874` OR meta::id(owner) = "13246_465874" LIMIT 30;
SELECT meta::id(id) AS id, noun, name FROM pe:`13246_465938`;
-- follow SPCO -> CATR -> SCOM for VALV
SELECT type::string(refno.CATR) AS catr, refno.CATR.name AS catr_name, type::string(refno.CATR.refno) AS catr_ref FROM pe:`13246_465874`;
SELECT type::string(refno.CATR) AS catr, refno.CATR.name AS catr_name FROM pe:`13246_465938`;
