#!/usr/bin/env python3
"""
MEDIDA · W3.6 · Iceberg como formato de dataset del bucket, con el árbol (git) de catálogo
(20 de septiembre; 0032 «Lo que se aparca hasta W3.6»)

La pregunta: ¿encaja Iceberg como ESTÁNDAR DE DATASET en el bucket del inquilino, con
nuestro git encima haciendo lo que hace un catálogo (Polaris/Nessie/BigLake): guardar la
referencia vigente de cada tabla y cambiarla de forma atómica y con historia? Antes de
diseñar `write()` (0031 W3.6), lo que hay que saber, medido:

  §1  EL CATÁLOGO ES GIT     `ArbolCatalog`: un catálogo de PyIceberg cuyo estado es un
                             repo git —`catalogo/<ns>/<tabla>.json` apunta al
                             `metadata.json` vigente— y cuyo commit ES el swap atómico
                             (compare-and-set sobre el puntero + `git commit`). Cuánto
                             cuesta un commit, y si dos escritores desde el mismo
                             snapshot chocan como Iceberg manda
  §2  ESCRIBIR A ESCALA      10 M de filas: Parquet + ORECOPY1 (lo de hoy) frente a una
                             tabla Iceberg (PyIceberg): ms, bytes, objetos, metadatos
  §3  LEER                   la misma tabla por DuckDB (`iceberg_scan` sobre el
                             `metadata.json`, sin catálogo) y por PyIceberg → Arrow,
                             frente a `read_parquet` de la copia
  §4  EVOLUCIONAR            un append (snapshot 2), una promoción int → long y una
                             columna nueva (lo que 0032 permite), y el viaje en el
                             tiempo al snapshot 1 — desde DuckDB y desde PyIceberg
  §5  LOS TIPOS              los 23 tipos difíciles de `medida-w3-leer.py`: cuáles acepta
                             Iceberg, cuáles cambian y cuáles no entran
  §6  LA COPIA PEQUEÑA       una copia como las 31 de demo y victor (100 k filas): lo que
                             pesa el sobre frente a los metadatos de Iceberg
  §7  EL BUCKET (--bucket)   lo mismo que §1–§2 en pequeño contra GCS de verdad (gcsfs):
                             la latencia de un commit cuando cada metadato es un PUT

Uso:
  python pruebas-de-fuego/medida-w3-iceberg.py [--filas 10000000] [--bucket gs://<bucket>/<prefijo>]

Hacen falta pyiceberg[pyarrow,gcsfs] (`python -m pip install "pyiceberg[pyarrow,gcsfs]"`),
duckdb ≥ 1.2 con la extensión `iceberg` (se instala sola) y git. Nada de pago, nada en el
clúster; `--bucket` escribe y borra bajo el prefijo dado con la sesión de gcloud de quien
lo corre.
"""
import datetime as dt
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import uuid

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

espec = importlib.util.spec_from_file_location("leer", os.path.join(RAIZ, "pruebas-de-fuego", "medida-w3-leer.py"))
leer = importlib.util.module_from_spec(espec)
espec.loader.exec_module(leer)


def fila(k, v, nota=""):
    print("  %-34s %-40s %s" % (k, v, nota))


def ms(t0):
    return int((time.time() - t0) * 1000)


def git(repo, *args):
    r = subprocess.run(["git", "-C", repo, *args], capture_output=True)
    if r.returncode != 0:
        raise RuntimeError("git %s: %s" % (" ".join(args), r.stderr.decode("utf-8", "replace").strip()))
    return r.stdout.decode("utf-8", "replace").strip()


def carpetas(tabla):
    """(datos, metadatos) de una tabla de Iceberg en disco, venga la ubicación como venga."""
    loc = tabla.location()
    for pre in ("file:///", "file://"):
        if loc.startswith(pre):
            loc = loc[len(pre):]
            break
    return loc + "/data", loc + "/metadata"


