import argparse
import json
import subprocess
import sys
import time


def request(base: str, method: str, path: str, payload=None):
    cmd = ['curl', '-sS', '-X', method, f'{base}{path}']
    if payload is not None:
        cmd += ['-H', 'content-type: application/json', '-d', json.dumps(payload, ensure_ascii=False)]
    out = subprocess.check_output(cmd, text=True)
    return json.loads(out)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument('--base-url', default='http://127.0.0.1:3110')
    args = parser.parse_args()

    stamp = str(int(time.time()))
    create_task_payload = {
        'title': f'dedup-validation-{stamp}',
        'description': 'validate confirmed record dedup',
        'modelName': 'AvevaMarineSample',
        'checkerId': 'checker-debug',
        'checkerName': 'checker-debug',
        'approverId': 'approver-debug',
        'approverName': 'approver-debug',
        'reviewerId': 'checker-debug',
        'priority': 'medium',
        'components': [
            {
                'id': 'cmp-1',
                'name': 'BRAN-24381_145018',
                'refNo': '24381_145018',
                'type': 'BRAN',
            }
        ],
    }

    task_resp = request(args.base_url, 'POST', '/api/review/tasks', create_task_payload)
    if not task_resp.get('success') or not task_resp.get('task'):
        print(json.dumps({'stage': 'create_task', 'response': task_resp}, ensure_ascii=False))
        return 2

    task = task_resp['task']
    task_id = task['id']
    form_id = task['formId']

    payload_v1 = {
        'taskId': task_id,
        'formId': form_id,
        'type': 'batch',
        'annotations': [
            {
                'id': 'ann-1',
                'kind': 'text',
                'text': 'first-note',
                'position': {'x': 1, 'y': 2, 'z': 3},
            }
        ],
        'cloudAnnotations': [],
        'rectAnnotations': [],
        'obbAnnotations': [],
        'measurements': [],
        'note': 'note-v1',
    }

    resp1 = request(args.base_url, 'POST', '/api/review/records', payload_v1)
    time.sleep(1.2)
    resp2 = request(args.base_url, 'POST', '/api/review/records', payload_v1)
    time.sleep(1.2)
    payload_v2 = dict(payload_v1)
    payload_v2['note'] = 'note-v2'
    resp3 = request(args.base_url, 'POST', '/api/review/records', payload_v2)
    list_resp = request(args.base_url, 'GET', f'/api/review/records/by-task/{task_id}')

    summary = {
        'taskId': task_id,
        'formId': form_id,
        'firstId': resp1.get('record', {}).get('id'),
        'secondId': resp2.get('record', {}).get('id'),
        'thirdId': resp3.get('record', {}).get('id'),
        'firstConfirmedAt': resp1.get('record', {}).get('confirmedAt'),
        'secondConfirmedAt': resp2.get('record', {}).get('confirmedAt'),
        'thirdConfirmedAt': resp3.get('record', {}).get('confirmedAt'),
        'listCount': len(list_resp.get('records') or []),
        'listRecordIds': [r.get('id') for r in (list_resp.get('records') or [])],
        'listNotes': [r.get('note') for r in (list_resp.get('records') or [])],
        'samePayloadSameId': resp1.get('record', {}).get('id') == resp2.get('record', {}).get('id'),
        'samePayloadSameConfirmedAt': resp1.get('record', {}).get('confirmedAt') == resp2.get('record', {}).get('confirmedAt'),
        'changedPayloadSameId': resp1.get('record', {}).get('id') == resp3.get('record', {}).get('id'),
        'changedPayloadUpdatedConfirmedAt': (resp3.get('record', {}).get('confirmedAt') or 0) > (resp2.get('record', {}).get('confirmedAt') or 0),
    }

    print(json.dumps({
        'summary': summary,
        'createTask': task_resp,
        'firstSave': resp1,
        'secondSave': resp2,
        'thirdSave': resp3,
        'list': list_resp,
    }, ensure_ascii=False))

    if not summary['samePayloadSameId']:
        return 3
    if not summary['samePayloadSameConfirmedAt']:
        return 4
    if not summary['changedPayloadSameId']:
        return 5
    if not summary['changedPayloadUpdatedConfirmedAt']:
        return 6
    if summary['listCount'] != 1:
        return 7
    return 0


if __name__ == '__main__':
    sys.exit(main())
