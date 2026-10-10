"""El banco de mentira de Libro (ADR 0058, P7): manda avisos de pago a libro-webhooks.

Como un banco de verdad: reintenta hasta que le contestan 2xx, y a veces manda el mismo aviso dos
veces. Un aviso entregado va a avisos.jsonl: el conciliador comprueba que está apuntado UNA vez y
con su importe (invariante 3). Los errores que un reintento absorbió se apuntan (`errores`).

Uno cada INTERVALO segundos de media; con RELOJ=1 (P7·2), al ritmo del día de reloj.json y ninguno
de noche.
"""
import json, os, random, sys, time, uuid

sys.path.insert(0, "/libro")
from comun import Primera, ahora, apunta, en_pausa, fase, intensidad, pide, reloj  # noqa: E402

WEBHOOKS = os.environ.get("WEBHOOKS", "http://libro-webhooks:8081")
INTERVALO = float(os.environ.get("INTERVALO", "2"))
DUPLICADOS = float(os.environ.get("DUPLICADOS", "0.1"))
RELOJ = os.environ.get("RELOJ", "0") == "1"


def entrega(aviso):
    """Reintenta hasta 2xx. → (nuevo, ms, intentos, errores absorbidos)."""
    t0, errores = time.time(), []
    while True:
        codigo, r = pide("POST", f"{WEBHOOKS}/avisos", aviso)
        if codigo == 200:
            return r.get("nuevo"), int((time.time() - t0) * 1000), len(errores) + 1, errores
        errores.append(f"{codigo} {str(r)[:160]}")
        if len(errores) % 10 == 0:
            print(f"aviso {aviso['clave']}: {len(errores)} intentos; {errores[-1]}", flush=True)
        time.sleep(min(10, 0.2 * 2 ** min(len(errores), 6)))


def avisa(aviso, repetido, ciclo=0, es_primera=False):
    nuevo, ms, intentos, errores = entrega(aviso)
    if not repetido:
        apunta("avisos.jsonl", {"t": ahora(), **aviso})
    # Un repetido que entra como nuevo, o un original que no, es un error del lado de Libro.
    fila = {"t": ahora(), "pieza": "libro-webhooks", "op": "aviso-repetido" if repetido else "aviso",
            "resultado": "hecha" if nuevo == (not repetido) else "duplicado-mal", "ms": ms, "intentos": intentos,
            "error": None, "ciclo": ciclo}
    if errores:
        fila["errores"] = errores
    if es_primera:
        fila["primera"] = True
    apunta("operaciones.jsonl", fila)


if __name__ == "__main__":
    if sys.argv[1:] == ["--uno"]:
        # Un aviso y fuera (la prueba del despertar, lab/p71.sh).
        ids = json.load(open("/datos/cuentas.json"))
        aviso = {"clave": f"banco-{uuid.uuid4()}", "cuenta": random.choice(ids), "importe": random.randint(1000, 50000)}
        nuevo, ms, intentos, _ = entrega(aviso)
        apunta("avisos.jsonl", {"t": ahora(), **aviso})
        print(json.dumps({"nuevo": nuevo, "ms": ms, "intentos": intentos}, separators=(",", ":")))
        sys.exit(0 if nuevo else 1)
    while not os.path.exists("/datos/cuentas.json"):
        time.sleep(1)
    ids = json.load(open("/datos/cuentas.json"))
    r = reloj() if RELOJ else None
    primera = Primera()
    print(f"banco · {len(ids)} cuentas, un aviso cada {INTERVALO} s{' a mediodía' if r else ''}", flush=True)
    while True:
        k = intensidad(r) if r else 1.0
        if en_pausa() or k == 0:
            time.sleep(1)
            continue
        ciclo, _, x = fase(r) if r else (0, "dia", 0)
        aviso = {"clave": f"banco-{uuid.uuid4()}", "cuenta": random.choice(ids), "importe": random.randint(1000, 50000)}
        avisa(aviso, False, ciclo, bool(r) and primera.es(ciclo, x))
        if random.random() < DUPLICADOS:
            avisa(aviso, True, ciclo)
        time.sleep(random.expovariate(k / INTERVALO))
