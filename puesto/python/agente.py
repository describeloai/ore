#!/usr/bin/env python3
"""
EL AGENTE DEL PUESTO (0031 W3.1) — lo que corre dentro de la sesión Python.

Es un kernel pequeño y un cliente: pide trabajo a `ore-serve` por HTTP (polling
largo: el puesto no acepta conexiones, «sin entrada»), ejecuta cada celda en un
espacio de nombres que dura toda la sesión, y devuelve la salida TIPADA —tabla,
texto, error, vacía— al mismo sitio. La consola nunca habla con este proceso.

  GET  /puestos/{id}/pendiente            → 200 {pendiente, celda, lenguaje, texto}
  POST /puestos/{id}/celdas/{n}/salida    ← {tipo, ms, …}

Y desde 0037 ③a también lleva **el servidor de lenguaje** de su entorno —pyright
para Python— y hace de correa entre él y el editor:

  GET  /puestos/{id}/lsp/agente           → flujo de eventos: lo que el editor manda
  POST /puestos/{id}/lsp/salida           ← lo que el servidor de lenguaje contesta

El servidor de lenguaje corre AQUÍ y no en el navegador porque aquí están el SDK
(`/opt/ore/ore`) y la capa del repositorio (`/capa`): saber qué devuelve `over()`
sólo se puede saber donde vive `over`.

Y el de SQL (`ore.lsp_sql`), por lo mismo: su esquema es el índice del árbol y su
comprobador el DuckDB de este puesto. Ése corre DENTRO de este proceso, no como
otro: la correa le da lo que es de SQL y el resto sigue yendo a pyright.

Quién es: dentro del clúster, el agente de la celda (`ore-agente-<n>`, client
credentials contra el IdP, `rubix_tipo: agente`); en las pruebas, la cabecera
`x-ore-sujeto: agente:…` (`ORE_SUJETO`). `ore-serve` ata el puesto al primer
agente que lo reclama y contesta 403 a cualquier otro.

Cuándo muere: sin celdas durante `TTL` segundos (el TTL de inactividad es del
agente, no de la cola: medido en 0031), o cuando `ore-serve` dice que el puesto
está cerrado (410). El tope de sesión lo pone el Job (`activeDeadlineSeconds`).
"""
import ast
import contextlib
import io
import json
import os
import shlex
import shutil
import subprocess
import sys
import threading
import time
import traceback
import urllib.parse
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

# ⭐⭐ LA CAPA VA DETRÁS, Y ES UNA DECISIÓN (0031 W3.2 · el orden).
#
# Hasta hoy la capa se montaba con `PYTHONPATH=/capa`, y Python pone lo que hay
# ahí ANTES QUE TODO LO SUYO — medido: un `json.py` en la capa tapa hasta el de
# la biblioteca estándar, y un `pyarrow` de la capa tapa el de la imagen.
#
# ⛔ Eso es exactamente lo contrario de lo que decidimos para la JVM (0037 ③c),
#   y por un motivo que aquí es MÁS grave: el SDK está compilado contra el
#   `pyarrow` y el `duckdb` de la imagen —el contrato de tipos (0032) está
#   medido contra ellos, y las extensiones del lago están compiladas para esa
#   versión de DuckDB—. Y si la capa trae un `numpy` que no case con el
#   `pyarrow` de la imagen, el fallo no es una excepción: es un ABI binario mal
#   casado, que puede matar al intérprete de un segfault. Eso no deja celda con
#   error: deja pod muerto.
#
# ⇒ MANDA EL CONTENEDOR, igual que en la JVM. La capa AÑADE; no sustituye. Y el
#   Job que la resuelve tampoco copia ya lo que la imagen pone, así que esto es
#   el cinturón: si algo se colara, sigue ganando la imagen.
CAPA = os.environ.get("ORE_CAPA_DIR", "/capa")
if os.path.isdir(CAPA) and CAPA not in sys.path:
    sys.path.append(CAPA)
# ⭐ 0050 P2: y lo de desarrollo (el grupo `dev`: pytest, hypothesis, stubs),
#   DETRÁS de lo que corre. Sólo existe en una sesión: `traer-la-capa` no lo
#   baja para una invocación.
CAPA_DEV = os.path.join(CAPA, ".dev")
if os.path.isdir(CAPA_DEV) and CAPA_DEV not in sys.path:
    sys.path.append(CAPA_DEV)

