"""DUCKDB POR EL CATALOGO · la fase 3, medida antes de construirla.

Hoy `sql()` manda el texto a ore-serve (`POST /puestos/{id}/sql`), que dice
con el tokenizador y el arbol que nombres lee y los resuelve por `datos`
(8d18994). La fase 3 seria que DuckDB le pregunte al catalogo como Spark
(e3a691d): `ATTACH` de `/v1`, y como DuckDB 1.5 no pide vistas
(`medida-las-vistas-por-el-catalogo.py`), el SDK registra las Views del arbol
al abrir la sesion (`listViews` + `loadView`, su SQL de DuckDB). Si sale, el
tokenizador sobra. Antes de tocar nada:

  §1  ¿QUE RESUELVE ATTACH HOY?   cada clase de nombre tras las mejoras de
                                  /v1 para Spark, sin `search_path` y con el.
  §2  ¿DONDE VIVEN LAS VIEWS?     registradas en `memory.hr`, en un catalogo
                                  aparte, o con su SQL calificada con `ore`:
                                  ¿leen las filas de ORE y siguen viendose
                                  las tablas?
  §3  ¿CUANTO CUESTA REGISTRAR?   al abrir la sesion, con 2 Views y con 50.
  §4  ¿LA MISMA SEMANTICA?        los casos del tokenizador y los errores
                                  (errata, View virtual, conducto) por el SDK
                                  de hoy y por ATTACH + Views: que ve la celda.
  §5  ¿CUANTO POR CONSULTA?       sql() de hoy frente a ATTACH.
  §6  ¿LO DECLARADO?              dentro de un transform (⑤): leer un input
                                  declarado y uno que no, por los dos caminos.

    python pruebas-de-fuego/medida-duckdb-por-el-catalogo.py [--filas 20000] [--guardar DIR]

El banco de `medida-el-catalogo-como-resolutor.py` (ore-serve de verdad, S3 de
mentira, proxy que cuenta, un puesto abierto). DuckDB 1.5.x de Python (la del
puesto). No toca el cluster ni la red de nadie salvo `extensions.duckdb.org`
si faltan las extensiones.
"""
import importlib.util
import json
import os
import re
import statistics
import sys
import time

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
AQUI = os.path.dirname(os.path.abspath(__file__)).replace("\\", "/")
RAIZ = os.path.dirname(AQUI)
_sp = importlib.util.spec_from_file_location("resolutor", AQUI + "/medida-el-catalogo-como-resolutor.py")
R = importlib.util.module_from_spec(_sp)
_sp.loader.exec_module(R)  # lee --filas y --guardar de sys.argv
fila, corto = R.fila, R.corto
FILAS = R.FILAS
CAB = dict(R.AGENTE, **{"x-ore-puesto": R.PUESTO})
MEDIDO = {}


def vista(nombre, where, fields="{ id: id, pais: pais }"):
    return """apiVersion: oos.dev/v1alpha12
kind: View
metadata: { name: %s, namespace: hr }
spec:
  owner: team:hr
  from: { dataset: hr.ventas }
  where: %s
  fields: %s
""" % (nombre, where, fields)


def ms(t0):
    return (time.perf_counter() - t0) * 1000


def probar(con, q):
    try:
        v = con.execute(q).fetchall()
        return True, (v[0][0] if len(v) == 1 and len(v[0]) == 1 else v)
    except Exception as e:
        return False, "%s: %s" % (type(e).__name__, corto(e, 150))


def marca(ok, v, esperado):
    if not ok:
        return "✗ " + v
    return ("%s ✓" % v) if esperado is None or v == esperado else ("%s ≠ %s" % (v, esperado))


# ═════════════════════════════════════════════════════════════════════════════
# Las Views del catalogo, como las registraria el SDK al abrir la sesion
# ═════════════════════════════════════════════════════════════════════════════
def vistas_del_catalogo(b):
    c, j = R.http("GET", b.base + "/v1/namespaces", cabeceras=CAB)
    vs, fallos, pet = [], [], 1
    for (ns,) in j["namespaces"]:
        c, l = R.http("GET", b.base + "/v1/namespaces/%s/views" % ns, cabeceras=CAB)
        pet += 1
        for i in l.get("identifiers", []):
            c, v = R.http("GET", b.base + "/v1/namespaces/%s/views/%s" % (ns, i["name"]), cabeceras=CAB)
            pet += 1
            if c != 200:
                fallos.append((ns + "." + i["name"], c, (v or {}).get("error", {}).get("message", "")))
                continue
            reps = v["metadata"]["versions"][0]["representations"]
            sql = next(r["sql"] for r in reps if r["dialect"] == "duckdb")
            vs.append((ns, i["name"], sql))
    return vs, fallos, pet


