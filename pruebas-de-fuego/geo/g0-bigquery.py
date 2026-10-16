"""G0·2 (ADR 0067): el dataset BigQuery `banco_geo` del banco geo, con columnas GEOGRAPHY.

Por REST con el `gcloud` de quien lo corre (el CLI `bq` no está sano en este PC). Crea el dataset en
europe-west1 con lectura para la cuenta del driver de victor, y carga `lugares` y `municipios` desde
gs://…-banco-geo/carga/ (WKT → GEOGRAPHY). La fuente se da de alta aparte (`bigquery://<p>/banco_geo`).

    python pruebas-de-fuego/geo/g0-bigquery.py
"""
import json
import subprocess
import time
import urllib.error
import urllib.request

P = 'project-8853a180-450d-47be-b83'
B = f'{P}-banco-geo'
API = f'https://bigquery.googleapis.com/bigquery/v2/projects/{P}'
DRIVER = f'ore-driver-victor@{P}.iam.gserviceaccount.com'
TOK = subprocess.run('gcloud auth print-access-token', shell=True, capture_output=True, text=True).stdout.strip()


def pedir(metodo, url, cuerpo=None):
    r = urllib.request.Request(url, method=metodo, data=json.dumps(cuerpo).encode() if cuerpo else None,
                               headers={'authorization': f'Bearer {TOK}', 'content-type': 'application/json'})
    try:
        with urllib.request.urlopen(r) as x:
            return x.status, json.load(x)
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read() or b'{}')


c, r = pedir('POST', f'{API}/datasets', {
    'datasetReference': {'projectId': P, 'datasetId': 'banco_geo'}, 'location': 'europe-west1',
    'labels': {'ore-adr': '0067'}, 'description': 'Banco geo de ORE (ADR 0067 G0): GEOGRAPHY de Overture'})
print('dataset:', c, r.get('error', {}).get('message', 'creado'))
c, ds = pedir('GET', f'{API}/datasets/banco_geo')
if not any(a.get('userByEmail') == DRIVER for a in ds['access']):
    ds['access'].append({'role': 'READER', 'userByEmail': DRIVER})
    c, _ = pedir('PATCH', f'{API}/datasets/banco_geo', {'access': ds['access']})
    print('lectura para el driver:', c)
ESQ = {
    'lugares': [('id', 'STRING'), ('nombre', 'STRING'), ('categoria', 'STRING'), ('geo', 'GEOGRAPHY')],
    'municipios': [('id', 'STRING'), ('nombre', 'STRING'), ('tipo', 'STRING'), ('geo', 'GEOGRAPHY')],
}
for tabla, campos in ESQ.items():
    c, j = pedir('POST', f'{API}/jobs', {'configuration': {'load': {
        'sourceUris': [f'gs://{B}/carga/bq_{tabla}.csv'], 'sourceFormat': 'CSV', 'skipLeadingRows': 1,
        'allowQuotedNewlines': True, 'writeDisposition': 'WRITE_TRUNCATE',
        'destinationTable': {'projectId': P, 'datasetId': 'banco_geo', 'tableId': tabla},
        'schema': {'fields': [{'name': n, 'type': t} for n, t in campos]}}},
        'jobReference': {'projectId': P, 'location': 'europe-west1'}})
    jid = j['jobReference']['jobId']
    while True:
        _, j = pedir('GET', f'{API}/jobs/{jid}?location=europe-west1')
        if j['status']['state'] == 'DONE':
            break
        time.sleep(2)
    err = j['status'].get('errorResult')
    print(f'{tabla}:', err['message'] if err else f"{j['statistics']['load']['outputRows']} filas")
