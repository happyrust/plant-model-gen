#!/usr/bin/env bash
# 基于真实 workflow/sync 返回生成本地 PMS 模拟联调页，并启动本地静态服务。
#
# 用法：
#   BASE_URL=http://123.57.182.243 ./shells/run_pms_workflow_mock_page.sh
#
# 环境变量：
#   BASE_URL      目标后端地址，默认 http://127.0.0.1:3100
#   PROJECT_ID    默认 2410
#   USER_ID       默认 SJ
#   PORT          本地静态服务端口，默认 8765
#   OUT_DIR       页面输出目录，默认 /tmp/pms_workflow_mock_page
#   OPEN_BROWSER  是否自动打开浏览器，默认 true
#   CLEANUP_FORM  是否在生成完成后删除测试 form 数据，默认 true

set -euo pipefail

BASE_URL="${BASE_URL:-http://127.0.0.1:3100}"
BASE_URL="${BASE_URL%/}"
PROJECT_ID="${PROJECT_ID:-2410}"
USER_ID="${USER_ID:-SJ}"
PORT="${PORT:-8765}"
OUT_DIR="${OUT_DIR:-/tmp/pms_workflow_mock_page}"
OPEN_BROWSER="${OPEN_BROWSER:-true}"
CLEANUP_FORM="${CLEANUP_FORM:-true}"

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "缺少命令: $1" >&2
    exit 1
  }
}

need_cmd curl
need_cmd python3
need_cmd jq

mkdir -p "$OUT_DIR"
export BASE_URL PROJECT_ID USER_ID OUT_DIR CLEANUP_FORM

python3 - <<'PY'
import base64
import json
import os
import subprocess
import tempfile
import time
import uuid
from pathlib import Path

BASE = os.environ['BASE_URL']
PROJECT_ID = os.environ['PROJECT_ID']
USER_ID = os.environ['USER_ID']
OUT_DIR = Path(os.environ['OUT_DIR'])
CLEANUP_FORM = os.environ['CLEANUP_FORM'].lower() == 'true'

form_id = 'FORM-' + uuid.uuid4().hex[:12].upper()


def run(cmd, input=None, check=True):
    return subprocess.run(cmd, input=input, text=True, capture_output=True, check=check)


def curl_json(args):
    out = run(['curl', '-fsS', *args]).stdout
    return json.loads(out)


def write_json(name, data):
    path = OUT_DIR / name
    path.write_text(json.dumps(data, ensure_ascii=False, indent=2), encoding='utf-8')

embed = curl_json([
    '-X', 'POST', f'{BASE}/api/review/embed-url',
    '-H', 'Content-Type: application/json',
    '-d', json.dumps({'project_id': PROJECT_ID, 'user_id': USER_ID, 'form_id': form_id}, ensure_ascii=False)
])
token = embed['data']['token']
create = curl_json([
    '-X', 'POST', f'{BASE}/api/review/tasks',
    '-H', 'Content-Type: application/json',
    '-H', f'Authorization: Bearer {token}',
    '-d', json.dumps({
        'title': f'PMS模拟页-{int(time.time())}',
        'description': '仓库内正式 PMS 联调页生成数据',
        'modelName': PROJECT_ID,
        'checkerId': 'JH',
        'approverId': 'SH',
        'reviewerId': 'JH',
        'formId': form_id,
        'priority': 'medium',
        'components': [
            {'id': 'c1', 'refNo': '24381_145018', 'name': '管道A', 'type': 'PIPE'},
            {'id': 'c2', 'refNo': '24381_145020', 'name': '阀门B', 'type': 'VALVE'},
        ],
    }, ensure_ascii=False),
])
task_id = create['task']['id']
annotation_id = f'anno-mock-text-{int(time.time())}'

