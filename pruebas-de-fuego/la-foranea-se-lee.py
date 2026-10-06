"""0057 B4·0 · la matriz de LA FORANEA SE LEE (lo lanza `la-foranea-se-lee.sh`).

Cada fila es una clase de consulta (T1–T7 sobre tablas y vistas, M1–M4 sobre la
coleccion virtual; v1alpha27, ORE 0057), cada columna un camino. Una celda dice
lo que contesta: `✓ n` (filas), o el codigo y el motivo, cortos. Falla si una
celda no da lo esperado (`NO`: lo que tiene que negarse, y con que codigo).
"""
import json
import os
import sys
import time
import urllib.request
import warnings

BASE, PUESTO = sys.argv[1], sys.argv[2]
sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "puesto", "python"))
import ore  # noqa: E402

ore.session._cabeceras = {"x-ore-sujeto": "agente:local"}
SUJ = {"x-ore-sujeto": "persona:ana"}


def corto(e):
    t = str(e).replace("\n", " ")
    for k in ("OOS2051", "OOS2049", "OOS2044", "OOS2045", "OOS4011", "OOS2018", "409", "403", "404", "422", "500"):
        if k in t:
            return f"✗ {k}: {t[:90]}"
    return f"✗ {type(e).__name__}: {t[:90]}"


def con_sql(q, limpia=True):
    def f():
        # Cada fila en un DuckDB nuevo: lo registrado por otra no tapa nada.
        if limpia:
            ore._con = None
        with warnings.catch_warnings(record=True) as w:
            warnings.simplefilter("always")
            df = ore.sql(q)
            aviso = " (cortada)" if any("Truncated" in type(x.message).__name__ for x in w) else ""
        return f"✓ {len(df)}{aviso}"
    return f


def con_over(v):
    def f():
        return f"✓ {len(ore.over(v))}"
    return f


def http(ruta, quien=None):
    def f():
        req = urllib.request.Request(BASE + ruta, headers=quien or SUJ)
        try:
            with urllib.request.urlopen(req, timeout=60) as r:
                d = json.loads(r.read() or b"{}")
                filas = d.get("filas") or d.get("rows") or d.get("datos") or []
                return f"✓ {len(filas)}"
        except urllib.error.HTTPError as e:
            cuerpo = e.read().decode("utf-8", "replace")
            return f"✗ {e.code}: {cuerpo[:90]}"
    return f


def editor(texto, fichero="consulta.sql"):
    """Un `.sql` en el editor: `POST /puestos/{id}/ejecutar` (lenguaje sql), el
    agente del puesto la corre y la salida se lee de su celda."""
    def f():
        cuerpo = json.dumps({"lenguaje": "sql", "texto": texto, "fichero": fichero}).encode()
        req = urllib.request.Request(BASE + f"/puestos/{PUESTO}/ejecutar", data=cuerpo, method="POST",
                                     headers={**SUJ, "content-type": "application/json"})
        try:
            with urllib.request.urlopen(req, timeout=60) as r:
                d = json.loads(r.read() or b"{}")
        except urllib.error.HTTPError as e:
            return f"✗ {e.code}: {e.read().decode('utf-8', 'replace')[:90]}"
        celdas = d.get("celdas") or [d.get("celda")]
        salidas = []
        for n in celdas:
            for _ in range(240):
                with urllib.request.urlopen(urllib.request.Request(BASE + f"/puestos/{PUESTO}/celdas/{n}", headers=SUJ), timeout=30) as r:
                    c = json.loads(r.read() or b"{}")
                sal = c.get("salida")
                if sal:
                    salidas.append(sal)
                    break
                time.sleep(0.25)
            else:
                return f"✗ la celda {n} no acabó"
        out = []
        for sal in salidas:
            if sal.get("tipo") == "error":
                m = str(sal.get("mensaje")).replace("\n", " ")
                return f"✗ {sal.get('nombre')}: {m if len(m) < 110 else m[:40] + ' … ' + m[-110:]}"
            filas = (sal.get("tabla") or {}).get("filas") or sal.get("filas")
            out.append(f"{len(filas)} fila(s)" if isinstance(filas, list) else (sal.get("texto") or sal.get("tipo") or "").strip()[:60])
        return "✓ " + " · ".join(out)
    return f


def explica(q):
    def f():
        import io, contextlib
        with contextlib.redirect_stdout(io.StringIO()):
            p = ore.explain(q)
        lect = p.get("lecturas", []) if isinstance(p, dict) else []
        return f"✓ {len(lect)} lectura(s)"
    return f


T = "vivo.datos.clientes"
P = "vivo.datos.pedidos"
V1 = "vivo.informes.clientes_es"
V2 = "vivo.informes.ventas_por_pais"
M = "vivo.docs.contratos"