def tam_dir(ruta, sufijos=None):
    """Bytes y número de ficheros bajo `ruta` (los que acaben en `sufijos`, si se dan)."""
    total, n = 0, 0
    for d, _, fs in os.walk(ruta):
        for f in fs:
            if sufijos and not f.endswith(sufijos):
                continue
            total += os.path.getsize(os.path.join(d, f))
            n += 1
    return total, n


# ── §1 · el catálogo cuyo estado es git ─────────────────────────────────────
def catalogo_arbol():
    """Construye la clase aquí para que el módulo importe sin pyiceberg."""
    from pyiceberg.catalog import Catalog, MetastoreCatalog
    from pyiceberg.exceptions import CommitFailedException, NoSuchTableError, TableAlreadyExistsError
    from pyiceberg.io import load_file_io
    from pyiceberg.serializers import FromInputFile
    from pyiceberg.table import CommitTableResponse, Table

    class ArbolCatalog(MetastoreCatalog):
        """El árbol como catálogo de Iceberg.

        Lo único que un catálogo de Iceberg tiene que hacer es el *swap* atómico del
        puntero al `metadata.json` vigente. Aquí el puntero es un fichero del árbol
        (`catalogo/<ns>/<tabla>.json`) y el swap es un commit de git: se comprueba que
        el puntero sigue siendo el que el escritor leyó (compare-and-set: si otro
        escritor ganó, `CommitFailedException`, como en cualquier catálogo) y se
        confirma. En el clúster la forja rechaza el push que no es fast-forward, que es
        el mismo CAS un piso más arriba. Cada tabla queda con su historia en `git log`.
        """

        def __init__(self, name, arbol, **properties):
            super().__init__(name, **properties)
            self.arbol = arbol
            os.makedirs(os.path.join(arbol, "catalogo"), exist_ok=True)
            if not os.path.isdir(os.path.join(arbol, ".git")):
                git(arbol, "init", "-q")
                git(arbol, "config", "user.email", "medida@invalido")
                git(arbol, "config", "user.name", "medida")
            self.commits_ms = []
            self.choques = 0

        # el puntero
        def _puntero(self, identifier):
            ns = Catalog.namespace_to_string(Catalog.namespace_from(identifier))
            return os.path.join(self.arbol, "catalogo", ns, Catalog.table_name_from(identifier) + ".json")

        def _leer_puntero(self, identifier):
            try:
                with open(self._puntero(identifier), encoding="utf-8") as f:
                    return json.load(f)
            except FileNotFoundError:
                raise NoSuchTableError("no hay tabla %s en el árbol" % (identifier,))

        def _confirmar(self, identifier, puntero, mensaje, esperado=None):
            """El swap: CAS sobre el puntero y commit. `esperado` es el metadata_location
            que el escritor leyó; si el árbol ya dice otro, otro escritor ganó."""
            t0 = time.time()
            ruta = self._puntero(identifier)
            if esperado is not None:
                actual = self._leer_puntero(identifier)["metadata_location"]
                if actual != esperado:
                    self.choques += 1
                    raise CommitFailedException("otro escritor confirmó antes: el árbol apunta a %s" % actual)
            elif os.path.exists(ruta):
                raise TableAlreadyExistsError("la tabla %s ya está en el árbol" % (identifier,))
            os.makedirs(os.path.dirname(ruta), exist_ok=True)
            with open(ruta, "w", encoding="utf-8") as f:
                json.dump(puntero, f, indent=1, sort_keys=True)
            rel = os.path.relpath(ruta, self.arbol).replace("\\", "/")
            git(self.arbol, "add", "--", rel)
            git(self.arbol, "commit", "-q", "-m", mensaje, "--", rel)
            self.commits_ms.append(ms(t0))

        # lo que PyIceberg pide
        def create_table(self, identifier, schema, location=None, partition_spec=None, sort_order=None, properties=None):
            from pyiceberg.partitioning import UNPARTITIONED_PARTITION_SPEC
            from pyiceberg.table.sorting import UNSORTED_SORT_ORDER

            staged = self._create_staged_table(identifier, schema, location, partition_spec or UNPARTITIONED_PARTITION_SPEC, sort_order or UNSORTED_SORT_ORDER, properties or {})
            self._write_metadata(staged.metadata, staged.io, staged.metadata_location)
            self._confirmar(identifier, {"metadata_location": staged.metadata_location, "previous_metadata_location": None}, "catálogo: nace %s" % Catalog.table_name_from(identifier))
            return self.load_table(identifier)

        def load_table(self, identifier):
            p = self._leer_puntero(identifier)
            loc = p["metadata_location"]
            io = load_file_io(properties=self.properties, location=loc)
            metadata = FromInputFile.table_metadata(io.new_input(loc))
            return Table(identifier=Catalog.identifier_to_tuple(identifier), metadata=metadata, metadata_location=loc, io=self._load_file_io(metadata.properties, loc), catalog=self)

        def commit_table(self, table, requirements, updates):
            identifier = table.name()
            try:
                actual = self.load_table(identifier)
            except NoSuchTableError:
                actual = None
            try:
                # Los requisitos de Iceberg (`assert-ref-snapshot-id`…) se comprueban
                # contra lo que el árbol dice AHORA: es el primer guardia del CAS.
                staged = self._update_and_stage_table(actual, identifier, requirements, updates)
            except CommitFailedException:
                self.choques += 1
                raise
            if actual and staged.metadata == actual.metadata:
                return CommitTableResponse(metadata=actual.metadata, metadata_location=actual.metadata_location)
            self._write_metadata(staged.metadata, staged.io, staged.metadata_location)
            self._confirmar(identifier, {"metadata_location": staged.metadata_location, "previous_metadata_location": actual.metadata_location if actual else None}, "catálogo: %s → snapshot %s" % (Catalog.table_name_from(identifier), staged.metadata.current_snapshot_id), esperado=actual.metadata_location if actual else None)
            return CommitTableResponse(metadata=staged.metadata, metadata_location=staged.metadata_location)

        def table_exists(self, identifier):
            return os.path.exists(self._puntero(identifier))

        def drop_table(self, identifier):
            ruta = self._puntero(identifier)
            rel = os.path.relpath(ruta, self.arbol).replace("\\", "/")
            git(self.arbol, "rm", "-q", "--", rel)
            git(self.arbol, "commit", "-q", "-m", "catálogo: se retira %s" % Catalog.table_name_from(identifier), "--", rel)

        def list_tables(self, namespace):
            ns = Catalog.namespace_to_string(namespace)
            d = os.path.join(self.arbol, "catalogo", ns)
            return [(ns, f[:-5]) for f in sorted(os.listdir(d))] if os.path.isdir(d) else []

        def create_namespace(self, namespace, properties=None):
            os.makedirs(os.path.join(self.arbol, "catalogo", Catalog.namespace_to_string(namespace)), exist_ok=True)

        def namespace_exists(self, namespace):
            return True

        def list_namespaces(self, namespace=()):
            return []

        def load_namespace_properties(self, namespace):
            return {}

        # lo que esta medida no usa
        def _no(self, *a, **k):
            raise NotImplementedError("esta medida no lo usa")

        rename_table = purge_table = register_table = drop_namespace = update_namespace_properties = _no
        create_table_transaction = create_view = drop_view = list_views = load_view = register_view = view_exists = _no

        def supports_server_side_planning(self):
            return False

    return ArbolCatalog