import ore  # noqa: E402 — el SDK, al lado de este fichero

FILAS_MAXIMAS = 200


def log(*a):
    print("agente ·", *a, flush=True)


# ── Quién soy: el token ────────────────────────────────────────────────────
class Testigo:
    def __init__(self):
        self.sujeto = os.environ.get("ORE_SUJETO")
        self.direccion = os.environ.get("DIRECCION", "").rstrip("/")
        self.realm = os.environ.get("REALM", "rubix")
        self.cliente = self._fichero("agente-cliente")
        self.secreto = self._fichero("agente-secreto")
        self.token = None
        self.caduca = 0
        # 0049 B2·3: la cabecera la piden a la vez la celda (por el SDK), el
        # latido y el servidor de lenguaje: una renovación cada vez.
        self.candado = threading.Lock()

    @staticmethod
    def _token_del_pod():
        try:
            with open(os.environ.get("ORE_TOKEN_DEL_POD", "/var/run/ore/pod/token"), encoding="utf-8") as f:
                return f.read().strip() or None
        except OSError:
            return None

    @staticmethod
    def _fichero(nombre):
        ruta = os.path.join(os.environ.get("PUESTO_DIR", "/puesto"), nombre)
        try:
            with open(ruta, encoding="utf-8") as f:
                return f.read().strip()
        except OSError:
            return None

    def cabeceras(self):
        if self.sujeto:
            return {"x-ore-sujeto": self.sujeto}
        # ⭐ R1 · Si el pod trae su token (de Kubernetes, ligado a ESTE pod), se
        #   presenta con él: el servidor sabe qué puesto habla. Se lee cada vez
        #   porque el kubelet lo renueva en el sitio. Mientras el pod traiga
        #   TAMBIÉN la credencial del agente, van las dos: un `ore-serve` que no
        #   sabe de pods sigue con la de siempre, y uno que sí, con la del pod.
        pod = self._token_del_pod()
        if pod and not (self.cliente and self.secreto):
            return {"x-ore-pod": pod}
        with self.candado:
            h = self._cabeceras_vigentes()
        return dict(h, **{"x-ore-pod": pod}) if pod else h

    def _cabeceras_vigentes(self):
        if not (self.cliente and self.secreto and self.direccion):
            raise SystemExit("agente · sin identidad: ni ORE_SUJETO ni /puesto/agente-{cliente,secreto} con DIRECCION")
        if time.time() > self.caduca - 60:
            datos = urllib.parse.urlencode({"grant_type": "client_credentials", "client_id": self.cliente, "client_secret": self.secreto}).encode()
            with urllib.request.urlopen("%s/realms/%s/protocol/openid-connect/token" % (self.direccion, self.realm), data=datos, timeout=20) as r:
                t = json.load(r)
            self.token = t["access_token"]
            self.caduca = time.time() + int(t.get("expires_in", 300))
            log("token del agente renovado · caduca en %ds" % int(t.get("expires_in", 300)))
        return {"authorization": "Bearer " + self.token}


# ── El latido: vivo mientras trabaja (0049 B2·3) ───────────────────────────
class Latido:
    """Mientras corre una celda el agente no pide trabajo, y sin latido el puesto
    pasa a perdido a los 90 s. Esto lo da cada `CADA` segundos, en un hilo, hasta
    que la celda termina. Un latido que falla no para la celda: se dice y sigue."""

    CADA = 30

    def __init__(self, puesto):
        self.puesto = puesto
        self.parar = threading.Event()
        self.hilo = threading.Thread(target=self._latir, daemon=True)

    def __enter__(self):
        self.hilo.start()
        return self

    def __exit__(self, *_):
        self.parar.set()
        self.hilo.join(timeout=5)

    def _latir(self):
        while not self.parar.wait(self.CADA):
            try:
                codigo, r = self.puesto.pedir("POST", "/puestos/%s/latido" % self.puesto.id, plazo=10)
                if codigo not in (200, 204):
                    log("el latido contestó %s: %s" % (codigo, r))
            except Exception as e:  # noqa: BLE001 — la red se cae; el siguiente lo intenta
                log("el latido no llegó: %s" % e)


