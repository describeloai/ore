"""**ore-consulta** (Query Q4, `docs/query.md` §3.2): un trabajador DuckDB de Query.

Lo que la consola Query corre, en una capacidad de la celda siempre caliente y
compartida, y no en el puesto de cada persona. `ore-serve` admite cada consulta
(su clase, su memoria y sus hilos), resuelve sus nombres en nombre de quien
pregunta y presta las credenciales de SUS datasets; aquí sólo llega eso:

    GET  {ORE_SERVE}/consultas/trabajo        (espera larga; el token de este pod)
         x-ore-libre-mb, x-ore-libres-hilos: lo que cabe ahora
      → 200 {id, texto, fuentes, limite, clase, memoria_mb, hilos} · 204 nada
    POST {ORE_SERVE}/consultas/{id}/resultado
         {ok, arrow: <Arrow IPC en base64>, filas, truncada, …} | {ok: false, codigo, error}

**No acepta conexiones**, como el puesto: pide trabajo cuando le cabe, y así el
reparto entre trabajadores sale solo. **Sin identidad propia sobre el lago**: su
cuenta no tiene Workload Identity y lo que lee, lo lee con la credencial que trae
cada consulta (`ore._instancia`: sin ella, `PermissionError`).

**Una instancia de DuckDB por consulta** (Q0b, §2.7), sacada de un montón de
instancias ya preparadas (las extensiones cargadas: 200 ms menos), con su tope de
memoria y de hilos, su derrame acotado en `/trabajo/<id>`, las rutas de sus
datasets y nada más (`enable_external_access = false`, `allowed_directories`) y la
configuración cerrada (`lock_configuration`) antes de correr lo del usuario. Una
sola sentencia, y que lea.

    ORE_SERVE=http://ore-serve:8080 python ore_consulta.py
"""
import base64
import io
import json
import os
import queue
import shutil
import signal
import socket
import sys
import threading
import time
import urllib.error
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import ore  # noqa: E402

SERVE = os.environ.get("ORE_SERVE", "http://127.0.0.1:8080").rstrip("/")
TOKEN_DEL_POD = os.environ.get("ORE_TOKEN_DEL_POD", "/var/run/ore/pod/token")
TRABAJO = os.environ.get("ORE_TRABAJO", "/trabajo")
VIVO = os.environ.get("ORE_CONSULTA_VIVO", "/tmp/ore-consulta-vivo")
NOMBRE = os.environ.get("HOSTNAME") or socket.gethostname()
# Lo de un `select` que vuelve: por debajo del cuerpo máximo de `ore-serve` (1 MiB)
# una vez en base64. Lo que pasa, se corta y se dice (`truncada`); Q6 pagina.
BYTES_DE_VUELTA = 700_000
# La espera larga de `ore-serve` es de 25 s: algo más aquí.
PLAZO_DE_PEDIR = 40


def _cgroup_mb():
    try:
        v = open("/sys/fs/cgroup/memory.max").read().strip()
        return int(v) // 2**20 if v != "max" else None
    except (OSError, ValueError):
        return None


def _entero(nombre, si_no):
    v = os.environ.get(nombre)
    return int(v) if v else si_no


# Lo que se reparte entre las consultas: ~80 % del pod (§3.2), lo mismo que la
# admisión supone (`Umbrales::trabajador_mb` × `reparto_pct`).
MEMORIA_MB = _entero("ORE_CONSULTA_MEMORIA_MB", int((_cgroup_mb() or 8192) * 0.8))
HILOS = _entero("ORE_CONSULTA_HILOS", os.cpu_count() or 2)
# El derrame de todas, en `/trabajo` (su `sizeLimit` menos holgura); cada una el
# suyo, en proporción a su memoria. 0 = sin tope (en local).
DISCO_MB = _entero("ORE_CONSULTA_DISCO_MB", 0)
PREPARADAS = _entero("ORE_CONSULTA_PREPARADAS", 4)
# Lo más pequeño que se admite (L, `Umbrales::l_memoria_mb`): con menos libre, no se pide.
MINIMO_MB = _entero("ORE_CONSULTA_MINIMO_MB", 2048)
# Q5·2 · Un trabajador de quita y pon (un Job de la cola) se va solo tras tantos
# segundos sin consultas: devolver capacidad es no hacer nada. 0 = nunca (el suelo).
OCIOSO_S = _entero("ORE_CONSULTA_OCIOSO_S", 0)

# Las extensiones que una consulta puede necesitar, cargadas al preparar: con la
# configuración cerrada ya no se puede cargar nada (medido en Q0b).
EXTENSIONES = ("json", "icu", "avro", "iceberg", "httpfs", "spatial")


