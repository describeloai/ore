"""Los usuarios de Libro (ADR 0058, P7): dan de alta las cuentas y después transfieren y consultan
contra libro-api, como lo harían clientes de verdad.

Una transferencia lleva su clave de idempotencia (`id`): ante un fallo (sin respuesta o 5xx) se
reintenta con la MISMA clave, así que nunca se transfiere dos veces. Una confirmada (200) va a
transferencias.jsonl: el conciliador comprueba que existe (invariante 2). Los errores que un
reintento absorbió se apuntan igual (`errores`): el informe los cuenta.

Con RELOJ=1 (P7·2), la carga sigue el día y la noche de reloj.json: OPS operaciones por segundo a
mediodía, menos al amanecer y al anochecer, ninguna de noche. Sin él, OPS constantes (P7·1).
Reanudable: las altas son idempotentes y el reloj no vuelve a empezar.
"""
import json, os, random, sys, threading, time, uuid

sys.path.insert(0, "/libro")
from comun import Primera, ahora, apunta, en_pausa, fase, intensidad, pide, reloj  # noqa: E402

API = os.environ.get("API", "http://libro-api:8080")
CUENTAS = int(os.environ.get("CUENTAS", "50"))
HILOS = int(os.environ.get("HILOS", "4"))
OPS = float(os.environ.get("OPS", "4"))
REINTENTOS = int(os.environ.get("REINTENTOS", "30"))
RELOJ = os.environ.get("RELOJ", "0") == "1"


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
    """→ (resultado, ms, intentos, último error, errores absorbidos)."""
    origen, destino = random.sample(ids, 2)
    t = {"id": str(uuid.uuid4()), "origen": origen, "destino": destino, "importe": random.randint(1, 2000)}
    t0, errores = time.time(), []
    for intento in range(1, REINTENTOS + 1):
        codigo, r = pide("POST", f"{API}/transferencias", t)
        ms = int((time.time() - t0) * 1000)
        if codigo == 200:
            apunta("transferencias.jsonl", {"t": ahora(), "id": t["id"], "importe": t["importe"]})
            return "hecha", ms, intento, None, errores
        if codigo == 409:
            return "sin-fondos", ms, intento, None, errores
        if codigo == 400:
            return "mala", ms, intento, str(r)[:200], errores
        errores.append(f"{codigo} {str(r)[:160]}")
        time.sleep(min(10, 0.2 * 2 ** intento))
    return "fallo", int((time.time() - t0) * 1000), REINTENTOS, errores[-1], errores


def lectura(ids):
    """Una consulta es idempotente: ante un fallo, se reintenta igual que una transferencia (P7·3)."""
    c = random.choice(ids)
    ruta = f"/cuentas/{c}" if random.random() < 0.6 else f"/cuentas/{c}/movimientos?limite=20"
    t0, errores = time.time(), []
    for intento in range(1, REINTENTOS + 1):
        codigo, r = pide("GET", f"{API}{ruta}")
        if codigo == 200:
            return "hecha", int((time.time() - t0) * 1000), intento, None, errores
        errores.append(f"{codigo} {str(r)[:160]}")
        time.sleep(min(10, 0.2 * 2 ** intento))
    return "fallo", int((time.time() - t0) * 1000), REINTENTOS, errores[-1], errores


def hilo(ids, r, primera):
    while True:
        k = intensidad(r) if RELOJ else 1.0
        if en_pausa() or k == 0:
            time.sleep(1)
            continue
        ciclo, _, x = fase(r) if RELOJ else (0, "dia", 0)
        es_primera = RELOJ and primera.es(ciclo, x)
        op = "transferencia" if random.random() < 0.7 else "lectura"
        resultado, ms, intentos, error, errores = (transferencia if op == "transferencia" else lectura)(ids)
        fila = {"t": ahora(), "pieza": "libro-api", "op": op, "resultado": resultado, "ms": ms,
                "intentos": intentos, "error": error, "ciclo": ciclo}
        if errores:
            fila["errores"] = errores
        if es_primera:
            fila["primera"] = True
        apunta("operaciones.jsonl", fila)
        time.sleep(random.expovariate(OPS * k / HILOS))


if __name__ == "__main__":
    if sys.argv[1:] == ["--una"]:
        # Una transferencia y fuera (la prueba del despertar, lab/p71.sh): su resultado por la salida.
        resultado, ms, intentos, error, _ = transferencia(json.load(open("/datos/cuentas.json")))
        print(json.dumps({"resultado": resultado, "ms": ms, "intentos": intentos, "error": error}, separators=(",", ":")))
        sys.exit(0 if resultado in ("hecha", "sin-fondos") else 1)
    ids = altas()
    r = reloj() if RELOJ else None
    if r:
        print(f"reloj · días de {r['dia_s']:.0f} s y noches de {r['noche_s']:.0f} s; ahora, {fase(r)[:2]}", flush=True)
    primera = Primera()
    for _ in range(HILOS):
        threading.Thread(target=hilo, args=(ids, r, primera), daemon=True).start()
    while True:
        time.sleep(3600)
