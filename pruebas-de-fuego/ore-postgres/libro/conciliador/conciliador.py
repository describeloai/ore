"""El conciliador de Libro (ADR 0058, P7): Python y psycopg, directo al cómputo (TCP). Cada
INTERVALO segundos, como el cron de un departamento financiero, abre UNA conexión, comprueba los
invariantes y la cierra. Entre una vuelta y otra la base puede dormirse: cada vuelta puede ser un
despertar.

Los invariantes:
  1. el dinero ni se crea ni se pierde: la suma de los saldos es la de los avisos; el saldo de cada
     cuenta es la suma de sus movimientos; cada transferencia, dos movimientos que suman 0;
  2. toda transferencia que la API confirmó (transferencias.jsonl) existe;
  3. todo aviso que el banco dio por entregado (avisos.jsonl) está apuntado, una vez y con su importe.

**Preparado para días** (P7·4). Un soak de una semana son millones de filas; mirarlo todo en cada
vuelta acabaría midiendo al conciliador y no a la base. Así que hay dos clases de vuelta:
  · **incremental** (la de cada INTERVALO): recuerda en conciliador.json hasta dónde leyó los
    ficheros y, por cuenta, la suma de los movimientos hasta un id de control; mira solo lo nuevo.
    El id de control solo avanza sobre movimientos de hace más de HORIZONTE segundos: una
    transacción de la API dura como mucho 30 s, así que ningún commit lento con un id menor puede
    aparecer después por debajo de él. Cuesta lo nuevo, no lo acumulado.
  · **completa** (al arrancar, con --una, y cada COMPLETA_S): todo desde cero; los ids confirmados
    suben a una tabla temporal (COPY) y se cruzan de una vez. Hace falta: un commit perdido de
    verdad se lleva también su cambio de saldo, así que el dinero sigue cuadrando y sólo se ve
    buscando su id; la incremental lo ve si es nuevo, la completa si es viejo.

Cada vuelta, una línea en conciliacion.jsonl (y por la salida): `ok` true, false (un invariante
roto: error atribuible) o null (no se pudo mirar, con el error).
"""
import json, os, sys, time

import psycopg

DATOS = "/datos"
ESTADO = os.path.join(DATOS, "conciliador.json")
INTERVALO = float(os.environ.get("INTERVALO", "120"))
COMPLETA_S = float(os.environ.get("COMPLETA_S", "3600"))
HORIZONTE = int(os.environ.get("HORIZONTE", "300"))
CONEXION = os.environ.get("CONEXION", "")


def ahora():
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def nuevas(fichero, desde):
    """Las líneas completas desde el byte `desde`. → (filas, byte siguiente)."""
    try:
        with open(os.path.join(DATOS, fichero), "rb") as f:
            f.seek(desde)
            bloque = f.read()
    except FileNotFoundError:
        return [], desde
    fin = bloque.rfind(b"\n") + 1  # una línea a medio escribir se queda para la próxima
    filas = [json.loads(l) for l in bloque[:fin].splitlines() if l.strip()]
    return filas, desde + fin


def leer_estado():
    try:
        with open(ESTADO) as f:
            return json.load(f)
    except (FileNotFoundError, ValueError):
        return None


def guardar_estado(e):
    tmp = ESTADO + ".tmp"
    with open(tmp, "w") as f:
        json.dump(e, f)
    os.replace(tmp, ESTADO)


def faltan_avisos(k, entregados, tabla=None):
    """→ (faltan, mal): entregados que no están, y los que están con otro importe o ≠ 1 movimiento."""
    if tabla:
        k.execute(f"""select count(*) filter (where a.clave is null),
                             count(*) filter (where a.clave is not null and (a.importe <> e.importe or
                               (select count(*) from movimiento m where m.aviso = a.clave) <> 1))
                        from {tabla} e left join aviso a on a.clave = e.clave""")
        return k.fetchone()
    if not entregados:
        return 0, 0
    k.execute("select clave, importe, (select count(*) from movimiento m where m.aviso = a.clave) "
              "from aviso a where clave = any(%s)", (list(entregados),))
    vistos = {c: (i, n) for c, i, n in k.fetchall()}
    faltan = sum(1 for c in entregados if c not in vistos)
    mal = sum(1 for c, (i, n) in vistos.items() if i != entregados[c] or n != 1)
    return faltan, mal