record = curl_json([
    '-X', 'POST', f'{BASE}/api/review/records',
    '-H', 'Content-Type: application/json',
    '-H', f'Authorization: Bearer {token}',
    '-d', json.dumps({
        'taskId': task_id,
        'type': 'batch',
        'annotations': [
            {'id': annotation_id, 'type': 'text', 'content': '模拟页：模型文本意见', 'position': {'x': 10, 'y': 20, 'z': 30}}
        ],
        'cloudAnnotations': [
            {'id': f'anno-mock-cloud-{int(time.time())}', 'type': 'cloud', 'shape': 'ellipse', 'points': [{'x': 0, 'y': 0, 'z': 0}, {'x': 1, 'y': 1, 'z': 1}]}
        ],
        'rectAnnotations': [],
        'obbAnnotations': [],
        'measurements': [
            {'id': f'measure-mock-{int(time.time())}', 'type': 'distance', 'value': 12.34, 'unit': 'mm'}
        ],
        'note': '模拟页：请点击下方附件链接',
    }, ensure_ascii=False),
])

comment = curl_json([
    '-X', 'POST', f'{BASE}/api/review/comments',
    '-H', 'Content-Type: application/json',
    '-H', f'Authorization: Bearer {token}',
    '-d', json.dumps({
        'annotationId': annotation_id,
        'annotationType': 'text',
        'authorId': USER_ID,
        'authorName': '设计',
        'authorRole': 'sj',
        'content': '模拟页：批注评论',
    }, ensure_ascii=False),
])

png_bytes = base64.b64decode('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+iS9cAAAAASUVORK5CYII=')
pdf_bytes = b'%PDF-1.4\n1 0 obj\n<<>>\nendobj\ntrailer\n<<>>\n%%EOF\n'
fd, png_path = tempfile.mkstemp(suffix='.png')
os.close(fd)
fd2, pdf_path = tempfile.mkstemp(suffix='.pdf')
os.close(fd2)
Path(png_path).write_bytes(png_bytes)
Path(pdf_path).write_bytes(pdf_bytes)

upload_png = json.loads(run([
    'curl', '-fsS', '-X', 'POST', f'{BASE}/api/review/attachments',
    '-F', f'taskId={task_id}',
    '-F', f'formId={form_id}',
    '-F', 'modelRefnos=["24381_145018"]',
    '-F', 'fileType=markup',
    '-F', 'description=模拟页截图',
    '-F', f'file=@{png_path};type=image/png'
]).stdout)

upload_pdf = json.loads(run([
    'curl', '-fsS', '-X', 'POST', f'{BASE}/api/review/attachments',
    '-F', f'taskId={task_id}',
    '-F', f'formId={form_id}',
    '-F', 'modelRefnos=["24381_145020"]',
    '-F', 'fileType=file',
    '-F', 'description=模拟页文档',
    '-F', f'file=@{pdf_path};type=application/pdf'
]).stdout)

sync = curl_json([
    '-X', 'POST', f'{BASE}/api/review/workflow/sync',
    '-H', 'Content-Type: application/json',
    '-d', json.dumps({
        'form_id': form_id,
        'token': token,
        'action': 'query',
        'actor': {'id': USER_ID, 'name': '设计', 'roles': 'sj'},
    }, ensure_ascii=False),
])

write_json('embed.json', embed)
write_json('task.json', create)
write_json('record.json', record)
write_json('comment.json', comment)
write_json('upload_png.json', upload_png)
write_json('upload_pdf.json', upload_pdf)
write_json('response.json', sync)

summary = {
    'base_url': BASE,
    'form_id': form_id,
    'task_id': task_id,
    'records_count': len(sync['data'].get('records', [])),
    'annotation_comments_count': len(sync['data'].get('annotation_comments', [])),
    'attachment_types': [item.get('type') for item in sync['data'].get('attachments', [])],
    'attachments': sync['data'].get('attachments', []),
}
write_json('summary.json', summary)

