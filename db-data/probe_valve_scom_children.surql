SELECT meta::id(id) AS id, noun, name FROM pe:`13246_243978`;
SELECT meta::id(id) AS id, noun, name FROM pe WHERE owner = pe:`13246_243978`;
SELECT meta::id(in) AS parent, meta::id(out) AS child FROM pe_owner WHERE in = pe:`13246_243978` LIMIT 40;
SELECT meta::id(in) AS parent, meta::id(out) AS child FROM pe_owner WHERE out = pe:`13246_243978` LIMIT 20;
