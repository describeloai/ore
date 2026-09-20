#!/usr/bin/env python3
"""
MEDIDA · W3.6a · ¿escribe iceberg-rust lo que el Job de copia necesita? (20 de septiembre; 0031 §10)

La copia la sella `ore-store` en Rust. Para que la copia sea un dataset Iceberg
(«todo es un dataset») el escritor tiene que ser Rust —o, mientras no lo sea, PyIceberg
en la imagen del Job—. Antes de decidir, lo que hay que saber de `iceberg` 0.10 +
`iceberg-storage-opendal`, medido con un crate desechable
(`pruebas-de-fuego/medida-w3-iceberg-rust/`, fuera del workspace):

  §0  LO QUE PESA         cuánto tarda en compilar, cuántos crates trae y cuánto ocupa el
                          binario (el Job de copia hoy es `ore-store-gcs`, pequeño); y la
                          versión de arrow/parquet que exige (58) frente a la del árbol (56)
  §1  ESCRIBIR            10 M de filas en lotes de 1 M: Parquet + manifiestos + metadata +
                          commit (`fast_append`), ms y bytes; DuckDB las cuenta después
  §2  EVOLUCIONAR         un segundo append (snapshot 2), una columna nueva, expirar el
                          snapshot 1 — y lo que 0.10 NO sabe hacer (promover int → long)
  §3  LOS TIPOS           los diez físicos del contrato (0032 §1) escritos desde Arrow en
                          Rust y leídos por DuckDB: cotejo campo a campo
  §4  EL BUCKET           lo mismo en pequeño contra gs:// (opendal con el token de gcloud,
                          que en el pod sería el del servidor de metadatos); DuckDB lo lee
                          por https con el token

Uso:
  python pruebas-de-fuego/medida-w3-iceberg-rust.py [--filas 10000000] [--bucket gs://<bucket>/<prefijo>] [--sin-compilar]

Hacen falta cargo (con el toolchain del árbol), duckdb con la extensión iceberg, gcloud
para `--bucket`. Nada de pago, nada en el clúster; `--bucket` escribe y borra bajo el
prefijo dado.
"""
import datetime as dt
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CRATE = os.path.join(RAIZ, "pruebas-de-fuego", "medida-w3-iceberg-rust")


def fila(k, v, nota=""):
    print("  %-36s %-44s %s" % (k, v, nota))


def sh(*args, cwd=None, env=None):
    exe = shutil.which(args[0]) or args[0]
    r = subprocess.run((exe,) + tuple(args[1:]), capture_output=True, cwd=cwd, env=env)
    return r.returncode, r.stdout.decode("utf-8", "replace"), r.stderr.decode("utf-8", "replace")


def win(p):
    return p.replace("\\", "/")


def compilar():
    env = dict(os.environ)
    if os.name == "nt" and os.path.isdir("C:/msys64/mingw64/bin"):
        env["PATH"] = "C:/msys64/mingw64/bin;" + env["PATH"]
    t0 = time.time()
    c, out, err = sh("cargo", "build", "--release", cwd=CRATE, env=env)
    if c != 0:
        print(err[-2000:])
        raise SystemExit("no compila")
    ms_ = int((time.time() - t0) * 1000)
    c, out, _ = sh("cargo", "tree", "--edges", "normal", "--prefix", "none", cwd=CRATE, env=env)
    crates = len(set(l.split()[0] for l in out.splitlines() if l.strip()))
    c, out, _ = sh("cargo", "tree", "--edges", "normal", "--prefix", "none", "-i", "arrow-array", cwd=CRATE, env=env)
    arrow = next((l.split()[1] for l in out.splitlines() if l.startswith("arrow-array ")), "?")
    exe = os.path.join(CRATE, "target", "release", "medida-w3-iceberg-rust" + (".exe" if os.name == "nt" else ""))
    return exe, ms_, crates, arrow, os.path.getsize(exe)


def correr(exe, bodega, punteros, filas, env=None):
    c, out, err = sh(exe, "--bodega", bodega, "--punteros", punteros, "--filas", str(filas), env=env)
    hechos = [json.loads(l[4:]) for l in out.splitlines() if l.startswith("### ")]
    if c != 0:
        fila("  el programa", "✗ salió con %d" % c, (err.strip().splitlines() or [""])[-1][:120])
    return {h["que"]: h for h in hechos}


def scan(meta, base=None):
    """El fragmento `iceberg_scan` por la raíz y la versión, como el SDK (0031 §10)."""
    raiz, fichero = meta.rsplit("/metadata/", 1)
    if base is not None:
        raiz = base + raiz[raiz.index("/", 5):] if raiz.startswith("gs://") else raiz
    for pre in ("file:///", "file://"):
        if raiz.startswith(pre):
            raiz = raiz[len(pre):]
    return "iceberg_scan('%s', version='%s', allow_moved_paths=true)" % (raiz.replace("'", "''"), fichero.replace(".metadata.json", ""))


