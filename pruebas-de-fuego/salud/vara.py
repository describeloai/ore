"""LA VARA DE DATA HEALTH (ADR 0063 S0): M1–M8 como pruebas, con el resultado de la visión.

Cada caso cambia el origen, pasa la copia de verdad (un Job con la plantilla de la copia de
`main`, sólo sobre los datasets de la vara) y compara lo que ORE dice —el puntero y la salud de
`/assets`— con lo que la visión dice que debería decir. Un caso en rojo dice qué paso del plan lo
pone en verde. No es un banco: corre contra `victor`, en producción (la memoria «medir en prod»).

Los orígenes son propios y pequeños (1000 filas), y la vara los deja en su estado base al empezar
y al terminar:
  - Postgres (Neon, fuente `postgresql_20260918_1920`): `ore_vara.pedidos`, por un Job en la
    celda con la credencial del cofre (`vara-pg.sh`). La URL no sale de la celda.
  - BigQuery (fuente `bigquery_20260927_1428`): `ventas.ore_vara_pedidos`, por REST con el
    `gcloud` de quien la corre.

LO QUE SE MONTA UNA VEZ, Y LO HACE UNA PERSONA (`montar` lo explica): volver a leer el catálogo
de las dos fuentes y crear cuatro bases — `vara_pg` y `vara_bq` (standard) y `vara_pg_f` y
`vara_bq_f` (foreign). Recatalogar y retirar una base es gobierno: el agente de la celda no
puede, y la vara no se lo salta.

Uso:
  python vara.py montar     las tablas en su estado base, y qué falta montar a mano
  python vara.py correr     los casos (unos 15 min); deja el informe en vara-<fecha>.md
  python vara.py desmontar  borra las tablas de prueba (las bases, a mano: es gobierno)
"""
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

import yaml

S = os.path.dirname(os.path.abspath(__file__)) + '/'
NS = 't-victor'
PROYECTO = 'project-8853a180-450d-47be-b83'
API = 'https://victor.ore.paladio.io'
IDP = 'https://login.paladio.io/realms/rubix'
GCLOUD = 'gcloud.cmd' if os.name == 'nt' else 'gcloud'
FUENTE_PG, FUENTE_BQ = 'postgresql_20260918_1920', 'bigquery_20260927_1428'
# Los datasets de la vara: el puntero en el árbol, y la tabla de la foránea para leer en vivo.
ORIGENES = {
    'pg': {'vista': 'vara_pg.ore_vara.pedidos', 'puntero': 'datasets/vara_pg/ore_vara/pedidos.json',
           'foranea': 'vara_pg_f.ore_vara.pedidos', 'fuente': FUENTE_PG, 'objeto': 'ore_vara.pedidos'},
    'bq': {'vista': 'vara_bq.ventas.ore_vara_pedidos', 'puntero': 'datasets/vara_bq/ventas/ore_vara_pedidos.json',
           'foranea': 'vara_bq_f.ventas.ore_vara_pedidos', 'fuente': FUENTE_BQ, 'objeto': 'ventas.ore_vara_pedidos'},
}
BQ_T = '`%s.ventas.ore_vara_pedidos`' % PROYECTO


def sh(args, entrada=None):
    r = subprocess.run(args, capture_output=True, text=True, encoding='utf-8', input=entrada)
    return r.returncode, (r.stdout or '') + (r.stderr or '')


def hora():
    return time.strftime('%H:%M:%S')


def decir(t):
    print('  %s %s' % (hora(), t), flush=True)


# ── El origen ────────────────────────────────────────────────────────────────

def _bq_token():
    return subprocess.run([GCLOUD, 'auth', 'print-access-token'], capture_output=True, text=True).stdout.strip()


def bq(sql):
    """Una sentencia en BigQuery (DDL/DML: sin streaming, que la señal no ve, M3)."""
    r = urllib.request.Request(
        'https://bigquery.googleapis.com/bigquery/v2/projects/%s/queries' % PROYECTO, method='POST',
        data=json.dumps({'query': sql.replace('{T}', BQ_T), 'useLegacySql': False, 'timeoutMs': 90000}).encode(),
        headers={'authorization': 'Bearer ' + _bq_token(), 'content-type': 'application/json'})
    try:
        with urllib.request.urlopen(r) as x:
            j = json.loads(x.read() or b'{}')
    except urllib.error.HTTPError as e:
        raise SystemExit('BigQuery: %s' % e.read().decode()[:300])
    if not j.get('jobComplete', True):
        raise SystemExit('BigQuery: la sentencia no terminó en 90 s')
    return j


