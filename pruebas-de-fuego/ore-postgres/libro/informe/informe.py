"""El informe de una corrida de Libro (ADR 0058, P7·2): los SLOs y los errores, de /datos.

  python informe.py [--desde 2026-10-10T12:00:00Z] [--hasta …] [--medidas DIR] [--json]

Lee lo que dejaron las piezas (operaciones, conciliacion e informes .jsonl) y saca:
  · **disponibilidad**: de las operaciones de los clientes, las que salieron bien (una transferencia
    rechazada por falta de fondos salió bien: es la respuesta correcta);
  · **commit**: la latencia de las transferencias confirmadas, p50, p95 y p99, desde el cliente
    (petición HTTP a la API, la transacción y la vuelta);
  · **despertar**: la latencia de la primera operación de cada pieza tras cada noche;
  · **invariantes**: las conciliaciones, y si alguna no cuadró;
  · **los errores, clasificados**:
      - atribuibles, perdidos: la operación falló aunque el cliente reintentó; un invariante roto
        (commit perdido, aviso perdido o doble, descuadre); un cron que no pudo conectar;
      - atribuibles, absorbidos: un error que el reintento del cliente tapó (lo vio, aunque no lo
        sufriera); cuentan, pero no quitan disponibilidad;
      - del cliente, no cuentan: sin fondos, petición mal formada; los conflictos `serializable` los
        reintenta la API dentro y no salen de ella.

Con --medidas (el soak en GCP, P7·4), lee además lo que dejan en esa carpeta:
  · eventos.jsonl (golpes.sh): cada golpe abre una ventana desde que empieza hasta VENTANA_S después
    de que acaba; la disponibilidad se da dentro y fuera, y por golpe, lo que vio Libro. El hecho
    `pool-encendido` separa los despertares en frío de los del pool;
  · recursos.jsonl (muestrea.sh): si la memoria de cada pieza crece, y si de noche queda alguna VM
    o algún nodo `pg` (con el reloj de la corrida).

Los objetivos son los del soak en GCP, aprobados el 2026-10-10 (ADR 0058, P7·4); en el laboratorio
dicen si se cumplirían. Sale con 1 si hay algún atribuible perdido.
"""
import json, math, os, sys
from collections import Counter, defaultdict, deque
from datetime import datetime

DATOS = os.environ.get("DATOS", "/datos")
VENTANA_S = 600  # lo que dura una ventana de golpe después de que el golpe acaba
# Los del soak en GCP (aprobados el 2026-10-10).
OBJETIVOS = {
    "disponibilidad": 99.9, "disponibilidad_fuera": 99.99, "commit_p50_ms": 50, "commit_p99_ms": 250,
    "despertar_frio_p95_ms": 45000, "despertar_pool_p95_ms": 8000, "reparar_frio_ms": 60000,
    "reparar_pool_ms": 15000, "pageserver_max_ms": 30000, "crecimiento_pct": 20, "noche_sin_vms_min": 15,
}
GOLPES = ("matar-vm", "pageserver", "safekeeper", "proxy", "migrar", "ore-postgres")


def epoca(t):
    return datetime.fromisoformat(t.replace("Z", "+00:00")).timestamp()


def lineas(fichero, desde, hasta=None, carpeta=None):
    """Una a una, sin cargar el fichero: lo que pesa es lo que se guarda, no lo que se lee."""
    try:
        with open(os.path.join(carpeta or DATOS, fichero), encoding="utf-8") as f:
            for l in f:
                if l.strip().endswith("}"):
                    d = json.loads(l)
                    if (not desde or d.get("t", "") >= desde) and (not hasta or d.get("t", "") <= hasta):
                        yield d
    except FileNotFoundError:
        return