VERDAD = {
    "entero": ["9007199254740993", None, "-1"],
    "real": ["1.5", None, "nan"],
    "logico": ["true", None, "false"],
    "texto": ["ñandú 🐍", None, ""],
    "decimal": ["12345678901.234567890123456789", None, "-0.000000000000000001"],
    "dinero": ["1.50", None, "-0.05"],
    "fecha": ["2026-09-19", None, "1969-12-31"],
    "hora": ["14:30:00.123456", None, "00:00:00"],
    "fecha_hora": ["2026-09-19 14:30:00.123456", None, "1969-12-31 23:59:59"],
    "instante": ["2026-09-19 14:30:00.123456+00", None, "1969-12-31 23:59:59+00"],
}


def cotejar_tipos(con, meta, base=None):
    con.execute("set TimeZone = 'UTC'")
    tipos = [str(t) for t in con.sql("select * from %s limit 0" % scan(meta, base)).types]
    r = con.execute("select columns(*)::varchar from %s order by 1 desc nulls last" % scan(meta, base))
    cols = [d[0] for d in r.description]
    filas_ = r.fetchall()
    # el orden por `entero` desc nulls last: 9007…, -1, null → la verdad va [0, 2, 1]
    malas = []
    for i, c in enumerate(cols):
        vals = [f[i] for f in filas_]
        esperado = [VERDAD[c][0], VERDAD[c][2], VERDAD[c][1]]
        if vals != esperado:
            malas.append("%s: %s ≠ %s" % (c, vals, esperado))
    print("  %-12s %-22s %s" % ("columna", "duckdb", "✓/≠"))
    for i, c in enumerate(cols):
        print("  %-12s %-22s %s" % (c, tipos[i][:22], "✓" if not any(m.startswith(c + ":") for m in malas) else next(m for m in malas if m.startswith(c + ":"))[:80]))
    return len(cols) - len(malas), len(cols)


