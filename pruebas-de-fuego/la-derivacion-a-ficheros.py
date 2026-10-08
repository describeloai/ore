"""0049 B9·4 · FICHEROS QUE DAN FICHEROS: el ejecutor de Python de
`conformidad/media/casos/derivar.json`, contra el banco de la media.

Cada caso instala la entrada (`conformidad.default.derivar`: `docs/a.pdf`,
`docs/c.pdf` en `v1` e `img/b.png`, de la muestra), crea una colección escrita
nueva para su salida (`{salida}`) y corre sus pasos con el SDK de verdad
(`collection.apply(fn, output=…)`, `Transaction.delete`, `derivations()`). El
banco hace lo que `ore-medios` al sellar (B9·2, probado aparte en Rust).

    python pruebas-de-fuego/la-derivacion-a-ficheros.py
"""
import base64
import json
import os
import sys

import banco_media as banco

banco.arrancar()
import ore  # noqa: E402

RAIZ = banco.RAIZ
MUESTRA = json.load(open(os.path.join(RAIZ, "conformidad", "media", "muestra.json"), encoding="utf-8"))
CASOS = json.load(open(os.path.join(RAIZ, "conformidad", "media", "casos", "derivar.json"), encoding="utf-8"))


def contenido(item, version=None):
    if "texto" in item:
        return item["texto"].encode()
    if "base64" in item:
        return base64.b64decode(item["base64"])
    v = next(x for x in item["versiones"] if version is None or x["id"] == version)
    return v["texto"].encode()


def instalar():
    """La entrada de `derivar`, como dice la muestra (`solo`)."""
    por_ruta = {i["path"]: i for i in MUESTRA["items"] if "path" in i}
    banco.ENTRADA.clear()
    for s in MUESTRA["colecciones"]["derivar"]["solo"]:
        ruta, _, version = s.partition("@")
        banco.ENTRADA[ruta] = contenido(por_ruta[ruta], version or None)


# ── las funciones con nombre (README · un caso por pasos) ───────────────────

LLAMADAS = []   # las rutas sobre las que se llamó la función, en todo el caso


def lineas_pdf(item):
    LLAMADAS.append(item.ref.path)
    if not item.ref.path.endswith(".pdf"):
        return []
    lineas = item.read_bytes().decode().split("\n")
    if lineas and lineas[-1] == "":
        lineas.pop()
    return [ore.File("l%d.txt" % n, l.encode(), "text/plain", {"kind": "page", "page": n})
            for n, l in enumerate(lineas, 1)]


def nombre_repetido(item):
    LLAMADAS.append(item.ref.path)
    if not item.ref.path.endswith(".pdf"):
        return []
    return [ore.File("x.txt", b"uno"), ore.File("x.txt", b"dos")]


FUNCIONES = {"lineas_pdf": lineas_pdf, "nombre_repetido": nombre_repetido}


class Corte(Exception):
    """Lo que el ejecutor lanza para cortar una pasada (`cortar_tras_guardar`)."""


# ── el vocabulario de `espera` ──────────────────────────────────────────────

def mira(espera, ctx):
    salida = ctx["salida"]
    col = ore.collection(salida)
    st = banco.ESCRITAS[salida]
    actuales = {r["path"]: r for r in st["items"].values()}
    registro = {((e.get("source") or {}).get("uri") or "").split("/", 3)[-1].split("?")[0]: e
                for e in banco.registro_de(st)}
    for k, v in espera.items():
        if k == "resumen":
            r = ctx["resumen"]
            for kk, vv in v.items():
                assert r.get(kk) == vv, "resumen.%s: %r y se esperaba %r (%s)" % (kk, r.get(kk), vv, dict(r))
        elif k == "ficheros":
            assert sorted(actuales) == sorted(v), "ficheros: %s" % sorted(actuales)
        elif k == "ficheros_incluye":
            assert all(c in actuales for c in v), "faltan %s" % [c for c in v if c not in actuales]
        elif k == "no_ficheros":
            assert not any(c in actuales for c in v), "sobran %s" % [c for c in v if c in actuales]
        elif k == "origen":
            for f, o in v.items():
                s = actuales[f]["source"]
                assert ("/" + o + "?") in s["uri"], "%s sale de %s" % (f, s["uri"])
                import hashlib
                assert s["digest"] == "sha256:" + hashlib.sha256(banco.ENTRADA[o]).hexdigest(), s
        elif k == "ancla":
            for f, a in v.items():
                assert actuales[f]["source"]["anchor"] == a, actuales[f]["source"]
        elif k == "derivacion":
            for f, d in v.items():
                for kk, vv in d.items():
                    assert actuales[f]["derivation"][kk] == vv, actuales[f]["derivation"]
        elif k == "marcas":
            for o, estado in v.items():
                e = registro.get(o)
                if estado is None:
                    assert e is None or e["state"] == "files", "%s sigue con su marca: %s" % (o, e)
                else:
                    assert e is not None and e["state"] == estado, "%s: %s" % (o, e)
        elif k == "sin_escribir":
            assert st["tx"] == ctx["tx_antes"], "la salida pasó de la transacción %s a %s" % (ctx["tx_antes"], st["tx"])
        elif k == "bytes_de":
            for f, t in v.items():
                assert banco.LAGO[actuales[f]["digest"][7:]] == t.encode(), f
        elif k == "stat_de":
            for f, e in v.items():
                try:
                    col.stat(f)
                    raise AssertionError("%s se ve con stat" % f)
                except ore.MediaError as x:
                    assert x.type == e["error"], x
        elif k == "despues_no_en_list":
            assert not any(c in {it.ref.path for it in col.items()} for c in v), v
        elif k == "estados":
            assert {o: registro[o]["state"] for o in v} == v, {o: (registro.get(o) or {}).get("state") for o in v}
        elif k == "ficheros_de":
            for o, n in v.items():
                assert len(registro[o]["files"]) == n, registro[o]
        elif k == "status":
            assert ctx.get("status", 200) == v
        elif k == "invariante":
            for i in v:
                assert i == "sin_recalculo"
                otra_vez = set(LLAMADAS[ctx["llamadas_antes"]:]) & ctx["confirmados_antes"]
                assert not otra_vez, "se recalculó lo ya confirmado: %s" % sorted(otra_vez)
        else:
            raise AssertionError("`espera.%s` no es del vocabulario" % k)


