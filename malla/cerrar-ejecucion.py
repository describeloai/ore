#!/usr/bin/env python3
# ═══════════════════════════════════════════════════════════════════════════
# CERRAR LA EJECUCIÓN DE UNA INGESTA (docs/ejecuciones.md §8.12, X5)
#
#   cerrar-ejecucion.py --id <ejecución> --rc <código> --inicio <ms> --log <fichero>
#                       [--vistas a,b] [--arbol <clon>] [--fuente <f>] [--secretas VAR,VAR]
#
# Lo llama, al salir (`trap … EXIT`), quien ejecuta una ingesta: el Job de copia
# (48), el de catálogo (44), el de invocación (49) y la pasada del copiador
# residente (`copiar-worker.sh`). `ore-serve` abrió la ejecución al encolarla; esto
# la cierra con `POST /ejecuciones/{id}/cierre` y el token del agente:
#
#   · status   `ok` si salió con 0 y ningún paso falló; si no, `failed`;
#   · steps    uno por dataset, de su puntero (`datasets/<b>/<s>/<n>.json`):
#              `copiada` → ok, `al-dia` → skipped, `error` → failed con su motivo;
#              un puntero que esta pasada no tocó, failed («no llegó a él»);
#   · log      el final de lo que escribió (64 KiB), sin el valor de ninguna
#              credencial que recibió (`--secretas`, más el testigo del agente).
#
# ⛔ Nunca falla el Job: lo que no puede cerrar lo dice en una línea y sale con 0.
#   La ejecución que no se cierra la da por `lost` `ore-serve` al pasar su plazo.
# ⛔ Ni el testigo ni el secreto del agente salen a la salida ni a un fichero.
# ═══════════════════════════════════════════════════════════════════════════
import argparse
import calendar
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

TOPE_DEL_LOG = 64 * 1024
# El reloj de un pod y el de quien escribió el puntero no son el mismo: margen.
HOLGURA_MS = 60_000


def leer(ruta):
    try:
        with open(ruta, encoding="utf-8", errors="replace") as f:
            return f.read()
    except OSError:
        return ""


def ms_de(iso):
    """`2026-10-27T10:15:00Z` → milisegundos de época, o None."""
    try:
        return calendar.timegm(time.strptime(iso, "%Y-%m-%dT%H:%M:%SZ")) * 1000
    except (TypeError, ValueError):
        return None


