#!/usr/bin/env python3
"""UNA ORG DE SALESFORCE DE MENTIRA (ADR 0083 paso 5).

No hay emulador de Salesforce: esto es una org hecha de lo que X0 midió contra
la de verdad (ADR 0083 §3), para que el CI ejerza `ore-read-salesforce` sin
cuenta, sin cuota y con casos que una org real no deja provocar (cuota agotada,
un 503 cada N peticiones, un token que caduca, un `Date` adelantado, 10⁶ filas,
un borrado fuera de la Papelera).

Lo que contesta (todo bajo `/services/data/v<versión>/`, con el token de portador
que da `POST /services/oauth2/token`):

  limits · sobjects · sobjects/X/describe · sobjects/X/deleted
  query · queryAll · query/<cursor>       (REST; páginas de PAGINA)
  jobs/query (Bulk 2.0)                    (CSV; locator; abortar)

más `POST/PATCH/DELETE /sobjects/X` (lo que usan las pruebas de cambios), y un intérprete de SOQL con las RAREZAS MEDIDAS, que es lo que lo hace útil:

  · `=`, `LIKE` e `IN` sobre texto NO distinguen mayúsculas (X0 B2)
  · `!= 5` INCLUYE los nulos (X0 B3)
  · sin NULLS, el orden pone los nulos PRIMERO en ASC y en DESC (X0 B5)
  · `NOT` solo entre paréntesis; AND y OR mezclados sin paréntesis, error
  · un campo no filtrable / no ordenable, un error de verdad (INVALID_FIELD)
  · la URL de un GET de más de 16 KB, un 431 (X0 B8)
  · el mensaje de error empieza por un salto de línea, como el de Salesforce
  · SystemModstamp con precisión de SEGUNDOS: varias filas con el mismo sello
  · un `datetime` guarda SEGUNDOS (escribes `…:00.500`, devuelve `…:00.000`) y un
    literal de SOQL IGNORA la fracción: `= …17.999Z` casa con `…17.000` (2026-11-01)
  · REST escribe la fecha `…000+0000` y Bulk `…000Z`; el nulo, `null` y `""`
    (Bulk lo entrecomilla: el bug que solo cazó la org real)
  · getDeleted trunca los segundos y no ve más allá de la Papelera (15 días)
  · cada petición cuenta en la cuota y trae `Sforce-Limit-Info: api-usage=n/max`

Se configura con el entorno:

  PAGINA        filas por página REST (2000; `Sforce-Query-Options: batchSize`
                la baja si la pide menos, nunca la sube)
  CADA_503      cada N peticiones de datos, un 503 con `Retry-After: 1` (0 = no)
  CUOTA_MAX     el máximo de la cuota diaria (15000)
  JOB_MS        lo que tarda un job de Bulk en completarse (300)
  LENTA_S       lo que tarda un objeto «lento» en dar su primera fila (9)
  SF_PUBLICA    la clave PÚBLICA (PEM): con ella se VERIFICA la firma del JWT con
                `openssl`; sin ella solo se mira su forma
  OPENSSL       el binario de `openssl` (por defecto, el del PATH)
  SF_USUARIOS   los usuarios aprobados, separados por comas (`kit@lab.ore`)
  SF_ENDPOINT   `http://127.0.0.1:<puerto>` (la `aud` que el JWT debe llevar)

La API de siembra y de maldades (`/_mentira/…`, sin token, JSON):

  POST objeto     crea o rehace un objeto {nombre, campos, filas | generar}
  POST fila       {objeto, valores}                       (un alta)
  POST actualizar {objeto, donde:{campo:valor}, valores}  (mueve SystemModstamp)
  POST borrar     {objeto, donde:{campo:valor}}           (a la Papelera)
  POST fallos     {cada_503}         cambia el 503 periódico en caliente
  POST reloj      {desfase_s}        la hora de la org va adelantada o atrasada
  POST cuota      {usadas, max}      para agotarla
  POST caducar    todos los tokens caducan (el siguiente uso es un 401)
  POST papelera   {dias}             hasta dónde llega getDeleted
  POST lenta      {segundos}
  GET  cuentas    contadores: peticiones, canjes, sin_token, escrituras, vivas
  GET  vivas?objeto=X   consultas que siguen vivas sobre X

Arranque: `python3 salesforce-de-mentira.py <puerto>`; escribe `listo` al
terminar de arrancar.
"""
import base64
import json
import os
import re
import select
import socket
import subprocess
import sys
import tempfile
import threading
import time
import uuid
from decimal import Decimal, InvalidOperation
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit

VERSION = "68.0"
PREFIJO = f"/services/data/v{VERSION}"
PAGINA = int(os.environ.get("PAGINA", "2000"))
CADA_503 = int(os.environ.get("CADA_503", "0"))
JOB_MS = int(os.environ.get("JOB_MS", "300"))
LENTA_S = float(os.environ.get("LENTA_S", "9"))
USUARIOS = {u for u in os.environ.get("SF_USUARIOS", "kit@lab.ore,otro@lab.ore").split(",") if u}
PUBLICA = os.environ.get("SF_PUBLICA", "")
OPENSSL = os.environ.get("OPENSSL", "openssl")
MAX_URL = 16000  # X0 B8: ~8 KB pasa, 16 KB da 431

CERRADURA = threading.RLock()


# ── la hora de la org ─────────────────────────────────────────────────────────
class Estado:
    desfase_s = 0.0
    usadas = 0
    max = int(os.environ.get("CUOTA_MAX", "15000"))
    papelera_dias = 15
    cada_503 = CADA_503
    jobs_creados = 0
    lenta_s = LENTA_S
    epoca_tokens = 0
    peticiones_datos = 0
    canjes = 0
    sin_token = 0
    con_token = 0
    escrituras = 0
    micros_truncados = 0
    lecturas = 0
    tokens = {}  # token -> (usuario, epoca)
    cursores = {}  # id -> estado de paginación REST
    jobs = {}  # id -> job de Bulk
    vivas = {}  # objeto -> consultas que siguen vivas
    objetos = {}  # nombre en minúsculas -> Objeto
    endpoint = os.environ.get("SF_ENDPOINT", "")


def ahora_ms():
    return int((time.time() + Estado.desfase_s) * 1000)


def sello_ms():
    """Lo que escribe SystemModstamp: la hora de la org, TRUNCADA a segundos (X0 D1)."""
    return ahora_ms() // 1000 * 1000