# ── El kernel: un espacio de nombres para toda la sesión ───────────────────
class Kernel:
    def __init__(self):
        self.espacio = {"__name__": "__main__", "over": ore.over, "sql": ore.sql, "write": ore.write, "declare": ore.declare, "transform": ore.transform, "person": ore.person, "persona": ore.person, "ore": ore}

    def correr(self, texto, lenguaje="python"):
        # 0055 B2: lo que la celda dejó para el informe (un build: filas,
        # snapshot, el error con su fichero y su línea), en su salida.
        tomar = getattr(ore, "_tomar_informe", None)
        if tomar:
            tomar()
        r = self._correr(texto, lenguaje)
        informe = tomar() if tomar else None
        if informe:
            r["informe"] = informe
        return r

    def _correr(self, texto, lenguaje="python"):
        t0 = time.time()
        salida = io.StringIO()
        valor = None
        # Una celda SQL (W3.3): la consulta entera va a `ore.sql`, sobre las copias.
        if lenguaje == "sql":
            try:
                with contextlib.redirect_stdout(salida), contextlib.redirect_stderr(salida):
                    valor = ore.sql(texto)
            except Exception as e:  # noqa: BLE001
                return {"tipo": "error", "nombre": type(e).__name__, "mensaje": str(e), "traza": traceback.format_exc(),
                        "texto": salida.getvalue(), "ms": ms(t0)}
            return self.salida_de(valor, salida.getvalue(), t0)
        try:
            arbol = ast.parse(texto, mode="exec")
            ultimo = None
            if arbol.body and isinstance(arbol.body[-1], ast.Expr):
                ultimo = ast.Expression(arbol.body.pop().value)
            with contextlib.redirect_stdout(salida), contextlib.redirect_stderr(salida):
                exec(compile(arbol, "<celda>", "exec"), self.espacio)
                if ultimo is not None:
                    valor = eval(compile(ultimo, "<celda>", "eval"), self.espacio)
        except Exception as e:  # noqa: BLE001 — la celda puede fallar como quiera
            traza = traceback.format_exc()
            return {"tipo": "error", "nombre": type(e).__name__, "mensaje": str(e), "traza": traza,
                    "texto": salida.getvalue(), "ms": ms(t0)}
        return self.salida_de(valor, salida.getvalue(), t0)

    @staticmethod
    def salida_de(valor, texto, t0):
        """La salida tipada de una celda, según su último valor (las salidas de
        una celda): una tabla (DataFrame, Series, Arrow) → `tabla`; un ítem de
        una colección (`MediaRef`, `Item`) o una lista de ellos → `media` (S2);
        una imagen (sus bytes, un `PIL.Image`, una figura de matplotlib, un
        `ore.File` de imagen) → `imagen` (S2); un `dict`, `list`, `tuple`,
        `set` o `@dataclass` → `json` (S1); nada → `texto` con lo impreso, o
        `vacia`; lo demás (un número, una cadena, un objeto), `texto` con su
        `repr`."""
        tabla = como_tabla(valor)
        if tabla is not None:
            tabla.update({"tipo": "tabla", "texto": texto, "ms": ms(t0)})
            return tabla
        for como, tipo in ((como_media, "media"), (como_imagen, "imagen")):
            s = como(valor)
            if s is not None:
                s.update({"tipo": tipo, "texto": texto, "ms": ms(t0)})
                return s
        arbol = como_json(valor)
        if arbol is not None:
            arbol.update({"tipo": "json", "texto": texto, "ms": ms(t0)})
            return arbol
        if valor is None:
            if texto.strip():
                return {"tipo": "texto", "texto": texto, "ms": ms(t0)}
            return {"tipo": "vacia", "ms": ms(t0)}
        return {"tipo": "texto", "texto": texto + repr(valor), "ms": ms(t0)}


def ms(t0):
    return int((time.time() - t0) * 1000)


def como_tabla(valor):
    """La salida `tabla` del contrato: la hace el SDK (`ore.table`), que es lo que
    una celda también puede pedir; aquí sólo se le pone el límite de la consola."""
    return ore.table(valor, FILAS_MAXIMAS)


# ── S1 · `json`: un valor compuesto, como árbol ─────────────────────────────
# Topes de lo que viaja (la salida se guarda en el historial de la celda): por
# nivel, por cadena, de hondo y en total. Lo que no cabe se recorta y se dice
# (`recortado`); si ni así cabe, la celda sale como texto.
JSON_POR_NIVEL = 500
JSON_CADENA = 5000
JSON_HONDO = 20
# ⛔ ore-serve admite como mucho 1 MB por cuerpo (`ore_entrada::http::CUERPO_MAXIMO`):
#   una salida mayor es un 413 y la celda se queda sin ella.
JSON_BYTES = 768 * 1024


