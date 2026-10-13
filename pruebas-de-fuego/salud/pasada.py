"""Una pasada de copia como la de la malla, sólo sobre los datasets de prueba. Uso: pasada.py <nombre> [rehacer]"""
import json, sys, time
S = 'C:/Users/PC/AppData/Local/Temp/claude/C--ORE/8b0a9474-07d2-469a-b4bc-ca1102ef4b2a/scratchpad/'
nombre = sys.argv[1]; rehacer = len(sys.argv) > 2
d = json.load(open(S + 'copiar-base.json', encoding='utf-8'))
d = {'apiVersion': 'batch/v1', 'kind': 'Job',
     'metadata': {'name': nombre, 'namespace': 't-victor', 'labels': {'kueue.x-k8s.io/queue-name': 'cola', 'ore.dev/rol': 'driver', 'ore.dev/tenant': 'victor'}},
     'spec': {k: v for k, v in d['spec'].items() if k not in ('selector',)}}
t = d['spec']['template']
t['metadata'] = {'labels': {'ore.dev/rol': 'driver', 'ore.dev/tenant': 'victor'}}
for c in t['spec'].get('initContainers', []) + t['spec']['containers']:
    for e in c.get('env', []):
        if e['name'] == 'VISTAS': e['value'] = 'pg_test_s.ore_salud.pedidos,bq_test_s.ventas.ore_salud_pedidos'
        if e['name'] == 'REHACER': e['value'] = time.strftime('%Y%m%dT%H%M%SZ', time.gmtime()) if rehacer else ''
        if e['name'] == 'RAMA': e['value'] = ''
d['spec'].pop('suspend', None)
json.dump(d, open(S + nombre + '.json', 'w', encoding='utf-8'), ensure_ascii=False)
print('ok', nombre, 'rehacer' if rehacer else 'normal')
