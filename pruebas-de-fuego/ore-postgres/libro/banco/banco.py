"""El banco de mentira de Libro (ADR 0058, P7·1): manda avisos de pago a libro-webhooks.

Como un banco de verdad: reintenta hasta que le contestan 2xx, y a veces manda el mismo aviso dos
veces. Un aviso entregado va a avisos.jsonl: el conciliador comprueba que está apuntado UNA vez y
con su importe (invariante 3). Uno cada INTERVALO segundos, de media.
"""
import json, os, random, sys, time, uuid

sys.path.insert(0, "/libro")
from comun import ahora, apunta, en_pausa, pide  # noqa: E402

WEBHOOKS = os.environ.get("WEBHOOKS", "http://libro-webhooks:8081")
INTERVALO = float(os.environ.get("INTERVALO", "2"))
DUPLICADOS = float(os.environ.get("DUPLICADOS", "0.1"))


def entrega(aviso):
    """Reintenta hasta 2xx. → (nuevo, ms, intentos)."""
    t0, intentos = time.time(), 0
    while True:
        intentos += 1
        codigo, r = pide("POST", f"{WEBHOOKS}/avisos", aviso)
        if codigo == 200:
            return r.get("nuevo"), int((time.time() - t0) * 1000), intentos, None
        error = f"{codigo} {str(r)[:200]}"
        if intentos % 10 == 0:
            print(f"aviso {aviso['clave']}: {intentos} intentos; {error}", flush=True)
        time.sleep(min(10, 0.2 * 2 ** min(intentos, 6)))


def avisa(aviso, repetido):
    nuevo, ms, intentos, _ = entrega(aviso)
    if not repetido:
        apunta("avisos.jsonl", {"t": ahora(), **aviso})
    # Un repetido que entra como nuevo, o un original que no, es un error del lado de Libro.
    resultado = "hecha" if nuevo == (not repetido) else "duplicado-mal"
    apunta("operaciones.jsonl", {"t": ahora(), "pieza": "libro-webhooks", "op": "aviso-repetido" if repetido else "aviso",
                                 "resultado": resultado, "ms": ms, "intentos": intentos, "error": None})


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
    print(f"banco · {len(ids)} cuentas, un aviso cada {INTERVALO} s", flush=True)
    while True:
        if en_pausa():
            time.sleep(1)
            continue
        aviso = {"clave": f"banco-{uuid.uuid4()}", "cuenta": random.choice(ids), "importe": random.randint(1000, 50000)}
        avisa(aviso, False)
        if random.random() < DUPLICADOS:
            avisa(aviso, True)
        time.sleep(random.expovariate(1 / INTERVALO))