def job_en_la_celda(nombre, guion, env):
    """Un Job con `job-base.json` (la credencial del agente montada) que corre `guion`."""
    d = json.load(open(S + 'job-base.json', encoding='utf-8'))
    d['metadata']['name'] = nombre
    k = d['spec']['template']['spec']['containers'][0]
    k['command'] = ['/bin/sh', '-c', open(S + guion, encoding='utf-8').read()]
    k['env'] = [e for e in k['env'] if e['name'] not in env] + [{'name': n, 'value': v} for n, v in env.items()]
    d['spec']['activeDeadlineSeconds'] = 600
    d['spec']['backoffLimit'] = 0
    return d


def esperar_job(nombre, plazo=900):
    fin = time.time() + plazo
    while time.time() < fin:
        _, o = sh(['kubectl', '-n', NS, 'get', 'job', nombre, '-o', 'jsonpath={.status.succeeded}/{.status.failed}'])
        if o.startswith('1/') or o.endswith('/1'):
            return o.startswith('1/')
        time.sleep(5)
    return False


def lanzar(d):
    c, o = sh(['kubectl', 'create', '-f', '-'], json.dumps(d))
    if c != 0:
        raise SystemExit('kubectl create %s: %s' % (d['metadata']['name'], o[-300:]))


def pg(sql):
    """Una sentencia en el Postgres de la vara, por un Job en la celda (≈30 s)."""
    n = 'vara-pg-%d' % (time.time() * 1000 % 10 ** 9)
    lanzar(job_en_la_celda(n, 'vara-pg.sh', {'SQL': sql}))
    esperar_job(n, 300)
    _, o = sh(['kubectl', '-n', NS, 'logs', 'job/' + n])
    sh(['kubectl', '-n', NS, 'delete', 'job', n, '--wait=false'])
    if '## ok' not in o:
        raise SystemExit('Postgres: %s' % o[-400:])
    return o


PG_BASE = """
CREATE SCHEMA IF NOT EXISTS ore_vara;
DROP TABLE IF EXISTS ore_vara.pedidos;
CREATE TABLE ore_vara.pedidos(id int PRIMARY KEY, cliente text, total numeric(10,2) NOT NULL, creado timestamptz NOT NULL);
INSERT INTO ore_vara.pedidos SELECT g, CASE WHEN g % 10 = 0 THEN NULL ELSE 'cliente-' || (g % 37) END,
  round((g * 1.37)::numeric, 2), timestamptz '2026-10-01 00:00:00+00' + g * interval '1 minute' FROM generate_series(1, 1000) g;
"""
BQ_BASE = """CREATE OR REPLACE TABLE {T} (id INT64 NOT NULL, cliente STRING, total NUMERIC NOT NULL, creado TIMESTAMP NOT NULL) AS
SELECT id, IF(MOD(id, 10) = 0, NULL, CONCAT('cliente-', CAST(MOD(id, 37) AS STRING))), CAST(ROUND(id * 1.37, 2) AS NUMERIC),
  TIMESTAMP_ADD(TIMESTAMP '2026-10-01 00:00:00+00', INTERVAL id MINUTE) FROM UNNEST(GENERATE_ARRAY(1, 1000)) AS id"""

# Lo que cada caso le hace al origen, en los dos dialectos.
CAMBIOS = {
    'base': (PG_BASE, BQ_BASE),
    'actualizar': ("UPDATE ore_vara.pedidos SET total = total + 1 WHERE id = 7",
                   "UPDATE {T} SET total = total + 1 WHERE id = 7"),
    'vaciar': ("DELETE FROM ore_vara.pedidos", "DELETE FROM {T} WHERE TRUE"),
    'columna_nueva': ("ALTER TABLE ore_vara.pedidos ADD COLUMN nota text",
                      "ALTER TABLE {T} ADD COLUMN nota STRING"),
    'renombrar': ("ALTER TABLE ore_vara.pedidos RENAME COLUMN cliente TO comprador",
                  "ALTER TABLE {T} RENAME COLUMN cliente TO comprador"),
    'tirar': ("DROP TABLE ore_vara.pedidos", "DROP TABLE {T}"),
    'retirar': ("DROP SCHEMA IF EXISTS ore_vara CASCADE", "DROP TABLE IF EXISTS {T}"),
}