class Histograma:
    """Latencias en memoria fija (P7·4): exacto al milisegundo por debajo de 1 s, a 10 ms hasta
    10 s y a 100 ms por encima; el máximo, exacto. Unos pocos miles de casillas sean cuantas sean
    las muestras: el error de un percentil es, como mucho, el ancho de su casilla (1 %)."""

    def __init__(self):
        self.casillas, self.n, self.max = Counter(), 0, None

    def mete(self, ms):
        b = ms if ms < 1000 else (ms // 10 * 10 if ms < 10000 else ms // 100 * 100)
        self.casillas[b] += 1
        self.n += 1
        self.max = ms if self.max is None else max(self.max, ms)

    def percentil(self, q):
        if not self.n:
            return None
        objetivo, visto = min(self.n - 1, max(0, round(q * (self.n - 1)))), 0
        for b in sorted(self.casillas):
            visto += self.casillas[b]
            if visto > objetivo:
                return b
        return self.max


TOPE = 100  # de cada lista de errores se guardan los primeros; el total, siempre


def informe(desde=None, hasta=None, medidas=None):
    perdidos, n_perdidos, absorbidos = [], 0, Counter()

    def pierde(texto):
        nonlocal n_perdidos
        n_perdidos += 1
        if len(perdidos) < TOPE:
            perdidos.append(texto)

    # Los golpes y el pool, de golpes.sh.
    eventos = list(lineas("eventos.jsonl", desde, hasta, medidas)) if medidas else []
    ventanas = [{"golpe": e["golpe"], "t": e["t"], "de": epoca(e["t"]), "a": epoca(e["fin"]) + VENTANA_S,
                 "ok_golpe": e.get("ok"), "total": 0, "bien": 0, "max_ms": 0, "absorbidos": 0, "perdidos": 0}
                for e in eventos if e["golpe"] in GOLPES]
    pool_desde = next((epoca(e["t"]) for e in eventos if e["golpe"] == "pool-encendido"), math.inf)
    for v in ventanas:
        v["con_pool"] = v["de"] >= pool_desde

    por_pieza = defaultdict(lambda: {"total": 0, "bien": 0, "max_ms": 0})
    fuera = {"total": 0, "bien": 0}
    commits, despertares, ciclos = Histograma(), defaultdict(list), set()
    desp_frio, desp_pool = Histograma(), Histograma()
    primera_t = ultima_t = None
    for o in lineas("operaciones.jsonl", desde, hasta):
        primera_t = primera_t or o["t"]
        ultima_t = o["t"]
        ciclos.add(o.get("ciclo", 0))
        bien = o["resultado"] in ("hecha", "sin-fondos")
        p = por_pieza[o["pieza"]]
        p["total"] += 1
        p["max_ms"] = max(p["max_ms"], o["ms"])
        if bien:
            p["bien"] += 1
        elif o["resultado"] != "mala":
            pierde(f"{o['t']} {o['pieza']} {o['op']}: {o['resultado']} {o.get('error') or ''}".strip())
        for e in o.get("errores", []):
            clave = f"{o['pieza']}: {e[:90]}"
            if clave in absorbidos or len(absorbidos) < 1000:
                absorbidos[clave] += 1
            else:
                absorbidos["(otros)"] += 1
        if ventanas:
            t = epoca(o["t"])
            dentro = [v for v in ventanas if v["de"] <= t <= v["a"]]
            for v in dentro:
                v["total"] += 1
                v["bien"] += bien
                v["max_ms"] = max(v["max_ms"], o["ms"])
                v["absorbidos"] += len(o.get("errores", []))
                v["perdidos"] += (not bien and o["resultado"] != "mala")
            if not dentro:
                fuera["total"] += 1
                fuera["bien"] += bien
        if o.get("primera"):  # uno por pieza y por noche: pocos, se guardan
            despertares[o["pieza"]].append(o["ms"])
            (desp_pool if epoca(o["t"]) >= pool_desde else desp_frio).mete(o["ms"])
        elif o["op"] == "transferencia" and o["resultado"] == "hecha":
            commits.mete(o["ms"])  # el commit, sin los despertares
    conc = {"total": 0, "cuadran": 0}
    for c in lineas("conciliacion.jsonl", desde, hasta):
        conc["total"] += 1
        if c.get("ok") is True:
            conc["cuadran"] += 1
        elif c.get("ok") is False:
            rotos = {k: c[k] for k in ("suma", "descuadres", "cojas", "faltan_transferencias", "faltan_avisos", "avisos_mal") if c.get(k)}
            pierde(f"{c['t']} libro-conciliador: invariante roto {rotos}")
        else:
            pierde(f"{c['t']} libro-conciliador: no pudo mirar: {c.get('error')}")
    cierres = {"total": 0, "bien": 0, "exportadas_ultima": None}
    for i in lineas("informes.jsonl", desde, hasta):
        cierres["total"] += 1
        if i.get("ok") is True:
            cierres["bien"] += 1
            cierres["exportadas_ultima"] = i.get("exportadas")
        else:
            pierde(f"{i['t']} libro-informes: {i.get('error')}")
    recursos = leer_recursos(desde, hasta, medidas) if medidas else None
    total = sum(p["total"] for p in por_pieza.values())
    bien = sum(p["bien"] for p in por_pieza.values())
    desp = Histograma()
    for v in despertares.values():
        for ms in v:
            desp.mete(ms)
    pct = lambda b, t: round(100.0 * b / t, 3) if t else None  # noqa: E731
    r = {
        "desde": desde or primera_t,
        "hasta": ultima_t,
        "dias": len(ciclos),
        "operaciones": total,
        "disponibilidad": pct(bien, total),
        "disponibilidad_fuera": pct(fuera["bien"], fuera["total"]) if ventanas else None,
        "por_pieza": {k: {**v, "disponibilidad": pct(v["bien"], v["total"])} for k, v in sorted(por_pieza.items())},
        "commit_ms": {"n": commits.n, "p50": commits.percentil(0.5), "p95": commits.percentil(0.95),
                      "p99": commits.percentil(0.99), "max": commits.max},
        "despertar_ms": {"n": desp.n, "p50": desp.percentil(0.5), "p95": desp.percentil(0.95), "max": desp.max,
                         "frio": {"n": desp_frio.n, "p95": desp_frio.percentil(0.95), "max": desp_frio.max},
                         "pool": {"n": desp_pool.n, "p95": desp_pool.percentil(0.95), "max": desp_pool.max},
                         "por_pieza": {k: sorted(v) for k, v in sorted(despertares.items())}},
        "golpes": [{k: v[k] for k in ("golpe", "t", "ok_golpe", "con_pool", "total", "bien", "max_ms", "absorbidos", "perdidos")}
                   for v in ventanas],
        "recursos": recursos,
        "conciliaciones": conc,
        "cierres": cierres,
        "atribuibles_perdidos": perdidos,
        "perdidos_total": n_perdidos,
        "atribuibles_absorbidos": dict(absorbidos.most_common(10)),
        "absorbidos_total": sum(absorbidos.values()),
    }
    r["objetivos"] = objetivos(r)
    return r


def leer_recursos(desde, hasta, medidas):
    """→ {crecimiento_mem_pct: {rol: %}, noches: {muestras, limpias}} de recursos.jsonl. El
    crecimiento: la mediana de las 6 últimas muestras frente a la de las 6 primeras."""
    primeras, ultimas = defaultdict(list), defaultdict(lambda: deque(maxlen=6))
    actual, t_actual = defaultdict(float), None

    def cierra():
        for rol, mem in actual.items():
            if len(primeras[rol]) < 6:
                primeras[rol].append(mem)
            ultimas[rol].append(mem)

    reloj = None
    try:
        with open(os.path.join(DATOS, "reloj.json")) as f:
            reloj = json.load(f)
    except (FileNotFoundError, ValueError):
        pass
    noches = {"muestras": 0, "limpias": 0}
    for x in lineas("recursos.jsonl", desde, hasta, medidas):
        if x["tipo"] == "pod":
            if x["t"] != t_actual:
                cierra()
                actual, t_actual = defaultdict(float), x["t"]
            actual[x["rol"]] += x["mem_mi"]
        elif x["tipo"] == "computo" and reloj:
            pasado = epoca(x["t"]) - reloj["origen"]
            dentro = pasado % (reloj["dia_s"] + reloj["noche_s"]) - reloj["dia_s"]
            if dentro >= OBJETIVOS["noche_sin_vms_min"] * 60:  # de noche, pasados 15 min
                noches["muestras"] += 1
                noches["limpias"] += x["vms"] == 0 and x["nodos_pg"] == 0
    cierra()
    mediana = lambda v: sorted(v)[len(v) // 2]  # noqa: E731
    crecimiento = {rol: round(100.0 * (mediana(list(ultimas[rol])) / mediana(primeras[rol]) - 1), 1)
                   for rol in primeras if primeras[rol] and mediana(primeras[rol]) > 0}
    return {"crecimiento_mem_pct": crecimiento, "noches": noches}


def objetivos(r):
    """Cada objetivo: [lo pedido, lo medido, cumple]; cumple es None si no hay con qué decirlo."""
    O = OBJETIVOS
    si = lambda v, f: None if v is None else bool(f(v))  # noqa: E731
    g = {x["golpe"]: x for x in r["golpes"]}
    rec = r["recursos"] or {}
    crec = rec.get("crecimiento_mem_pct", {})
    peor = max((crec[k] for k in ("ore-postgres", "proxy") if k in crec), default=None)
    noches = rec.get("noches", {})
    rep = g.get("matar-vm")
    tope_rep = (O["reparar_pool_ms"] if rep["con_pool"] else O["reparar_frio_ms"]) if rep else None
    sin_nada = lambda x: None if x is None else x["perdidos"] == 0 and x["absorbidos"] == 0  # noqa: E731
    ps = g.get("pageserver")
    return {
        "1 · atribuibles perdidos": ["0", r["perdidos_total"], r["perdidos_total"] == 0],
        "2 · disponibilidad": [f"≥ {O['disponibilidad']} %", r["disponibilidad"],
                               si(r["disponibilidad"], lambda v: v >= O["disponibilidad"])],
        "2 · fuera de golpes": [f"≥ {O['disponibilidad_fuera']} %", r["disponibilidad_fuera"],
                                si(r["disponibilidad_fuera"], lambda v: v >= O["disponibilidad_fuera"])],
        "3 · commit p50": [f"≤ {O['commit_p50_ms']} ms", r["commit_ms"]["p50"],
                           si(r["commit_ms"]["p50"], lambda v: v <= O["commit_p50_ms"])],
        "3 · commit p99": [f"≤ {O['commit_p99_ms']} ms", r["commit_ms"]["p99"],
                           si(r["commit_ms"]["p99"], lambda v: v <= O["commit_p99_ms"])],
        "4 · despertar en frío p95": [f"≤ {O['despertar_frio_p95_ms']} ms", r["despertar_ms"]["frio"]["p95"],
                                      si(r["despertar_ms"]["frio"]["p95"], lambda v: v <= O["despertar_frio_p95_ms"])],
        "5 · despertar desde el pool p95": [f"≤ {O['despertar_pool_p95_ms']} ms", r["despertar_ms"]["pool"]["p95"],
                                            si(r["despertar_ms"]["pool"]["p95"], lambda v: v <= O["despertar_pool_p95_ms"])],
        "6 · reparar un cómputo muerto": [f"≤ {tope_rep} ms" if tope_rep else "—", rep["max_ms"] if rep else None,
                                          si(rep["max_ms"] if rep else None, lambda v: v <= tope_rep)],
        "7 · pageserver": [f"nada perdido, ≤ {O['pageserver_max_ms']} ms", ps["max_ms"] if ps else None,
                           None if not ps else ps["perdidos"] == 0 and ps["max_ms"] <= O["pageserver_max_ms"]],
        "7 · un safekeeper": ["nada visible", g["safekeeper"]["absorbidos"] if "safekeeper" in g else None,
                              sin_nada(g.get("safekeeper"))],
        "7 · migración en vivo": ["ninguna sesión cortada", g["migrar"]["absorbidos"] if "migrar" in g else None,
                                  sin_nada(g.get("migrar"))],
        "8 · de noche, ni VM ni nodo": ["todas las muestras",
                                        f"{noches['limpias']}/{noches['muestras']}" if noches.get("muestras") else None,
                                        None if not noches.get("muestras") else noches["limpias"] == noches["muestras"]],
        "9 · memoria de ore-postgres y del proxy": [f"≤ +{O['crecimiento_pct']} %", peor,
                                                    si(peor, lambda v: v <= O["crecimiento_pct"])],
    }


def en_texto(r):
    s = lambda b: "—" if b is None else ("✓" if b else "✗")  # noqa: E731
    out = [f"Libro · de {r['desde']} a {r['hasta']} · {r['dias']} días · {r['operaciones']} operaciones"]
    for k, v in r["por_pieza"].items():
        out.append(f"    {k:<16} {v['bien']}/{v['total']} ({v['disponibilidad']} %), la espera más larga {v['max_ms']} ms")
    c, d = r["commit_ms"], r["despertar_ms"]
    out.append(f"  commit           p50 {c['p50']} · p95 {c['p95']} · p99 {c['p99']} · máx {c['max']} ms ({c['n']}, sin despertares)")
    out.append(f"  despertar        p50 {d['p50']} · p95 {d['p95']} · máx {d['max']} ms ({d['n']}; "
               f"en frío {d['frio']['n']}, desde el pool {d['pool']['n']})")
    for k, v in d["por_pieza"].items():
        out.append(f"    {k:<16} {v}")
    if r["golpes"]:
        out.append("  golpes           (ventana: desde que empieza hasta 10 min después de acabar)")
        for x in r["golpes"]:
            aviso = "" if x["ok_golpe"] else " · ⚠ el golpe no salió"
            out.append(f"    {x['t'][:19]} {x['golpe']:<13} {x['bien']}/{x['total']} bien · espera máx {x['max_ms']} ms · "
                       f"{x['absorbidos']} absorbidos · {x['perdidos']} perdidos{' · con pool' if x['con_pool'] else ''}{aviso}")
    if r["recursos"]:
        out.append(f"  memoria          crecimiento: {r['recursos']['crecimiento_mem_pct']}")
    out.append(f"  invariantes      {r['conciliaciones']['cuadran']}/{r['conciliaciones']['total']} conciliaciones cuadran")
    out.append(f"  cierres          {r['cierres']['bien']}/{r['cierres']['total']} (el último exportó {r['cierres']['exportadas_ultima']})")
    out.append(f"  atribuibles      {r['perdidos_total']} perdidos · {r['absorbidos_total']} absorbidos por un reintento")
    for e in r["atribuibles_perdidos"][:10]:
        out.append(f"    ✗ {e[:200]}")
    for e, n in r["atribuibles_absorbidos"].items():
        out.append(f"    · {n} × {e}")
    out.append("  objetivos (los del soak en GCP)")
    for k, (pedido, medido, cumple) in r["objetivos"].items():
        out.append(f"    {s(cumple)} {k:<40} {pedido:<28} medido: {medido if medido is not None else '—'}")
    return "\n".join(out)


if __name__ == "__main__":
    a = sys.argv[1:]
    desde = a[a.index("--desde") + 1] if "--desde" in a else None
    hasta = a[a.index("--hasta") + 1] if "--hasta" in a else None
    medidas = a[a.index("--medidas") + 1] if "--medidas" in a else None
    r = informe(desde, hasta, medidas)
    with open(os.path.join(DATOS, "informe.json"), "w", encoding="utf-8") as f:
        json.dump(r, f, ensure_ascii=False, indent=1)
    print(json.dumps(r, ensure_ascii=False) if "--json" in a else en_texto(r))
    sys.exit(1 if r["perdidos_total"] else 0)