def log(*a):
    print("ore-consulta ·", *a, flush=True)


class NoSeCorre(Exception):
    def __init__(self, codigo, mensaje):
        super().__init__(mensaje)
        self.codigo, self.mensaje = codigo, mensaje


# ── El montón de instancias preparadas ──────────────────────────────────────
def preparar():
    """Una instancia de DuckDB lista para una consulta: en memoria, sin instalar
    nada, con las extensiones cargadas y en UTC. Sin secretos ni topes: eso es de
    cada consulta."""
    import duckdb

    config = {"autoinstall_known_extensions": "false"}
    if os.path.isdir(ore.EXTENSIONES):
        config["extension_directory"] = ore.EXTENSIONES
    con = duckdb.connect(config=config)
    con.execute("set TimeZone = 'UTC'")
    for e in EXTENSIONES:
        ore._cargar(con, e)
    try:
        con.execute("load h3")
    except duckdb.Error:
        pass
    return con


class Monton:
    def __init__(self, n):
        self.listas = queue.Queue()
        self.n = n
        self.falta = threading.Event()
        self.falta.set()
        threading.Thread(target=self._rellenar, daemon=True, name="monton").start()

    def _rellenar(self):
        while True:
            self.falta.wait()
            while self.listas.qsize() < self.n:
                try:
                    self.listas.put(preparar())
                except Exception as e:  # noqa: BLE001 — se reintenta; la consulta prepara la suya
                    log("no se pudo preparar una instancia: %s" % e)
                    time.sleep(5)
                    break
            self.falta.clear()

    def tomar(self):
        try:
            con = self.listas.get_nowait()
        except queue.Empty:
            con = preparar()
        self.falta.set()
        return con


# ── Las ranuras: lo que cabe ahora ──────────────────────────────────────────
class Ranuras:
    def __init__(self, memoria_mb, hilos):
        self.cond = threading.Condition()
        self.mb, self.hilos = memoria_mb, hilos
        self.corriendo = 0
        self.ultima = time.time()

    def esperar_hueco(self, minimo_mb, parar):
        with self.cond:
            while not parar.is_set() and (self.mb < minimo_mb or self.hilos < 1):
                self.cond.wait(1)
            return self.mb, self.hilos

    def reservar(self, mb, hilos):
        with self.cond:
            self.mb -= mb
            self.hilos -= hilos
            self.corriendo += 1
            self.ultima = time.time()

    def soltar(self, mb, hilos):
        with self.cond:
            self.mb += mb
            self.hilos += hilos
            self.corriendo -= 1
            self.ultima = time.time()
            self.cond.notify_all()

    def ocioso(self):
        """Segundos sin consultas (0 si corre alguna)."""
        with self.cond:
            return 0 if self.corriendo else time.time() - self.ultima


# ── Una consulta ────────────────────────────────────────────────────────────
def _raiz(metadata_location):
    """Lo que DuckDB lee de un dataset: su raíz, como la escribe `ore._iceberg`."""
    raiz = metadata_location.rsplit("/metadata/", 1)[0]
    if raiz.startswith("gs://"):
        raiz = "https://storage.googleapis.com/" + raiz[5:]
    return raiz.rstrip("/") + "/"


def _comprobar(nombre, rd, raices):
    """Que `rd` se lea aquí, con su credencial, y sus raíces a `raices`."""
    rd = rd or {}
    if rd.get("federada") is not None or rd.get("vistaFederada") is not None or rd.get("collection"):
        raise NoSeCorre("consulta/en-el-puesto", "`%s` no se lee en un trabajador de Query: corre en el puesto" % nombre)
    if rd.get("consulta"):
        for d, sub in (rd.get("datasets") or {}).items():
            _comprobar(d, sub, raices)
        return
    ml = rd.get("metadata_location")
    if not ml:
        raise NoSeCorre("consulta/copia-antigua", "`%s` no es un dataset de Iceberg: se lee en un puesto" % nombre)
    cred = rd.get("credencial") or {}
    if not (cred.get("gcs.oauth2.token") or cred.get("s3.access-key-id")):
        raise NoSeCorre("consulta/sin-credencial", "`%s` llega sin su credencial prestada: un trabajador no lee nada con la suya" % nombre)
    raices.add(_raiz(ml))