def cambiar(que):
    decir('origen: %s' % que)
    p, b = CAMBIOS[que]
    pg(p)
    bq(b)


# ── La copia ─────────────────────────────────────────────────────────────────

def ore_serve(guion):
    c, o = sh(['kubectl', '-n', NS, 'exec', 'deploy/ore-serve', '-c', 'ore-serve', '--', 'sh', '-c', guion])
    if c != 0:
        raise SystemExit('ore-serve: %s' % o[-300:])
    return o


GIT = 'git -c http.extraheader="Authorization: token $(cat /testigo/forja)"'
FORJA = 'http://forja.t-victor.svc.cluster.local:3000/t-victor'
_plantilla = None


def plantilla():
    """La plantilla de la copia que ore-serve rinde (`plantilla-copia.txt` de la cola), la de hoy."""
    global _plantilla
    if _plantilla is None:
        t = ore_serve('cd /tmp && rm -rf vara-cola && %s clone -q --depth 1 %s/trabajo.git vara-cola && '
                      'cat vara-cola/plantilla-copia.txt && rm -rf vara-cola' % (GIT, FORJA))
        _plantilla = yaml.safe_load(t)
    return json.loads(json.dumps(_plantilla))


SOLO = 'SOLO=""; for v in $(echo "$VISTAS" | tr \',\' \' \'); do SOLO="$SOLO --vista $v"; done\n'


def job_de_pasada(nombre, vistas):
    """Una pasada como la de `main`, sólo sobre `vistas` (sin `REHACER` ni rama, la plantilla
    materializa el árbol entero)."""
    d = plantilla()
    d['metadata'] = {'name': nombre, 'namespace': NS,
                     'labels': {'kueue.x-k8s.io/queue-name': 'cola', 'ore.dev/rol': 'driver', 'ore.dev/tenant': 'victor'}}
    d['spec'].pop('suspend', None)
    d['spec'].pop('selector', None)
    t = d['spec']['template']
    t['metadata'] = {'labels': {'ore.dev/rol': 'driver', 'ore.dev/tenant': 'victor'}}
    for c in t['spec'].get('initContainers', []) + t['spec']['containers']:
        for e in c.get('env', []):
            if e['name'] == 'VISTAS':
                e['value'] = vistas
            if e['name'] in ('REHACER', 'RAMA'):
                e['value'] = ''
        a = c.get('args') or []
        if a and 'ore materialize' in a[-1]:
            x = a[-1].replace('set -e\n', 'set -e\n' + SOLO, 1)
            x = x.replace('ore materialize . --preparar /trabajo/calculo \\\n', 'ore materialize . --preparar /trabajo/calculo $SOLO \\\n')
            x = x.replace('ore materialize . --recoger --reclaman', 'ore materialize . $SOLO --recoger --reclaman')
            if x.count('$SOLO') < 3:
                raise SystemExit('la plantilla de la copia cambió: no sé limitarla a las vistas de la vara')
            a[-1] = x
    return d


def cola_quieta(plazo=1200):
    """Que no corra otra copia: dos pasadas empujando a `main` a la vez se pisan."""
    fin = time.time() + plazo
    while time.time() < fin:
        _, o = sh(['kubectl', '-n', NS, 'get', 'jobs', '-o', 'json'])
        activos = [j['metadata']['name'] for j in json.loads(o or '{"items":[]}')['items']
                   if j['metadata']['name'].startswith('copiar-') and j.get('status', {}).get('active')]
        if not activos:
            return
        decir('espero a %s' % ', '.join(activos))
        time.sleep(20)


_n = [0]


def pasar():
    """Una pasada de copia sobre los dos datasets de la vara. Devuelve su log."""
    cola_quieta()
    _n[0] += 1
    n = 'vara-pasada-%d-%d' % (_n[0], time.time() % 10 ** 6)
    lanzar(job_de_pasada(n, ','.join(o['vista'] for o in ORIGENES.values())))
    bien = esperar_job(n)
    _, log = sh(['kubectl', '-n', NS, 'logs', 'job/' + n, '-c', 'copiar'])
    sh(['kubectl', '-n', NS, 'delete', 'job', n, '--wait=false'])
    decir('pasada %d: %s' % (_n[0], 'terminó' if bien else 'FALLÓ el Job'))
    return log


