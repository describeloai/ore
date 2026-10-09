"""EL BANCO DE LA CONFORMIDAD (0049 JM1): la muestra de `conformidad/media`,
servida como la serviría la celda, para cualquier ejecutor de la suite.

Sobre `banco_media.py` (sus dos servidores, por sus ganchos), dos colecciones:

- `conformidad.default.mantenida`: la versión de cada ítem es su contenido
  (`m-<sha12>`), y el `digest` se conoce siempre;
- `conformidad.default.virtual`: un origen versionado que el ejecutor controla;
  la versión la da el origen (`s3-<n>`), y el `digest` sólo se sabe después de
  una lectura entera (`open-007`).

Cada ingesta es una transacción (`as_of`). La muestra entra en dos: la primera
con `docs/c.pdf` en `v1`, la segunda con `v2`. Los bytes, por `/contenido` de
`ore-medios` con un permiso, con sus cabeceras (`ETag`, `ORE-Media-Version`,
`Repr-Digest` si se conoce) y `Range`; los errores, `problem+json`.

Lo que el ejecutor hace a la muestra va por `POST /_banco/<orden>` en la celda:
`instalar`, `version`, `as_of`, `sobrescribir`, `borrar_version`, `credencial`,
`token`, `lento`, `enviados`; y, para las pruebas del SDK sobre la colección de
siempre (`legal.archivo.contratos`), `registro` (lo que llegó a cada servidor,
sin el mando), `contados` y `rama`. No es parte de ningún contrato: es el mando
del banco.

⚠️ Lo que es del SERVIDOR —que `url` recorte el ttl, que un 403 no diga si el
ítem existe, el sha256 visto al paso, el tipo por los bytes— aquí lo hace el
banco: el ejecutor lo comprueba, pero el veredicto de verdad es el de prod.
"""
import base64
import hashlib
import json
import os
import threading
import time
import urllib.parse

import banco_media as banco

#: Cuánto de verdad es un segundo de un caso (`credencial_dura_s`, `lento`,
#: `esperar_s`): un caso de 90 s en el banco dura 4,5.
ESCALA = float(os.environ.get("JM_ESCALA", "0.05"))
COLECCIONES = {"conformidad.default.mantenida": "mantenida", "conformidad.default.virtual": "virtual"}
LIMITE = 1000
TTL = (30, 300, 3600)   # mínimo, por defecto, máximo: los de `ore-medios`

_cerrojo = threading.RLock()
ESTADO = {}
#: JM2 · Lo que `ore-serve` fija al declarar un transform (0049 B4·2): la
#: transacción de cada colección de sus `inputs`, por su nombre corto. Fuera de
#: `ESTADO`: un caso instala la muestra DENTRO del transform que declaró.
FIJADAS = {}
#: Las que dio la última declaración, aunque el transform ya se retirara: lo que
#: el servidor escribiría en el linaje de lo que ese transform confirmó.
DECLARADAS = {}
#: JM4b · Listados que el ejecutor pone tal cual (colección → [MediaRef]): las
#: pruebas de `apply()` en filas cambian el listado entre pasadas, como Python.
LISTADOS = {}


def _corto(nombre):
    p = nombre.split(".")
    return "%s.%s" % (p[0], p[2]) if len(p) == 3 and p[1] == "default" else nombre


def _nuevo():
    return {"txs": {}, "ultima": 0, "versiones": {}, "borradas": set(), "vistos": set(), "n": 0}