def main():
    filas = int(sys.argv[sys.argv.index("--filas") + 1]) if "--filas" in sys.argv else 10_000_000
    bucket = sys.argv[sys.argv.index("--bucket") + 1] if "--bucket" in sys.argv else None
    print("MEDIDA · W3.6a · iceberg-rust como escritor · %s" % dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%d %H:%MZ"))
    import duckdb

    con = duckdb.connect()
    con.execute("install iceberg; load iceberg; install httpfs; load httpfs")
    print()
    print("§0 · lo que pesa")
    if "--sin-compilar" in sys.argv:
        exe = os.path.join(CRATE, "target", "release", "medida-w3-iceberg-rust" + (".exe" if os.name == "nt" else ""))
        fila("binario", "%.1f MB" % (os.path.getsize(exe) / 1e6), "(sin compilar: lo que había)")
    else:
        exe, ms_, crates, arrow, tam = compilar()
        fila("cargo build --release", "%d ms (incremental si ya estaba)" % ms_, "iceberg 0.10.1 + iceberg-storage-opendal (fs, gcs)")
        fila("crates en el cierre", str(crates), "arrow-array %s (el árbol lleva 56: hay que subir ore-store)" % arrow)
        fila("binario", "%.1f MB" % (tam / 1e6), "ore-store-gcs de hoy: ~10 MB")

    tmp = tempfile.mkdtemp(prefix="ore-ice-rust-")
    tmp = win(tmp)
    try:
        os.makedirs(tmp + "/bodega")
        os.makedirs(tmp + "/punteros")
        print()
        print("§1 · escribir %d filas en local (lotes de 1 M)" % filas)
        h = correr(exe, "file:///" + tmp + "/bodega", tmp + "/punteros", filas)
        g = h.get("grande", {})
        if g:
            fila("crear la tabla", "%s ms" % g["crear_ms"])
            fila("escribir Parquet (%d fichero(s))" % g["ficheros"], "%s ms · %.1f MB" % (g["escribir_ms"], g["bytes"] / 1e6), "snappy; %.1f M filas/s" % (filas / max(g["escribir_ms"], 1) / 1000))
            fila("commit (manifiesto + lista + metadata)", "%s ms" % g["commit_ms"])
            t0 = time.time()
            n = con.execute("select count(*) from %s" % scan(g["metadata_location"])).fetchone()[0]
            fila("DuckDB cuenta el snapshot 1", "%d filas · %d ms" % (n, (time.time() - t0) * 1000), "✓" if n == filas else "✗ esperaba %d" % filas)
        print()
        print("§2 · evolucionar")
        a = h.get("append", {})
        if a:
            fila("append de 1 M (snapshot 2)", "escribir %s ms · commit %s ms" % (a["escribir_ms"], a["commit_ms"]), "%d snapshots · sin reescribir los %d M" % (a["snapshots"], filas // 1_000_000))
            n = con.execute("select count(*) from %s" % scan(a["metadata_location"])).fetchone()[0]
            fila("  DuckDB cuenta", "%d filas" % n, "✓" if n == filas + 1_000_000 else "✗")
        e = h.get("esquema", {})
        fila("columna nueva (add_column)", ("✓ %s ms" % e.get("ms")) if e.get("ok") else "✗", ", ".join(e.get("columnas", [])) or e.get("porque", "")[:100])
        fila("promover int → long (update_column)", "✗ no existe en 0.10", "sólo add/delete/rename: la promoción, por PyIceberg o a mano en el metadata")
        x = h.get("expirar", {})
        fila("expirar snapshots (retain_last 1)", ("✓ %s ms · quedan %s" % (x.get("ms"), x.get("snapshots"))) if x.get("ok") else "✗", x.get("porque", "")[:100] if not x.get("ok") else ("el snapshot 1 sigue: 0.10 sólo expira los más viejos que `expire_older_than_ms`; el retain_last solo no basta" if x.get("snapshots", 0) > 1 else "el snapshot 1 fuera"))
        if e.get("ok"):
            n = con.execute("select count(*) from %s where canal is null" % scan(e["metadata_location"])).fetchone()[0]
            fila("  DuckDB lee la columna nueva", "%d filas con canal = null" % n, "✓" if n == filas + 1_000_000 else "✗")
        print()
        print("§3 · los diez físicos del contrato, escritos desde Arrow en Rust y leídos por DuckDB")
        t = h.get("tipos", {})
        if t.get("ok"):
            fila("Iceberg dice", ", ".join(t["iceberg"])[:44], "instante: Arrow lo quiere tz=\"+00:00\", no \"UTC\" (misma física; el escritor lo casa)")
            ok, total = cotejar_tipos(con, t["metadata_location"])
            fila("cotejo", "%d/%d ✓" % (ok, total))
        else:
            fila("tipos", "✗", t.get("porque", "")[:140])
        with open(tmp + "/punteros/grande.json", encoding="utf-8") as f:
            fila("el puntero que dejaría materialize", json.load(f)["metadata_location"].rsplit("/", 1)[1][:44], "`copias/<p>_<v>.json` → metadata_location + snapshot")

        if bucket:
            print()
            print("§4 · en el bucket: %s" % bucket)
            tok = subprocess.run(["gcloud", "auth", "print-access-token"], capture_output=True, shell=(os.name == "nt")).stdout.decode().strip()
            if not tok:
                raise SystemExit("gcloud auth print-access-token no dio nada")
            import uuid

            prefijo = bucket.rstrip("/") + "/medida-" + uuid.uuid4().hex[:8]
            env = dict(os.environ, GCS_TOKEN=tok)
            t0 = time.time()
            hb = correr(exe, prefijo, tmp + "/punteros-gcs", 1_000_000, env=env)
            total_ms = int((time.time() - t0) * 1000)
            g = hb.get("grande", {})
            if g:
                fila("1 M filas · escribir + commit", "%s + %s ms" % (g["escribir_ms"], g["commit_ms"]), "%d fichero(s) · %.1f MB; todo el programa %d ms" % (g["ficheros"], g["bytes"] / 1e6, total_ms))
                a = hb.get("append", {})
                fila("append de 1 M", "%s + %s ms" % (a.get("escribir_ms"), a.get("commit_ms")), "%s snapshots" % a.get("snapshots"))
                con.execute("create or replace secret gcs_http (type http, bearer_token '%s')" % tok.replace("'", "''"))
                base = "https://storage.googleapis.com/" + bucket[5:].split("/", 1)[0]
                t0 = time.time()
                n = con.execute("select count(*) from %s" % scan(a["metadata_location"], base)).fetchone()[0]
                fila("DuckDB lee de gs:// por https", "%d filas · %d ms" % (n, (time.time() - t0) * 1000), "✓" if n == 2_000_000 else "✗")
                tt = hb.get("tipos", {})
                if tt.get("ok"):
                    ok, total = cotejar_tipos(con, tt["metadata_location"], base)
                    fila("los tipos desde el bucket", "%d/%d ✓" % (ok, total))
            c, out, err = sh("gcloud", "storage", "rm", "-r", prefijo, "--quiet")
            fila("limpieza del bucket", "prefijo %s" % ("borrado" if c == 0 else "NO borrado: " + err[-80:]))
    finally:
        con.close()
        shutil.rmtree(tmp, ignore_errors=True)
        fila("limpieza", "el temporal fuera", "(target/ del crate queda para la próxima)")


if __name__ == "__main__":
    main()