def punteros():
    t = ore_serve('cd /tmp && ([ -d vara-arbol ] || %s clone -q %s/ontologia.git vara-arbol) && cd vara-arbol && '
                  '%s pull -q && for f in %s; do echo "@@$f"; cat "$f" 2>/dev/null || echo null; echo; done'
                  % (GIT, FORJA, GIT, ' '.join(o['puntero'] for o in ORIGENES.values())))
    salida = {}
    for b in t.split('@@')[1:]:
        f, _, j = b.partition('\n')
        clave = next(k for k, o in ORIGENES.items() if o['puntero'] == f.strip())
        salida[clave] = json.loads(j) if j.strip() not in ('', 'null') else None
    return salida


# ── Lo que lee quien lee ─────────────────────────────────────────────────────

_tok = [None, 0]


def token():
    if _tok[0] and time.time() - _tok[1] < 200:
        return _tok[0]
    cli = sh([GCLOUD, 'secrets', 'versions', 'access', 'latest', '--secret=%s-agente-cliente' % NS, '--project=' + PROYECTO])[1].strip()
    sec = sh([GCLOUD, 'secrets', 'versions', 'access', 'latest', '--secret=%s-agente-secreto' % NS, '--project=' + PROYECTO])[1].strip()
    cuerpo = urllib.parse.urlencode({'grant_type': 'client_credentials', 'client_id': cli, 'client_secret': sec})
    # Con `curl` y no con urllib: Cloudflare, delante del IdP, contesta 403 a `Python-urllib`. Y el
    # secreto por la entrada (`-K -`), nunca en la línea de órdenes.
    conf = 'url = "%s/protocol/openid-connect/token"\ndata = "%s"\n' % (IDP, cuerpo)
    c, o = sh(['curl', '-s', '-m', '20', '-K', '-'], conf)
    try:
        _tok[0], _tok[1] = json.loads(o)['access_token'], time.time()
    except (ValueError, KeyError):
        raise SystemExit('el IdP no dio token al agente: %s' % o[:160])
    return _tok[0]


def serve(metodo, camino, cuerpo=None):
    """`curl` con el token por la entrada (`-K -`): ni en la línea de órdenes ni en un fichero."""
    conf = 'url = "%s%s"\nrequest = "%s"\nheader = "authorization: Bearer %s"\n' % (API, camino, metodo, token())
    if cuerpo is not None:
        conf += 'header = "content-type: application/json"\ndata = %s\n' % json.dumps(json.dumps(cuerpo))
    c, o = sh(['curl', '-s', '-m', '120', '-w', '\n%{http_code}', '-K', '-'], conf)
    b, _, cod = o.rpartition('\n')
    return (int(cod) if cod.strip().isdigit() else 0), b.encode()


def saludes():
    """La salud de cada dataset de la vara, como `/assets` se la da a la consola."""
    c, b = serve('GET', '/assets')
    if c != 200:
        raise SystemExit('/assets: %s %s' % (c, b[:200]))
    items = json.loads(b).get('items', {})
    salida = {}
    for k, o in ORIGENES.items():
        it = items.get('dataset:' + o['vista'])  # la clave del índice: `kind:qname`, en minúsculas
        salida[k] = (it or {}).get('salud') if it else None
    return salida


def leer_en_vivo(k):
    """Una lectura en vivo de la foránea, por `/federation/read`: lo que SQL vería."""
    c, b = serve('POST', '/federation/read', {'tabla': ORIGENES[k]['foranea'], 'columnas': ['id', 'cliente'], 'limit': 5})
    try:
        j = json.loads(b)
    except ValueError:
        j = {}
    return c, j


# ── Los casos ────────────────────────────────────────────────────────────────

RESULTADOS = []


def caso(cid, m, que, paso, origen, bien, visto):
    RESULTADOS.append((cid, m, que, paso, origen, bien, visto))
    print('    %s %s · %s · %s — %s' % ('✅' if bien else '❌', cid, origen, que, visto), flush=True)