def calificar(sql, paquetes):
    """`"hr"."ventas"` → `"ore"."hr"."ventas"`: solo las hojas que `a_sql` escribe."""
    for ns in paquetes:
        sql = re.sub(r'(?<![."])"%s"\."' % re.escape(ns), '"ore"."%s"."' % ns, sql)
    return sql


NO_REGISTRADAS = []


def registrar(con, vs, variante, paquetes, tolerante=False):
    """`CREATE VIEW` resuelve sus nombres AL CREARLA (medido: pide loadTable de
    cada dataset que lee). Con `tolerante`, una View que no se crea se salta y
    se apunta, como tendria que hacer el SDK al abrir la sesion."""
    del NO_REGISTRADAS[:]
    for ns, n, sql in vs:
        try:
            if variante in ("memory.hr", "memory.hr, search_path antes"):
                con.execute('create schema if not exists "%s"' % ns)
                con.execute('create or replace view "%s"."%s" as %s' % (ns, n, sql))
            elif variante == "catalogo vistas":
                con.execute('create schema if not exists vistas."%s"' % ns)
                con.execute('create or replace view vistas."%s"."%s" as %s' % (ns, n, sql))
            elif variante == "memory.hr, SQL calificada":
                con.execute('create schema if not exists "%s"' % ns)
                con.execute('create or replace view "%s"."%s" as %s' % (ns, n, calificar(sql, paquetes)))
        except Exception as e:
            if not tolerante:
                raise
            NO_REGISTRADAS.append((ns + "." + n, corto(e, 120)))


# El search_path que funciona (§2): las Views de la sesion primero, luego
# cada paquete del catalogo, luego lo de siempre.
SP = "memory.hr,ore.hr,ore.ventas,memory.main"
BUENA = "memory.hr, search_path antes"


def sesion(b, variante=None, vs=None, search_path=None, paquetes=("hr", "ventas"), tolerante=False):
    con = R.conexion()
    R.atar(con, b, extra=", read_only")
    if variante == "catalogo vistas":
        con.execute("attach ':memory:' as vistas")
    if variante == "memory.hr, search_path antes":
        con.execute("create schema if not exists hr")
        con.execute("set search_path = '%s'" % SP)
    if variante:
        registrar(con, vs, variante, paquetes, tolerante)
    if search_path:
        con.execute("set search_path = '%s'" % search_path)
    return con


