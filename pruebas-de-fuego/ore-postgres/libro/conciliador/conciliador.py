"""El conciliador de Libro (ADR 0058, P7·1): Python y psycopg, directo al cómputo (TCP). Cada
INTERVALO segundos, como el cron de un departamento financiero, abre UNA conexión, comprueba los
invariantes en una sola foto (`repeatable read`, solo lectura) y la cierra. Entre una vuelta y otra
la base puede dormirse: cada vuelta puede ser un despertar.

Los invariantes:
  1. el dinero ni se crea ni se pierde: la suma de los saldos es la de los avisos; el saldo de cada
     cuenta es la suma de sus movimientos; cada transferencia, dos movimientos que suman 0;
  2. toda transferencia que la API confirmó (transferencias.jsonl) existe;
  3. todo aviso que el banco dio por entregado (avisos.jsonl) está apuntado, una vez y con su importe.

Cada vuelta, una línea en conciliacion.jsonl (y por la salida): `ok` true, false (un invariante
roto: error atribuible) o null (no se pudo mirar, con el error).
"""
import json, os, time

import psycopg

DATOS = "/datos"
INTERVALO = float(os.environ.get("INTERVALO", "120"))
CONEXION = os.environ["CONEXION"]


def ahora():
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def lineas(fichero):
    try:
        with open(os.path.join(DATOS, fichero), encoding="utf-8") as f:
            return [json.loads(l) for l in f if l.strip().endswith("}")]
    except FileNotFoundError:
        return []


def una_vuelta():
    # Lo confirmado ANTES de la foto tiene que estar en ella.
    confirmadas = [t["id"] for t in lineas("transferencias.jsonl")]
    entregados = {a["clave"]: a for a in lineas("avisos.jsonl")}
    t0 = time.time()
    with psycopg.connect(CONEXION, connect_timeout=60, application_name="libro-conciliador") as c:
        conectar_ms = int((time.time() - t0) * 1000)
        c.read_only = True
        c.isolation_level = psycopg.IsolationLevel.REPEATABLE_READ
        with c.transaction(), c.cursor() as k:
            k.execute("select count(*), coalesce(sum(saldo), 0) from cuenta")
            cuentas, total = k.fetchone()
            k.execute("select count(*), coalesce(sum(importe), 0) from aviso")
            avisos, ingresado = k.fetchone()
            k.execute("select count(*) from transferencia")
            (transferencias,) = k.fetchone()
            k.execute("""select count(*) from cuenta c
                          where c.saldo <> coalesce((select sum(m.importe) from movimiento m where m.cuenta = c.id), 0)""")
            (descuadres,) = k.fetchone()
            k.execute("""select count(*) from transferencia t
                          where (select count(*) from movimiento m where m.transferencia = t.id) <> 2
                             or (select sum(m.importe) from movimiento m where m.transferencia = t.id) <> 0""")
            (cojas,) = k.fetchone()
            k.execute("select id from transferencia where id = any(%s)", (confirmadas,))
            faltan_t = len(confirmadas) - k.rowcount
            k.execute("select clave, importe, (select count(*) from movimiento m where m.aviso = a.clave) "
                      "from aviso a where clave = any(%s)", (list(entregados),))
            vistos = {clave: (importe, n) for clave, importe, n in k.fetchall()}
    faltan_a = sum(1 for c in entregados if c not in vistos)
    mal_a = sum(1 for c, (importe, n) in vistos.items() if importe != entregados[c]["importe"] or n != 1)
    rotos = {
        "suma": total != ingresado,
        "descuadres": descuadres,
        "cojas": cojas,
        "faltan_transferencias": faltan_t,
        "faltan_avisos": faltan_a,
        "avisos_mal": mal_a,
    }
    ok = not any(rotos.values())
    return {"ok": ok, "cuentas": cuentas, "transferencias": transferencias, "avisos": avisos,
            "total": int(total), "ingresado": int(ingresado), "confirmadas": len(confirmadas),
            "entregados": len(entregados), **{k: v for k, v in rotos.items() if v}, "conectar_ms": conectar_ms,
            "ms": int((time.time() - t0) * 1000)}


def apunta(fila):
    linea = json.dumps(fila, separators=(",", ":"))
    print(linea, flush=True)
    with open(os.path.join(DATOS, "conciliacion.jsonl"), "a", encoding="utf-8") as f:
        f.write(linea + "\n")


if __name__ == "__main__":
    import sys
    if sys.argv[1:] == ["--una"]:
        # Una vuelta y fuera: sale con 0 sólo si los invariantes cuadran.
        try:
            fila = una_vuelta()
        except Exception as e:  # noqa: BLE001
            fila = {"ok": None, "error": f"{type(e).__name__}: {str(e).strip()[:300]}"}
        apunta({"t": ahora(), "una": True, **fila})
        sys.exit(0 if fila["ok"] else 1)
    while True:
        # Sin pausa de noche: es un cron, corre a su hora haya clientes o no.
        try:
            fila = una_vuelta()
        except Exception as e:  # noqa: BLE001
            fila = {"ok": None, "error": f"{type(e).__name__}: {str(e).strip()[:300]}"}
        apunta({"t": ahora(), **fila})
        time.sleep(INTERVALO)