def razones(s):
    return [r.get('code') for r in (s or {}).get('reasons', [])]


def correr():
    t0 = time.time()
    print('== la vara de Data Health · %s' % time.strftime('%Y-%m-%d %H:%M'))
    _, c = sh(['kubectl', '-n', NS, 'get', 'deploy', 'ore-serve', '-o', 'jsonpath={.spec.template.metadata.annotations.ore\\.dev/commit}'])
    print('   ore-serve en %s' % c[:8])
    s = saludes_o_vacio()
    faltan = [o['vista'] for k, o in ORIGENES.items() if s.get(k) is None]
    if faltan:
        raise SystemExit('faltan las bases de la vara (%s): `python vara.py montar`' % ', '.join(faltan))

    # C1 · la hora de los datos (M1 → S2)
    cambiar('base')
    antes = time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())
    pasar()
    p, s = punteros(), saludes()
    for k in ORIGENES:
        d = (p[k] or {}).get('datos_de')
        caso('C1', 'M1', 'la copia sabe de cuándo son sus datos', 'S2', k,
             bool(d) and d >= antes and (s[k] or {}).get('dataAsOf') == d,
             'datos_de=%s, /assets dataAsOf=%s' % (d, (s[k] or {}).get('dataAsOf')))

    # C2 · sin cambios: comprobado, no copiado (M6 → S3)
    pasar()
    p, s = punteros(), saludes()
    for k in ORIGENES:
        x = p[k] or {}
        caso('C2', 'M6', 'nada cambió: «al día» comprobado, sin leer', 'S3', k,
             (x.get('intento') or {}).get('resultado') == 'sin-cambios' and (x.get('comprobado') or '') > (x.get('datos_de') or '~')
             and (s[k] or {}).get('state') == 'healthy',
             'intento=%s, comprobado=%s, estado=%s' % ((x.get('intento') or {}).get('resultado'), x.get('comprobado'), (s[k] or {}).get('state')))

    # C3 · un UPDATE, que el cursor no ve (M2/M6 → S3)
    previos = {k: (p[k] or {}).get('datos_de') for k in ORIGENES}
    cambiar('actualizar')
    pasar()
    p = punteros()
    for k in ORIGENES:
        x = p[k] or {}
        caso('C3', 'M2', 'una fila actualizada se copia', 'S3', k,
             (x.get('intento') or {}).get('resultado') == 'publicada' and (x.get('datos_de') or '') > (previos[k] or ''),
             'intento=%s, datos_de=%s' % ((x.get('intento') or {}).get('resultado'), x.get('datos_de')))

    # C4 · una columna nueva en el origen (P3 → S5)
    cambiar('columna_nueva')
    pasar()
    p, s = punteros(), saludes()
    for k in ORIGENES:
        caso('C4', 'P3', 'una columna nueva se dice (source-schema-changed)', 'S5', k,
             'source-schema-changed' in razones(s[k]),
             'estado=%s, motivos=%s, filas=%s' % ((s[k] or {}).get('state'), razones(s[k]), (p[k] or {}).get('filas')))

    # C5 y C6 · una columna renombrada: en vivo (M8/P4 → S5) y en la copia (P4 → S2 + S5)
    cambiar('renombrar')
    for k in ORIGENES:
        c, j = leer_en_vivo(k)
        cod = j.get('codigo') or j.get('code') or ''
        msg = (j.get('mensaje') or j.get('message') or j.get('error') or '')[:120]
        caso('C5', 'M8', 'en vivo, una columna renombrada es «schema drift», no «Not found»', 'S5', k,
             c >= 400 and 'schema' in (cod + msg).lower() and 'not found' not in msg.lower(),
             'HTTP %s, codigo=%s, «%s»' % (c, cod, msg))
    pasar()
    p, s = punteros(), saludes()
    for k in ORIGENES:
        x = p[k] or {}
        caso('C6', 'M5', 'la copia que falla sirve lo último bueno', 'S2', k,
             bool(x.get('metadata_location')) and (s[k] or {}).get('servingLastGood') is True and (s[k] or {}).get('state') == 'degraded',
             'estado=%s, servingLastGood=%s, motivos=%s' % ((s[k] or {}).get('state'), (s[k] or {}).get('servingLastGood'), razones(s[k])))
        caso('C6b', 'P4', 'y dice que el esquema del origen cambió', 'S5', k,
             'source-schema-changed' in razones(s[k]), 'motivos=%s' % razones(s[k]))

    # C7 · el origen vaciado (P5 → S4): se retiene, no se publica con 0 filas
    cambiar('base')
    pasar()
    cambiar('vaciar')
    pasar()
    p, s = punteros(), saludes()
    for k in ORIGENES:
        x = p[k] or {}
        caso('C7', 'P5', 'un origen vaciado se retiene (refresh-held), no se publica', 'S4', k,
             (x.get('intento') or {}).get('resultado') == 'retenida' and x.get('filas') == 1000,
             'intento=%s, filas=%s, motivos=%s' % ((x.get('intento') or {}).get('resultado'), x.get('filas'), razones(s[k])))

    # C8 · el origen desaparece (M5/M7 → S2)
    cambiar('base')
    pasar()
    cambiar('tirar')
    pasar()
    p, s = punteros(), saludes()
    for k in ORIGENES:
        x = p[k] or {}
        caso('C8', 'M7', 'sin origen: degraded, refresh-failed, y lo último bueno se sigue leyendo', 'S2', k,
             x.get('estado') == 'error' and bool(x.get('metadata_location')) and 'refresh-failed' in razones(s[k])
             and (s[k] or {}).get('servingLastGood') is True,
             'estado=%s, motivos=%s, servingLastGood=%s' % (x.get('estado'), razones(s[k]), (s[k] or {}).get('servingLastGood')))

    # Y la vara se deja como la encontró: la tabla base, copiada.
    cambiar('base')
    pasar()
    informe(time.time() - t0)