def instalar():
    """La muestra, desde cero, en las dos colecciones."""
    m = json.load(open(os.path.join(banco.RAIZ, "conformidad", "media", "muestra.json"), encoding="utf-8"))
    with _cerrojo:
        ESTADO.clear()
        ESTADO.update({"cols": {c: _nuevo() for c in COLECCIONES}, "permisos": {}, "ids": {},
                       "credencial": {"modo": None, "dura_s": None}, "tokens": {}, "lentos": {}, "enviados": {}})
        primera, segunda = {}, {}
        for it in m["items"]:
            if "generar" in it:
                g = it["generar"]
                for i in range(g["n"]):
                    camino = g["prefijo"] + g["nombre"].replace("{i:04}", "%04d" % i)
                    primera[camino] = (g["texto"].replace("{i}", str(i)).encode(), g["content_type"], None)
            elif "versiones" in it:
                v1, v2 = it["versiones"][0], it["versiones"][-1]
                primera[it["path"]] = (v1["texto"].encode(), it["content_type"], v1["id"])
                segunda[it["path"]] = (v2["texto"].encode(), it["content_type"], v2["id"])
            else:
                datos = it["texto"].encode() if "texto" in it else base64.b64decode(it["base64"])
                primera[it["path"]] = (datos, it["content_type"], None)
        for c in COLECCIONES:
            _ingerir(c, primera)
            _ingerir(c, segunda)
        # JM3 · la escrita de la muestra, vacía: la sirve el índice de escritas
        # del banco de la media (`ESCRITAS`), como una que `apply()` llena.
        banco.ESCRITAS["conformidad.default.escrita"] = {"tx": 0, "items": {}, "marcas": {}}


def _version(col, datos):
    st = ESTADO["cols"][col]
    if COLECCIONES[col] == "mantenida":
        return "m-" + hashlib.sha256(datos).hexdigest()[:12]
    st["n"] += 1
    return "s3-%d" % st["n"]


def _ingerir(col, cambios):
    """Una transacción nueva: la anterior con `cambios` (camino → (bytes, tipo, id))."""
    st = ESTADO["cols"][col]
    previa = st["txs"].get(st["ultima"], {})
    nueva = dict(previa)
    for camino, (datos, tipo, ident) in cambios.items():
        v = _version(col, datos)
        st["versiones"][(camino, v)] = (datos, tipo)
        if ident:
            ESTADO["ids"][(col, camino, ident)] = v
        nueva[camino] = v
    st["ultima"] += 1
    st["txs"][st["ultima"]] = nueva


def _sha(b):
    return hashlib.sha256(b).hexdigest()


def _ref(col, camino, v, tx):
    datos, tipo = ESTADO["cols"][col]["versiones"][(camino, v)]
    sha = _sha(datos)
    mantenida = COLECCIONES[col] == "mantenida"
    visto = (camino, v) in ESTADO["cols"][col]["vistos"]
    return {"uri": "ore://%s/%s?v=%s" % (col, camino, v), "collection": col, "path": camino, "version": v,
            "digest": "sha256:" + sha if (mantenida or visto) else None, "size": len(datos),
            "content_type": tipo, "content_type_detected": banco.tipo_por_bytes(datos[:512]),
            "checksum": "etag:" + sha[:16], "modified": "2026-10-01T00:00:00Z", "state": "actual",
            "transaction": str(tx)}


def _problema(h, codigo, tipo, detalle):
    b = json.dumps({"type": tipo, "status": codigo, "title": tipo, "detail": detalle}).encode()
    h.send_response(codigo)
    h.send_header("content-type", "application/problem+json")
    h.send_header("content-length", str(len(b)))
    h.end_headers()
    h.wfile.write(b)
    return True


def _json(h, codigo, cuerpo, extra=None):
    banco._json(h, codigo, cuerpo, extra)
    return True


def _credencial_mal(h):
    """La de la celda (el token de ORE), si el ejecutor la puso a caducar."""
    cred = ESTADO["credencial"]
    if cred["modo"] == "sin-concesion":
        return _problema(h, 403, "media/sin-permiso", "no hay ninguna concesión sobre la colección")
    if cred["dura_s"] is None:
        return False
    t = (h.headers.get("authorization") or "").removeprefix("Bearer ").strip()
    caduca = ESTADO["tokens"].get(t)
    if caduca is None or time.time() > caduca:
        return _problema(h, 401, "media/credencial", "el token falta o caducó")
    return False


