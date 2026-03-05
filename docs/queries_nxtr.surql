-- NXTR 24381/35961 结构分析查询（每行单语句，便于管道执行）
-- 用法: Get-Content docs\queries_nxtr.surql | surreal sql -e ws://127.0.0.1:8020 -u root -p root --namespace 1516 --database AvevaMarineSample

SELECT * FROM ONLY pe:`24381_35961`;
SELECT id, noun, name, deleted FROM pe:`24381_35961`<-pe_owner.in;
SELECT children[WHERE noun IN ["PAVE", "PONT"]].refno.POS as positions, children[WHERE noun IN ["PAVE", "PONT"]].refno.FRAD as frads, refno.HEIG as height FROM pe:`24381_35961`.children WHERE noun IN ["LOOP", "PLOO"];
SELECT refno.HEIG as height, noun as noun, record::id(owner) as owner FROM pe:`24381_35961`;
SELECT id, noun, name FROM pe:`24381_35960`<-pe_owner.in;
SELECT id, noun FROM pe:`24381_35962`<-pe_owner.in;
SELECT * FROM ONLY pe:`24381_35962`;
SELECT refno.POS as pos, refno.FRAD as frad FROM pe:`24381_35963`;
