import urllib.request
import urllib.parse
import json

req = urllib.request.Request('http://127.0.0.1:8020/sql',
   data="SELECT id, status FROM inst_relate_bool WHERE refno = pe:⟨24381_40064⟩;".encode('utf-8'),
   headers={'Accept': 'application/json', 'NS': '1516', 'DB': 'AvevaMarineSample'},
   method='POST')
req.add_header('Authorization', 'Basic cm9vdDpyb290')

try:
    with urllib.request.urlopen(req) as response:
        print(response.read().decode('utf-8'))
except Exception as e:
    print(e)