def completa(c):
    """Todo desde cero. → (fila, estado nuevo)."""
    t_filas, t_fin = nuevas("transferencias.jsonl", 0)
    a_filas, a_fin = nuevas("avisos.jsonl", 0)
    with c.cursor() as k:
        k.execute("create temp table conf (id text primary key) on commit preserve rows")
        k.execute("create temp table entr (clave text primary key, importe bigint) on commit preserve rows")
        with k.copy("copy conf (id) from stdin") as cp:
            for t in {t["id"] for t in t_filas}:
                cp.write_row((t,))
        with k.copy("copy entr (clave, importe) from stdin") as cp:
            for a in {a["clave"]: a for a in a_filas}.values():
                cp.write_row((a["clave"], a["importe"]))
        k.execute("analyze conf; analyze entr")
    c.commit()
    c.read_only = True
    c.isolation_level = psycopg.IsolationLevel.REPEATABLE_READ
    with c.transaction(), c.cursor() as k:
        k.execute("select count(*), coalesce(sum(saldo), 0) from cuenta")
        cuentas, total = k.fetchone()
        k.execute("select count(*), coalesce(sum(importe), 0) from aviso")
        avisos, ingresado = k.fetchone()
        k.execute("select count(*) from transferencia")
        (transferencias,) = k.fetchone()
        # El id de control: el mayor de los movimientos de hace más de HORIZONTE s.
        k.execute("select coalesce(max(id), 0) from movimiento where cuando < now() - make_interval(secs => %s)", (HORIZONTE,))
        (control,) = k.fetchone()
        k.execute("""select c.id, c.saldo, coalesce(m.s, 0), coalesce(m.hasta, 0)
                       from cuenta c left join (select cuenta, sum(importe) s,
                                                       sum(importe) filter (where id <= %s) hasta
                                                  from movimiento group by cuenta) m on m.cuenta = c.id""", (control,))
        filas = k.fetchall()
        descuadres = sum(1 for _, saldo, s, _ in filas if saldo != s)
        k.execute("""select count(*) from (select transferencia from movimiento where transferencia is not null
                       group by transferencia having count(*) <> 2 or sum(importe) <> 0) x""")
        (cojas,) = k.fetchone()
        k.execute("select count(*) from transferencia t where not exists (select 1 from movimiento m where m.transferencia = t.id)")
        (sin_mov,) = k.fetchone()
        k.execute("select count(*) from conf left join transferencia t using (id) where t.id is null")
        (faltan_t,) = k.fetchone()
        faltan_a, mal_a = faltan_avisos(k, None, "entr")
    estado = {"t_off": t_fin, "a_off": a_fin, "confirmadas": len({t["id"] for t in t_filas}),
              "entregados": len({a["clave"] for a in a_filas}), "control": control,
              "sumas": {cid: int(hasta) for cid, _, _, hasta in filas}, "ultima_completa": time.time()}
    rotos = {"suma": total != ingresado, "descuadres": descuadres, "cojas": cojas + sin_mov,
             "faltan_transferencias": faltan_t, "faltan_avisos": faltan_a, "avisos_mal": mal_a}
    return {"modo": "completa", "cuentas": cuentas, "transferencias": transferencias, "avisos": avisos,
            "total": int(total), "ingresado": int(ingresado), "confirmadas": estado["confirmadas"],
            "entregados": estado["entregados"], **rotos}, estado