def _transform(h, metodo):
    """Declarar (y retirar) un transform, como `ore-serve`: lo anota en
    `TRANSFORMS` (las pruebas de siempre lo miran) y fija las colecciones de la
    muestra que estén en sus `inputs`."""
    if metodo == "DELETE":
        banco.TRANSFORMS.append(("DELETE", None))
        FIJADAS.clear()
        return _json(h, 200, {"transform": False})
    c = banco._cuerpo(h)
    banco.TRANSFORMS.append(("POST", c))
    FIJADAS.clear()
    with _cerrojo:
        for col in COLECCIONES:
            if _corto(col) in (c.get("inputs") or []) and col in ESTADO.get("cols", {}):
                FIJADAS[_corto(col)] = str(ESTADO["cols"][col]["ultima"])
    DECLARADAS.clear()
    DECLARADAS.update(FIJADAS)
    return _json(h, 200, {"transform": c.get("nombre"), "inputs": c.get("inputs"), "output": c.get("output"),
                          "fijadas": dict(FIJADAS)})


def celda(h, metodo, servidor):
    u = urllib.parse.urlparse(h.path)
    if servidor == "celda" and metodo == "POST" and u.path.startswith("/_banco/"):
        return _mando(h, u.path[len("/_banco/"):], banco._cuerpo(h))
    if servidor == "celda" and metodo in ("POST", "DELETE") and u.path == "/puestos/p1/transform":
        return _transform(h, metodo)
    p = u.path.split("/")
    if servidor != "celda" or len(p) < 6 or p[1] != "media":
        return False
    col = ".".join(p[2:5])
    if col in LISTADOS and metodo == "GET" and p[5] == "items":
        return _json(h, 200, {"as_of": "1", "items": LISTADOS[col], "cursor": None})
    if col not in COLECCIONES:
        return False
    # Con keep-alive el cuerpo se lee siempre, conteste lo que conteste: lo que
    # quedara sin leer sería el principio de la petición siguiente.
    cuerpo = banco._cuerpo(h) if metodo == "POST" else {}
    if _credencial_mal(h):
        return True
    q = dict(urllib.parse.parse_qsl(u.query))
    with _cerrojo:
        st = ESTADO["cols"][col]
        op = p[5]
        if metodo == "GET" and op == "items":
            return _items(h, col, st, q)
        if metodo == "GET" and op == "item":
            return _item(h, col, st, q)
        if metodo == "GET" and op == "content":
            return _contenido(h, col, st, q)
        if metodo == "POST" and op == "urls":
            return _urls(h, col, st, cuerpo)
        if metodo == "POST" and op == "transactions":
            return _problema(h, 409, "media/no-escribible", "`%s` se mantiene desde su `from`" % col)
    return _problema(h, 404, "media/no-existe", "%s %s no es una ruta" % (metodo, u.path))


def _items(h, col, st, q):
    # Dentro de un transform, la transacción fijada al declararlo (B4·2).
    tx = int(FIJADAS.get(_corto(col), st["ultima"]))
    cursor = q.get("cursor")
    desde = 0
    if cursor:
        # El cursor lleva su transacción: un recorrido es de una sola.
        tx_s, _, d = cursor.partition(":")
        tx, desde = int(tx_s), int(d)
    elif q.get("as_of"):
        if not q["as_of"].isdigit() or int(q["as_of"]) not in st["txs"]:
            return _problema(h, 404, "media/no-existe", "la colección no tuvo la transacción %s" % q["as_of"])
        tx = int(q["as_of"])
    limite = max(1, min(LIMITE, int(q.get("limit") or LIMITE)))
    caminos = sorted(c for c in st["txs"][tx] if c.startswith(q.get("prefix") or ""))
    pagina = caminos[desde:desde + limite]
    siguiente = "%d:%d" % (tx, desde + limite) if desde + limite < len(caminos) else None
    return _json(h, 200, {"as_of": str(tx), "items": [_ref(col, c, st["txs"][tx][c], tx) for c in pagina],
                          "cursor": siguiente})