def como_json(valor):
    """`{valor, recortado}` de un valor compuesto —`dict`, `list`, `tuple`,
    `set`, `@dataclass`—, o `None` si no lo es. Las hojas van como el JSON del
    contrato (`ore.to_json`: fechas, decimales, enteros grandes); los bytes, como
    `"bytes · N B"` (aquí se enseñan, no se copian); un objeto que no es JSON,
    por su `repr`."""
    import dataclasses

    if not (isinstance(valor, (dict, list, tuple, set, frozenset))
            or (dataclasses.is_dataclass(valor) and not isinstance(valor, type))):
        return None
    recortado = [False]

    def hoja(v):
        if isinstance(v, (bytes, bytearray, memoryview)):
            return "bytes · %d B" % len(bytes(v))
        if isinstance(v, str):
            if len(v) > JSON_CADENA:
                recortado[0] = True
                return v[:JSON_CADENA] + "… (%d more characters)" % (len(v) - JSON_CADENA)
            return v
        j = ore.to_json(v)
        if isinstance(j, (dict, list)):   # p. ej. un objeto con `.item()` que da un compuesto
            return a(j, 0)
        return j if (j is None or isinstance(j, (bool, int, float, str))) else repr(v)

    def a(v, hondo):
        if dataclasses.is_dataclass(v) and not isinstance(v, type):
            v = {f.name: getattr(v, f.name) for f in dataclasses.fields(v)}
        if isinstance(v, (set, frozenset)):
            v = sorted(v, key=repr)
        if isinstance(v, (dict, list, tuple)):
            if hondo >= JSON_HONDO:
                recortado[0] = True
                return "…"
            pares = list(v.items()) if isinstance(v, dict) else list(enumerate(v))
            sobran = len(pares) - JSON_POR_NIVEL
            if sobran > 0:
                recortado[0] = True
                pares = pares[:JSON_POR_NIVEL]
            if isinstance(v, dict):
                out = {str(k): a(x, hondo + 1) for k, x in pares}
                if sobran > 0:
                    out["…"] = "%d more keys" % sobran
                return out
            out = [a(x, hondo + 1) for _, x in pares]
            if sobran > 0:
                out.append("… %d more items" % sobran)
            return out
        if v is None or isinstance(v, (bool, int, float)) and not isinstance(v, type):
            return ore.to_json(v)
        if isinstance(v, (str, bytes, bytearray, memoryview)):
            return hoja(v)
        try:
            return hoja(v)
        except Exception:  # noqa: BLE001 — lo que no se sabe convertir, por su repr
            return repr(v)[:JSON_CADENA]

    arbol = a(valor, 0)
    if len(json.dumps(arbol, ensure_ascii=False, default=str)) > JSON_BYTES:
        return None
    return {"valor": arbol, "recortado": recortado[0]}


# ── S2 · `media` e `imagen` ─────────────────────────────────────────────────
MEDIA_MAXIMOS = 200
#: Lo que una imagen ocupa como mucho en la salida, en crudo (en base64, ~4/3):
#: por debajo del cuerpo máximo de ore-serve. Una mayor se reduce.
IMAGEN_BYTES = 512 * 1024
IMAGEN_LADO = 1600
_FIRMAS = ((b"\x89PNG\r\n\x1a\n", "image/png"), (b"\xff\xd8\xff", "image/jpeg"), (b"GIF87a", "image/gif"),
           (b"GIF89a", "image/gif"))


def tipo_de_imagen(b):
    """El tipo de unos bytes, si son una imagen que un navegador pinta."""
    for firma, tipo in _FIRMAS:
        if b.startswith(firma):
            return tipo
    if b[:4] == b"RIFF" and b[8:12] == b"WEBP":
        return "image/webp"
    cabeza = b[:256].lstrip().lower()
    if cabeza.startswith(b"<svg") or (cabeza.startswith(b"<?xml") and b"<svg" in b[:1024].lower()):
        return "image/svg+xml"
    return None


