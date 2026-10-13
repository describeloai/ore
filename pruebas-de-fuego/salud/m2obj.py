"""M2/M3 en objetos: el testigo `listing` es sha256 de las líneas «clave\\tetag\\ttamaño» ordenadas
(ore-read-objetos/src/catalogo.rs:736); el «etag» es la generación en GCS (ore-gcs/src/origen.rs:60)
y el ETag en S3/R2. Se mide qué hace con cada clase de cambio, y qué hace `updated`/`last-modified`.
Prefijo propio, se borra al final."""
import hashlib, json, subprocess, time, urllib.parse, urllib.request, urllib.error

PREF = 'ore-salud-m2/'


def http(metodo, url, tok, cuerpo=None, tipo='application/json'):
    datos = cuerpo if isinstance(cuerpo, (bytes, type(None))) else json.dumps(cuerpo).encode()
    r = urllib.request.Request(url, method=metodo, data=datos, headers={'authorization': 'Bearer ' + tok, 'content-type': tipo})
    t = time.time()
    try:
        with urllib.request.urlopen(r) as x:
            b = x.read()
            return (json.loads(b) if b and b[:1] in b'{[' else {}), int((time.time() - t) * 1000)
    except urllib.error.HTTPError as e:
        return {'error': e.code, 'texto': e.read().decode()[:300]}, int((time.time() - t) * 1000)


def testigo(lineas):
    return hashlib.sha256('\n'.join(sorted(lineas)).encode()).hexdigest()[:16]


class Gcs:
    nombre = 'GCS'
    B = 'project-8853a180-450d-47be-b83-origen-gcs'

    def __init__(self):
        self.tok = subprocess.run(['cmd', '/c', 'gcloud auth print-access-token'], capture_output=True, text=True).stdout.strip()

    def poner(self, clave, datos):
        u = f'https://storage.googleapis.com/upload/storage/v1/b/{self.B}/o?uploadType=media&name=' + urllib.parse.quote(clave, safe='')
        return http('POST', u, self.tok, datos, 'application/octet-stream')

    def meta(self, clave):
        u = f'https://storage.googleapis.com/storage/v1/b/{self.B}/o/' + urllib.parse.quote(clave, safe='')
        return http('PATCH', u, self.tok, {'metadata': {'ore': str(time.time())}})

    def borrar(self, clave):
        return http('DELETE', f'https://storage.googleapis.com/storage/v1/b/{self.B}/o/' + urllib.parse.quote(clave, safe=''), self.tok)

    def listar(self):
        r, ms = http('GET', f'https://storage.googleapis.com/storage/v1/b/{self.B}/o?prefix=' + urllib.parse.quote(PREF, safe=''), self.tok)
        objs = [(o['name'], o['generation'], o['size'], o.get('updated'), o.get('md5Hash'), o.get('metageneration')) for o in r.get('items', [])]
        return objs, [f'{n}\t{g}\t{s}' for n, g, s, *_ in objs], ms


class R2:
    nombre = 'R2'
    B = 'ore-origen-r2'

    def __init__(self):
        tok = None
        for l in open('C:/rubix-platform/.env.local', encoding='utf-8'):
            if l.startswith('CLOUDFLARE_API_TOKEN='):
                tok = l.split('=', 1)[1].strip().strip('"').strip("'")
        self.tok = tok
        r, _ = http('GET', 'https://api.cloudflare.com/client/v4/accounts', tok)
        self.cuenta = r['result'][0]['id']
        self.base = f'https://api.cloudflare.com/client/v4/accounts/{self.cuenta}/r2/buckets/{self.B}/objects'

    def poner(self, clave, datos):
        return http('PUT', self.base + '/' + urllib.parse.quote(clave, safe='/'), self.tok, datos, 'application/octet-stream')

    def meta(self, clave):
        return {'nota': 'sin cambio sólo de metadatos por esta API'}, 0

    def borrar(self, clave):
        return http('DELETE', self.base + '/' + urllib.parse.quote(clave, safe='/'), self.tok)

    def listar(self):
        r, ms = http('GET', self.base + '?prefix=' + urllib.parse.quote(PREF, safe=''), self.tok)
        res = r.get('result', [])
        if isinstance(res, dict):
            res = res.get('objects', [])
        objs = [(o['key'], o.get('etag'), str(o.get('size')), o.get('last_modified') or o.get('uploaded'), None, None) for o in res]
        if 'error' in r:
            print('   listar:', r)
        return objs, [f'{n}\t{e}\t{s}' for n, e, s, *_ in objs], ms


for O in (Gcs, R2):
    o = O()
    print(f'===== {o.nombre}')
    previo = {}

    def mirar(paso):
        objs, lineas, ms = o.listar()
        w = testigo(lineas)
        x = [ob for ob in objs if ob[0] == PREF + 'x.csv']
        etag, upd = (x[0][1], x[0][3]) if x else (None, None)
        extra = f' md5={x[0][4]} metagen={x[0][5]}' if x and x[0][4] else ''
        print(f'{paso:<48} testigo={w} ({ms} ms, {len(objs)} objetos) · x: etag/gen={etag} last-modified={upd}{extra}')
        if previo:
            print(f'{"":<48} => testigo {"IGUAL" if previo["w"] == w else "CAMBIA"} · last-modified de x {"IGUAL" if previo["u"] == upd else "CAMBIA"}')
        previo.update(w=w, u=upd)

    r, _ = o.poner(PREF + 'x.csv', b'id,v\n1,hola\n'); assert 'error' not in r, r
    time.sleep(1.5); mirar('0 · base (x.csv)')
    time.sleep(1.5); o.poner(PREF + 'x.csv', b'id,v\n1,hola\n'); mirar('1 · reescribir x con el MISMO contenido')
    time.sleep(1.5); o.poner(PREF + 'x.csv', b'id,v\n1,holb\n'); mirar('2 · contenido distinto, MISMO tamaño')
    time.sleep(1.5); r, _ = o.meta(PREF + 'x.csv'); mirar('3 · cambiar sólo metadatos' + (' (n/a)' if 'nota' in r else ''))
    time.sleep(1.5); o.poner('ore-salud-m2-otro/y.csv', b'id\n1\n'); mirar('4 · objeto nuevo FUERA del prefijo')
    time.sleep(1.5); o.poner(PREF + 'z.csv', b'id\n9\n'); mirar('5 · objeto nuevo DENTRO del prefijo')
    time.sleep(1.5); o.borrar(PREF + 'z.csv'); mirar('6 · borrar ese objeto')
    for k in (PREF + 'x.csv', PREF + 'z.csv', 'ore-salud-m2-otro/y.csv'):
        o.borrar(k)
    print('   limpio:', len(o.listar()[0]), 'objetos en el prefijo')
