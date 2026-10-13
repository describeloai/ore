"""M2/M3 en BigQuery: qué ve el testigo `field` (MAX del cursor, la consulta del driver) y qué ve
la hora de última modificación de la tabla, ante cada clase de cambio. Dataset propio, se borra."""
import json, subprocess, time, urllib.request, urllib.error

P = 'project-8853a180-450d-47be-b83'
D = 'ore_salud_m2'
TOK = subprocess.run(['cmd', '/c', 'gcloud auth print-access-token'], capture_output=True, text=True).stdout.strip()
assert TOK, 'sin token'


def api(metodo, ruta, cuerpo=None):
    url = 'https://bigquery.googleapis.com/bigquery/v2/projects/' + P + ruta
    r = urllib.request.Request(url, method=metodo, data=json.dumps(cuerpo).encode() if cuerpo is not None else None,
                               headers={'authorization': 'Bearer ' + TOK, 'content-type': 'application/json'})
    t = time.time()
    try:
        with urllib.request.urlopen(r) as x:
            b = x.read()
            return (json.loads(b) if b else {}), int((time.time() - t) * 1000)
    except urllib.error.HTTPError as e:
        return {'error': e.read().decode()[:300]}, int((time.time() - t) * 1000)


def sql(texto):
    r, ms = api('POST', '/queries', {'query': texto, 'useLegacySql': False, 'location': LOC, 'timeoutMs': 30000})
    if 'error' in r:
        print('   ERROR', r['error'])
    return r, ms


ventas, _ = api('GET', '/datasets/ventas')
LOC = ventas.get('location', 'EU')
print('location', LOC)
api('DELETE', '/datasets/' + D + '?deleteContents=true')
r, _ = api('POST', '/datasets', {'datasetReference': {'projectId': P, 'datasetId': D}, 'location': LOC})
print('dataset', 'error' not in r)
sql(f'CREATE TABLE `{P}.{D}.t` (id INT64, ts TIMESTAMP, v STRING)')
sql(f'CREATE TABLE `{P}.{D}.u` (id INT64, v STRING)')
sql(f"INSERT `{P}.{D}.t` VALUES (1, TIMESTAMP '2026-10-01 10:00:00', 'x'), (2, TIMESTAMP '2026-10-02 10:00:00', 'y'), (3, TIMESTAMP '2026-10-03 10:00:00', 'z')")
time.sleep(2)

previo = {}


def mirar(paso):
    w, msw = sql(f'SELECT CAST(MAX(`ts`) AS STRING) AS m FROM `{P}.{D}.t`')
    testigo = w.get('rows', [{}])[0].get('f', [{}])[0].get('v') if 'rows' in w else None
    facturado = w.get('totalBytesBilled', w.get('totalBytesProcessed'))
    g, msg = api('GET', f'/datasets/{D}/tables/t')
    lm, filas = g.get('lastModifiedTime'), g.get('numRows')
    gu, _ = api('GET', f'/datasets/{D}/tables/u')
    cambio_t = 'testigo ' + ('IGUAL' if previo.get('w') == testigo else 'CAMBIA')
    cambio_m = 'lastModified ' + ('IGUAL' if previo.get('m') == lm else 'CAMBIA')
    print(f'{paso:<44} testigo={testigo} ({msw} ms, {facturado} B facturados) · lastModifiedTime={lm} numRows={filas} ({msg} ms) · u.lastModified={gu.get("lastModifiedTime")}')
    if previo:
        print(f'{"":<44} => {cambio_t} · {cambio_m}')
    previo.update(w=testigo, m=lm)


mirar('0 · base (3 filas)')
time.sleep(2); sql(f"UPDATE `{P}.{D}.t` SET v = 'x2' WHERE id = 1"); mirar('1 · UPDATE sin mover el cursor')
time.sleep(2); sql(f"DELETE `{P}.{D}.t` WHERE id = 2"); mirar('2 · DELETE de una fila que no es la máxima')
time.sleep(2); sql(f"INSERT `{P}.{D}.t` VALUES (4, TIMESTAMP '2026-09-15 10:00:00', 'tarde')"); mirar('3 · INSERT tardío (cursor menor)')
time.sleep(2); sql(f"INSERT `{P}.{D}.u` VALUES (1, 'otro')"); mirar('4 · cambio en OTRA tabla (u)')
time.sleep(2); sql(f'SELECT COUNT(*) FROM `{P}.{D}.t`'); mirar('5 · sólo una lectura')
time.sleep(2); sql(f"INSERT `{P}.{D}.t` VALUES (5, TIMESTAMP '2026-10-05 10:00:00', 'nueva')"); mirar('6 · INSERT con cursor mayor')
time.sleep(2)
r, _ = api('POST', f'/datasets/{D}/tables/t/insertAll', {'rows': [{'json': {'id': 6, 'ts': '2026-10-06 10:00:00', 'v': 'stream'}}]})
print('   insertAll', 'error' not in r and not r.get('insertErrors'))
time.sleep(5); mirar('7 · inserción en streaming (insertAll)')
time.sleep(2); sql(f'TRUNCATE TABLE `{P}.{D}.u`'); mirar('8 · TRUNCATE de OTRA tabla')

t3, _ = sql(f'SELECT table_id, last_modified_time, row_count FROM `{P}.{D}.__TABLES__` ORDER BY 1')
print('__TABLES__', [[c['v'] for c in f['f']] for f in t3.get('rows', [])], 'facturado', t3.get('totalBytesBilled'))
api('DELETE', '/datasets/' + D + '?deleteContents=true')
print('dataset borrado')