def como_media(valor):
    """`{items: [{collection, path, version, content_type, size, digest}]}` de un
    ítem de una colección —`MediaRef`, `Item`— o de una lista de ellos, o
    `None`. Sin URL ni bytes: la consola firma cada uno en la rama de la sesión
    al pintarlo (una URL firmada no se guarda en el historial de la celda)."""
    from ore.medios import Item, MediaRef

    def ref(v):
        if isinstance(v, Item):
            return v.ref
        return v if isinstance(v, MediaRef) else None

    uno = ref(valor)
    if uno is not None:
        refs, sobran = [uno], 0
    elif isinstance(valor, (list, tuple)) and valor and all(ref(v) is not None for v in valor):
        refs, sobran = [ref(v) for v in valor[:MEDIA_MAXIMOS]], max(0, len(valor) - MEDIA_MAXIMOS)
    else:
        return None
    items = [{"collection": r.collection, "path": r.path, "version": r.version, "content_type": r.content_type,
              "size": r.size, "digest": r.digest} for r in refs]
    return {"items": items, "total": len(items) + sobran, "recortado": sobran > 0}


def _bytes_de_imagen(valor):
    """`(bytes, tipo, nombre)` de lo que es una imagen, o `None`."""
    from ore.medios import File

    nombre = None
    if isinstance(valor, File):
        nombre, datos = valor.name, valor.data
        if isinstance(datos, (str, os.PathLike)):
            try:
                with open(datos, "rb") as f:
                    datos = f.read()
            except OSError:
                return None
        elif hasattr(datos, "read"):
            return None   # un fichero abierto: leerlo aquí lo consumiría
        valor = datos
    if isinstance(valor, (bytes, bytearray, memoryview)):
        b = bytes(valor)
        t = tipo_de_imagen(b)
        return (b, t, nombre) if t else None
    modulo = type(valor).__module__ or ""
    if modulo.startswith("PIL.") and hasattr(valor, "save") and hasattr(valor, "size"):
        out = io.BytesIO()
        valor.save(out, "PNG")
        return out.getvalue(), "image/png", nombre
    if modulo.startswith("matplotlib.") and hasattr(valor, "savefig"):
        out = io.BytesIO()
        valor.savefig(out, format="png", dpi=100, bbox_inches="tight")
        return out.getvalue(), "image/png", nombre
    return None


def como_imagen(valor):
    """`{mime, base64, ancho, alto, bytes, nombre, reducida}` de una imagen, o
    `None`. Una que pasa de `IMAGEN_BYTES` se reduce (a `IMAGEN_LADO` de lado, y
    a JPEG si hace falta) y se dice; si ni así cabe, no es imagen."""
    import base64

    r = _bytes_de_imagen(valor)
    if r is None:
        return None
    b, tipo, nombre = r
    original, ancho, alto, reducida = len(b), None, None, False
    try:
        from PIL import Image
        if tipo != "image/svg+xml":
            im = Image.open(io.BytesIO(b))
            ancho, alto = im.size
            if len(b) > IMAGEN_BYTES:
                im.thumbnail((IMAGEN_LADO, IMAGEN_LADO))
                for formato, mime, lado in (("PNG", "image/png", IMAGEN_LADO), ("JPEG", "image/jpeg", IMAGEN_LADO),
                                            ("JPEG", "image/jpeg", 800)):
                    im.thumbnail((lado, lado))
                    out = io.BytesIO()
                    (im.convert("RGB") if formato == "JPEG" else im).save(out, formato, quality=85)
                    if out.tell() <= IMAGEN_BYTES:
                        b, tipo, reducida = out.getvalue(), mime, True
                        break
    except Exception:  # noqa: BLE001 — sin PIL, o unos bytes que no abre: va tal cual
        pass
    if len(b) > IMAGEN_BYTES:
        return None
    out = {"mime": tipo, "base64": base64.b64encode(b).decode("ascii"), "ancho": ancho, "alto": alto,
           "bytes": original, "reducida": reducida}
    if nombre:
        out["nombre"] = nombre
    return out


def llano(v):
    """Un valor suelto (el resultado de una celda que no es tabla) → JSON."""
    return ore.to_json(v)


# ── El bucle ───────────────────────────────────────────────────────────────
# ═══════════════════════════════════════════════════════════════════════════
# EL SERVIDOR DE LENGUAJE, Y LA CORREA (0037 ③a)
# ═══════════════════════════════════════════════════════════════════════════
#
# ⭐ Aquí no se entiende LSP tampoco. Lo que llega por el flujo se escribe tal
#   cual en la entrada del servidor de lenguaje —con su cabecera
#   `Content-Length`, que es como se enmarca un mensaje de LSP— y lo que el
#   servidor escribe se manda tal cual. La correa no opina.
#
# ⛔ Y NO ARRANCA SOLO. Un pyright son ~210 MB medidos: si nadie abre un fichero
#   en el editor, esta sesión no los paga. El primer mensaje lo enciende.

