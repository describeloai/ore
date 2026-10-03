# 0049 B5·0 · medida: ¿se escribe una tabla anclada (v1alpha17 `03`) desde una celda,
# con `_item`, `_anchor`, `_derivation` y `_status` como structs? ¿cuánto cuesta
# reescribirla entera (que es lo que haría `aplicar()` en cada ejecución)?
# Se pega en una celda de un puesto Python, en una rama que no sea main.
import time, json, hashlib
import pyarrow as pa
import ore

COL = "s3_standard.nueva_carpeta.contratos_sql"
SALIDA = "s3_standard.nueva_carpeta.medida_b5"
SIN_GUION = True

S, F, I = pa.string(), pa.float64(), pa.int64()
ITEM = pa.struct([("uri", S), ("collection", S), ("path", S), ("version", S), ("digest", S),
                  ("size", I), ("content_type", S), ("content_type_detected", S), ("checksum", S)])
BBOX = pa.struct([("x", F), ("y", F), ("w", F), ("h", F)])
ANCLA = pa.struct([("kind", S), ("page", I), ("bbox", BBOX), ("t_start", F), ("t_end", F),
                   ("frame", I), ("char_start", I), ("char_end", I), ("text_of", S),
                   ("offset", I), ("length", I)])
DERIV = pa.struct([("key", S), ("fn", S), ("fn_version", S), ("model", S), ("model_rev", S),
                   ("params_hash", S), ("run", S), ("created", S)])
ESTADO = pa.struct([("state", S), ("error_type", S), ("error_message", S), ("attempts", I)])
ESQUEMA = pa.schema([("_item", ITEM), ("_anchor", ANCLA), ("_anchor_id", S), ("_anchor_parent", S),
                     ("_derivation", DERIV), ("_status", ESTADO), ("texto", S)])

refs = [it.ref for it in ore.coleccion(COL).items()]
print("① listado:", len(refs), "ítems · digest del primero:", refs[0].digest)


def sha(*partes):
    return hashlib.sha256("|".join(str(p) for p in partes).encode()).hexdigest()


def filas(n):
    out = []
    for i in range(n):
        r = refs[i % len(refs)]
        pagina = i // len(refs) + 1
        ident = r.digest or "%s|%s|%s" % (r.collection, r.path, r.version)
        out.append({
            "_item": {k: getattr(r, k, None) for k in ITEM.names},
            "_anchor": {"kind": "page", "page": pagina},
            "_anchor_id": sha(ident, "page", pagina, "medir"),
            "_anchor_parent": None,
            "_derivation": {"key": sha(ident, "medir", "1", None, None), "fn": "medir", "fn_version": "1",
                            "model": None, "model_rev": None, "params_hash": None, "run": "b50",
                            "created": "2026-10-03T00:00:00Z"},
            "_status": {"state": "ok", "error_type": None, "error_message": None, "attempts": 1},
            "texto": "página %d de %s" % (pagina, r.path),
        })
    t = pa.Table.from_pylist(out, schema=ESQUEMA)
    # B5·0, 2.ª pasada: write() aún no sabe de `anchoredTo` y un dataset normal no admite
    # columnas `_…`; los mismos structs sin el `_`, para medir el lago y el coste.
    return t.rename_columns([c.lstrip("_") for c in t.column_names]) if SIN_GUION else t

# ② una tabla anclada pequeña: ¿la acepta write()? ¿qué documento deja?
t = filas(8)
try:
    t0 = time.time(); w = ore.write(SALIDA, t); print("② write 8 filas: %.2f s" % (time.time() - t0), json.dumps({k: w.get(k) for k in ("filas", "operacion", "snapshot")}))
except Exception as e:
    print("② write 8 filas FALLA:", type(e).__name__, str(e)[:600])

# ③ leerla de vuelta: ¿siguen siendo structs?
try:
    v = ore.over(SALIDA, como="arrow"); print("③ leída:", v.num_rows, "filas ·", v.schema)
except Exception as e:
    print("③ leer FALLA:", type(e).__name__, str(e)[:400])

# ④ el coste de reescribirla entera, a 10 000 y 100 000 filas
for n in (10_000, 100_000):
    t = filas(n)
    try:
        t0 = time.time(); ore.write(SALIDA, t); print("④ reescribir %d filas: %.2f s" % (n, time.time() - t0))
    except Exception as e:
        print("④ reescribir %d FALLA:" % n, type(e).__name__, str(e)[:300]); break

# ⑤ y leerla entera (lo que haría aplicar() para saber qué hay hecho)
try:
    t0 = time.time(); v = ore.over(SALIDA, como="arrow"); print("⑤ leer %d filas: %.2f s" % (v.num_rows, time.time() - t0))
except Exception as e:
    print("⑤ leer FALLA:", type(e).__name__, str(e)[:300])
