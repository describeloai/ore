"""Lo común a los clientes de Libro en Python (usuarios y banco): apuntar en /datos y hablar HTTP.

/datos es un volumen compartido; cada línea, un JSON. Lo que vale como prueba:
  operaciones.jsonl     cada operación de cada pieza (P7·2 saca de aquí los SLOs)
  transferencias.jsonl  las transferencias que la API CONFIRMÓ (invariante 2)
  avisos.jsonl          los avisos que el banco dio por entregados (invariante 3)
"""
import json, os, threading, time, urllib.error, urllib.request

DATOS = os.environ.get("DATOS", "/datos")
_cerrojo = threading.Lock()


def ahora():
    return time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime()) + f".{int(time.time() * 1000) % 1000:03d}Z"


def apunta(fichero, fila):
    linea = json.dumps(fila, separators=(",", ":")) + "\n"
    with _cerrojo, open(os.path.join(DATOS, fichero), "a", encoding="utf-8") as f:
        f.write(linea)


def en_pausa():
    return os.path.exists(os.path.join(DATOS, "pausa"))


def pide(metodo, url, cuerpo=None, plazo=60):
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