# Lo que se arranca, y cómo. Se puede cambiar por el entorno: es lo que deja que
# la prueba de fuego ponga un servidor de mentira y ejercite LA CORREA sin
# descargar nada.
LSP = os.environ.get("ORE_LSP", "node /opt/ore/pyright/langserver.index.js --stdio")


class Correa:
    """Entre el flujo de `ore-serve` y el servidor de lenguaje del puesto."""

    def __init__(self, puesto, testigo):
        self.p = puesto
        self.testigo = testigo
        # ⭐ 0050 P4: el repositorio en disco, sus pruebas y sus tipos.
        from repositorio import Repositorio
        self.repo = Repositorio(puesto, testigo)
        # La última vez que el editor habló: también es trabajar (el TTL).
        self.actividad = 0.0
        self.proceso = None
        self.sql = None
        self.entregando = False
        self.salientes = []
        self.candado = threading.Lock()
        self.vivo = False

    # ── el proceso ──────────────────────────────────────────────────────────
    def encender(self):
        if self.proceso is not None:
            return self.proceso.poll() is None
        # ⛔ `shlex` y no `split()`: la orden puede traer una ruta entrecomillada
        #   —`node "/opt/ore/pyright/langserver.index.js" --stdio`— y partir por
        #   espacios le deja las comillas dentro. Medido: el servidor arrancaba
        #   y se moria al instante, y el editor se quedaba sin ayuda sin decir
        #   por que.
        orden = shlex.split(LSP)
        # P4: con el repositorio en disco, pyright resuelve lo que un fichero
        # importa de otro del repositorio (`from example import …`).
        try:
            with self.repo.candado:
                self.repo.materializar()
        except Exception as e:  # noqa: BLE001 — sin él, pyright sigue con lo abierto
            log("sin repositorio en disco (%s)" % e)
        if not shutil.which(orden[0]):
            log("no hay servidor de lenguaje (`%s`): el editor se queda sin ayuda" % orden[0])
            self.proceso = False
            return False
        try:
            self.proceso = subprocess.Popen(
                orden, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL, cwd=os.environ.get("TRABAJO_DIR", "/trabajo"))
        except OSError as e:
            log("el servidor de lenguaje no arranca (%s)" % e)
            self.proceso = False
            return False
        log("servidor de lenguaje arrancado: %s" % LSP)
        threading.Thread(target=self._leer_del_servidor, daemon=True).start()
        self._arrancar_entregas()
        return True

    def _arrancar_entregas(self):
        with self.candado:
            if self.entregando:
                return
            self.entregando = True
        threading.Thread(target=self._entregar, daemon=True).start()

    # ── el de SQL, en este proceso ──────────────────────────────────────────
    # ⭐ El reparto: lo que el cliente de SQL del editor manda (id `sql:n`, una
    #   uri `.sql`, su `initialize`) va a `ore.lsp_sql`; lo demás, a pyright.
    #   Medido en `pruebas-de-fuego/prototipo-lsp-sql/canal.py`: con dos
    #   clientes sobre el mismo flujo, nada se cruza.
    def _sql(self):
        if self.sql is None:
            from ore import lsp_sql
            self.sql = lsp_sql.Servidor(cargar=self._indice, mandar=self._encolar, log=log)
            self._arrancar_entregas()
            log("servidor de SQL en marcha (en este proceso)")
        return self.sql

    def _encolar(self, texto):
        with self.candado:
            self.salientes.append(texto)

    def _indice(self):
        """El índice del árbol en la rama del puesto (`GET /assets`)."""
        self.p._cabeceras = self.testigo.cabeceras()
        c, ficha = self.p.pedir("GET", "/puestos/%s" % self.p.id)
        rama = (ficha or {}).get("rama") if c == 200 else None
        c, j = self.p.pedir("GET", "/assets", cabeceras={"x-ore-rama": rama} if rama else None, plazo=60)
        if c != 200:
            raise RuntimeError("GET /assets contestó %s: %s" % (c, j))
        return j

    def escribir(self, mensaje):
        """Un mensaje del editor, hacia su servidor de lenguaje."""
        if os.environ.get("ORE_TRAZA_LSP"):
            log("TRAZA recibe %.3f %s" % (time.time(), mensaje[:80]))
        self.actividad = time.time()
        try:
            m = json.loads(mensaje)
        except ValueError:
            m = None
        # ⭐ 0050 P4: las peticiones propias, en su hilo: unas pruebas no
        #   detienen un hover, ni esperan en la cola de pyright.
        if isinstance(m, dict) and m.get("method") in ("ore/probar", "ore/comprobar"):
            threading.Thread(target=self._propia, args=(m,), daemon=True).start()
            return
        if isinstance(m, dict):
            self.repo.espejo(m)
        if isinstance(m, dict):
            from ore import lsp_sql
            if lsp_sql.es_sql(m):
                try:
                    self._sql().atender(m)
                except Exception as e:  # noqa: BLE001 — el editor no se queda colgado
                    log("el servidor de SQL falló: %s" % e)
                return
        if not self.encender():
            return
        b = mensaje.encode("utf-8")
        try:
            self.proceso.stdin.write(b"Content-Length: %d\r\n\r\n" % len(b) + b)
            self.proceso.stdin.flush()
        except OSError as e:
            log("el servidor de lenguaje se fue (%s)" % e)
            self.proceso = None

    def _propia(self, m):
        if m["method"] == "ore/probar":
            result = self.repo.probar(m.get("params") or {})
        else:
            result = self.repo.comprobar()
        self._encolar(json.dumps({"jsonrpc": "2.0", "id": m.get("id"), "result": result}, ensure_ascii=False))
        self._arrancar_entregas()

    def _leer_del_servidor(self):
        """Lo que el servidor contesta, a la cola de salida."""
        f = self.proceso.stdout
        while True:
            largo = None
            while True:
                linea = f.readline()
                if not linea:
                    log("el servidor de lenguaje cerró su salida")
                    return
                linea = linea.strip()
                if not linea:
                    break
                if linea.lower().startswith(b"content-length:"):
                    largo = int(linea.split(b":")[1])
            if largo is None:
                continue
            cuerpo = f.read(largo).decode("utf-8", "replace")
            with self.candado:
                self.salientes.append(cuerpo)

    def _entregar(self):
        """Cada 20 ms, lo que haya, DE GOLPE.

        ⛔ Un mensaje por petición serían decenas de conexiones por segundo
          —`ore-entrada` no tiene keep-alive y admite 64 a la vez—: teclear
          dejaría al inquilino sin plazas. Se agrupan.
        """
        while True:
            time.sleep(0.02)
            with self.candado:
                lote, self.salientes = self.salientes, []
            if not lote:
                continue
            self.p._cabeceras = self.testigo.cabeceras()
            if os.environ.get("ORE_TRAZA_LSP"):
                log("TRAZA entrega %.3f %d: %s" % (time.time(), len(lote), " | ".join(x[:60] for x in lote)))
            try:
                codigo, r = self.p.pedir(
                    "POST", "/puestos/%s/lsp/salida" % self.p.id, cuerpo={"mensajes": lote})
                if codigo not in (200, 201, 202):
                    log("la salida del servidor de lenguaje no se aceptó: %s %s" % (codigo, r))
            except Exception as e:  # noqa: BLE001 — la red se cae; se sigue
                log("no pude entregar %d mensajes del servidor de lenguaje: %s" % (len(lote), e))

    # ── el flujo de entrada ─────────────────────────────────────────────────
    def escuchar(self):
        """El flujo de `ore-serve`: lo que el editor manda, según lo manda.

        Se reconecta sola: el servidor se despide a los 240 s («vuelve») y lo
        que se recoge por aquí SE CONSUME, así que no hay nada que retomar.
        """
        self.vivo = True
        while self.vivo:
            try:
                self._una_vuelta()
            except Exception as e:  # noqa: BLE001
                log("el flujo del servidor de lenguaje se cortó (%s): vuelvo en 3 s" % e)
                time.sleep(3)

    def _una_vuelta(self):
        req = urllib.request.Request(
            "%s/puestos/%s/lsp/agente" % (self.p.servidor, self.p.id), method="GET")
        req.add_header("accept", "text/event-stream")
        for k, v in self.testigo.cabeceras().items():
            req.add_header(k, v)
        with urllib.request.urlopen(req, timeout=300) as r:
            evento = None
            for linea in r:
                linea = linea.decode("utf-8", "replace").rstrip("\n").rstrip("\r")
                if linea.startswith("event: "):
                    evento = linea[7:]
                elif linea.startswith("data: "):
                    if evento == "lsp":
                        self.escribir(linea[6:])
                    elif evento == "fin":
                        return


