# 0049 B10·5 · Lo que dejaron `paginas.sql` y `copias.sql`: sólo lee.
#
# Con Run en cualquier repositorio de Python de victor (Functions vale), después
# de cada Build. Dice, de cada colección: sus ficheros por contrato, su registro
# (una entrada por contrato: files, empty o error), el linaje de un fichero, y
# que los bytes son lo que tienen que ser.
import hashlib

import ore

ORIGEN = "s3_stuff.nueva_carpeta.contratos"
PNG = b"\x89PNG\r\n\x1a\n"


def ver(nombre, comprobar):
    print("\n── %s" % nombre)
    try:
        items = list(ore.collection(nombre).items())
    except ore.MediaNotFound:
        print("  todavía no existe: ¿se construyó?")
        return
    por_origen = {}
    for it in items:
        por_origen.setdefault(it.ref.path.split("/")[0], []).append(it)
    for c in sorted(por_origen):
        print("  %-32s %d fichero(s)" % (c, len(por_origen[c])))
    for d in ore.collection(nombre).derivations():
        uri = d["source"]["uri"]
        print("  registro · %-40s %-6s %d%s" % (uri.split("/")[-1].split("?")[0][-40:], d["state"],
                                                len(d.get("files") or []),
                                                (" · " + d["error"]["message"][:80]) if d.get("error") else ""))
    if items:
        uno = items[0]
        print("  un fichero: %s (%s, %s B)" % (uno.ref.path, uno.ref.content_type, uno.ref.size))
        print("    source:     %s" % uno.ref.source)
        print("    derivation: %s" % {k: (uno.ref.derivation or {}).get(k) for k in ("fn", "fn_version", "key")})
        comprobar(uno)


def es_png(it):
    b = it.read_bytes()
    print("    %s" % ("✓ es un PNG" if b[:8] == PNG else "✗ no es un PNG: %r" % b[:8]))


def es_copia(it):
    origen = it.ref.path.rsplit("/", 1)[0]
    a = hashlib.sha256(it.read_bytes()).hexdigest()
    b = hashlib.sha256(ore.collection(ORIGEN).stat(path=origen).read_bytes()).hexdigest()
    print("    %s" % ("✓ los mismos bytes que %s" % origen if a == b else "✗ no son los bytes de %s" % origen))


ver("sandbox.default.b105_paginas", es_png)
ver("sandbox.default.b105_copias", es_copia)
