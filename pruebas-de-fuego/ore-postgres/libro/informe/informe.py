"""El informe de una corrida de Libro (ADR 0058, P7·2): los SLOs y los errores, de /datos.

  python informe.py [--desde 2026-10-10T12:00:00Z] [--hasta …] [--json]

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

Sale con 1 si hay algún atribuible perdido. Los objetivos son PROVISIONALES: se fijan con el soak
en producción (P7·4); aquí dicen si el laboratorio los cumple.
"""
import json, os, sys
from collections import Counter, defaultdict

DATOS = os.environ.get("DATOS", "/datos")
OBJETIVOS = {"disponibilidad": 99.9, "commit_p99_ms": 1000, "despertar_p95_ms": 10000}


def lineas(fichero, desde, hasta=None):
    """Una a una, sin cargar el fichero: lo que pesa es lo que se guarda, no lo que se lee."""
    try:
        with open(os.path.join(DATOS, fichero), encoding="utf-8") as f:
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


def informe(desde=None, hasta=None):
    perdidos, n_perdidos, absorbidos = [], 0, Counter()

    def pierde(texto):
        nonlocal n_perdidos
        n_perdidos += 1
        if len(perdidos) < TOPE:
            perdidos.append(texto)

    por_pieza = defaultdict(lambda: {"total": 0, "bien": 0, "max_ms": 0})
    commits, despertares, ciclos = Histograma(), defaultdict(list), set()
    primera_t = ultima_t = None
    for o in lineas("operaciones.jsonl", desde, hasta):
        primera_t = primera_t or o["t"]
        ultima_t = o["t"]
        ciclos.add(o.get("ciclo", 0))
        p = por_pieza[o["pieza"]]
        p["total"] += 1
        p["max_ms"] = max(p["max_ms"], o["ms"])
        if o["resultado"] in ("hecha", "sin-fondos"):
            p["bien"] += 1
        elif o["resultado"] != "mala":
            pierde(f"{o['t']} {o['pieza']} {o['op']}: {o['resultado']} {o.get('error') or ''}".strip())
        for e in o.get("errores", []):
            clave = f"{o['pieza']}: {e[:90]}"
            if clave in absorbidos or len(absorbidos) < 1000:
                absorbidos[clave] += 1
            else:
                absorbidos["(otros)"] += 1
        if o["op"] == "transferencia" and o["resultado"] == "hecha":
            commits.mete(o["ms"])
        if o.get("primera"):  # uno por pieza y por noche: pocos, se guardan
            despertares[o["pieza"]].append(o["ms"])
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
    total = sum(p["total"] for p in por_pieza.values())
    bien = sum(p["bien"] for p in por_pieza.values())
    desp = Histograma()
    for v in despertares.values():
        for ms in v:
            desp.mete(ms)
    r = {
        "desde": desde or primera_t,
        "hasta": ultima_t,
        "dias": len(ciclos),
        "operaciones": total,
        "disponibilidad": round(100.0 * bien / total, 3) if total else None,
        "por_pieza": {k: {**v, "disponibilidad": round(100.0 * v["bien"] / v["total"], 3)} for k, v in sorted(por_pieza.items())},
        "commit_ms": {"n": commits.n, "p50": commits.percentil(0.5), "p95": commits.percentil(0.95),
                      "p99": commits.percentil(0.99), "max": commits.max},
        "despertar_ms": {"n": desp.n, "p50": desp.percentil(0.5), "p95": desp.percentil(0.95), "max": desp.max,
                         "por_pieza": {k: sorted(v) for k, v in sorted(despertares.items())}},
        "conciliaciones": conc,
        "cierres": cierres,
        "atribuibles_perdidos": perdidos,
        "perdidos_total": n_perdidos,
        "atribuibles_absorbidos": dict(absorbidos.most_common(10)),
        "absorbidos_total": sum(absorbidos.values()),
    }
    # None: sin datos para decirlo (una corrida sin noches no tiene despertares).
    cumple = lambda v, f: None if v is None else f(v)  # noqa: E731
    r["objetivos"] = {
        "disponibilidad": [OBJETIVOS["disponibilidad"], cumple(r["disponibilidad"], lambda v: v >= OBJETIVOS["disponibilidad"])],
        "commit_p99_ms": [OBJETIVOS["commit_p99_ms"], cumple(r["commit_ms"]["p99"], lambda v: v <= OBJETIVOS["commit_p99_ms"])],
        "despertar_p95_ms": [OBJETIVOS["despertar_p95_ms"], cumple(r["despertar_ms"]["p95"], lambda v: v <= OBJETIVOS["despertar_p95_ms"])],
    }
    return r


def en_texto(r):
    s = lambda b: "—" if b is None else ("✓" if b else "✗")  # noqa: E731
    o = r["objetivos"]
    out = [f"Libro · de {r['desde']} a {r['hasta']} · {r['dias']} días · {r['operaciones']} operaciones",
           f"  disponibilidad   {r['disponibilidad']} %   {s(o['disponibilidad'][1])} objetivo ≥ {o['disponibilidad'][0]} %"]
    for k, v in r["por_pieza"].items():
        out.append(f"    {k:<16} {v['bien']}/{v['total']} ({v['disponibilidad']} %), la espera más larga {v['max_ms']} ms")
    c = r["commit_ms"]
    out.append(f"  commit           p50 {c['p50']} · p95 {c['p95']} · p99 {c['p99']} · máx {c['max']} ms ({c['n']})   "
               f"{s(o['commit_p99_ms'][1])} objetivo p99 ≤ {o['commit_p99_ms'][0]} ms")
    d = r["despertar_ms"]
    out.append(f"  despertar        p50 {d['p50']} · p95 {d['p95']} · máx {d['max']} ms ({d['n']})   "
               f"{s(o['despertar_p95_ms'][1])} objetivo p95 ≤ {o['despertar_p95_ms'][0]} ms")
    for k, v in d["por_pieza"].items():
        out.append(f"    {k:<16} {v}")
    out.append(f"  invariantes      {r['conciliaciones']['cuadran']}/{r['conciliaciones']['total']} conciliaciones cuadran")
    out.append(f"  cierres          {r['cierres']['bien']}/{r['cierres']['total']} (el último exportó {r['cierres']['exportadas_ultima']})")
    out.append(f"  atribuibles      {r['perdidos_total']} perdidos · {r['absorbidos_total']} absorbidos por un reintento")
    for e in r["atribuibles_perdidos"][:10]:
        out.append(f"    ✗ {e[:200]}")
    for e, n in r["atribuibles_absorbidos"].items():
        out.append(f"    · {n} × {e}")
    return "\n".join(out)


if __name__ == "__main__":
    a = sys.argv[1:]
    desde = a[a.index("--desde") + 1] if "--desde" in a else None
    hasta = a[a.index("--hasta") + 1] if "--hasta" in a else None
    r = informe(desde, hasta)
    with open(os.path.join(DATOS, "informe.json"), "w", encoding="utf-8") as f:
        json.dump(r, f, ensure_ascii=False, indent=1)
    print(json.dumps(r, ensure_ascii=False) if "--json" in a else en_texto(r))
    sys.exit(1 if r["perdidos_total"] else 0)
