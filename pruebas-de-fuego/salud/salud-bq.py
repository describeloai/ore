"""La tabla de prueba de M4–M8 en BigQuery (ventas.ore_salud_pedidos). Uso: salud-bq.py [SQL...]"""
import json, subprocess, sys, urllib.request, urllib.error
P = 'project-8853a180-450d-47be-b83'; T = f'`{P}.ventas.ore_salud_pedidos`'
TOK = subprocess.run(['cmd', '/c', 'gcloud auth print-access-token'], capture_output=True, text=True).stdout.strip()
def api(m, ruta, c=None):
    r = urllib.request.Request('https://bigquery.googleapis.com/bigquery/v2/projects/' + P + ruta, method=m,
        data=json.dumps(c).encode() if c is not None else None, headers={'authorization': 'Bearer ' + TOK, 'content-type': 'application/json'})
    try:
        with urllib.request.urlopen(r) as x: return json.loads(x.read() or b'{}')
    except urllib.error.HTTPError as e: return {'error': e.read().decode()[:400]}
def sql(t):
    r = api('POST', '/queries', {'query': t, 'useLegacySql': False, 'location': 'EU', 'timeoutMs': 60000})
    if 'error' in r: print('  ERROR', r['error'])
    return r
for t in sys.argv[1:]:
    t = t.replace('{T}', T); print('##', t); sql(t)
r = sql(f'SELECT COUNT(*), COUNT(cliente), COUNT(total), CAST(MAX(creado) AS STRING) FROM {T}')
if 'rows' in r: print('  filas=%s cliente_no_nulos=%s total_no_nulos=%s max_creado=%s' % tuple(c['v'] for c in r['rows'][0]['f']))
g = api('GET', '/datasets/ventas/tables/ore_salud_pedidos')
print('  lastModifiedTime=', g.get('lastModifiedTime'), 'numRows=', g.get('numRows'), 'esquema=', [(f['name'], f['type'], f.get('mode')) for f in g.get('schema', {}).get('fields', [])])
