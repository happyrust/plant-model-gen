-- VALV SPCO 13246_465874 follow CATR / GMSE
SELECT meta::id(id) AS id, noun, name, type::string(refno.CATR) AS catr, refno.CATR.name AS catr_name FROM pe:`13246_465874`;
SELECT meta::id(id) AS id, noun, name FROM pe WHERE owner = pe:`13246_465874`;
-- OLET SPCO
SELECT meta::id(id) AS id, noun, name, type::string(refno.CATR) AS catr, refno.CATR.name AS catr_name FROM pe:`13246_465938`;
SELECT meta::id(id) AS id, noun, name FROM pe WHERE owner = pe:`13246_465938`;
