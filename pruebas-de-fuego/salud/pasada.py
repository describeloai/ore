"""Una pasada de copia como la de la malla, sólo sobre los datasets dados. Uso: pasada.py <nombre> <vistas,separadas,por,comas> [rehacer]"""
import json, sys, time
import os
# copiar-base.json: `kubectl -n t-victor get job <copiar-…> -o json`, de una pasada de main de la malla (no de rama: su guion puede ser anterior).
S = os.path.dirname(os.path.abspath(__file__)) + '/'
nombre = sys.argv[1]; vistas = sys.argv[2]; rehacer = len(sys.argv) > 3
d = json.load(open(S + 'copiar-base.json', encoding='utf-8'))
d = {'apiVersion': 'batch/v1', 'kind': 'Job',
     'metadata': {'name': nombre, 'namespace': 't-victor', 'labels': {'kueue.x-k8s.io/queue-name': 'cola', 'ore.dev/rol': 'driver', 'ore.dev/tenant': 'victor'}},
     'spec': {k: v for k, v in d['spec'].items() if k not in ('selector',)}}
t = d['spec']['template']
t['metadata'] = {'labels': {'ore.dev/rol': 'driver', 'ore.dev/tenant': 'victor'}}
for c in t['spec'].get('initContainers', []) + t['spec']['containers']:
    for e in c.get('env', []):
        if e['name'] == 'VISTAS': e['value'] = vistas
        if e['name'] == 'REHACER': e['value'] = time.strftime('%Y%m%dT%H%M%SZ', time.gmtime()) if rehacer else ''
        if e['name'] == 'RAMA': e['value'] = ''
# Sin `REHACER` ni rama, la plantilla materializa el árbol ENTERO (y sólo abre
# las credenciales de las vistas pedidas): aquí, sólo las vistas pedidas.
SOLO = """SOLO=""; for v in $(echo "$VISTAS" | tr ',' ' '); do SOLO="$SOLO --vista $v"; done\n"""
for c in t['spec'].get('initContainers', []) + t['spec']['containers']:
    a = c.get('args') or []
    if a and 'ore materialize' in a[-1]:
        x = a[-1].replace('set -e\n', 'set -e\n' + SOLO, 1)
        x = x.replace('ore materialize . --preparar /trabajo/calculo \\\n', 'ore materialize . --preparar /trabajo/calculo $SOLO \\\n')
        x = x.replace('ore materialize . --recoger --reclaman', 'ore materialize . $SOLO --recoger --reclaman')
        assert x.count('$SOLO') >= 3, c['name']
        a[-1] = x
d['spec'].pop('suspend', None)
json.dump(d, open(S + nombre + '.json', 'w', encoding='utf-8'), ensure_ascii=False)
print('ok', nombre, 'rehacer' if rehacer else 'normal')
