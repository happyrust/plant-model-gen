SELECT meta::id(id) AS id, noun, name, owner FROM pe:`13246_243868`;
SELECT meta::id(in) AS parent, meta::id(out) AS child FROM pe_owner WHERE in = pe:`13246_243868` LIMIT 50;
SELECT meta::id(id) AS id, noun, name FROM pe WHERE meta::id(id) IN ["13246_243868","13246_243869","13246_243870","13246_243871","13246_243872"];