FILAS = [
    ("T1 columnas+filtro+limit · tabla", con_sql(f"select id, alta from {T} where pais = 'ES' limit 2")),
    ("T1 · vista que se empuja", con_sql(f"select * from {V1}")),
    ("T2 orden top-N", con_sql(f"select id, alta from {T} order by alta desc limit 2")),
    ("T3 agregado", con_sql(f"select pais, count(*) n from {T} group by pais")),
    ("T3 · vista con junta y agregado", con_sql(f"select * from {V2}")),
    ("T5 junta de dos tablas", con_sql(f"select c.pais, sum(p.importe) t from {P} p join {T} c on p.cliente = c.id group by 1")),
    ("T6 CTE + ventana", con_sql(f"with x as (select id, pais from {T}) select pais, row_number() over (partition by pais order by id) r from x")),
    ("T6 subconsulta + union", con_sql(f"select id from {T} where id in (select cliente from {P}) union select 99")),
    ("T7 explain", explica(f"select id from {T} where pais = 'ES'")),
    ("M1 listado de la coleccion", con_sql(f"select key, size from {M}")),
    ("M1 · por particion", con_sql(f"select key from {M} where anio = '2026'")),
    ("M3 metadatos x tabla", con_sql(f"select m.key, c.pais from {M} m cross join (select pais from {T} limit 1) c")),
    ("M4 agregado del listado", con_sql(f"select anio, count(*) n, sum(size) b from {M} group by anio")),
    ("CONGELADA · tabla", con_sql("select id from congelada.datos.clientes limit 1")),
    ("CONGELADA · coleccion", con_sql("select key from congelada.docs.contratos")),
    ("por el nombre de la fuente (s3.datos.clientes)", con_sql("select count(*) n from s3.datos.clientes")),
    ("over(tabla expuesta)", con_over(T)),
    ("over(vista de la foranea)", con_over(V1)),
    ("preview · tabla expuesta", http("/preview/table/vivo/datos/clientes?limite=5")),
    ("preview · vista que se empuja", http("/preview/view/vivo/informes/clientes_es?limite=5")),
    ("preview · vista con junta", http("/preview/view/vivo/informes/ventas_por_pais?limite=5")),
    ("preview · coleccion (objecttable)", http("/preview/objecttable/vivo/docs/contratos?limite=5")),
    ("preview · congelada", http("/preview/table/congelada/datos/clientes?limite=5")),
    # ── un fichero .sql en el editor: el agente del puesto lo corre ──
    ("E0 .sql · select (T1)", editor(f"select id, alta from {T} where pais = 'ES'")),
    ("E0 .sql · junta + agregado (T5)", editor(f"select c.pais, sum(p.importe) t from {P} p join {T} c on p.cliente = c.id group by 1")),
    ("G1 create view en la foranea", editor("create view vivo.informes.clientes_pt as select id from vivo.datos.clientes where pais = 'PT'")),
    ("G1 · y leerla", editor("select * from vivo.informes.clientes_pt")),
    ("G2 T8 create dataset (standard) as select", editor("create or replace dataset std.copias.clientes as select id, pais from vivo.datos.clientes")),
    ("G3 T8 create materialized view", editor("create materialized view std.copias.es as select id from vivo.datos.clientes where pais = 'ES'")),
    ("G4 create dataset en la foranea", editor("create or replace dataset vivo.datos.nuevo as select 1 as a")),
    ("G5 insert en el origen", editor("insert into vivo.datos.clientes select 9, 'IT', date '2026-02-01'")),
    ("G6 guion de dos sentencias", editor("create view vivo.informes.fr as select id from vivo.datos.clientes where pais = 'FR';\nselect count(*) n from vivo.informes.fr")),
    ("datos del puesto · vista", http(f"/puestos/{PUESTO}/datos/{V1}", {"x-ore-sujeto": "agente:local", "x-ore-puesto": PUESTO})),
]

# Lo esperado: `✓`, o `✗` con lo que tiene que decir. Lo que aún no está
# (B4·3: la preview y los datos de una vista sin motor) se espera como es hoy,
# y se dice: cuando llegue, esta tabla cambia con él.
NO = {
    "CONGELADA · tabla": "OOS2051",
    "CONGELADA · coleccion": "OOS2051",
    "preview · congelada": "OOS2051",
    "G4 create dataset en la foranea": "OOS2049",
    "G5 insert en el origen": "OOS2049",
    "preview · vista que se empuja": "409",  # B4·3
    "preview · vista con junta": "409",  # B4·3
    "datos del puesto · vista": "409",  # B4·3
}

print()
print("  la foranea se lee (0057 B4·0) · Python")
print("  " + "─" * 100)
MAL = []
for nombre, f in FILAS:
    t0 = time.time()
    try:
        r = f()
    except Exception as e:  # noqa: BLE001 — se mide
        r = corto(e)
    esperado = NO.get(nombre)
    bien = (r.startswith("✓") if esperado is None else (r.startswith("✗") and esperado in r))
    if not bien:
        MAL.append(nombre)
    print(f"  {' ' if bien else '✗'} {nombre:<48} {r[:170]}  ({int((time.time() - t0) * 1000)} ms)")

print()
if MAL:
    print("  ✗ no da lo esperado: " + " · ".join(MAL))
    sys.exit(1)
print("  ✓ la foranea se lee: %d caminos x clases, como se espera" % len(FILAS))