def correr(c, n):
    instalar()
    LLAMADAS.clear()
    salida = "conformidad.default.salida_%02d" % n
    ore.create_collection(salida, "document", ["txt"])
    for paso in c["pasos"]:
        st = banco.ESCRITAS[salida]
        ctx = {"salida": salida, "tx_antes": st["tx"], "llamadas_antes": len(LLAMADAS),
               "confirmados_antes": {(e["source"]["uri"].split("/", 3)[-1]).split("?")[0]
                                     for e in banco.registro_de(st)}}
        if "apply" in paso:
            a = dict(paso["apply"])
            fn = FUNCIONES[a.pop("funcion")]
            falla = set(a.pop("falla_en", []))
            cortar = a.pop("cortar_tras_guardar", None)
            if falla:
                base = fn

                def fn(item, base=base):
                    if item.ref.path in falla:
                        LLAMADAS.append(item.ref.path)
                        raise ValueError("falla a propósito en %s" % item.ref.path)
                    return base(item)
                fn.__name__ = base.__name__
            original = ore.Transaction.commit
            if cortar:
                hechos = {"n": 0}

                def commit(self, original=original):
                    r = original(self)
                    hechos["n"] += 1
                    if hechos["n"] >= cortar:
                        raise Corte()
                    return r
                ore.Transaction.commit = commit
            try:
                ctx["resumen"] = ore.collection(banco.DERIVAR).apply(fn, output=ore.collection(salida), threads=1, **a)
            except Corte:
                ctx["resumen"] = {}
            finally:
                ore.Transaction.commit = original
        elif "sobrescribir" in paso:
            banco.ENTRADA[paso["sobrescribir"]["path"]] = paso["sobrescribir"]["texto"].encode()
        elif "borrar" in paso:
            del banco.ENTRADA[paso["borrar"]["path"]]
        elif "copiar" in paso:
            banco.ENTRADA[paso["copiar"]["a"]] = banco.ENTRADA[paso["copiar"]["de"]]
        elif "derivations" in paso:
            list(ore.collection(salida).derivations())
        mira(paso.get("espera") or {}, ctx)


def correr_put(c, n):
    """Los casos de `put` con linaje: el cotejo del `commit`."""
    salida = "conformidad.default.salida_%02d" % n
    ore.create_collection(salida, "document", ["txt"])
    col = ore.collection(salida)
    pide, espera = c["pide"], c["espera"]

    def con_linaje(t, commit):
        for k, v in commit.items():
            t._linaje[k] = json.loads(json.dumps(v).replace("{coleccion}", salida))
    error = None
    try:
        t = col.transaction()
        if "path" in pide:
            t.put(pide["path"], pide["texto"].encode())
        if isinstance(pide.get("commit"), dict):
            con_linaje(t, pide["commit"])
        t.commit()
        if "despues" in pide:
            t2 = col.transaction()
            con_linaje(t2, pide["despues"]["commit"])
            t2.commit()
    except ore.MediaError as e:
        error = e
    if "error" in espera:
        assert error is not None and error.type == espera["error"] and error.status == espera["status"], error
    else:
        assert error is None, error
    for ruta in espera.get("despues_no_en_list", []):
        assert ruta not in {it.ref.path for it in col.items()}, ruta


print("── 0049 B9 · ficheros que dan ficheros: casos/derivar.json contra el banco")
for n, c in enumerate(CASOS, 1):
    f = correr if c["op"] == "apply" else correr_put
    banco.caso(c["id"], lambda c=c, n=n, f=f: (f(c, n), banco.bien("%s · %s" % (c["id"], c["norma"].split("·", 1)[1].strip())))[0])

print("todo bien" if not banco.fallos["n"] else "%d fallos" % banco.fallos["n"])
sys.exit(1 if banco.fallos["n"] else 0)