def _buscar(col, st, q):
    camino, v = q.get("path"), q.get("version")
    if not camino:
        return None
    if v:
        return (camino, v) if (camino, v) in st["versiones"] else None
    v = st["txs"][st["ultima"]].get(camino)
    return (camino, v) if v else None


def _item(h, col, st, q):
    hallado = _buscar(col, st, q)
    if not hallado:
        return _problema(h, 404, "media/no-existe", "`%s` no tiene ese ítem" % col)
    camino, v = hallado
    r = _ref(col, camino, v, st["ultima"])
    r["current"] = st["txs"][st["ultima"]].get(camino) == v
    return _json(h, 200, r)


def _permiso(col, camino, v, ttl):
    ESTADO["n_permiso"] = ESTADO.get("n_permiso", 0) + 1
    k = "k%d" % ESTADO["n_permiso"]
    dura = ESTADO["credencial"]["dura_s"]
    ESTADO["permisos"][k] = {"col": col, "camino": camino, "v": v,
                             "caduca": time.time() + (dura * ESCALA if dura else ttl)}
    return k


def _contenido(h, col, st, q):
    hallado = _buscar(col, st, q)
    if not hallado:
        return _problema(h, 404, "media/no-existe", "`%s` no tiene ese ítem" % col)
    camino, v = hallado
    k = _permiso(col, camino, v, TTL[1])
    url = "%s/contenido?permiso=%s" % (banco._medios(), k)
    cuerpo = {"url": url, "desde": "lago" if COLECCIONES[col] == "mantenida" else "medios", "version": v,
              "ttl_s": TTL[1], "expires_ms": int((time.time() + TTL[1]) * 1000),
              "item": _ref(col, camino, v, st["ultima"])}
    return _json(h, 307, cuerpo, {"location": url})


def _urls(h, col, st, cuerpo):
    pedidos = cuerpo.get("items") or []
    if not pedidos or len(pedidos) > LIMITE:
        return _problema(h, 413, "media/limite", "de 1 a %d ítems por petición" % LIMITE)
    ttl = max(TTL[0], min(TTL[2], int(cuerpo.get("ttl_s") or TTL[1])))
    salida = []
    for pd in pedidos:
        hallado = _buscar(col, st, pd)
        if not hallado:
            salida.append({"error": {"type": "media/no-existe", "status": 404,
                                     "detail": "`%s` no tiene ese ítem" % col}})
            continue
        camino, v = hallado
        k = _permiso(col, camino, v, ttl)
        salida.append({"item": _ref(col, camino, v, st["ultima"]),
                       "url": "%s/contenido?permiso=%s" % (banco._medios(), k),
                       "expires_ms": int((time.time() + ttl) * 1000), "ttl_s": ttl})
    return _json(h, 200, {"urls": salida})