def saludes_o_vacio():
    try:
        return saludes()
    except SystemExit:
        return {}


def informe(segundos):
    verdes = sum(1 for r in RESULTADOS if r[5])
    lineas = ['# La vara de Data Health · %s' % time.strftime('%Y-%m-%d %H:%M'), '',
              '%d de %d en verde · %d min · `victor`' % (verdes, len(RESULTADOS), segundos // 60), '',
              '| | Caso | M | Lo que la visión espera | Lo pone en verde | Origen | Visto |', '|---|---|---|---|---|---|---|']
    for cid, m, que, paso, origen, bien, visto in RESULTADOS:
        lineas.append('| %s | %s | %s | %s | %s | %s | %s |' % ('✅' if bien else '❌', cid, m, que, paso, origen, visto.replace('|', '/')))
    f = S + 'vara-%s.md' % time.strftime('%Y%m%d-%H%M')
    open(f, 'w', encoding='utf-8').write('\n'.join(lineas) + '\n')
    print('\n'.join(lineas))
    print('\ninforme: %s' % f)


def montar():
    print('== montar la vara')
    cambiar('base')
    s = saludes_o_vacio()
    faltan = [o['vista'] for k, o in ORIGENES.items() if s.get(k) is None]
    if not faltan:
        print('   montada: las tablas en su estado base y las cuatro bases existen. `python vara.py correr`')
        return
    print("""
   Las tablas están en su estado base. Falta, UNA VEZ, desde la consola (es gobierno: el agente no puede):
     1. Volver a leer el catálogo de las fuentes `%s` (Neon) y `%s` (BigQuery).
     2. Crear cuatro bases sobre esas tablas, con «sólo» esa tabla:
          vara_pg    standard  ← %s · ore_vara.pedidos
          vara_pg_f  foreign   ← %s · ore_vara.pedidos
          vara_bq    standard  ← %s · ventas.ore_vara_pedidos
          vara_bq_f  foreign   ← %s · ventas.ore_vara_pedidos
   Y después: `python vara.py correr`.""" % (FUENTE_PG, FUENTE_BQ, FUENTE_PG, FUENTE_PG, FUENTE_BQ, FUENTE_BQ))


def desmontar():
    print('== desmontar la vara: las tablas de prueba fuera del origen')
    cambiar('retirar')
    print('   hecho. Las bases vara_pg, vara_pg_f, vara_bq y vara_bq_f se retiran desde la consola.')


if __name__ == '__main__':
    fase = sys.argv[1] if len(sys.argv) > 1 else ''
    {'montar': montar, 'correr': correr, 'desmontar': desmontar}.get(fase, lambda: sys.exit(__doc__))()
