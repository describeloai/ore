"""Rinde un Job de medida en t-victor con un guion de este directorio.

Uso: python hacer-job.py <nombre> <guion> <fase>  ->  <nombre>.json, para `kubectl apply -f`.
M2/M3 en Postgres: fase `a` (deriva, contadores, decodificacion), `c` (testigo del driver) y,
tras 7 min de reposo, `b` (los contadores tras suspender, y limpieza)."""
import json, sys
import os
S = os.path.dirname(os.path.abspath(__file__)) + '/'
nombre, guion, fase = sys.argv[1], sys.argv[2], sys.argv[3]
d = json.load(open(S + 'job-base.json', encoding='utf-8'))
d['metadata']['name'] = nombre
k = d['spec']['template']['spec']['containers'][0]
k['command'] = ['/bin/sh', '-c', open(S + guion, encoding='utf-8').read()]
k['env'] = [e for e in k['env'] if e['name'] != 'FASE'] + [{'name': 'FASE', 'value': fase}]
d['spec']['activeDeadlineSeconds'] = 1200
json.dump(d, open(S + nombre + '.json', 'w', encoding='utf-8'), ensure_ascii=False)
print('ok', nombre)