def puntero(arbol, vista):
    partes = vista.split(".")
    if len(partes) == 2:
        partes = [partes[0], "default", partes[1]]
    if len(partes) != 3:
        return None
    try:
        with open(os.path.join(arbol, "datasets", *partes[:2], partes[2] + ".json"), encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return None


def motivo_del_log(log, vista=None):
    """La última línea `✗ …` o `error: …` de `ore`, sin las de git (un push
    rechazado se reintenta y no es el motivo). Medido en victor el 2026-10-27:
    sin puntero, el paso decía «no pointer» y el motivo estaba en el log. Con
    `vista`, primero la que la nombra: en una pasada de varias, la última es la
    de otra (los dos pasos de `test1` decían el error de `visitas`)."""
    lineas = [l.strip() for l in reversed(log.splitlines())]

    def motivo(l):
        if "✗" in l:
            return l
        if l.startswith("error: ") and not l.startswith("error: failed to push"):
            return l[len("error: "):]
        return None

    if vista:
        suya = next((m for l in lineas if vista in l for m in [motivo(l)] if m), None)
        if suya:
            return suya
    return next((m for l in lineas for m in [motivo(l)] if m), None)


def duraciones(ruta):
    """`<dataset> <ms>` por la hora a la que `ore` empezó cada uno, y `fin <ms>` al
    acabar (0073 §9 H4): cada dataset dura hasta que empieza el siguiente. Sin el
    fichero, nada: el paso se queda sin duración, como antes."""
    marcas = []
    for l in leer(ruta).splitlines():
        partes = l.split()
        if len(partes) == 2 and partes[1].isdigit():
            marcas.append((int(partes[1]), partes[0]))
    marcas.sort()
    return {
        nombre: siguiente - t
        for (t, nombre), (siguiente, _) in zip(marcas, marcas[1:])
        if nombre != "fin"
    }


def pasos(arbol, vistas, inicio, log="", duracion=None):
    out = []
    for v in [x for x in vistas.split(",") if x.strip()]:
        p = puntero(arbol, v) if arbol else None
        paso = {"name": v}
        if duracion and v in duracion:
            paso["duration_ms"] = duracion[v]
        if p is None:
            paso.update(status="failed", error=motivo_del_log(log, v) or "no pointer for this dataset")
        else:
            intento = p.get("intento") or {}
            cuando = ms_de(intento.get("cuando")) or ms_de(p.get("comprobado"))
            estado = p.get("estado")
            if cuando is not None and cuando + HOLGURA_MS < inicio:
                paso.update(status="failed", error="this run did not reach it")
            elif estado == "copiada":
                paso["status"] = "ok"
            elif estado == "al-dia":
                paso["status"] = "skipped"
            else:
                paso.update(status="failed", error=p.get("motivo") or intento.get("motivo") or "estado %s" % estado)
            for k, c in (("filas", "rows"), ("bytes", "bytes")):
                if isinstance(p.get(k), int):
                    paso[c] = p[k]
        out.append(paso)
    return out


def tapado(texto, secretas):
    for v in secretas:
        if v and len(v) >= 4:
            texto = texto.replace(v, "***")
    return texto


def token():
    """El del agente, como lo pide el Job (client_credentials), sin imprimirlo."""
    direccion, realm = os.environ.get("DIRECCION"), os.environ.get("REALM")
    puesto = os.environ.get("PUESTO", "/puesto")
    if not direccion or not realm:
        raise RuntimeError("no DIRECCION/REALM in the environment")
    cliente = leer(os.path.join(puesto, "agente-cliente")).strip()
    secreto = leer(os.path.join(puesto, "agente-secreto")).strip()
    if not cliente or not secreto:
        raise RuntimeError("no agent credential in %s" % puesto)
    cuerpo = urllib.parse.urlencode(
        {"grant_type": "client_credentials", "client_id": cliente, "client_secret": secreto}
    ).encode()
    r = urllib.request.urlopen(
        urllib.request.Request("%s/realms/%s/protocol/openid-connect/token" % (direccion, realm), data=cuerpo),
        timeout=15,
    )
    return json.load(r)["access_token"], secreto


def main():
    a = argparse.ArgumentParser()
    a.add_argument("--id", default=os.environ.get("EJECUCION", ""))
    a.add_argument("--rc", type=int, default=0)
    a.add_argument("--inicio", type=int, default=0)
    a.add_argument("--log", default="")
    a.add_argument("--vistas", default="")
    a.add_argument("--arbol", default="")
    a.add_argument("--fuente", default="")
    a.add_argument("--secretas", default="")
    a.add_argument("--commit", default="")
    a.add_argument("--tiempos", default="")
    o = a.parse_args()
    if not o.id:
        print("ejecución: sin id (`EJECUCION`/`JOB`): no se cierra")
        return 0
    ahora = int(time.time() * 1000)
    log = leer(o.log) if o.log else ""
    duracion = duraciones(o.tiempos) if o.tiempos else {}
    steps = pasos(o.arbol, o.vistas, o.inicio, log, duracion) if o.vistas else []
    if o.fuente and not steps:
        steps = [{"name": o.fuente, "status": "ok" if o.rc == 0 else "failed"}]
    secretas = [os.environ.get(v, "") for v in o.secretas.split(",") if v]
    error = None
    if o.rc != 0:
        ultimas = [l for l in log.splitlines() if l.strip()]
        # El motivo: la última línea con `✗`; si no, la última `@@ <dataset> fallo <motivo>`
        # del copiador; si no, el código con que salió.
        error = (
            next((l.strip() for l in reversed(ultimas) if "✗" in l), None)
            or next((l.split(" fallo ", 1)[1].strip() for l in reversed(ultimas)
                     if l.startswith("@@ ") and " fallo " in l), None)
            or "exited with %d" % o.rc
        )
    elif any(p.get("status") == "failed" for p in steps):
        mal = [p for p in steps if p.get("status") == "failed"]
        error = "; ".join("%s: %s" % (p["name"], p.get("error", "failed")) for p in mal[:5])
    commit = o.commit
    if not commit and o.arbol:
        try:
            commit = subprocess.run(["git", "-C", o.arbol, "rev-parse", "--short", "HEAD"],
                                    capture_output=True, text=True, timeout=10).stdout.strip()
        except (OSError, subprocess.SubprocessError):
            commit = ""
    try:
        tok, secreto = token()
    except Exception as e:  # noqa: BLE001 · cerrar nunca tumba el Job
        print("ejecución `%s`: no se pudo cerrar (%s); ore-serve la dará por lost" % (o.id, e))
        return 0
    secretas += [tok, secreto]
    for p in steps:
        if p.get("error"):
            p["error"] = tapado(p["error"], secretas)[:2000]
    cuerpo = {
        "status": "ok" if o.rc == 0 and error is None else "failed",
        "started_at": o.inicio or None,
        "ended_at": ahora,
        "steps": steps,
        "log": tapado(log[-TOPE_DEL_LOG:], secretas),
    }
    if error:
        cuerpo["error_message"] = tapado(error, secretas)[:2000]
    if commit:
        cuerpo["commit"] = commit
    url = "%s/ejecuciones/%s/cierre" % (os.environ.get("ORE_SERVE", "http://ore-serve:8080"), urllib.parse.quote(o.id))
    # X6·1: `ore-serve` se reinicia en cada despliegue (`Recreate`): mientras no está, o
    # mientras rehace lo que tenía abierto (503), se reintenta unos dos minutos.
    esperas = [float(x) for x in os.environ.get("CIERRE_ESPERAS", "5,10,20,30,30,30").split(",") if x]
    for intento in range(len(esperas) + 1):
        pet = urllib.request.Request(url, data=json.dumps(cuerpo).encode(), method="POST",
                                     headers={"authorization": "Bearer " + tok, "content-type": "application/json"})
        try:
            r = urllib.request.urlopen(pet, timeout=20)
            print("ejecución `%s` cerrada · %s · %s" % (o.id, cuerpo["status"], r.read(200).decode(errors="replace")))
            return 0
        except urllib.error.HTTPError as e:
            dicho = e.read(200).decode(errors="replace")
            if e.code != 503 or intento == len(esperas):
                print("ejecución `%s`: ore-serve contestó %d · %s" % (o.id, e.code, dicho))
                return 0
            print("ejecución `%s`: ore-serve arranca (503); otra vez en %g s" % (o.id, esperas[intento]))
        except Exception as e:  # noqa: BLE001
            if intento == len(esperas):
                print("ejecución `%s`: ore-serve no contesta (%s); la dará por lost" % (o.id, e))
                return 0
            print("ejecución `%s`: ore-serve no contesta (%s); otra vez en %g s" % (o.id, e, esperas[intento]))
        time.sleep(esperas[intento])
    return 0


if __name__ == "__main__":
    sys.exit(main())