def main():
    p = ore.session
    if not p.id:
        raise SystemExit("agente · sin PUESTO en el entorno")
    ttl = int(os.environ.get("TTL", "1800"))
    # Un trabajo (W3.7 ④): `TRABAJO=<ruta>@<commit>` → una sola celda (el
    # fichero) y fuera, con el resultado como código de salida. El SDK lo
    # deja como `codigo` en la procedencia de lo que escriba.
    trabajo = os.environ.get("TRABAJO", "").strip()
    if trabajo:
        os.environ["ORE_CODIGO"] = trabajo
    testigo = Testigo()
    # 0049 B2·3: el SDK pide la cabecera al testigo en cada petición, también en
    # mitad de una celda; las asignaciones de `_cabeceras` de abajo quedan como
    # respaldo de lo que no pase por `pedir`.
    p._proveedor = testigo.cabeceras
    kernel = Kernel()
    # La correa escucha desde el principio; el servidor de lenguaje no arranca
    # hasta que llega el primer mensaje (un pyright son ~210 MB).
    correa = None
    if not trabajo:
        correa = Correa(p, testigo)
        threading.Thread(target=correa.escuchar, daemon=True).start()
    log("%s %s · ore-serve %s · TTL %ds · almacén %s" % ("trabajo" if trabajo else "puesto", p.id, p.servidor, ttl, p.almacen))
    ultimo = time.time()
    while True:
        # P4: lo que el editor pide (escribir, probar, comprobar) también cuenta.
        if time.time() - max(ultimo, correa.actividad if correa else 0) > ttl:
            log("sin actividad (celdas ni editor) durante %ds: cierro" % ttl)
            # Fuera de la cola: si no, Flux recrea el Job (el servidor barre
            # igualmente lo perdido; esto lo adelanta).
            try:
                p._cabeceras = testigo.cabeceras()
                p.pedir("POST", "/puestos/%s/cierre" % p.id)
            except Exception as e:  # noqa: BLE001
                log("no se pudo notificar el cierre: %s" % e)
            return 0
        p._cabeceras = testigo.cabeceras()
        try:
            codigo, r = p.pedir("GET", "/puestos/%s/pendiente" % p.id, plazo=40)
        except Exception as e:  # noqa: BLE001 — la red se cae; se reintenta
            log("ore-serve no contesta (%s): reintento en 5 s" % e)
            time.sleep(5)
            continue
        # Sin celda en la espera: `{pendiente: false}` (era `not r`, y un dict
        # con `pendiente: false` NO está vacío: el agente moría con KeyError
        # al primer poll vacío, y el puesto se daba por perdido a los 90 s).
        if codigo == 200 and not (r and r.get("pendiente")):
            continue
        if codigo == 410:
            log("el puesto está cerrado: adiós")
            return 0
        if codigo != 200:
            log("pendiente contestó %s: %s · reintento en 5 s" % (codigo, r))
            time.sleep(5)
            continue
        if not p.person:
            c2, ficha = p.pedir("GET", "/puestos/%s" % p.id)
            if c2 == 200 and (ficha or {}).get("persona"):
                p.person = ficha["persona"]
                log("el puesto es de %s" % p.person)
        n, texto, lenguaje = r["celda"], r.get("texto", ""), r.get("lenguaje") or "python"
        log("celda %s · %s · %d bytes" % (n, lenguaje, len(texto)))
        with Latido(p):
            salida = kernel.correr(texto, lenguaje)
        ultimo = time.time()
        p._cabeceras = testigo.cabeceras()
        try:
            codigo, r2 = p.pedir("POST", "/puestos/%s/celdas/%s/salida" % (p.id, n), cuerpo=salida)
            if codigo not in (200, 201):
                log("la salida de la celda %s no se aceptó: %s %s" % (n, codigo, r2))
        except Exception as e:  # noqa: BLE001
            log("no pude entregar la salida de la celda %s: %s" % (n, e))
        log("celda %s · %s · %d ms" % (n, salida.get("tipo"), salida.get("ms", 0)))
        if trabajo:
            log("trabajo %s: %s" % (trabajo, "error" if salida.get("tipo") == "error" else "hecho"))
            return 1 if salida.get("tipo") == "error" else 0


if __name__ == "__main__":
    sys.exit(main())