# ── fechas y números, como los escribe Salesforce ─────────────────────────────
def _civil(dias):
    z = dias + 719468
    era = z // 146097
    doe = z - era * 146097
    yoe = (doe - doe // 1460 + doe // 36524 - doe // 146096) // 365
    y = yoe + era * 400
    doy = doe - (365 * yoe + yoe // 4 - yoe // 100)
    mp = (5 * doy + 2) // 153
    d = doy - (153 * mp + 2) // 5 + 1
    m = mp + 3 if mp < 10 else mp - 9
    return (y + 1 if m <= 2 else y), m, d


def _dias(a, m, d):
    a = a - 1 if m <= 2 else a
    era = a // 400
    yoe = a - era * 400
    mp = (m + 9) % 12
    doy = (153 * mp + 2) // 5 + d - 1
    doe = yoe * 365 + yoe // 4 - yoe // 100 + doy
    return era * 146097 + doe - 719468


def texto_dt(ms, sufijo):
    s, milis = divmod(ms, 1000)
    y, m, d = _civil(s // 86400)
    r = s % 86400
    return f"{y:04d}-{m:02d}-{d:02d}T{r // 3600:02d}:{r % 3600 // 60:02d}:{r % 60:02d}.{milis:03d}{sufijo}"


def texto_http(ms):
    dias = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"]
    mes = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
    s = ms // 1000
    y, m, d = _civil(s // 86400)
    r = s % 86400
    return f"{dias[(s // 86400) % 7]}, {d:02d} {mes[m - 1]} {y:04d} {r // 3600:02d}:{r % 3600 // 60:02d}:{r % 60:02d} GMT"


RE_DT = re.compile(r"^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.(\d+))?(Z|[+-]\d{2}:?\d{2})?$")
RE_D = re.compile(r"^(\d{4})-(\d{2})-(\d{2})$")


def ms_de(texto, cuenta_micros=True):
    """Un instante en texto (`…Z`, `…+02:00`, sin zona = UTC) → ms de época."""
    m = RE_DT.match(texto)
    if not m:
        raise ValueError(f"`{texto}` no es un instante")
    a, mo, d, h, mi, s, frac, zona = m.groups()
    ms = 0
    if frac:
        if len(frac) > 3 and cuenta_micros:
            Estado.micros_truncados += 1  # Salesforce guarda milisegundos
        ms = int((frac + "000")[:3])
    t = (_dias(int(a), int(mo), int(d)) * 86400 + int(h) * 3600 + int(mi) * 60 + int(s)) * 1000 + ms
    if zona and zona != "Z":
        signo = 1 if zona[0] == "+" else -1
        z = zona[1:].replace(":", "")
        t -= signo * (int(z[:2]) * 3600 + int(z[2:]) * 60) * 1000
    return t


def java_double(f):
    """Un double como lo escribe Java: `20.0`, `1.0E10`, `1.5E-4`."""
    if f == int(f) and abs(f) < 1e7:
        return f"{int(f)}.0"
    a = abs(f)
    if 1e-3 <= a < 1e7:
        return repr(f)
    s = f"{f:.17e}"
    mant, exp = s.split("e")
    mant = mant.rstrip("0").rstrip(".")
    if "." not in mant:
        mant += ".0"
    # La mantisa más corta que vuelve al mismo número.
    for n in range(1, 18):
        c = f"{f:.{n}e}"
        if float(c) == f:
            mant, exp = c.split("e")
            if "." not in mant:
                mant += ".0"
            break
    return f"{mant}E{int(exp)}"


def decimal_texto(d):
    t = format(d.normalize() if d != 0 else Decimal(0), "f")
    if "." not in t:
        t += ".0"
    return t


# ── el modelo ─────────────────────────────────────────────────────────────────
NUMERICOS = ("int", "long", "double", "currency", "percent")
TEXTOS = ("string", "picklist", "textarea", "email", "phone", "url", "combobox", "multipicklist")


class Falla(Exception):
    def __init__(self, estado, codigo, mensaje, cabeceras=None):
        super().__init__(mensaje)
        self.estado, self.codigo, self.mensaje = estado, codigo, mensaje
        self.cabeceras = cabeceras or {}


def campo_estandar(nombre, tipo, etiqueta, **mas):
    c = {
        "name": nombre, "type": tipo, "label": etiqueta, "nillable": False,
        "filterable": True, "sortable": True, "calculated": False, "custom": False,
    }
    c.update(mas)
    return c


def campos_de_sistema():
    return [
        campo_estandar("Id", "id", "Record ID"),
        campo_estandar("IsDeleted", "boolean", "Deleted"),
        campo_estandar("Name", "string", "Name", length=80),
        campo_estandar("CreatedDate", "datetime", "Created Date"),
        campo_estandar("LastModifiedDate", "datetime", "Last Modified Date"),
        campo_estandar("SystemModstamp", "datetime", "System Modstamp"),
    ]


def nuevo_id(prefijo, n):
    # 15 caracteres + 3 de sufijo: el aspecto de un Id de 18, no su checksum.
    return f"{prefijo}{n:012d}AAA"


class Objeto:
    def __init__(self, nombre, campos, propio=True, replicable=True, etiqueta=None, prefijo="a0Z", sin_papelera=False):
        self.nombre = nombre
        self.propio = propio
        self.replicable = replicable
        self.etiqueta = etiqueta or nombre
        self.prefijo = prefijo
        self.sin_papelera = sin_papelera
        sistema = campos_de_sistema()  # también los objetos estándar (Account) los tienen
        nombres = {c["name"].lower() for c in sistema}
        self.campos = sistema + [c for c in campos if c["name"].lower() not in nombres]
        self.por_nombre = {c["name"].lower(): c for c in self.campos}
        self.filas = []  # dicts
        self.generada = None  # (n, función i -> dict)
        self.lenta = False
        self.consultable = True
        self.siguiente = 1

    def __len__(self):
        return self.generada[0] if self.generada else len(self.filas)

    def fila(self, i):
        return self.generada[1](i) if self.generada else self.filas[i]

    def describe(self):
        return {
            "name": self.nombre, "label": self.etiqueta, "custom": self.propio,
            "queryable": self.consultable, "replicateable": self.replicable, "retrieveable": True,
            "createable": False, "updateable": False, "deletable": False,
            "fields": [dict(c, **{"nillable": c.get("nillable", True)}) for c in self.campos],
        }

    def resumen(self):
        return {
            "name": self.nombre, "label": self.etiqueta, "custom": self.propio,
            "queryable": self.consultable, "replicateable": self.replicable,
            "urls": {"sobject": f"{PREFIJO}/sobjects/{self.nombre}"},
        }


# ── de texto canónico a lo que guarda Salesforce (y lo que pierde) ────────────
def guardar(campo, texto):
    """El valor de la semilla (texto) → lo que la org guarda. Lo que una org real
    rechazaría, falla (no se redondea ni se recorta en silencio)."""
    t = campo["type"]
    nombre = campo["name"]
    if texto is None:
        return None
    if t in TEXTOS or t in ("id", "reference"):
        # Salesforce guarda la cadena vacía como NULL.
        if texto == "":
            return None
        largo = campo.get("length")
        if largo and len(texto) > largo:
            raise Falla(400, "STRING_TOO_LONG", f"{nombre}: data value too large ({len(texto)} > {largo})")
        return texto
    if t in ("int", "long"):
        v = int(texto)
        lim = 2**31 if t == "int" else 2**63
        if not -lim <= v < lim:
            raise Falla(400, "NUMBER_OUTSIDE_VALID_RANGE", f"{nombre}: {texto} fuera de rango para {t}")
        return v
    if t in ("double", "currency", "percent"):
        p, e = campo.get("precision", 0), campo.get("scale", 0)
        if p == 0:
            return float(texto)
        try:
            d = Decimal(texto)
        except InvalidOperation:
            raise Falla(400, "INVALID_TYPE_ON_FIELD_IN_RECORD", f"{nombre}: `{texto}` no es un número")
        if abs(d) >= Decimal(10) ** (p - e):
            raise Falla(400, "NUMBER_OUTSIDE_VALID_RANGE", f"{nombre}: {texto} no cabe en ({p},{e})")
        return d.quantize(Decimal(1).scaleb(-e))
    if t == "boolean":
        return texto.lower() == "true"
    if t == "date":
        m = RE_D.match(texto)
        if not m or not 1700 <= int(m.group(1)) <= 4000:
            raise Falla(400, "INVALID_TYPE_ON_FIELD_IN_RECORD", f"{nombre}: `{texto}` no es una fecha válida")
        return texto
    if t == "datetime":
        # Un instante, siempre: una hora local sin zona se toma por UTC; y se guardan
        # SEGUNDOS: le escribes `…:00.500` y devuelve `…:00.000` (medido en una org real).
        return ms_de(texto, cuenta_micros=False) // 1000 * 1000
    raise Falla(400, "INVALID_TYPE", f"{nombre}: la mentira no guarda `{t}`")


def a_texto(campo, v, bulk):
    """Un valor guardado → el texto que lo escribe REST (JSON) o Bulk (CSV)."""
    t = campo["type"]
    if v is None:
        return None
    if t == "datetime":
        return texto_dt(v, "Z" if bulk else "+0000")
    if t == "boolean":
        return "true" if v else "false"
    if isinstance(v, Decimal):
        return decimal_texto(v)
    if isinstance(v, float):
        return java_double(v)
    return str(v)


def celda_csv(v):
    """Una celda de Bulk. El nulo es `""` ENTRECOMILLADO, no una celda vacía: el bug
    que solo cazó la org real (ADR 0083 §3 C11)."""
    if v is None:
        return '""'
    if any(c in v for c in (",", '"', "\n", "\r")):
        return '"' + v.replace('"', '""') + '"'
    return v


def json_valor(campo, v):
    t = campo["type"]
    if v is None:
        return "null"
    if t == "boolean":
        return "true" if v else "false"
    if t in NUMERICOS:
        return a_texto(campo, v, False)
    return json.dumps(a_texto(campo, v, False), ensure_ascii=False)


# ── SOQL ──────────────────────────────────────────────────────────────────────
class Malo(Exception):
    pass


RE_TOK = re.compile(
    r"""\s*(?:
     (?P<dt>\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2}))
    |(?P<d>\d{4}-\d{2}-\d{2})(?![\w])
    |(?P<num>-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)
    |(?P<str>'(?:[^'\\]|\\.)*')
    |(?P<op><=|>=|!=|<>|=|<|>)
    |(?P<p>[(),])
    |(?P<id>[A-Za-z_][A-Za-z0-9_.]*)
    )""",
    re.X,
)
ESCAPES = {"n": "\n", "r": "\r", "t": "\t", "b": "\b", "f": "\f", '"': '"', "'": "'", "\\": "\\"}


def desescapar(s):
    """`'O\\'Brien'` → `O'Brien`. `\\_` y `\\%` se quedan: son del LIKE."""
    sal, i = [], 1
    while i < len(s) - 1:
        c = s[i]
        if c == "\\":
            n = s[i + 1]
            sal.append(ESCAPES[n] if n in ESCAPES else "\\" + n)
            i += 2
        else:
            sal.append(c)
            i += 1
    return "".join(sal)


def tokens(q):
    sal, pos = [], 0
    while pos < len(q):
        if not q[pos:].strip():
            break
        m = RE_TOK.match(q, pos)
        if not m or m.end() == pos:
            raise Malo(f"unexpected token: '{q[pos:pos + 12].strip()}'")
        pos = m.end()
        tipo = m.lastgroup
        sal.append((tipo, m.group(tipo)))
    return sal


class Consulta:
    def __init__(self, soql, objetos):
        self.soql = soql
        self.t = tokens(soql)
        self.i = 0
        self.campos = []
        self.donde = None
        self.orden = []
        self.limite = None
        self._parsear(objetos)

    # lector
    def ver(self):
        return self.t[self.i] if self.i < len(self.t) else (None, None)

    def kw(self, *palabras):
        tipo, v = self.ver()
        return tipo == "id" and v.upper() in palabras

    def tomar(self):
        x = self.ver()
        self.i += 1
        return x

    def esperar_kw(self, p):
        if not self.kw(p):
            raise Malo(f"unexpected token: '{self.ver()[1]}'" if self.ver()[1] else "unexpected end of query")
        self.i += 1

    def _parsear(self, objetos):
        self.esperar_kw("SELECT")
        while True:
            tipo, v = self.tomar()
            if tipo != "id":
                raise Malo(f"unexpected token: '{v}'")
            self.campos.append(v)
            if self.ver() == ("p", ","):
                self.i += 1
                continue
            break
        self.esperar_kw("FROM")
        tipo, nombre = self.tomar()
        if tipo != "id":
            raise Malo("unexpected end of query")
        obj = objetos.get(nombre.lower())
        if obj is None or not obj.consultable:
            raise Falla(400, "INVALID_TYPE", f"\n{self.soql}\n^\nERROR at Row:1:Column:1\nsObject type '{nombre}' is not supported. If you are attempting to use a custom object, be sure to append the '__c' after the entity name. Please reference your WSDL or the describe call for the appropriate names.")
        self.obj = obj
        for n in self.campos:
            c = obj.por_nombre.get(n.lower())
            if c is None:
                raise self._no_campo(n, obj)
        if self.kw("WHERE"):
            self.i += 1
            self.donde = self._cond(obj)
        if self.kw("ORDER"):
            self.i += 1
            self.esperar_kw("BY")
            while True:
                tipo, n = self.tomar()
                c = obj.por_nombre.get((n or "").lower())
                if tipo != "id" or c is None:
                    raise self._no_campo(n, obj)
                if not c.get("sortable", True):
                    raise Falla(400, "INVALID_FIELD", f"\n{self.soql}\n^\nERROR at Row:1:Column:1\nfield '{c['name']}' can not be sorted in a query call")
                desc, nulos_primero = False, True  # X0 B5: sin NULLS, primero, en los dos sentidos
                if self.kw("ASC", "DESC"):
                    desc = self.tomar()[1].upper() == "DESC"
                if self.kw("NULLS"):
                    self.i += 1
                    if not self.kw("FIRST", "LAST"):
                        raise Malo("unexpected token after NULLS")
                    nulos_primero = self.tomar()[1].upper() == "FIRST"
                self.orden.append((c, desc, nulos_primero))
                if self.ver() == ("p", ","):
                    self.i += 1
                    continue
                break
        if self.kw("LIMIT"):
            self.i += 1
            tipo, n = self.tomar()
            if tipo != "num" or not n.isdigit():
                raise Malo("LIMIT espera un entero")
            self.limite = int(n)
        if self.i < len(self.t):
            raise Malo(f"unexpected token: '{self.ver()[1]}'")

    def _no_campo(self, n, obj):
        return Falla(400, "INVALID_FIELD", f"\n{self.soql}\n^\nERROR at Row:1:Column:1\nNo such column '{n}' on entity '{obj.nombre}'. If you are attempting to use a custom field, be sure to append the '__c' after the custom field name. Please reference your WSDL or the describe call for the appropriate names.")

    # condiciones
    def _cond(self, obj):
        """`OR` de cadenas de `AND`. Mezclarlos SIN paréntesis (`A AND B OR C`) lo rechaza Salesforce."""
        items = [self._y(obj)]
        while self.kw("OR"):
            self.i += 1
            items.append(self._y(obj))
        if len(items) == 1:
            return items[0]
        if any(getattr(f, "_cadena_de_and", False) for f in items):
            raise Malo("unexpected token: 'OR' (use parentheses to mix AND and OR)")
        return lambda r: any(f(r) for f in items)

    def _y(self, obj):
        items = [self._nega(obj)]
        while self.kw("AND"):
            self.i += 1
            items.append(self._nega(obj))
        if len(items) == 1:
            return items[0]

        def todos(r):
            return all(f(r) for f in items)

        todos._cadena_de_and = True
        return todos

    def _nega(self, obj):
        if self.kw("NOT"):
            self.i += 1
            if self.ver() != ("p", "("):
                raise Malo("unexpected token: 'NOT' (NOT solo admite una condición entre paréntesis)")
            self.i += 1
            f = self._cond(obj)
            if self.ver() != ("p", ")"):
                raise Malo("falta `)`")
            self.i += 1
            return lambda r: not f(r)
        if self.ver() == ("p", "("):
            self.i += 1
            f = self._cond(obj)
            if self.ver() != ("p", ")"):
                raise Malo("falta `)`")
            self.i += 1
            return lambda r: f(r)  # entre paréntesis es una unidad
        return self._comparacion(obj)

    def _literal(self):
        tipo, v = self.tomar()
        if tipo == "str":
            return ("str", desescapar(v))
        if tipo == "num":
            return ("num", Decimal(v))
        if tipo in ("dt", "d"):
            return (tipo, v)
        if tipo == "id" and v.upper() in ("NULL", "TRUE", "FALSE"):
            return ({"NULL": "null", "TRUE": "bool", "FALSE": "bool"}[v.upper()], None if v.upper() == "NULL" else v.upper() == "TRUE")
        raise Malo(f"unexpected token: '{v}'")

    def _tipado(self, c, lit):
        """Convierte el literal al tipo del campo, o falla como Salesforce."""
        k, v = lit
        if k == "null":
            return None
        t = c["type"]
        mal = lambda q: Malo(f"value of filter criterion for field '{c['name']}' must be of type {q}")
        if t in TEXTOS or t in ("id", "reference"):
            if k != "str":
                raise mal("string")
            return v
        if t in NUMERICOS:
            if k != "num":
                raise mal("number")
            return v
        if t == "boolean":
            if k != "bool":
                raise mal("boolean")
            return v
        if t == "date":
            if k != "d":
                raise mal("date and should not be enclosed in quotes")
            return v
        if t == "datetime":
            if k != "dt":
                raise mal("dateTime and should not be enclosed in quotes")
            # Salesforce IGNORA la fracción de segundo de un literal (X0, 2026-11-01):
            # `= …17.999Z` casa con un registro de `…17.000`.
            return ms_de(v, cuenta_micros=True) // 1000 * 1000
        raise Malo(f"el campo '{c['name']}' no se puede comparar")

    def _comparacion(self, obj):
        tipo, n = self.tomar()
        c = obj.por_nombre.get((n or "").lower())
        if tipo != "id" or c is None:
            raise self._no_campo(n, obj)
        if not c.get("filterable", True):
            raise Falla(400, "INVALID_FIELD", f"\n{self.soql}\n^\nERROR at Row:1:Column:1\nfield '{c['name']}' can not be filtered in a query call")
        nombre = c["name"]
        texto = c["type"] in TEXTOS or c["type"] in ("id", "reference")

        def valor(r):
            return r.get(nombre)

        def norma(x):
            return x.casefold() if texto and isinstance(x, str) and c["type"] not in ("id", "reference") else x

        if self.kw("LIKE"):
            self.i += 1
            k, p = self._literal()
            if k != "str" or not texto:
                raise Malo("LIKE espera un texto sobre un campo de texto")
            rx = re.compile(_like(p), re.I | re.S)
            return lambda r: valor(r) is not None and rx.fullmatch(valor(r)) is not None
        if self.kw("IN"):
            self.i += 1
            if self.ver() != ("p", "("):
                raise Malo("IN espera una lista entre paréntesis")
            self.i += 1
            miembros = []
            while True:
                miembros.append(self._tipado(c, self._literal()))
                if self.ver() == ("p", ","):
                    self.i += 1
                    continue
                break
            if self.ver() != ("p", ")"):
                raise Malo("falta `)`")
            self.i += 1
            conjunto = {norma(m) for m in miembros if m is not None}
            return lambda r: valor(r) is not None and norma(valor(r)) in conjunto
        tipo, op = self.tomar()
        if tipo != "op":
            raise Malo(f"unexpected token: '{op}'")
        lit = self._literal()
        b = self._tipado(c, lit)
        if lit[0] == "null":
            if op == "=":
                return lambda r: valor(r) is None
            if op in ("!=", "<>"):
                return lambda r: valor(r) is not None
            raise Malo("solo = y != admiten NULL")
        b = norma(b)
        if op == "=":
            return lambda r: valor(r) is not None and norma(valor(r)) == b
        if op in ("!=", "<>"):
            # X0 B3: los nulos CUMPLEN `!=`.
            return lambda r: valor(r) is None or norma(valor(r)) != b
        cmp = {"<": lambda a: a < b, "<=": lambda a: a <= b, ">": lambda a: a > b, ">=": lambda a: a >= b}[op]
        return lambda r: valor(r) is not None and cmp(norma(valor(r)))


def _like(p):
    sal, i = [], 0
    while i < len(p):
        c = p[i]
        if c == "\\" and i + 1 < len(p):
            sal.append(re.escape(p[i + 1]))
            i += 2
            continue
        sal.append(".*" if c == "%" else "." if c == "_" else re.escape(c))
        i += 1
    return "".join(sal)


def ejecutar(soql, todos):
    """La consulta → (Consulta, índices ordenados)."""
    try:
        q = Consulta(soql, Estado.objetos)
    except Malo as e:
        raise Falla(400, "MALFORMED_QUERY", f"\n{soql}\n^\nERROR at Row:1:Column:1\n{e}")
    obj = q.obj
    n = len(obj)
    visible = (lambda r: True) if todos else (lambda r: not r.get("IsDeleted"))
    if q.donde is None and (todos or obj.generada):
        idx = range(n)
    else:
        idx = [i for i in range(n) if visible(obj.fila(i)) and (q.donde is None or q.donde(obj.fila(i)))]
    if q.orden:
        idx = list(idx)
        for c, desc, nulos_primero in reversed(q.orden):
            nombre, texto = c["name"], c["type"] in TEXTOS

            def clave(i, nombre=nombre, texto=texto, desc=desc, nulos_primero=nulos_primero):
                v = obj.fila(i).get(nombre)
                if v is None:
                    return (0 if nulos_primero != desc else 2, 0)
                return (1, v.casefold() if texto else v)

            idx.sort(key=clave, reverse=desc)
    if q.limite is not None:
        idx = idx[: q.limite]
    return q, idx


# ── el servidor ───────────────────────────────────────────────────────────────
def verificar_jwt(aserto):
    """Mira la forma del JWT y, si hay clave pública, VERIFICA su firma."""
    try:
        cab, cuerpo, firma = aserto.split(".")
        pad = lambda s: s + "=" * (-len(s) % 4)
        h = json.loads(base64.urlsafe_b64decode(pad(cab)))
        c = json.loads(base64.urlsafe_b64decode(pad(cuerpo)))
        f = base64.urlsafe_b64decode(pad(firma))
    except Exception:
        raise Falla(400, "invalid_grant", "invalid assertion")
    if h.get("alg") != "RS256":
        raise Falla(400, "invalid_grant", "invalid assertion")
    if c.get("aud") != Estado.endpoint:
        raise Falla(400, "invalid_audience", "invalid audience")
    if int(c.get("exp", 0)) < time.time():
        raise Falla(400, "invalid_grant", "expired authorization code")
    if not c.get("iss"):
        raise Falla(400, "invalid_client_id", "client identifier invalid")
    if PUBLICA:
        with tempfile.TemporaryDirectory() as d:
            ruta_f, ruta_d = os.path.join(d, "firma"), os.path.join(d, "datos")
            open(ruta_f, "wb").write(f)
            open(ruta_d, "wb").write(f"{cab}.{cuerpo}".encode())
            r = subprocess.run([OPENSSL, "dgst", "-sha256", "-verify", PUBLICA, "-signature", ruta_f, ruta_d], capture_output=True)
            if r.returncode != 0:
                raise Falla(400, "invalid_grant", "invalid signature")
    if c.get("sub") not in USUARIOS:
        # Lo que Salesforce dice de un usuario que no aprobó la app.
        raise Falla(400, "invalid_grant", "user hasn't approved this consumer")
    return c["sub"]


class Manejador(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "Salesforce-de-mentira"

    def log_message(self, *a):
        pass

    def date_time_string(self, timestamp=None):
        # La cabecera `Date` es la hora de la ORG (con su desfase): de ahí toma el conector el testigo.
        return texto_http(ahora_ms())

    # respuesta
    def enviar(self, estado, cuerpo, tipo="application/json;charset=UTF-8", extra=None):
        datos = cuerpo.encode() if isinstance(cuerpo, str) else cuerpo
        self.send_response(estado)
        self.send_header("Content-Type", tipo)
        self.send_header("Content-Length", str(len(datos)))
        if getattr(self, "_datos_api", False):
            self.send_header("Sforce-Limit-Info", f"api-usage={Estado.usadas}/{Estado.max}")
        for k, v in (extra or {}).items():
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(datos)

    def error(self, f):
        cuerpo = json.dumps([{"message": f.mensaje, "errorCode": f.codigo}], ensure_ascii=False)
        if f.estado == 400 and f.codigo in ("invalid_grant", "invalid_client_id", "invalid_audience"):
            cuerpo = json.dumps({"error": f.codigo, "error_description": f.mensaje})
        self.enviar(f.estado, cuerpo, extra=f.cabeceras)

    def leer_cuerpo(self):
        n = int(self.headers.get("Content-Length") or 0)
        return self.rfile.read(n) if n else b""

    def desconectado(self):
        """¿Cerró el cliente? (mira el socket sin consumir)."""
        try:
            r, _, _ = select.select([self.connection], [], [], 0)
            if r:
                return self.connection.recv(1, socket.MSG_PEEK) == b""
        except (OSError, ValueError):
            return True
        return False

    # entrada
    def do_GET(self):
        self.despachar("GET")

    def do_POST(self):
        self.despachar("POST")

    def do_PATCH(self):
        self.despachar("PATCH")

    def do_DELETE(self):
        self.despachar("DELETE")

    def do_PUT(self):
        self.despachar("PUT")

    def despachar(self, metodo):
        self._datos_api = False
        try:
            u = urlsplit(self.path)
            if u.path.startswith("/_mentira/"):
                return self.admin(metodo, u)
            if u.path == "/services/oauth2/token" and metodo == "POST":
                return self.canje()
            if len(self.path) > MAX_URL:
                return self.enviar(431, "<html><body>Request Header Fields Too Large</body></html>", "text/html")
            if u.path.startswith(PREFIJO):
                return self.api(metodo, u)
            raise Falla(404, "NOT_FOUND", "The requested resource does not exist")
        except Falla as f:
            self.error(f)
        except (BrokenPipeError, ConnectionResetError):
            pass

    # OAuth
    def canje(self):
        form = parse_qs(self.leer_cuerpo().decode())
        with CERRADURA:
            Estado.canjes += 1
        if form.get("grant_type", [""])[0] != "urn:ietf:params:oauth:grant-type:jwt-bearer":
            raise Falla(400, "unsupported_grant_type", "grant type not supported")
        usuario = verificar_jwt(form.get("assertion", [""])[0])
        token = "00D" + uuid.uuid4().hex
        with CERRADURA:
            Estado.tokens[token] = (usuario, Estado.epoca_tokens)
        self.enviar(200, json.dumps({
            "access_token": token, "scope": "api refresh_token", "instance_url": Estado.endpoint,
            "id": f"{Estado.endpoint}/id/00Dmentira/005mentira", "token_type": "Bearer",
        }))

    def autenticar(self):
        auth = self.headers.get("Authorization", "")
        with CERRADURA:
            if not auth:
                Estado.sin_token += 1
                raise Falla(401, "INVALID_SESSION_ID", "Session expired or invalid")
            t = auth.removeprefix("Bearer ").strip()
            s = Estado.tokens.get(t)
            if s is None or s[1] != Estado.epoca_tokens:
                raise Falla(401, "INVALID_SESSION_ID", "Session expired or invalid")
            Estado.con_token += 1

    # la API
    def api(self, metodo, u):
        self.autenticar()
        with CERRADURA:
            Estado.peticiones_datos += 1
            n = Estado.peticiones_datos
            Estado.usadas += 1
            if metodo != "GET" and not u.path.startswith(PREFIJO + "/jobs/"):
                Estado.escrituras += 1
            agotada = Estado.usadas > Estado.max
        self._datos_api = True
        if agotada:
            raise Falla(403, "REQUEST_LIMIT_EXCEEDED", "TotalRequests Limit exceeded.")
        if Estado.cada_503 and n % Estado.cada_503 == 0:
            raise Falla(503, "SERVER_UNAVAILABLE", "Server Unavailable", {"Retry-After": "1"})
        ruta = u.path[len(PREFIJO):]
        q = parse_qs(u.query)
        if ruta == "/limits" and metodo == "GET":
            return self.enviar(200, json.dumps({
                "DailyApiRequests": {"Max": Estado.max, "Remaining": max(0, Estado.max - Estado.usadas)},
                "DailyBulkV2QueryJobs": {"Max": 10000, "Remaining": 10000},
            }))
        if ruta == "/sobjects" and metodo == "GET":
            return self.enviar(200, json.dumps({"encoding": "UTF-8", "maxBatchSize": 200, "sobjects": [o.resumen() for o in Estado.objetos.values()]}))
        m = re.fullmatch(r"/sobjects/([A-Za-z0-9_]+)/describe/?", ruta)
        if m and metodo == "GET":
            return self.enviar(200, json.dumps(self.objeto(m.group(1)).describe()))
        m = re.fullmatch(r"/sobjects/([A-Za-z0-9_]+)/deleted/?", ruta)
        if m and metodo == "GET":
            return self.borrados(self.objeto(m.group(1)), q)
        if ruta in ("/query", "/queryAll", "/query/", "/queryAll/") and metodo == "GET":
            return self.consulta(q.get("q", [""])[0], ruta.startswith("/queryAll"))
        m = re.fullmatch(r"/query(?:All)?/(01g[0-9a-f]+)-(\d+)", ruta)
        if m and metodo == "GET":
            return self.siguiente(m.group(1), int(m.group(2)))
        m = re.fullmatch(r"/sobjects/([A-Za-z0-9_]+)/?", ruta)
        if m and metodo == "POST":
            return self.rest_crear(self.objeto(m.group(1)))
        m = re.fullmatch(r"/sobjects/([A-Za-z0-9_]+)/([A-Za-z0-9]{15,18})", ruta)
        if m and metodo in ("PATCH", "DELETE"):
            return self.rest_cambiar(self.objeto(m.group(1)), m.group(2), metodo)
        if ruta == "/jobs/query" and metodo == "POST":
            return self.job_crear()
        m = re.fullmatch(r"/jobs/query/([0-9a-z]+)", ruta)
        if m:
            return self.job(metodo, m.group(1))
        m = re.fullmatch(r"/jobs/query/([0-9a-z]+)/results", ruta)
        if m and metodo == "GET":
            return self.job_resultados(m.group(1), q)
        raise Falla(404, "NOT_FOUND", "The requested resource does not exist")

    def objeto(self, nombre):
        o = Estado.objetos.get(nombre.lower())
        if o is None:
            raise Falla(404, "NOT_FOUND", "The requested resource does not exist")
        return o

    # REST: query
    def consulta(self, soql, todos):
        if not soql:
            raise Falla(400, "MALFORMED_QUERY", "\nunexpected end of query")
        q, idx = ejecutar(soql, todos)
        opciones = self.headers.get("Sforce-Query-Options", "")
        m = re.search(r"batchSize=(\d+)", opciones)
        tam = min(PAGINA, int(m.group(1))) if m else PAGINA
        if q.obj.lenta:
            self.esperar_lenta(q.obj)
        cid = "01g" + uuid.uuid4().hex[:12]
        with CERRADURA:
            Estado.lecturas += 1
            Estado.cursores[cid] = (q, idx, tam, todos)
        self.pagina(cid, 0)

    def esperar_lenta(self, obj):
        """Un objeto «lento» tarda en dar su primera fila. Cuenta como VIVA mientras
        espera, y deja de serlo si el cliente cierra."""
        with CERRADURA:
            Estado.vivas[obj.nombre] = Estado.vivas.get(obj.nombre, 0) + 1
        try:
            fin = time.time() + Estado.lenta_s
            while time.time() < fin:
                if self.desconectado():
                    raise BrokenPipeError()
                time.sleep(0.05)
        finally:
            with CERRADURA:
                Estado.vivas[obj.nombre] -= 1

    def siguiente(self, cid, pos):
        if cid not in Estado.cursores:
            raise Falla(400, "INVALID_QUERY_LOCATOR", "Invalid query locator")
        self.pagina(cid, pos)

    def pagina(self, cid, pos):
        q, idx, tam, todos = Estado.cursores[cid]
        total = len(idx)
        trozo = idx[pos: pos + tam]
        fin = pos + len(trozo) >= total
        cuerpos = []
        for i in trozo:
            r = q.obj.fila(i)
            partes = [f'"attributes":{{"type":"{q.obj.nombre}","url":"{PREFIJO}/sobjects/{q.obj.nombre}/{r.get("Id")}"}}']
            for n in q.campos:
                c = q.obj.por_nombre[n.lower()]
                partes.append(f'{json.dumps(c["name"])}:{json_valor(c, r.get(c["name"]))}')
            cuerpos.append("{" + ",".join(partes) + "}")
        sal = f'{{"totalSize":{total},"done":{"true" if fin else "false"},'
        if not fin:
            sal += f'"nextRecordsUrl":"{PREFIJO}/{"queryAll" if todos else "query"}/{cid}-{pos + tam}",'
        sal += '"records":[' + ",".join(cuerpos) + "]}"
        if fin:
            with CERRADURA:
                Estado.cursores.pop(cid, None)
        self.enviar(200, sal)

    # getDeleted
    def borrados(self, obj, q):
        try:
            ini = ms_de(q["start"][0])
            fin = ms_de(q["end"][0])
        except (KeyError, ValueError):
            raise Falla(400, "MALFORMED_QUERY", "start y end son obligatorios")
        if fin - ini < 60_000:
            raise Falla(400, "INVALID_REPLICATION_DATE", "Date range too short: end must be at least one minute after start")
        if obj.sin_papelera or not obj.replicable:
            raise Falla(400, "INVALID_TYPE", f"sObject type '{obj.nombre}' is not supported by getDeleted")
        mas_antiguo = ahora_ms() - Estado.papelera_dias * 86400 * 1000
        # Trunca los segundos (X0 D9).
        ini_t, fin_t = ini // 60000 * 60000, fin // 60000 * 60000
        if ini_t < mas_antiguo:
            raise Falla(400, "INVALID_REPLICATION_DATE", "Start date is before the earliest available date")
        regs = []
        for i in range(len(obj)):
            r = obj.fila(i)
            d = r.get("_borrado")
            if r.get("IsDeleted") and d is not None and ini_t <= d <= fin_t:
                regs.append({"id": r["Id"], "deletedDate": texto_dt(d, "+0000")})
        self.enviar(200, json.dumps({
            "deletedRecords": regs, "earliestDateAvailable": texto_dt(mas_antiguo, "+0000"),
            "latestDateCovered": texto_dt(fin_t, "+0000"),
        }))

    # REST: escrituras (lo que usan las pruebas de cambios para crear, cambiar y borrar cuentas)
    def rest_crear(self, o):
        try:
            c = json.loads(self.leer_cuerpo() or b"{}")
        except ValueError:
            raise Falla(400, "JSON_PARSER_ERROR", "el cuerpo no es JSON")
        valores = {k: (None if v is None else str(v).lower() if isinstance(v, bool) else str(v)) for k, v in c.items()}
        with CERRADURA:
            self.alta(o, valores)
            nuevo = o.filas[-1]["Id"]
        self.enviar(201, json.dumps({"id": nuevo, "success": True, "errors": []}))

    def rest_cambiar(self, o, rid, metodo):
        with CERRADURA:
            r = next((f for f in o.filas if f["Id"] == rid and not f.get("IsDeleted")), None)
            if r is None:
                raise Falla(404, "ENTITY_IS_DELETED", "entity is deleted")
            if metodo == "DELETE":
                r["IsDeleted"] = True
                r["_borrado"] = ahora_ms()
                r["SystemModstamp"] = sello_ms()
            else:
                try:
                    c = json.loads(self.leer_cuerpo() or b"{}")
                except ValueError:
                    raise Falla(400, "JSON_PARSER_ERROR", "el cuerpo no es JSON")
                for nombre, v in c.items():
                    campo = o.por_nombre.get(nombre.lower())
                    if campo is None:
                        raise Falla(400, "INVALID_FIELD", f"No such column '{nombre}' on sobject of type {o.nombre}")
                    r[campo["name"]] = guardar(campo, None if v is None else str(v).lower() if isinstance(v, bool) else str(v))
                r["LastModifiedDate"] = r["SystemModstamp"] = sello_ms()
        self.enviar(204, b"")

    # Bulk 2.0
    def job_crear(self):
        try:
            c = json.loads(self.leer_cuerpo() or b"{}")
        except ValueError:
            raise Falla(400, "INVALID_JOB", "el cuerpo no es JSON")
        if c.get("operation") not in ("query", "queryAll"):
            raise Falla(400, "InvalidJob", "operation must be query or queryAll")
        q, idx = ejecutar(c.get("query", ""), c["operation"] == "queryAll")
        # X0 D: Bulk 2.0 no admite lo que el traductor nunca emite; lo que sí,
        # lo hace como REST. (El tope de la URL no cuenta: el SOQL va en el cuerpo.)
        jid = "750" + uuid.uuid4().hex[:15]
        with CERRADURA:
            Estado.lecturas += 1
            Estado.jobs_creados += 1
            Estado.jobs[jid] = {
                "id": jid, "q": q, "idx": idx, "creado": time.time(), "estado": "UploadComplete",
                "operacion": c["operation"], "abortado": False, "fin": None,
            }
            if q.obj.lenta:
                Estado.vivas[q.obj.nombre] = Estado.vivas.get(q.obj.nombre, 0) + 1
        self.enviar(200, json.dumps(self.resumen_job(Estado.jobs[jid])))

    def estado_job(self, j):
        if j["abortado"]:
            return "Aborted"
        espera = (Estado.lenta_s if j["q"].obj.lenta else JOB_MS / 1000)
        t = time.time() - j["creado"]
        if t >= espera:
            return "JobComplete"
        return "InProgress" if t > 0.05 else "UploadComplete"

    def cerrar_viva(self, j):
        """Un job de un objeto lento deja de estar vivo al abortarse o completarse."""
        if j["q"].obj.lenta and j["fin"] is None:
            j["fin"] = True
            Estado.vivas[j["q"].obj.nombre] -= 1

    def resumen_job(self, j):
        e = self.estado_job(j)
        return {
            "id": j["id"], "operation": j["operacion"], "object": j["q"].obj.nombre, "state": e,
            "contentType": "CSV", "apiVersion": float(VERSION), "lineEnding": "LF",
            "numberRecordsProcessed": len(j["idx"]) if e == "JobComplete" else 0,
        }

    def job(self, metodo, jid):
        with CERRADURA:
            j = Estado.jobs.get(jid)
            if j is None:
                raise Falla(404, "NOT_FOUND", "The requested resource does not exist")
            if metodo == "GET":
                if self.estado_job(j) == "JobComplete":
                    self.cerrar_viva(j)
                return self.enviar(200, json.dumps(self.resumen_job(j)))
            if metodo == "PATCH":
                c = json.loads(self.leer_cuerpo() or b"{}")
                if c.get("state") == "Aborted":
                    j["abortado"] = True
                    self.cerrar_viva(j)
                return self.enviar(200, json.dumps(self.resumen_job(j)))
            if metodo == "DELETE":
                Estado.jobs.pop(jid, None)
                self.cerrar_viva(j)
                return self.enviar(204, b"")
        raise Falla(405, "METHOD_NOT_ALLOWED", "Method not allowed")

    def job_resultados(self, jid, q):
        with CERRADURA:
            j = Estado.jobs.get(jid)
        if j is None:
            raise Falla(404, "NOT_FOUND", "The requested resource does not exist")
        e = self.estado_job(j)
        if e != "JobComplete":
            raise Falla(400, "InvalidJob", f"Job state {e}: the results are not ready")
        with CERRADURA:
            self.cerrar_viva(j)
        maximo = int(q.get("maxRecords", ["50000"])[0])
        loc = q.get("locator", [""])[0]
        ini = 0
        if loc and loc != "null":
            try:
                ini = int(base64.urlsafe_b64decode(loc + "=" * (-len(loc) % 4)).decode().split(":")[1])
            except Exception:
                raise Falla(400, "InvalidJob", "locator inválido")
        obj, campos, idx = j["q"].obj, j["q"].campos, j["idx"]
        trozo = idx[ini: ini + maximo]
        cs = [obj.por_nombre[n.lower()] for n in campos]
        lineas = [",".join(celda_csv(obj.por_nombre[n.lower()]["name"]) for n in campos)]
        for i in trozo:
            r = obj.fila(i)
            lineas.append(",".join(celda_csv(a_texto(c, r.get(c["name"]), True)) for c in cs))
        cuerpo = "\n".join(lineas) + "\n"
        hasta = ini + len(trozo)
        siguiente = "null" if hasta >= len(idx) else base64.urlsafe_b64encode(f"{jid}:{hasta}".encode()).decode().rstrip("=")
        self.enviar(200, cuerpo, "text/csv", {"Sforce-Locator": siguiente, "Sforce-NumberOfRecords": str(len(trozo))})

    # la API de siembra
    def admin(self, metodo, u):
        ruta = u.path[len("/_mentira/"):]
        q = parse_qs(u.query)
        cuerpo = {}
        if metodo == "POST":
            crudo = self.leer_cuerpo()
            cuerpo = json.loads(crudo) if crudo else {}
        try:
            if ruta == "cuentas":
                with CERRADURA:
                    return self.enviar(200, json.dumps({
                        "peticiones": Estado.peticiones_datos, "canjes": Estado.canjes,
                        "sin_token": Estado.sin_token, "con_token": Estado.con_token,
                        "escrituras": Estado.escrituras, "usadas": Estado.usadas, "max": Estado.max,
                        "micros_truncados": Estado.micros_truncados, "lecturas": Estado.lecturas,
                        "jobs_creados": Estado.jobs_creados,
                        "vivas": dict(Estado.vivas),
                        "jobs_abiertos": sum(1 for j in Estado.jobs.values() if not j["abortado"] and self.estado_job(j) != "JobComplete"),
                    }, indent=None))
            if ruta == "vivas":
                return self.enviar(200, json.dumps({"vivas": Estado.vivas.get(q.get("objeto", [""])[0], 0)}))
            if ruta == "escrituras":
                return self.enviar(200, json.dumps({"escrituras": Estado.escrituras}))
            if metodo != "POST":
                raise Falla(405, "METHOD_NOT_ALLOWED", "POST")
            with CERRADURA:
                if ruta == "objeto":
                    self.sembrar_objeto(cuerpo)
                elif ruta == "fila":
                    self.alta(self.objeto(cuerpo["objeto"]), cuerpo["valores"])
                elif ruta == "actualizar":
                    self.cambiar(self.objeto(cuerpo["objeto"]), cuerpo["donde"], cuerpo["valores"])
                elif ruta == "borrar":
                    self.borrar(self.objeto(cuerpo["objeto"]), cuerpo["donde"])
                elif ruta == "reloj":
                    Estado.desfase_s = float(cuerpo["desfase_s"])
                elif ruta == "cuota":
                    Estado.usadas = int(cuerpo.get("usadas", Estado.usadas))
                    Estado.max = int(cuerpo.get("max", Estado.max))
                elif ruta == "caducar":
                    Estado.epoca_tokens += 1
                elif ruta == "papelera":
                    Estado.papelera_dias = float(cuerpo["dias"])
                elif ruta == "fallos":
                    Estado.cada_503 = int(cuerpo.get("cada_503", Estado.cada_503))
                elif ruta == "lenta":
                    Estado.lenta_s = float(cuerpo["segundos"])
                elif ruta == "reiniciar":
                    Estado.objetos.clear(); Estado.cursores.clear(); Estado.jobs.clear(); Estado.vivas.clear()
                    Estado.usadas = Estado.escrituras = Estado.peticiones_datos = 0
                else:
                    raise Falla(404, "NOT_FOUND", ruta)
            self.enviar(200, "{}")
        except Falla as f:
            self.error(f)
        except (KeyError, ValueError) as e:
            self.enviar(400, json.dumps([{"message": f"la siembra no entiende: {e!r}", "errorCode": "SIEMBRA"}]))

    def sembrar_objeto(self, c):
        campos = []
        for f in c.get("campos", []):
            d = {"label": f.get("etiqueta", f["nombre"]), "nillable": f.get("nulable", True), "filterable": f.get("filtrable", True),
                 "sortable": f.get("ordenable", True), "calculated": f.get("calculado", False), "custom": True}
            d.update({"name": f["nombre"], "type": f["tipo"]})
            for k_es, k_sf in (("precision", "precision"), ("escala", "scale"), ("largo", "length")):
                if k_es in f:
                    d[k_sf] = f[k_es]
            if "valores" in f:
                d["picklistValues"] = [{"value": v, "active": True} for v in f["valores"]]
            if "compuesto_de" in f:
                d["compoundFieldName"] = f["compuesto_de"]
            campos.append(d)
        o = Objeto(c["nombre"], campos, propio=c.get("propio", c["nombre"].endswith("__c")), replicable=c.get("replicable", True),
                   sin_papelera=c.get("sin_papelera", False), prefijo=c.get("prefijo", "a0Z"))
        o.lenta = bool(c.get("lenta"))
        o.consultable = c.get("consultable", True)
        g = c.get("generar")
        if g:
            o.generada = (g["n"], self.generador(o, g))
        for v in c.get("filas", []):
            self.alta(o, v, o_nuevo=True)
        Estado.objetos[o.nombre.lower()] = o

    def generador(self, o, g):
        """`Grande__c`: 10⁶ filas que no se guardan, se calculan. `grupo = id % 100`,
        `importe = id / 100`, `nota = 'fila-<id>'` (la fórmula de la semilla del kit)."""
        sello = sello_ms()
        cs = o.por_nombre

        def fila(i):
            k = i + 1
            return {
                "Id": nuevo_id(o.prefijo, k), "IsDeleted": False, "Name": f"n-{k}",
                "CreatedDate": sello, "LastModifiedDate": sello, "SystemModstamp": sello,
                g["id"]: k, g["grupo"]: k % 100,
                g["importe"]: Decimal(k) / 100,
                g["nota"]: f"fila-{k}",
            }

        for k in ("id", "grupo", "importe", "nota"):
            if g[k].lower() not in cs:
                raise ValueError(f"el generador usa `{g[k]}` y el objeto no lo declara")
        return fila

    def alta(self, o, valores, o_nuevo=False):
        sello = sello_ms()
        r = {c["name"]: None for c in o.campos}
        for nombre, texto in valores.items():
            c = o.por_nombre.get(nombre.lower())
            if c is None:
                raise Falla(400, "INVALID_FIELD", f"No such column '{nombre}' on entity '{o.nombre}'")
            r[c["name"]] = guardar(c, texto) if not isinstance(texto, (int, float, bool)) else guardar(c, str(texto).lower() if isinstance(texto, bool) else str(texto))
        n = o.siguiente
        o.siguiente += 1
        r["Id"] = r.get("Id") or nuevo_id(o.prefijo, n)
        r["IsDeleted"] = False
        r["Name"] = r.get("Name") or f"n-{n}"
        r["CreatedDate"] = r["LastModifiedDate"] = r["SystemModstamp"] = sello
        o.filas.append(r)

    def buscar(self, o, donde):
        k, v = next(iter(donde.items()))
        c = o.por_nombre[k.lower()]
        quiero = guardar(c, v)
        return [r for r in o.filas if r.get(c["name"]) == quiero and not r.get("IsDeleted")]

    def cambiar(self, o, donde, valores):
        sello = sello_ms()
        for r in self.buscar(o, donde):
            for nombre, texto in valores.items():
                c = o.por_nombre[nombre.lower()]
                r[c["name"]] = guardar(c, texto)
            r["LastModifiedDate"] = r["SystemModstamp"] = sello

    def borrar(self, o, donde):
        ahora = ahora_ms()
        for r in self.buscar(o, donde):
            r["IsDeleted"] = True
            r["_borrado"] = ahora
            r["SystemModstamp"] = sello_ms()


def main():
    puerto = int(sys.argv[1]) if len(sys.argv) > 1 else 8794
    if not Estado.endpoint:
        Estado.endpoint = f"http://127.0.0.1:{puerto}"
    ThreadingHTTPServer.daemon_threads = True
    ThreadingHTTPServer.request_queue_size = 64
    s = ThreadingHTTPServer(("127.0.0.1", puerto), Manejador)
    print("listo", flush=True)
    s.serve_forever()


if __name__ == "__main__":
    main()
