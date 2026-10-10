"""El informe de una corrida de Libro (ADR 0058, P7·2): los SLOs y los errores, de /datos.

  python informe.py [--desde 2026-10-10T12:00:00Z] [--json]

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


def lineas(fichero, desde):
    try:
        with open(os.path.join(DATOS, fichero), encoding="utf-8") as f:
            for l in f:
                if l.strip().endswith("}"):
                    d = json.loads(l)
                    if not desde or d.get("t", "") >= desde:
                        yield d
    except FileNotFoundError:
        return


def percentil(v, q):
    if not v:
        return None
    v = sorted(v)
    return v[min(len(v) - 1, max(0, round(q * (len(v) - 1))))]


def informe(desde=None):
    ops = list(lineas("operaciones.jsonl", desde))
    conc = list(lineas("conciliacion.jsonl", desde))
    infs = list(lineas("informes.jsonl", desde))
    perdidos, absorbidos = [], Counter()
    por_pieza = defaultdict(lambda: {"total": 0, "bien": 0})
    commits, despertares = [], defaultdict(list)
    for o in ops:
        p = por_pieza[o["pieza"]]
        p["total"] += 1
        if o["resultado"] in ("hecha", "sin-fondos"):
            p["bien"] += 1
        elif o["resultado"] != "mala":
            perdidos.append(f"{o['t']} {o['pieza']} {o['op']}: {o['resultado']} {o.get('error') or ''}".strip())
        for e in o.get("errores", []):
            absorbidos[f"{o['pieza']}: {e[:90]}"] += 1
        if o["op"] == "transferencia" and o["resultado"] == "hecha":
            commits.append(o["ms"])
        if o.get("primera"):
            despertares[o["pieza"]].append(o["ms"])
    for c in conc:
        if c.get("ok") is False:
            rotos = {k: c[k] for k in ("suma", "descuadres", "cojas", "faltan_transferencias", "faltan_avisos", "avisos_mal") if c.get(k)}
            perdidos.append(f"{c['t']} libro-conciliador: invariante roto {rotos}")
        elif c.get("ok") is None:
            perdidos.append(f"{c['t']} libro-conciliador: no pudo mirar: {c.get('error')}")
    for i in infs:
        if i.get("ok") is not True:
            perdidos.append(f"{i['t']} libro-informes: {i.get('error')}")
    total = sum(p["total"] for p in por_pieza.values())
    bien = sum(p["bien"] for p in por_pieza.values())
    todos_desp = [ms for v in despertares.values() for ms in v]
    r = {
        "desde": desde or (ops[0]["t"] if ops else None),
        "hasta": ops[-1]["t"] if ops else None,
        "dias": len({o.get("ciclo", 0) for o in ops}),
        "operaciones": total,
        "disponibilidad": round(100.0 * bien / total, 3) if total else None,
        "por_pieza": {k: {**v, "disponibilidad": round(100.0 * v["bien"] / v["total"], 3)} for k, v in sorted(por_pieza.items())},
        "commit_ms": {"n": len(commits), "p50": percentil(commits, 0.5), "p95": percentil(commits, 0.95),
                      "p99": percentil(commits, 0.99), "max": max(commits) if commits else None},
        "despertar_ms": {"n": len(todos_desp), "p50": percentil(todos_desp, 0.5), "p95": percentil(todos_desp, 0.95),
                         "max": max(todos_desp) if todos_desp else None,
                         "por_pieza": {k: sorted(v) for k, v in sorted(despertares.items())}},
        "conciliaciones": {"total": len(conc), "cuadran": sum(1 for c in conc if c.get("ok") is True)},
        "cierres": {"total": len(infs), "bien": sum(1 for i in infs if i.get("ok") is True),
                    "exportadas_ultima": next((i.get("exportadas") for i in reversed(infs) if i.get("ok")), None)},
        "atribuibles_perdidos": perdidos,
        "atribuibles_absorbidos": dict(absorbidos.most_common(10)),
        "absorbidos_total": sum(absorbidos.values()),
    }
    r["objetivos"] = {
        "disponibilidad": [OBJETIVOS["disponibilidad"], r["disponibilidad"] is not None and r["disponibilidad"] >= OBJETIVOS["disponibilidad"]],
        "commit_p99_ms": [OBJETIVOS["commit_p99_ms"], r["commit_ms"]["p99"] is not None and r["commit_ms"]["p99"] <= OBJETIVOS["commit_p99_ms"]],
        "despertar_p95_ms": [OBJETIVOS["despertar_p95_ms"], r["despertar_ms"]["p95"] is not None and r["despertar_ms"]["p95"] <= OBJETIVOS["despertar_p95_ms"]],
    }
    return r


def en_texto(r):
    s = lambda b: "✓" if b else "✗"  # noqa: E731
    o = r["objetivos"]
    out = [f"Libro · de {r['desde']} a {r['hasta']} · {r['dias']} días · {r['operaciones']} operaciones",
           f"  disponibilidad   {r['disponibilidad']} %   {s(o['disponibilidad'][1])} objetivo ≥ {o['disponibilidad'][0]} %"]
    for k, v in r["por_pieza"].items():
        out.append(f"    {k:<16} {v['bien']}/{v['total']} ({v['disponibilidad']} %)")
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
    out.append(f"  atribuibles      {len(r['atribuibles_perdidos'])} perdidos · {r['absorbidos_total']} absorbidos por un reintento")
    for e in r["atribuibles_perdidos"][:10]:
        out.append(f"    ✗ {e[:200]}")
    for e, n in r["atribuibles_absorbidos"].items():
        out.append(f"    · {n} × {e}")
    return "\n".join(out)


if __name__ == "__main__":
    a = sys.argv[1:]
    desde = a[a.index("--desde") + 1] if "--desde" in a else None
    r = informe(desde)
    with open(os.path.join(DATOS, "informe.json"), "w", encoding="utf-8") as f:
        json.dump(r, f, ensure_ascii=False, indent=1)
    print(json.dumps(r, ensure_ascii=False) if "--json" in a else en_texto(r))
    sys.exit(1 if r["atribuibles_perdidos"] else 0)