def medios(h, metodo, servidor):
    u = urllib.parse.urlparse(h.path)
    q = dict(urllib.parse.parse_qsl(u.query))
    k = q.get("permiso") or ""
    if servidor != "medios" or u.path != "/contenido" or not k.startswith("k"):
        return False
    with _cerrojo:
        pm = ESTADO["permisos"].get(k)
        if pm is None or time.time() > pm["caduca"]:
            return _problema(h, 401, "media/permiso", "el permiso no vale o caducó")
        col, camino, v = pm["col"], pm["camino"], pm["v"]
        st = ESTADO["cols"][col]
        if (camino, v) in st["borradas"]:
            return _problema(h, 412, "media/cambiado", "la versión %s ya no se puede leer" % v)
        datos, tipo = st["versiones"][(camino, v)]
        lento = ESTADO["lentos"].pop((col, camino), None)
    sha = _sha(datos)
    rango = h.headers.get("Range")
    a, z, codigo = 0, len(datos) - 1, 200
    if rango:
        x, y = rango.split("=", 1)[1].split("-")
        a = int(x)
        z = min(int(y), len(datos) - 1) if y else len(datos) - 1
        if a >= len(datos):
            return _problema(h, 416, "media/rango", "bytes=%s fuera de %d" % (rango, len(datos)))
        codigo = 206
    trozo = datos[a:z + 1]
    h.send_response(codigo)
    h.send_header("content-type", tipo)
    h.send_header("content-length", str(len(trozo)))
    h.send_header("etag", '"%s"' % sha)
    h.send_header("ore-media-version", v)
    h.send_header("accept-ranges", "bytes")
    if COLECCIONES[col] == "mantenida":
        h.send_header("repr-digest", "sha-256=:%s:" % base64.b64encode(hashlib.sha256(datos).digest()).decode())
    if codigo == 206:
        h.send_header("content-range", "bytes %d-%d/%d" % (a, z, len(datos)))
    h.end_headers()
    paso = 1 if lento else 16384
    pausa = (lento * ESCALA / max(1, len(trozo))) if lento else 0
    enviados = 0
    try:
        for i in range(0, len(trozo), paso):
            if lento:
                time.sleep(pausa)
                if (camino, v) in st["borradas"]:
                    # La versión que se leía ya no está: se corta, nunca se mezcla.
                    h.close_connection = True
                    break
            h.wfile.write(trozo[i:i + paso])
            h.wfile.flush()
            enviados += len(trozo[i:i + paso])
    except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
        pass
    with _cerrojo:
        ESTADO["enviados"][(col, camino)] = enviados
        # Lo que hace `ore-medios` (B3·1): el sha256 de una lectura entera, visto.
        if codigo == 200 and enviados == len(datos):
            st["vistos"].add((camino, v))
    return True


def _registro(lista):
    return [[m, r, {k.lower(): v for k, v in hs.items()}] for m, r, hs in lista if not r.startswith("/_banco/")]