def _derrame_mb(memoria_mb):
    if not DISCO_MB:
        return None
    return max(1024, DISCO_MB * memoria_mb // max(MEMORIA_MB, 1))


def _lista(xs):
    return "[%s]" % ", ".join("'%s'" % x.replace("'", "''") for x in sorted(xs))


def calcular(con, t, derrame):
    """El resultado de la consulta `t` en la instancia `con`, como
    `pyarrow.Table` (hasta `limite` filas), y si había más."""
    import duckdb
    import pyarrow as pa

    texto = t.get("texto") or ""
    try:
        sentencias = con.extract_statements(texto)
    except duckdb.Error as e:
        raise NoSeCorre("consulta/sql", str(e).splitlines()[0]) from None
    if len(sentencias) != 1 or sentencias[0].type != duckdb.StatementType.SELECT:
        raise NoSeCorre("consulta/sin-resultado", "Query sólo lee: una sentencia `select`")
    memoria, hilos = int(t["memoria_mb"]), int(t["hilos"])
    limite = int(t.get("limite") or 1000)
    con.execute("set memory_limit = '%dMiB'" % memoria)
    con.execute("set threads = %d" % max(1, hilos))
    os.makedirs(derrame, exist_ok=True)
    con.execute("set temp_directory = '%s'" % derrame.replace("'", "''"))
    if _derrame_mb(memoria):
        con.execute("set max_temp_directory_size = '%dMiB'" % _derrame_mb(memoria))
    fuentes = dict(t.get("fuentes") or {})
    fuentes.pop("__avisos", None)
    raices = set()
    for nombre, rd in sorted(fuentes.items()):
        _comprobar(nombre, rd, raices)
    with ore._instancia(con):
        for nombre, rd in sorted(fuentes.items()):
            fuente, _ = ore._fuente_de_respuesta(nombre, rd)
            ore._registra(con, nombre, fuente)
        ore._geo_si_lo_pide(con, texto)
    # ⛔ Cerrada antes de lo del usuario (§3.5): sólo sus datasets y su derrame,
    #   nada local ni de la red, y nadie lo vuelve a abrir con un `set`. Lo que
    #   `allowed_directories` no hace —normalizar `..` en una URL— lo para la
    #   admisión y, sobre todo, la credencial, que sólo abre estos datasets.
    con.execute("set allowed_directories = %s" % _lista(raices | {derrame.rstrip("/") + "/"}))
    con.execute("set enable_external_access = false")
    con.execute("set lock_configuration = true")
    try:
        r = con.execute(texto)
    except duckdb.Error as e:
        from ore.lsp_sql import dicho_geo
        raise NoSeCorre("consulta/sql", dicho_geo(str(e)) or ore._sin_disco(e) or str(e).splitlines()[0]) from None
    if r.description is None:
        raise NoSeCorre("consulta/sin-resultado", "la sentencia no devuelve filas: Query sólo lee")
    lector = r.to_arrow_reader(8192) if hasattr(r, "to_arrow_reader") else r.fetch_record_batch(8192)
    lotes, n = [], 0
    for lote in lector:
        if n + lote.num_rows > limite:
            lotes.append(lote.slice(0, limite - n))
            return pa.Table.from_batches(lotes, schema=lector.schema), True
        lotes.append(lote)
        n += lote.num_rows
    return pa.Table.from_batches(lotes, schema=lector.schema), False


def a_arrow(tabla, truncada):
    """El Arrow IPC de `tabla`, cortado hasta caber en [`BYTES_DE_VUELTA`]."""
    import pyarrow as pa

    while True:
        sumidero = io.BytesIO()
        with pa.ipc.new_stream(sumidero, tabla.schema) as w:
            w.write_table(tabla)
        datos = sumidero.getvalue()
        if len(datos) * 4 // 3 <= BYTES_DE_VUELTA or tabla.num_rows <= 1:
            return datos, tabla.num_rows, truncada
        tabla, truncada = tabla.slice(0, tabla.num_rows // 2), True


# ── La conversación con ore-serve ───────────────────────────────────────────
def _cabeceras():
    h = {"x-ore-trabajador": NOMBRE}
    sujeto = os.environ.get("ORE_SUJETO")  # en local, contra un ore-serve de pruebas
    if sujeto:
        h["x-ore-sujeto"] = sujeto
        return h
    with open(TOKEN_DEL_POD, encoding="utf-8") as f:  # el kubelet lo renueva en el sitio
        h["x-ore-pod"] = f.read().strip()
    return h


def pedir(libre_mb, hilos):
    h = dict(_cabeceras(), **{"x-ore-libre-mb": str(libre_mb), "x-ore-libres-hilos": str(hilos)})
    req = urllib.request.Request(SERVE + "/consultas/trabajo", headers=h)
    with urllib.request.urlopen(req, timeout=PLAZO_DE_PEDIR) as r:
        if r.status == 204:
            return None
        return json.loads(r.read() or b"null")


def entregar(id_, cuerpo):
    datos = json.dumps(cuerpo).encode("utf-8")
    h = dict(_cabeceras(), **{"content-type": "application/json"})
    req = urllib.request.Request("%s/consultas/%s/resultado" % (SERVE, id_), data=datos, headers=h, method="POST")
    for intento in range(3):
        try:
            with urllib.request.urlopen(req, timeout=30) as r:
                r.read()
                return True
        except urllib.error.HTTPError as e:
            log("%s · el resultado no se entregó: %s %s" % (id_, e.code, e.read()[:200].decode("utf-8", "replace")))
            return False
        except OSError as e:
            log("%s · el resultado no se entregó (intento %d): %s" % (id_, intento + 1, e))
            time.sleep(1 + intento * 2)
    return False


def correr(t, monton, ranuras):
    id_ = t.get("id", "?")
    memoria, hilos = int(t["memoria_mb"]), int(t["hilos"])
    derrame = os.path.join(TRABAJO, "consulta-" + "".join(c for c in id_ if c.isalnum() or c == "-"))
    t0 = time.time()
    con = None
    try:
        con = monton.tomar()
        t1 = time.time()
        tabla, truncada = calcular(con, t, derrame)
        t2 = time.time()
        datos, filas, truncada = a_arrow(tabla, truncada)
        cuerpo = {"ok": True, "arrow": base64.b64encode(datos).decode("ascii"), "filas": filas, "bytes": len(datos),
                  "truncada": truncada, "instancia_ms": int((t1 - t0) * 1000), "correr_ms": int((t2 - t1) * 1000)}
        log("%s · %s · %d filas%s · %d ms" % (id_, t.get("clase"), filas, " (cortada)" if truncada else "", (t2 - t0) * 1000))
    except NoSeCorre as e:
        cuerpo = {"ok": False, "codigo": e.codigo, "error": e.mensaje}
        log("%s · %s · %s" % (id_, t.get("clase"), e.codigo))
    except Exception as e:  # noqa: BLE001 — lo que sea, se dice a quien preguntó
        mensaje = str(e).splitlines()[0] if str(e) else type(e).__name__
        codigo = "consulta/sin-permiso" if isinstance(e, PermissionError) else "consulta/sql"
        cuerpo = {"ok": False, "codigo": codigo, "error": ore._sin_disco(e) or mensaje}
        log("%s · %s · %s" % (id_, t.get("clase"), codigo))
    finally:
        if con is not None:
            try:
                con.close()
            except Exception:  # noqa: BLE001
                pass
        shutil.rmtree(derrame, ignore_errors=True)
        ranuras.soltar(memoria, hilos)
    entregar(id_, cuerpo)


def servir(parar=None):
    """Pide trabajo mientras le quepa, y corre cada consulta en su hilo. Un solo
    hilo pide (con lo libre de ese momento) y reserva lo que toma antes de volver
    a pedir: dos peticiones no pueden prometer el mismo hueco."""
    parar = parar or threading.Event()
    monton = Monton(PREPARADAS)
    ranuras = Ranuras(MEMORIA_MB, HILOS)
    log("pide a %s · %d MB y %d hilos para consultas · %d instancias preparadas%s" % (
        SERVE, MEMORIA_MB, HILOS, PREPARADAS, " · se va tras %d s sin consultas" % OCIOSO_S if OCIOSO_S else ""))
    hilos = []
    while not parar.is_set():
        try:
            with open(VIVO, "w") as f:
                f.write(str(int(time.time())))
        except OSError:
            pass
        if OCIOSO_S and ranuras.ocioso() > OCIOSO_S:
            log("%d s sin consultas: me voy" % OCIOSO_S)
            break
        mb, h = ranuras.esperar_hueco(MINIMO_MB, parar)
        if parar.is_set():
            break
        try:
            t = pedir(mb, h)
        except urllib.error.HTTPError as e:
            log("ore-serve contestó %s: %s" % (e.code, e.read()[:200].decode("utf-8", "replace")))
            parar.wait(5)
            continue
        except (OSError, ValueError) as e:
            log("ore-serve no contesta: %s" % e)
            parar.wait(2)
            continue
        if not t:
            continue
        ranuras.reservar(int(t["memoria_mb"]), int(t["hilos"]))
        hilo = threading.Thread(target=correr, args=(t, monton, ranuras), name=t.get("id", "consulta"))
        hilo.start()
        hilos = [x for x in hilos if x.is_alive()] + [hilo]
    # Al pararlo (un despliegue), lo que corre termina y se entrega.
    for x in hilos:
        x.join()
    log("parado")


def main():
    parar = threading.Event()
    signal.signal(signal.SIGTERM, lambda *_: parar.set())
    servir(parar)


if __name__ == "__main__":
    main()
