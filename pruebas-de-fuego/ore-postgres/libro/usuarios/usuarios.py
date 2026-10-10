"""Los usuarios de Libro (ADR 0058, P7·1): dan de alta las cuentas y después transfieren y consultan
contra libro-api, como lo harían clientes de verdad.

Una transferencia lleva su clave de idempotencia (`id`): ante un fallo (sin respuesta o 5xx) se
reintenta con la MISMA clave, así que nunca se transfiere dos veces. Una confirmada (200) va a
transferencias.jsonl: el conciliador comprueba que existe (invariante 2).

P7·2 le da forma de día y noche; aquí, carga constante: OPS operaciones por segundo en HILOS hilos.
"""
import json, os, random, sys, threading, time, uuid

sys.path.insert(0, "/libro")
from comun import ahora, apunta, en_pausa, pide  # noqa: E402

API = os.environ.get("API", "http://libro-api:8080")
CUENTAS = int(os.environ.get("CUENTAS", "50"))
HILOS = int(os.environ.get("HILOS", "4"))
OPS = float(os.environ.get("OPS", "4"))
REINTENTOS = int(os.environ.get("REINTENTOS", "30"))


def altas():
    ids = [f"c{i:04d}" for i in range(1, CUENTAS + 1)]
    for i in ids:
        while True:
            codigo, r = pide("POST", f"{API}/cuentas", {"id": i, "titular": f"Titular {i}"})
            if codigo == 200:
                break
            print(f"alta {i}: {codigo} {str(r)[:120]}; reintento", flush=True)
            time.sleep(2)
    with open("/datos/cuentas.json", "w") as f:
        json.dump(ids, f)
    print(f"{len(ids)} cuentas", flush=True)
    return ids


def transferencia(ids):
    origen, destino = random.sample(ids, 2)
    t = {"id": str(uuid.uuid4()), "origen": origen, "destino": destino, "importe": random.randint(1, 2000)}
    t0, intentos, ultimo = time.time(), 0, None
    while intentos < REINTENTOS:
        intentos += 1
        codigo, r = pide("POST", f"{API}/transferencias", t)
        if codigo == 200:
            ms = int((time.time() - t0) * 1000)
            apunta("transferencias.jsonl", {"t": ahora(), "id": t["id"], "importe": t["importe"]})
            return "hecha", ms, intentos, None
        if codigo == 409:
            return "sin-fondos", int((time.time() - t0) * 1000), intentos, None
        if codigo == 400:
            return "mala", int((time.time() - t0) * 1000), intentos, str(r)[:200]
        ultimo = f"{codigo} {str(r)[:200]}"
        time.sleep(min(10, 0.2 * 2 ** intentos))
    return "fallo", int((time.time() - t0) * 1000), intentos, ultimo


def lectura(ids):
    c = random.choice(ids)
    ruta = f"/cuentas/{c}" if random.random() < 0.6 else f"/cuentas/{c}/movimientos?limite=20"
    t0 = time.time()
    codigo, r = pide("GET", f"{API}{ruta}")
    ms = int((time.time() - t0) * 1000)
    return ("hecha" if codigo == 200 else "fallo"), ms, 1, (None if codigo == 200 else f"{codigo} {str(r)[:200]}")


def hilo(ids):
    pausa_media = HILOS / OPS
    while True:
        if en_pausa():
            time.sleep(1)
            continue
        op = "transferencia" if random.random() < 0.7 else "lectura"
        resultado, ms, intentos, error = (transferencia if op == "transferencia" else lectura)(ids)
        apunta("operaciones.jsonl", {"t": ahora(), "pieza": "libro-api", "op": op, "resultado": resultado,
                                     "ms": ms, "intentos": intentos, "error": error})
        time.sleep(random.expovariate(1 / pausa_media))


if __name__ == "__main__":
    if sys.argv[1:] == ["--una"]:
        # Una transferencia y fuera (la prueba del despertar, lab/p71.sh): su resultado por la salida.
        resultado, ms, intentos, error = transferencia(json.load(open("/datos/cuentas.json")))
        print(json.dumps({"resultado": resultado, "ms": ms, "intentos": intentos, "error": error}, separators=(",", ":")))
        sys.exit(0 if resultado in ("hecha", "sin-fondos") else 1)
    ids = altas()
    for _ in range(HILOS):
        threading.Thread(target=hilo, args=(ids,), daemon=True).start()
    while True:
        time.sleep(3600)