def _mando(h, orden, c):
    with _cerrojo:
        if orden == "registro":
            r = {"serve": _registro(banco.SERVE), "bytes": _registro(banco.BYTES)}
            if c.get("limpiar"):
                banco.SERVE.clear()
                banco.BYTES.clear()
            return _json(h, 200, r)
        if orden == "contados":
            if "enviados" in c:
                banco.CONTADOS["enviados"] = c["enviados"]
            return _json(h, 200, dict(banco.CONTADOS))
        if orden == "transforms":
            return _json(h, 200, {"transforms": [[m, c2] for m, c2 in banco.TRANSFORMS], "fijadas": dict(DECLARADAS)})
        if orden == "punteros":
            return _json(h, 200, {"punteros": dict(banco.PUNTEROS)})
        if orden == "lago":
            d = banco.LAGO.get(c["sha256"])
            return _json(h, 200, {"esta": d is not None, "size": None if d is None else len(d)})
        if orden == "modos":
            banco.MODOS.update({k: v for k, v in c.items() if k in banco.MODOS})
            return _json(h, 200, dict(banco.MODOS))
        if orden == "documento":
            y = banco.DOCUMENTOS.get((c["kind"], c["base"], c.get("schema") or "default", c["nombre"]))
            return _json(h, 200, {"yaml": y})
        # JM4 · la entrada de `derivar` (B9) y lo que `apply()` escribió en una salida.
        if orden == "derivar":
            accion = c.get("accion")
            if accion == "instalar":
                m = json.load(open(os.path.join(banco.RAIZ, "conformidad", "media", "muestra.json"), encoding="utf-8"))
                por_ruta = {i["path"]: i for i in m["items"] if "path" in i}
                banco.ENTRADA.clear()
                for x in m["colecciones"]["derivar"]["solo"]:
                    ruta, _, version = x.partition("@")
                    it = por_ruta[ruta]
                    if "texto" in it:
                        banco.ENTRADA[ruta] = it["texto"].encode()
                    elif "base64" in it:
                        banco.ENTRADA[ruta] = base64.b64decode(it["base64"])
                    else:
                        banco.ENTRADA[ruta] = next(v for v in it["versiones"] if not version or v["id"] == version)["texto"].encode()
            elif accion == "sobrescribir":
                banco.ENTRADA[c["path"]] = c["texto"].encode()
            elif accion == "borrar":
                del banco.ENTRADA[c["path"]]
            elif accion == "copiar":
                banco.ENTRADA[c["a"]] = banco.ENTRADA[c["de"]]
            return _json(h, 200, {"entrada": {k: _sha(v) for k, v in banco.ENTRADA.items()}})
        if orden == "escrita":
            st = banco.ESCRITAS.get(c["coleccion"])
            if st is None:
                return _json(h, 404, {"error": "no es una escrita"})
            return _json(h, 200, {"tx": st["tx"], "items": st["items"], "registro": banco.registro_de(st),
                                  "entrada": {k: _sha(v) for k, v in banco.ENTRADA.items()}})
        if orden == "lago_bytes":
            d = banco.LAGO.get(c["sha256"])
            return _json(h, 200, {"base64": None if d is None else base64.b64encode(d).decode()})
        if orden == "listado":
            LISTADOS[c["coleccion"]] = c.get("items") or []
            return _json(h, 200, {"items": len(LISTADOS[c["coleccion"]])})
        if orden == "objeto":
            return _json(h, 200, {"base64": base64.b64encode(banco.OBJETOS[c["path"]]).decode()})
        if orden == "rama":
            banco.RAMA["r"] = c.get("rama")
            return _json(h, 200, {"rama": banco.RAMA["r"]})
        if orden == "instalar":
            instalar()
            return _json(h, 200, {"colecciones": sorted(COLECCIONES)})
        col = c.get("coleccion")
        st = ESTADO["cols"].get(col) if col else None
        if orden == "version":
            v = ESTADO["ids"].get((col, c["path"], c["id"]))
            return _json(h, 200 if v else 404, {"version": v})
        if orden == "as_of":
            return _json(h, 200, {"as_of": str(st["ultima"]), "version": st["txs"][st["ultima"]].get(c.get("path"))})
        if orden == "sobrescribir":
            datos = c["texto"].encode()
            camino = c["path"]
            tipo = st["versiones"][(camino, st["txs"][st["ultima"]][camino])][1]
            if c.get("ingerir", True):
                _ingerir(col, {camino: (datos, tipo, None)})
            return _json(h, 200, {"as_of": str(st["ultima"]), "version": st["txs"][st["ultima"]][camino]})
        if orden == "borrar_version":
            st["borradas"].add((c["path"], c["version"]))
            return _json(h, 200, {})
        if orden == "credencial":
            ESTADO["credencial"] = {"modo": c.get("modo"), "dura_s": c.get("dura_s")}
            return _json(h, 200, ESTADO["credencial"])
        if orden == "token":
            dura = (ESTADO["credencial"]["dura_s"] or 300) * ESCALA
            t = "t%d" % (len(ESTADO["tokens"]) + 1)
            ESTADO["tokens"][t] = time.time() + dura
            return _json(h, 200, {"token": t, "expires_in": dura})
        if orden == "lento":
            ESTADO["lentos"][(col, c["path"])] = c.get("total_s") or 10
            return _json(h, 200, {})
        if orden == "enviados":
            return _json(h, 200, {"enviados": ESTADO["enviados"].get((col, c["path"]))})
    return _problema(h, 404, "media/no-existe", "el banco no sabe `%s`" % orden)


def montar():
    """Los ganchos, y la muestra instalada."""
    instalar()
    banco.GANCHOS.extend([celda, medios])