# ═════════════════════════════════════════════════════════════════════════════
def s1(b):
    print()
    print("§1 · ¿QUE RESUELVE ATTACH HOY?  (read_only, testigo del agente + x-ore-puesto)")
    casos = [("hr.ventas", "escrito", FILAS), ("hr.clientes", "escrito", (FILAS + 99) // 100),
             ("ventas.pedidos", "otro paquete", (FILAS + 99) // 100), ("hr.ajena", "de otra persona", (FILAS + 99) // 100),
             ("hr.ventas_es", "copia mantenida", FILAS), ("hr.espanoles", "copia heredada", None),
             ("hr.ventasES", "View (sin registrar)", None), ("hr.nada", "no existe", None)]
    for sp in (None, "ore.hr,ore.ventas,memory.main"):
        con = sesion(b, search_path=sp)
        print("   search_path = %s" % (sp or "(el de siempre)"))
        for n, clase, esp in casos:
            ok, v = probar(con, "select count(*) from %s" % n)
            fila("  %s (%s)" % (n, clase), marca(ok, v, esp)[:90])
            MEDIDO["§1 %s %s" % ("sp" if sp else "-", n)] = ok
        con.close()


VARIANTES = ["memory.hr", "catalogo vistas", "memory.hr, search_path antes", "memory.hr, SQL calificada"]


def s2(b, vs):
    print()
    print("§2 · ¿DONDE VIVEN LAS VIEWS?  registradas al abrir; lo que ORE da: ventasES %d, rarasES %d" % ((FILAS + 3) // 4, (FILAS + 3) // 4))
    es = (FILAS + 3) // 4
    qs = [("View: hr.ventasES", "select count(*) from hr.ventasES", es),
          ("View con comilla y barra: hr.rarasES", "select count(*) from hr.rarasES", es),
          ("tabla del mismo paquete: hr.ventas", "select count(*) from hr.ventas", FILAS),
          ("View × tabla", "select count(*) from hr.ventasES e join hr.clientes c on e.id = c.id", (FILAS + 99) // 100),
          ("otro paquete: ventas.pedidos", "select count(*) from ventas.pedidos", (FILAS + 99) // 100),
          ("una tabla de la sesion", "create temp table t as select 1 as x; select count(*) from t", 1)]
    for var in VARIANTES:
        for sp in (None, "ore.hr,ore.ventas,memory.main,vistas.hr"):
            if var != "catalogo vistas" and sp:
                sp = SP
            try:
                con = sesion(b, var, vs, sp)
            except Exception as e:
                fila("  %s" % var, "✗ al registrar: " + corto(e, 100))
                continue
            print("   %s · search_path = %s" % (var, sp or "(el de siempre)"))
            bien = 0
            for nombre, q, esp in qs:
                partes = q.split("; ")
                for p in partes[:-1]:
                    con.execute(p)
                ok, v = probar(con, partes[-1])
                bien += ok and v == esp
                fila("    " + nombre, marca(ok, v, esp)[:100])
            MEDIDO["§2 %s %s" % (var, "sp" if sp else "-")] = "%d/%d" % (bien, len(qs))
            con.close()


def s3(b):
    print()
    print("§3 · ¿CUANTO CUESTA REGISTRAR?  (listar paquetes, listViews por paquete, loadView por View)")
    for n in (0, 48):
        for i in range(n):
            R.escribir(b.A + "/packages/hr/views/v%02d.yaml" % i, vista("v%02d" % i, "{ pais: ES }"))
        xs, pets = [], 0
        for _ in range(3):
            i0 = b.reg.marca()
            t0 = time.perf_counter()
            vs, fallos, _ = vistas_del_catalogo(b)
            con = R.conexion()
            R.atar(con, b, extra=", read_only")
            con.execute("create schema if not exists hr")
            con.execute("set search_path = '%s'" % SP)
            i1 = b.reg.marca()
            registrar(con, vs, BUENA, ("hr", "ventas"))
            al_crear = [p for p in b.reg.desde(i1) if "/tables/" in p["ruta"]]
            xs.append(ms(t0))
            pets = len(b.reg.desde(i0))
            con.close()
        srv = [p["ms"] for p in b.reg.todo()[-pets:] if "/views/" in p["ruta"]]
        fila("  %d Views" % len(vs), "%.0f ms" % statistics.median(xs),
             "%d peticiones (%d loadTable al CREATE VIEW) · loadView en el servidor %.0f ms (mediana)" % (
                 pets, len(al_crear), statistics.median(srv) if srv else 0))
        MEDIDO["§3 %d" % len(vs)] = statistics.median(xs)
    for i in range(48):
        os.remove(b.A + "/packages/hr/views/v%02d.yaml" % i)


CASOS = [
    ("comentario", "-- antes: from hr.nada\nselect count(*) from hr.ventas"),
    ("cadena", "select 'from hr.nada' as s, count(*) from hr.clientes"),
    ("from a, b", "select count(*) from hr.ventas a, hr.clientes b where a.id = b.id"),
    ("comillas", 'select count(*) from "hr"."ventas"'),
    ("una View", "select count(*) from hr.ventasES"),
    ("esquema de la sesion", "create schema tmp; create table tmp.t as select 1 as x; select count(*) from tmp.t"),
    ("mayusculas", "SELECT COUNT(*) FROM HR.VENTAS"),
    ("errata", "select count(*) from hr.nada"),
    ("View virtual", "select count(*) from hr.empleados"),
    ("copia heredada", "select count(*) from hr.espanoles"),
    ("de otra persona", "select count(*) from hr.ajena"),
]


def sdk(b):
    os.environ.update(ORE_SERVE=b.base, PUESTO=R.PUESTO, ORE_ALMACEN="dir:" + b.tmp)
    sys.path.insert(0, RAIZ + "/puesto/python")
    import ore
    ore.puesto.servidor, ore.puesto.id = b.base, R.PUESTO
    ore.puesto._cabeceras = dict(R.AGENTE)
    return ore


def celda_sdk(ore, q):
    try:
        partes = q.split("; ")
        c = ore._duckdb()
        for p in partes[:-1]:
            c.execute(p)
        t = ore.sql(partes[-1], como="arrow")
        return True, t.column(0)[0].as_py() if t.num_rows == 1 else t.num_rows
    except Exception as e:
        return False, "%s: %s" % (type(e).__name__, corto(e, 120))


def s4(b, ore, vs):
    print()
    print("§4 · ¿LA MISMA SEMANTICA?  lo que ve la celda: HOY (sql() del SDK) · FASE 3 (ATTACH + Views con search_path antes)")
    con = sesion(b, BUENA, vs, SP)
    iguales = 0
    for nombre, q in CASOS:
        a = celda_sdk(ore, q)
        partes = q.split("; ")
        for p in partes[:-1]:
            try:
                con.execute(p)
            except Exception:
                pass
        f = probar(con, partes[-1])
        mismo = a[0] == f[0] and (not a[0] or a[1] == f[1])
        iguales += mismo
        print("     %-18s HOY %s" % (nombre, ("✓ %s" % a[1]) if a[0] else "✗ " + a[1][:110]))
        print("     %-18s F3  %s%s" % ("", ("✓ %s" % f[1]) if f[0] else "✗ " + f[1][:110], "" if mismo else "   ← distinto"))
    MEDIDO["§4 iguales"] = "%d/%d" % (iguales, len(CASOS))
    con.close()
    # el conducto DE VERDAD: `total` high en el arbol, materialization.payload low
    A = b.A
    viejo = open(A + "/conduits.yaml", encoding="utf-8").read()
    R.escribir(A + "/lattice.yaml", "apiVersion: oos.dev/v1alpha3\nkind: Lattice\nmetadata: { name: sensitivity, namespace: gdpr }\nspec:\n  levels: [none, low, high]\n")
    R.escribir(A + "/conduits.yaml", "apiVersion: oos.dev/v1alpha1\nkind: ConduitPolicy\nmetadata: { name: demo }\nspec:\n  owner: team:security\n  conduits:\n    materialization.payload: { oos.maturity: DRAFT, gdpr.sensitivity: low }\n")
    R.escribir(A + "/packages/hr/entities/Venta.yaml", "apiVersion: oos.dev/v1alpha8\nkind: Entity\nmetadata: { name: Venta, namespace: hr }\nspec:\n  nature: event\n  backedBy: hr.ventasV\n  primaryKey: [id]\n  timeKey: cuando\n  properties:\n    id: { type: Integer }\n    pais: { type: String }\n    cuando: { type: DateTimeTz }\n    total: { type: Decimal, labels: { gdpr.sensitivity: high } }\n")
    try:
        vs2, fallos, _ = vistas_del_catalogo(b)
        fila("  con el conducto: Views servidas", "%d" % len(vs2), "; ".join("%s %d" % (n, c) for n, c, _ in fallos))
        con = sesion(b, "memory.hr, search_path antes", vs2, SP, tolerante=True)
        fila("  Views que no se registran al abrir", "%d" % len(NO_REGISTRADAS), "; ".join("%s: %s" % x for x in NO_REGISTRADAS)[:160])
        for nombre, q in (("conducto: la tabla", "select count(*) from hr.ventas"), ("conducto: una View", "select count(*) from hr.ventasES"),
                          ("conducto: otra tabla", "select count(*) from hr.clientes")):
            a = celda_sdk(ore, q)
            f = probar(con, q)
            print("     %-18s HOY %s" % (nombre, ("✓ %s" % a[1]) if a[0] else "✗ " + a[1][:110]))
            print("     %-18s F3  %s" % ("", ("✓ %s" % f[1]) if f[0] else "✗ " + f[1][:110]))
        con.close()
    finally:
        R.escribir(A + "/conduits.yaml", viejo)
        os.remove(A + "/packages/hr/entities/Venta.yaml")
        os.remove(A + "/lattice.yaml")


def s5(b, ore, vs):
    print()
    print("§5 · ¿CUANTO POR CONSULTA?  medianas de 5 (la 1ª, sola)")
    qs = [("1ª: hr.ventas", "select count(*) from hr.ventas"), ("2ª y siguientes", "select count(*) from hr.ventas"),
          ("una View", "select count(*) from hr.ventasES"),
          ("join ventas × clientes", "select count(*) from hr.ventas v join hr.clientes c on v.id = c.id")]
    con = sesion(b, BUENA, vs, SP)
    ore._duckdb().execute("select 1")
    for nombre, q in qs:
        for camino, fn in (("HOY", lambda: ore.sql(q, como="arrow")), ("F3 ", lambda: con.execute(q).fetchall())):
            veces = 1 if nombre.startswith("1ª") else 5
            i = b.reg.marca()
            xs = []
            for _ in range(veces):
                t0 = time.perf_counter()
                fn()
                xs.append(ms(t0))
            pet = b.reg.desde(i)
            fila("  %s %s" % (camino, nombre), "%.0f ms" % statistics.median(xs),
                 "%.1f pet. por consulta (%s)" % (len(pet) / veces, ", ".join(sorted({"%s %s" % (p["metodo"], R.plantilla(p["ruta"])) for p in pet}))[:80]))
    con.close()


def s6(b, ore, vs):
    print()
    print("§6 · ¿LO DECLARADO?  un transform con inputs=[hr.ventas], output=hr.salidaT (⑤)")
    c, _ = R.http("POST", b.base + "/puestos/%s/transform" % R.PUESTO,
                  {"nombre": "medida", "inputs": ["hr.ventas"], "output": "hr.salidaT"}, R.AGENTE)
    fila("  POST transform", "%d" % c)
    con = sesion(b, BUENA, vs, SP, tolerante=True)
    fila("  Views que no se registran al abrir", "%d" % len(NO_REGISTRADAS), "; ".join("%s: %s" % x for x in NO_REGISTRADAS)[:160])
    for nombre, q in (("input declarado: hr.ventas", "select count(*) from hr.ventas"),
                      ("NO declarado: hr.clientes", "select count(*) from hr.clientes"),
                      ("una View sobre el input: hr.ventasES", "select count(*) from hr.ventasES")):
        a = celda_sdk(ore, q)
        f = probar(con, q)
        fila("  HOY " + nombre, ("✓ %s" % a[1]) if a[0] else "✗ " + a[1][:100])
        fila("  F3  " + nombre, ("✓ %s" % f[1]) if f[0] else "✗ " + f[1][:100])
        MEDIDO["§6 %s" % nombre] = (a[0], f[0])
    con.close()
    c, _ = R.http("DELETE", b.base + "/puestos/%s/transform" % R.PUESTO, None, R.AGENTE)
    fila("  DELETE transform", "%d" % c)


def main():
    print("DUCKDB POR EL CATALOGO · la fase 3 · %d filas en hr.ventas" % FILAS)
    b = R.Banco()
    try:
        b.levantar()
        R.escribir(b.A + "/packages/hr/views/ventasES.yaml", vista("ventasES", "{ pais: ES }"))
        R.escribir(b.A + "/packages/hr/views/rarasES.yaml", vista("rarasES", '{ pais: ["O\'B\\\\x", ES] }'))
        import duckdb
        fila("duckdb", duckdb.__version__)
        s1(b)
        vs, fallos, pet = vistas_del_catalogo(b)
        print()
        fila("las Views del catalogo", "%d servidas" % len(vs), "; ".join("%s %d %s" % (n, c, corto(m, 60)) for n, c, m in fallos)[:150])
        s2(b, vs)
        s3(b)
        vs, _, _ = vistas_del_catalogo(b)
        ore = sdk(b)
        s4(b, ore, vs)
        s5(b, ore, vs)
        s6(b, ore, vs)
        print()
        print("LO MEDIDO")
        for k, v in MEDIDO.items():
            fila("  " + k, str(v))
    finally:
        b.cerrar()


if __name__ == "__main__":
    main()
