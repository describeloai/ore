"""Lo común a los clientes de Libro en Python (usuarios, banco, informe): /datos, el reloj y HTTP.

/datos es un volumen compartido; cada línea, un JSON. Lo que vale como prueba:
  operaciones.jsonl     cada operación de cada pieza (P7·2 saca de aquí los SLOs)
  transferencias.jsonl  las transferencias que la API CONFIRMÓ (invariante 2)
  avisos.jsonl          los avisos que el banco dio por entregados (invariante 3)
  reloj.json            el día y la noche de esta corrida (P7·2)

**El reloj** (P7·2): días de DIA_S segundos con forma (poca carga al amanecer y al anochecer, el
máximo a mediodía) y noches de NOCHE_S sin clientes, para que la base duerma muchas veces. El
primero que arranca escribe el origen en reloj.json; los demás, y quien se reinicie, lo leen: el
mismo día para todas las piezas, y reanudar no lo reinicia.
"""
import json, math, os, threading, time, urllib.error, urllib.request

DATOS = os.environ.get("DATOS", "/datos")
_cerrojo = threading.Lock()


def ahora():
    return time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime()) + f".{int(time.time() * 1000) % 1000:03d}Z"


def apunta(fichero, fila):
    linea = json.dumps(fila, separators=(",", ":")) + "\n"
    with _cerrojo, open(os.path.join(DATOS, fichero), "a", encoding="utf-8") as f:
        f.write(linea)


def reloj():
    """→ {origen, dia_s, noche_s}; lo crea el primero (O_EXCL), con DIA_S y NOCHE_S."""
    ruta = os.path.join(DATOS, "reloj.json")
    try:
        fd = os.open(ruta, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o644)
        with os.fdopen(fd, "w") as f:
            json.dump({"origen": time.time(), "dia_s": float(os.environ.get("DIA_S", "600")),
                       "noche_s": float(os.environ.get("NOCHE_S", "180"))}, f)
    except FileExistsError:
        pass
    for _ in range(50):  # quien lo crea puede no haber terminado de escribir
        try:
            with open(ruta) as f:
                return json.load(f)
        except ValueError:
            time.sleep(0.1)
    raise RuntimeError("reloj.json ilegible")


def fase(r, t=None):
    """→ (ciclo, "dia" | "noche", fracción de la fase transcurrida)."""
    pasado = (t or time.time()) - r["origen"]
    largo = r["dia_s"] + r["noche_s"]
    ciclo, dentro = int(pasado // largo), pasado % largo
    if dentro < r["dia_s"]:
        return ciclo, "dia", dentro / r["dia_s"]
    return ciclo, "noche", (dentro - r["dia_s"]) / r["noche_s"]


def intensidad(r):
    """De 0 a 1: 0 de noche; de día, 0,3 al amanecer y al anochecer y 1 a mediodía."""
    _, f, x = fase(r)
    return 0.0 if f == "noche" else 0.3 + 0.7 * math.sin(math.pi * x)


class Primera:
    """La primera operación de cada día de una pieza (a partir del segundo día: la primera tras una
    noche). Su latencia incluye el despertar, si la base durmió. Solo en el primer cuarto del día:
    una pieza que se reinicia a mediodía no cuenta su primera operación como un despertar."""

    def __init__(self):
        self._vistos, self._c = set(), threading.Lock()

    def es(self, ciclo, x):
        with self._c:
            if ciclo == 0 or ciclo in self._vistos or x > 0.25:
                return False
            self._vistos.add(ciclo)
            return True


def en_pausa():
    return os.path.exists(os.path.join(DATOS, "pausa"))


def pide(metodo, url, cuerpo=None, plazo=90):
    """→ (código, json o texto). Código 0: no se llegó (conexión, plazo)."""
    datos = json.dumps(cuerpo).encode() if cuerpo is not None else None
    r = urllib.request.Request(url, data=datos, method=metodo, headers={"content-type": "application/json"})
    try:
        with urllib.request.urlopen(r, timeout=plazo) as f:
            return f.status, json.loads(f.read() or b"null")
    except urllib.error.HTTPError as e:
        texto = e.read().decode(errors="replace")
        try:
            return e.code, json.loads(texto)
        except ValueError:
            return e.code, texto
    except Exception as e:  # noqa: BLE001
        return 0, f"{type(e).__name__}: {e}"