def incremental(c, e):
    """Sólo lo nuevo desde `e`. → (fila, estado nuevo)."""
    t_filas, t_fin = nuevas("transferencias.jsonl", e["t_off"])
    a_filas, a_fin = nuevas("avisos.jsonl", e["a_off"])
    nuevas_t = list({t["id"] for t in t_filas})
    entregados = {a["clave"]: a["importe"] for a in a_filas}
    c.read_only = True
    c.isolation_level = psycopg.IsolationLevel.REPEATABLE_READ
    with c.transaction(), c.cursor() as k:
        k.execute("select count(*), coalesce(sum(saldo), 0) from cuenta")
        cuentas, total = k.fetchone()
        k.execute("select count(*), coalesce(sum(importe), 0) from aviso")
        avisos, ingresado = k.fetchone()
        # El saldo de cada cuenta = lo sumado hasta el control + lo que hay por encima de él.
        k.execute("select cuenta, sum(importe) from movimiento where id > %s group by cuenta", (e["control"],))
        encima = dict(k.fetchall())
        k.execute("select id, saldo from cuenta")
        saldos = dict(k.fetchall())
        descuadres = sum(1 for cid, saldo in saldos.items()
                         if saldo != e["sumas"].get(cid, 0) + int(encima.get(cid, 0)))
        # Las transferencias nuevas: sus dos movimientos nacen en la misma transacción, así que
        # caen los dos del mismo lado del control.
        k.execute("""select count(*) from (select transferencia from movimiento
                       where id > %s and transferencia is not null
                       group by transferencia having count(*) <> 2 or sum(importe) <> 0) x""", (e["control"],))
        (cojas,) = k.fetchone()
        k.execute("select count(*) from transferencia where id = any(%s)", (nuevas_t,))
        (estan,) = k.fetchone()
        faltan_a, mal_a = faltan_avisos(k, entregados)
        # El control avanza sobre lo que ya tiene más de HORIZONTE s.
        k.execute("""select coalesce(max(id), %s) from movimiento
                      where id > %s and cuando < now() - make_interval(secs => %s)""", (e["control"], e["control"], HORIZONTE))
        (control,) = k.fetchone()
        k.execute("select cuenta, sum(importe) from movimiento where id > %s and id <= %s group by cuenta", (e["control"], control))
        sumas = dict(e["sumas"])
        for cid, s in k.fetchall():
            sumas[cid] = sumas.get(cid, 0) + int(s)
    estado = {**e, "t_off": t_fin, "a_off": a_fin, "confirmadas": e["confirmadas"] + len(nuevas_t),
              "entregados": e["entregados"] + len(entregados), "control": control, "sumas": sumas}
    rotos = {"suma": total != ingresado, "descuadres": descuadres, "cojas": cojas,
             "faltan_transferencias": len(nuevas_t) - estan, "faltan_avisos": faltan_a, "avisos_mal": mal_a}
    # Sin contar `transferencia` entera: es lo único que crecería con los días (está en la completa).
    return {"modo": "incremental", "cuentas": cuentas, "avisos": avisos,
            "total": int(total), "ingresado": int(ingresado), "confirmadas": estado["confirmadas"],
            "entregados": estado["entregados"], "nuevas": len(nuevas_t), "nuevos_avisos": len(entregados),
            **rotos}, estado


def una_vuelta(forzar_completa=False):
    e = leer_estado()
    toca = forzar_completa or e is None or time.time() - e.get("ultima_completa", 0) >= COMPLETA_S
    t0 = time.time()
    with psycopg.connect(CONEXION, connect_timeout=60, application_name="libro-conciliador") as c:
        conectar_ms = int((time.time() - t0) * 1000)
        fila, nuevo = completa(c) if toca else incremental(c, e)
    # Con un invariante roto, el estado no avanza: la vuelta siguiente lo vuelve a ver.
    ok = not any(fila[k] for k in ("suma", "descuadres", "cojas", "faltan_transferencias", "faltan_avisos", "avisos_mal"))
    if ok:
        guardar_estado(nuevo)
    limpia = {k: v for k, v in fila.items() if v or k not in ("suma", "descuadres", "cojas", "faltan_transferencias", "faltan_avisos", "avisos_mal")}
    return {"ok": ok, **limpia, "conectar_ms": conectar_ms, "ms": int((time.time() - t0) * 1000)}


def apunta(fila):
    linea = json.dumps(fila, separators=(",", ":"))
    print(linea, flush=True)
    with open(os.path.join(DATOS, "conciliacion.jsonl"), "a", encoding="utf-8") as f:
        f.write(linea + "\n")


def vuelta_apuntada(forzar_completa=False):
    try:
        fila = una_vuelta(forzar_completa)
    except Exception as e:  # noqa: BLE001
        fila = {"ok": None, "error": f"{type(e).__name__}: {str(e).strip()[:300]}"}
    apunta({"t": ahora(), **fila})
    return fila


if __name__ == "__main__":
    a = sys.argv[1:]
    if "--una" in a or "--incremental" in a:
        # Una vuelta y fuera: completa (--una) o incremental (--incremental, si hay estado).
        # Sale con 0 sólo si los invariantes cuadran.
        fila = vuelta_apuntada(forzar_completa="--una" in a)
        sys.exit(0 if fila.get("ok") else 1)
    while True:
        # Sin pausa de noche: es un cron, corre a su hora haya clientes o no.
        vuelta_apuntada()
        time.sleep(INTERVALO)