def main():
    filas = 10_000_000
    if "--filas" in sys.argv:
        filas = int(sys.argv[sys.argv.index("--filas") + 1])
    bucket = sys.argv[sys.argv.index("--bucket") + 1] if "--bucket" in sys.argv else None
    print("MEDIDA · W3.6 · Iceberg con git de catálogo · %s" % dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%d %H:%MZ"))
    import duckdb
    import pyarrow as pa
    import pyarrow.parquet as pq
    import pyiceberg
    from pyiceberg.exceptions import CommitFailedException
    from pyiceberg.types import LongType

    ArbolCatalog = catalogo_arbol()
    fila("versiones", "pyiceberg %s · pyarrow %s · duckdb %s" % (pyiceberg.__version__, pa.__version__, duckdb.__version__))
    con = duckdb.connect()
    con.execute("install iceberg; load iceberg;")

    tmp = tempfile.mkdtemp(prefix="ore-iceberg-")
    tmp = tmp.replace("\\", "/")
    almacen = tmp + "/almacen"       # el bucket de hoy: ORECOPY1
    bodega = tmp + "/bodega"         # el bucket con Iceberg
    arbol = tmp + "/arbol"           # el árbol: git
    os.makedirs(almacen)
    os.makedirs(bodega)
    os.makedirs(arbol)
    try:
        # ── §1 · el catálogo es git ────────────────────────────────────────
        print()
        print("§1 · el catálogo cuyo estado es git (ArbolCatalog: puntero en el árbol + commit = swap)")
        cat = ArbolCatalog("arbol", arbol, warehouse=bodega)
        cat.create_namespace("ventas")
        # 50 commits de una fila: cuánto cuesta el swap (metadatos + git)
        chica = pa.table({"id": pa.array([1], pa.int64()), "v": pa.array(["a"])})
        t = cat.create_table("ventas.latidos", chica.schema)
        t0 = time.time()
        for i in range(50):
            t.append(chica)
        total_ms = ms(t0)
        fila("50 appends de 1 fila (commit cada uno)", "%d ms · %.0f ms/commit" % (total_ms, total_ms / 50), "de los que git: %.0f ms/commit" % (sum(cat.commits_ms[-50:]) / 50))
        fila("  historia", "%d snapshots · %d commits en git" % (len(t.history()), int(git(arbol, "rev-list", "--count", "HEAD"))), "`git log` es el viaje en el tiempo")
        m_bytes, m_n = tam_dir(carpetas(t)[1])
        fila("  metadatos acumulados", "%d ficheros · %.0f KB" % (m_n, m_bytes / 1e3), "metadata.json + manifest-list + manifest por commit; se compactan con `expire_snapshots`")
        # dos escritores desde el mismo snapshot: el segundo tiene que chocar
        a = cat.load_table("ventas.latidos")
        b = cat.load_table("ventas.latidos")
        antes_snaps, antes_choques = len(a.history()), cat.choques
        a.append(chica)
        b.append(chica)
        b.refresh()
        fila("  dos escritores, mismo snapshot", "%d choque(s) en el CAS · %d snapshots nuevos" % (cat.choques - antes_choques, len(b.history()) - antes_snaps), "el segundo chocó contra el puntero del árbol, refrescó y reintentó (un append conmuta): lo que Iceberg manda")
        # y una operación que NO conmuta (cambiar el esquema desde un snapshot viejo) se niega
        a = cat.load_table("ventas.latidos")
        b = cat.load_table("ventas.latidos")
        with a.update_schema() as up:
            up.add_column("x", pa_to_iceberg_string())
        try:
            with b.update_schema() as up:
                up.add_column("y", pa_to_iceberg_string())
            fila("  dos cambios de esquema a la vez", "los dos entran", "%s" % ", ".join(f.name for f in cat.load_table("ventas.latidos").schema().fields))
        except CommitFailedException as e:
            fila("  dos cambios de esquema a la vez", "✓ el segundo se niega", str(e)[:70])
        # un árbol grande: ¿git commit escala?
        for i in range(2000):
            d = os.path.join(arbol, "packages", "p%d" % (i % 20))
            os.makedirs(d, exist_ok=True)
            with open(os.path.join(d, "doc%d.yaml" % i), "w") as f:
                f.write("kind: Table\nname: t%d\n" % i)
        git(arbol, "add", "-A")
        git(arbol, "commit", "-q", "-m", "2000 documentos")
        antes = len(cat.commits_ms)
        for i in range(10):
            t.append(chica)
        fila("  con 2000 ficheros en el árbol", "git %.0f ms/commit" % (sum(cat.commits_ms[antes:]) / 10), "el commit por ruta no mira el resto")

        # ── §2 · escribir a escala ─────────────────────────────────────────
        print()
        print("§2 · escribir %d filas (4 columnas)" % filas)
        t0 = time.time()
        grande = leer.tabla_grande(filas)
        fila("la tabla en memoria", "%d ms" % ms(t0), "%.0f MB en Arrow" % (grande.nbytes / 1e6))
        t0 = time.time()
        clave, bytes_pq = leer.sobre(grande, almacen, "ventas.grande")
        sobre_ms = ms(t0)
        fila("Parquet + ORECOPY1 (hoy)", "%d ms · %.1f MB · 1 objeto" % (sobre_ms, bytes_pq / 1e6), "snappy")
        t0 = time.time()
        tg = cat.create_table("ventas.grande", grande.schema)
        crear_ms = ms(t0)
        t0 = time.time()
        tg.append(grande)
        append_ms = ms(t0)
        d_bytes, d_n = tam_dir(carpetas(tg)[0])
        m_bytes, m_n = tam_dir(carpetas(tg)[1])
        fila("Iceberg (PyIceberg: create + append)", "%d + %d ms · %.1f MB · %d ficheros de datos" % (crear_ms, append_ms, d_bytes / 1e6, d_n), "zstd por defecto; metadatos %d ficheros · %.0f KB · git %d ms" % (m_n, m_bytes / 1e3, cat.commits_ms[-1]))
        metadata_v = tg.metadata_location

        # ── §3 · leer ──────────────────────────────────────────────────────
        print()
        print("§3 · leer las %d filas" % filas)
        with open(os.path.join(almacen, clave), "rb") as f:
            crudo = f.read()
        n = int.from_bytes(crudo[8:12], "little")
        pq_path = tmp + "/copia.parquet"
        with open(pq_path, "wb") as f:
            f.write(crudo[12 + n:])
        t0 = time.time()
        r = con.execute("select pais, count(*) n, sum(importe) s from read_parquet('%s') group by 1 order by 2 desc" % pq_path).fetchall()
        fila("DuckDB · read_parquet (la copia)", "%d ms" % ms(t0), "%d grupos" % len(r))
        t0 = time.time()
        r2 = con.execute("select pais, count(*) n, sum(importe) s from iceberg_scan('%s') group by 1 order by 2 desc" % metadata_v).fetchall()
        r, r2 = sorted(r), sorted(r2)
        igual = len(r) == len(r2) and all(x[0] == y[0] and x[1] == y[1] and abs(x[2] - y[2]) < 1e-3 * max(1.0, abs(x[2])) for x, y in zip(r, r2))
        fila("DuckDB · iceberg_scan(metadata.json)", "%d ms" % ms(t0), "%d grupos · sin catálogo: el puntero del árbol basta%s" % (len(r2), " · el mismo resultado" if igual else " · ≠ RESULTADO"))
        t0 = time.time()
        n_pq = pq.read_table(pq_path).num_rows
        fila("pyarrow · read_table (la copia)", "%d ms" % ms(t0), "%d filas" % n_pq)
        t0 = time.time()
        n_ib = tg.scan().to_arrow().num_rows
        fila("PyIceberg · scan().to_arrow()", "%d ms" % ms(t0), "%d filas" % n_ib)
        t0 = time.time()
        n_f = tg.scan(row_filter="pais = 'A'").to_arrow().num_rows
        fila("PyIceberg · scan(pais = 'A')", "%d ms" % ms(t0), "%d filas · poda por estadísticas del manifiesto" % n_f)

        # ── §4 · evolucionar ───────────────────────────────────────────────
        print()
        print("§4 · evolucionar: append, promoción, columna nueva, viaje en el tiempo")
        snap1 = tg.current_snapshot().snapshot_id
        delta = leer.tabla_grande(1_000_000)
        t0 = time.time()
        tg.append(delta)
        fila("append de 1 M (snapshot 2)", "%d ms" % ms(t0), "%d ficheros de datos · la copia de hoy tendría que reescribir los %d M" % (tam_dir(carpetas(tg)[0])[1], filas // 1_000_000))
        t0 = time.time()
        with tg.update_schema() as up:
            up.update_column("cliente", LongType())
            up.add_column("canal", pa_to_iceberg_string())
        fila("int → long + columna nueva", "%d ms" % ms(t0), "sin tocar un fichero de datos: %s" % ", ".join("%s:%s" % (f.name, f.field_type) for f in tg.schema().fields))
        t0 = time.time()
        n_ahora = tg.scan().to_arrow().num_rows
        fila("leer ahora", "%d ms" % ms(t0), "%d filas (cliente es int64, canal es null)" % n_ahora)
        t0 = time.time()
        n_antes = tg.scan(snapshot_id=snap1).to_arrow().num_rows
        fila("leer el snapshot 1 (PyIceberg)", "%d ms" % ms(t0), "%d filas" % n_antes)
        t0 = time.time()
        n_duck = con.execute("select count(*) from iceberg_scan('%s', snapshot_from_id=%d)" % (tg.metadata_location, snap1)).fetchone()[0]
        fila("leer el snapshot 1 (DuckDB)", "%d ms" % ms(t0), "%d filas" % n_duck)
        fila("historia", "%d snapshots · %d commits de git de la tabla" % (len(tg.history()), len(git(arbol, "log", "--oneline", "--", "catalogo/ventas/grande.json").splitlines())), "`git log -- catalogo/ventas/grande.json`")

        # ── §5 · los tipos ─────────────────────────────────────────────────
        print()
        print("§5 · los 23 tipos difíciles en Iceberg (v2)")
        tt = leer.tabla_de_tipos()
        verdad = leer.verdad(tt)
        aceptadas, rechazadas = {}, {}
        for c in tt.column_names:
            col = tt.select([c])
            try:
                tc = cat.create_table("ventas.tipo_" + c, col.schema)
                tc.append(col)
                aceptadas[c] = tc
            except Exception as e:
                rechazadas[c] = str(e).splitlines()[0][:70]
        print("  %-13s %-28s %-30s %-22s %s" % ("columna", "arrow", "iceberg", "vuelve (PyIceberg)", "vuelve (DuckDB)"))
        for c in tt.column_names:
            if c in rechazadas:
                print("  %-13s %-28s ✗ %s" % (c, verdad[c]["tipo"][:28], rechazadas[c]))
                continue
            tc = aceptadas[c]
            tipo_i = str(tc.schema().fields[0].field_type)
            vuelta = tc.scan().to_arrow()
            v_py = [leer.canonico(x) for x in vuelta.column(0).to_pylist()]
            ok_py = "✓" if v_py == verdad[c]["valores"] else "≠ %s" % next((x for x, y in zip(v_py, verdad[c]["valores"]) if x != y), "?")
            try:
                vd = con.execute("select * from iceberg_scan('%s')" % tc.metadata_location).to_arrow_table()
                v_d = [leer.canonico(x) for x in vd.column(0).to_pylist()]
                ok_d = "✓" if v_d == verdad[c]["valores"] else "≠ %s" % str(next((x for x, y in zip(v_d, verdad[c]["valores"]) if x != y), "?"))[:24]
            except Exception as e:
                ok_d = "✗ %s" % str(e).splitlines()[0][:26]
            print("  %-13s %-28s %-30s %-22s %s" % (c, verdad[c]["tipo"][:28], tipo_i[:30], ("%s %s" % (ok_py, str(vuelta.schema.field(0).type)[:16]))[:22], ok_d))
        fila("aceptadas / rechazadas", "%d / %d" % (len(aceptadas), len(rechazadas)))

        # ── §6 · la copia pequeña ──────────────────────────────────────────
        print()
        print("§6 · una copia como las de demo/victor: 100 k filas × 3 columnas")
        chica = leer.tabla_grande(100_000).select(["id", "cliente", "pais"])
        t0 = time.time()
        _, b_pq = leer.sobre(chica, almacen, "ventas.chica")
        s_ms = ms(t0)
        t0 = time.time()
        tc = cat.create_table("ventas.chica", chica.schema)
        tc.append(chica)
        i_ms = ms(t0)
        d_b, d_n = tam_dir(carpetas(tc)[0])
        m_b, m_n = tam_dir(carpetas(tc)[1])
        fila("ORECOPY1", "%d ms · %.0f KB · 1 objeto" % (s_ms, b_pq / 1e3), "cabecera ~300 B")
        fila("Iceberg", "%d ms · %.0f KB datos + %.0f KB metadatos · %d objetos" % (i_ms, d_b / 1e3, m_b / 1e3, d_n + m_n), "%d commits de git" % 2)

        # ── §7 · el bucket ─────────────────────────────────────────────────
        if bucket:
            print()
            print("§7 · en el bucket: %s" % bucket)
            import gcsfs
            from google.oauth2.credentials import Credentials

            # La sesión de gcloud de quien lo corre, en memoria y sin escribirla:
            # gcsfs y PyIceberg no leen la de gcloud (quieren ADC).
            tok = subprocess.run(["gcloud", "auth", "print-access-token"], capture_output=True, shell=(os.name == "nt")).stdout.decode().strip()
            if not tok:
                raise SystemExit("gcloud auth print-access-token no dio nada: inicia sesión con gcloud")
            fs = gcsfs.GCSFileSystem(token=Credentials(token=tok))
            prefijo = bucket.rstrip("/") + "/medida-" + uuid.uuid4().hex[:8]
            caduca = str(int((time.time() + 3000) * 1000))
            catg = ArbolCatalog("arbol-gcs", tmp + "/arbol-gcs", **{"warehouse": prefijo, "py-io-impl": "pyiceberg.io.fsspec.FsspecFileIO", "gcs.oauth2.token": tok, "gcs.oauth2.token-expires-at": caduca})
            catg.create_namespace("ventas")
            medio = leer.tabla_grande(1_000_000)
            # cuánto cuesta UN objeto pequeño desde aquí: la unidad de todo lo demás
            t0 = time.time()
            for i in range(5):
                with fs.open("%s/latido-%d" % (prefijo[5:], i), "wb") as f:
                    f.write(b"x" * 300)
            fila("1 PUT de 300 B (media de 5)", "%.0f ms" % (ms(t0) / 5), "desde esta máquina hasta el bucket; en el clúster (misma región) son ~20–50 ms")
            t0 = time.time()
            clave_g = "%s/copia.orecopy" % prefijo[5:]
            with fs.open(clave_g, "wb") as f:
                import io as _io
                b = _io.BytesIO()
                pq.write_table(medio, b)
                f.write(b.getvalue())
            fila("1 M filas · ORECOPY1 (1 PUT)", "%d ms" % ms(t0))
            t0 = time.time()
            tgcs = catg.create_table("ventas.medio", medio.schema)
            c_ms = ms(t0)
            t0 = time.time()
            tgcs.append(medio)
            a_ms = ms(t0)
            fila("1 M filas · Iceberg (create + append)", "%d + %d ms" % (c_ms, a_ms), "metadata.json, manifest-list, manifest, datos: PUTs contra GCS")
            t0 = time.time()
            for i in range(5):
                tgcs.append(chica.slice(0, 1))
            fila("5 appends de 1 fila", "%.0f ms/commit" % (ms(t0) / 5), "de los que git: %.0f ms" % (sum(catg.commits_ms[-5:]) / 5))
            t0 = time.time()
            n_g = tgcs.scan().to_arrow().num_rows
            fila("leer con PyIceberg desde GCS", "%d ms" % ms(t0), "%d filas" % n_g)
            objetos = fs.find(prefijo[5:])
            fila("objetos bajo el prefijo", str(len(objetos)))
            for o in objetos:
                fs.rm(o)
            fila("limpieza del bucket", "%d objetos borrados" % len(objetos))
    finally:
        con.close()
        shutil.rmtree(tmp, ignore_errors=True)
        fila("limpieza", "el temporal fuera")


def pa_to_iceberg_string():
    from pyiceberg.types import StringType

    return StringType()


if __name__ == "__main__":
    main()
