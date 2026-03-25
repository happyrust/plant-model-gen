#!/bin/bash
curl -k -L -s --request POST \
  --url http://127.0.0.1:8020/sql \
  --header 'Accept: application/json' \
  --header 'Content-Type: text/plain' \
  --header 'NS: 1516' \
  --header 'DB: AvevaMarineSample' \
  --user 'root:root' \
  --data 'SELECT * FROM neg_relate LIMIT 10;
SELECT * FROM inst_relate_bool LIMIT 10;
SELECT id, status FROM inst_relate_bool WHERE refno = pe:⟨24381_40064⟩;'