html = f'''<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="UTF-8" />
<meta name="viewport" content="width=device-width, initial-scale=1.0" />
<title>PMS 模拟联调页</title>
<style>
body {{ font-family: -apple-system,BlinkMacSystemFont,Segoe UI,Arial,sans-serif; margin: 24px; background:#f7f8fa; color:#1f2328; }}
.card {{ background:#fff; border:1px solid #d0d7de; border-radius:12px; padding:16px; margin-bottom:16px; box-shadow:0 1px 2px rgba(0,0,0,.04); }}
a {{ color:#0969da; word-break:break-all; }}
pre {{ background:#0d1117; color:#c9d1d9; padding:12px; border-radius:8px; overflow:auto; white-space:pre-wrap; }}
.badge {{ display:inline-block; padding:2px 8px; border-radius:999px; background:#ddf4ff; color:#0969da; font-size:12px; margin-right:8px; }}
.badge.markup {{ background:#fff8c5; color:#9a6700; }}
.badge.file {{ background:#ddf4ff; color:#0969da; }}
.small {{ color:#57606a; font-size:13px; }}
</style>
</head>
<body>
<h1>PMS 模拟联调页</h1>
<div class="card">
  <div><strong>BASE_URL：</strong>{BASE}</div>
  <div><strong>form_id：</strong>{form_id}</div>
  <div><strong>task_id：</strong>{task_id}</div>
  <div><strong>原始返回：</strong><a href="/response.json" target="_blank">打开 workflow/sync JSON</a></div>
  <div class="small">本页直接展示当前 workflow/sync 的 records / annotation_comments / attachments。</div>
</div>
<div class="card">
  <h2>附件 / 截图链接</h2>
  <div id="attachments"></div>
</div>
<div class="card">
  <h2>批注主体 records</h2>
  <pre id="records"></pre>
</div>
<div class="card">
  <h2>批注评论 annotation_comments</h2>
  <pre id="comments"></pre>
</div>
<script>
fetch('/response.json').then(r => r.json()).then(data => {{
  const payload = data.data || {{}};
  const attachments = (payload.attachments || []).map(item =>
    `<div style="margin:10px 0;">` +
    `<span class="badge ${{item.type}}">${{item.type}}</span>` +
    `<strong>${{item.description || '(无描述)'}}<\\/strong><br/>` +
    `<a href="{BASE}${{item.route_url}}" target="_blank">{BASE}${{item.route_url}}<\\/a>` +
    `<div class="small">route_url=${{item.route_url}} | file_ext=${{item.file_ext}}<\\/div>` +
    `</div>`
  ).join('');
  document.getElementById('attachments').innerHTML = attachments;
  document.getElementById('records').textContent = JSON.stringify(payload.records || [], null, 2);
  document.getElementById('comments').textContent = JSON.stringify(payload.annotation_comments || [], null, 2);
}});
</script>
</body>
</html>
'''
(OUT_DIR / 'index.html').write_text(html, encoding='utf-8')

cleanup = {'skipped': True}
if CLEANUP_FORM:
    cleanup = curl_json([
        '-X', 'POST', f'{BASE}/api/review/delete',
        '-H', 'Content-Type: application/json',
        '-d', json.dumps({'form_ids': [form_id], 'operator_id': USER_ID, 'token': token}, ensure_ascii=False),
    ])
write_json('cleanup.json', cleanup)

Path(png_path).unlink(missing_ok=True)
Path(pdf_path).unlink(missing_ok=True)

print(json.dumps({
    'form_id': form_id,
    'task_id': task_id,
    'summary_path': str(OUT_DIR / 'summary.json'),
    'response_path': str(OUT_DIR / 'response.json'),
    'cleanup': cleanup,
}, ensure_ascii=False))
PY

if lsof -tiTCP:"$PORT" -sTCP:LISTEN >/dev/null 2>&1; then
  kill $(lsof -tiTCP:"$PORT" -sTCP:LISTEN) || true
  sleep 1
fi

nohup python3 -m http.server "$PORT" --directory "$OUT_DIR" </dev/null >"$OUT_DIR/server.log" 2>&1 &
server_pid=$!
disown "$server_pid" 2>/dev/null || true
sleep 1

echo "PMS mock page ready: http://127.0.0.1:${PORT}/index.html"
echo "Raw response JSON: http://127.0.0.1:${PORT}/response.json"
echo "Summary JSON: http://127.0.0.1:${PORT}/summary.json"

if [[ "$OPEN_BROWSER" == "true" ]] && command -v open >/dev/null 2>&1; then
  open "http://127.0.0.1:${PORT}/index.html" >/dev/null 2>&1 || true
fi

curl -fsS "http://127.0.0.1:${PORT}/summary.json" | jq .
