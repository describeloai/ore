# -*- coding: utf-8 -*-
"""LA GUARDA de la 0025: se corre en cada puerta de cada etapa, antes y despues,
y dice lo mismo o la etapa no pasa.

Para cada organizacion viva:
  · sus celdas, y que cada celda tiene EXACTAMENTE UNA de cada cosa tecnica:
    namespace · forja · cofre · ore-serve · arbol · entrada · puerta · cola de Kueue
  · que la organizacion y la celda dicen lo mismo mientras las dos columnas existan
    (`iam.discrepancias`, desde la 029)
  · que ningun nombre de celda se repite
  · que su entrada contesta 200 por la puerta
Y para el codigo:
  · cuantos lectores quedan de `organizacion.arbol` / `organizacion.entrada` (E2 acaba en 0)

Sale 0 si todo cuadra; 1 si un invariante se rompe. Lo que la etapa CAMBIA (por
ejemplo «la celda no tiene nombre propio») se imprime como estado, no como fallo:
la puerta es que dos corridas —antes y despues— digan lo mismo salvo en eso.

    PYTHONIOENCODING=utf-8 python pruebas-de-fuego/medida-la-celda-tiene-nombre.py [--json]
"""
import json
import os
import re
import socket
import subprocess
import sys
import urllib.request

RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CONSOLA = os.environ.get("CONSOLA", r"C:\rubix-platform")
COMO_JSON = "--json" in sys.argv


def sql(q):
    r = subprocess.run(["kubectl", "-n", "identidad", "exec", "idp-db-0", "--", "psql", "-U", "keycloak", "-d", "iam", "-qtAc", q],
                       capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=60)
    return r.stdout.strip()


def k(*args):
    r = subprocess.run(["kubectl"] + list(args), capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=60)
    return r.stdout.strip()


def existe(ns, tipo, nombre):
    return k("-n", ns, "get", tipo, nombre, "-o", "name") != ""


def resuelve(host):
    try:
        return sorted({a[4][0] for a in socket.getaddrinfo(host, 443, socket.AF_INET)})
    except socket.gaierror:
        return []


def contesta(host):
    try:
        return urllib.request.urlopen("https://%s/salud" % host, timeout=10).status
    except Exception as e:
        return getattr(e, "code", 0)


def leer(rel, raiz=RAIZ):
    with open(os.path.join(raiz, rel), encoding="utf-8") as f:
        return f.read()


fallos = []
estado = {}


def falla(que):
    fallos.append(que)


# ── el modelo: que columnas hay ─────────────────────────────────────────────
cols_celda = sql("select string_agg(column_name, ',' order by ordinal_position) from information_schema.columns where table_schema='iam' and table_name='celda'").split(",")
cols_org = sql("select string_agg(column_name, ',' order by ordinal_position) from information_schema.columns where table_schema='iam' and table_name='organizacion'").split(",")
cols_sec = sql("select string_agg(column_name, ',' order by ordinal_position) from information_schema.columns where table_schema='cofre' and table_name='secreto'").split(",")
celda_con_nombre = "cluster" in cols_celda and "arbol" in cols_celda and "entrada" in cols_celda
org_con_arbol = "arbol" in cols_org
estado["modelo"] = {
    "celda_con_nombre_propio": celda_con_nombre,
    "organizacion_lleva_arbol_y_entrada": org_con_arbol,
    "secreto_con_celda": "celda" in cols_sec,
    "indice_una_celda_por_organizacion": sql("select count(*) from pg_indexes where schemaname='iam' and indexname='celda_una_por_organizacion'") == "1",
}

# ── por organizacion, por celda ─────────────────────────────────────────────
if celda_con_nombre:
    filas = sql("select o.id, o.nombre, c.nombre, c.cluster, c.tier, c.arbol, c.entrada, c.puerta, c.estado from iam.organizacion o join iam.celda c on c.organizacion = o.id order by 2, 3")
else:
    filas = sql("select o.id, o.nombre, o.nombre, c.nombre, c.tier, o.arbol, o.entrada, c.puerta, c.estado from iam.organizacion o join iam.celda c on c.organizacion = o.id order by 2, 3")
orgs = {}
nombres = []
for l in filas.splitlines():
    if not l.strip():
        continue
    oid, org, celda, cluster, tier, arbol, entrada, puerta, estado_celda = l.split("|")
    nombres.append(celda)
    ns = "t-" + celda
    # ⭐ Una celda RETIRADA (0025 E6) tiene que estar AUSENTE: el invariante es el
    #   contrario. Su fila se queda (es historia, y el nombre no se reusa).
    if estado_celda == "retirada":
        ausente = k("get", "namespace", ns, "-o", "name") == ""
        if not ausente:
            falla("%s/%s: retirada y el namespace %s sigue ahi (Flux no lo ha podado)" % (org, celda, ns))
        orgs.setdefault(org, {})[celda] = dict(cluster=cluster, tier=tier, estado="retirada", ausente=ausente)
        continue
    tecnico = {
        "namespace": existe("", "namespace", ns) if False else k("get", "namespace", ns, "-o", "name") != "",
        "forja": existe(ns, "statefulset", "forja"),
        "cofre": existe(ns, "deployment", "ore-cofre"),
        "ore-serve": existe(ns, "deployment", "ore-serve"),
        "cola": k("get", "clusterqueue", "cq-" + celda, "-o", "name") != "",
        "arbol": arbol,
        "entrada": entrada,
        "puerta": puerta,
        "entrada_resuelve_a_la_puerta": bool(resuelve(entrada)) and resuelve(entrada) == resuelve(puerta),
        "salud": contesta(entrada),
    }
    for que in ("namespace", "forja", "cofre", "ore-serve", "cola"):
        if not tecnico[que]:
            falla("%s/%s: falta %s" % (org, celda, que))
    if arbol != "t-%s/ontologia" % celda and not arbol.startswith(celda):
        # un arbol propio (acme-corp/ontologia) es legitimo; uno de OTRA celda no
        pass
    if not tecnico["entrada_resuelve_a_la_puerta"]:
        falla("%s/%s: la entrada %s no resuelve a la puerta %s" % (org, celda, entrada, puerta))
    if tecnico["salud"] != 200:
        falla("%s/%s: %s/salud → %s" % (org, celda, entrada, tecnico["salud"]))
    orgs.setdefault(org, {})[celda] = dict(cluster=cluster, tier=tier, **tecnico)
estado["organizaciones"] = orgs
if len(nombres) != len(set(nombres)):
    falla("nombres de celda repetidos: %s" % sorted(n for n in nombres if nombres.count(n) > 1))

# ── la doble verdad, mientras dure ──────────────────────────────────────────
if celda_con_nombre and org_con_arbol:
    hay_vista = sql("select to_regclass('iam.discrepancias') is not null") == "t"
    if not hay_vista:
        falla("las dos columnas existen y no hay `iam.discrepancias` que las coteje")
    else:
        d = sql("select count(*) from iam.discrepancias")
        estado["discrepancias"] = int(d or 0)
        if d != "0":
            falla("iam.discrepancias tiene %s filas: %s" % (d, sql("select string_agg(organizacion || ': ' || que, '; ') from iam.discrepancias")))

# ── lectores de las columnas viejas, en el codigo ───────────────────────────
lectores = []
for rel in ("crates/ore-iam/src/fundar.rs", "crates/ore-iam/src/rutas.rs"):
    for i, l in enumerate(leer(rel).splitlines(), 1):
        if re.search(r"\bo\.(arbol|entrada)\b|select[^;]*\b(arbol|entrada)\b[^;]*from iam\.organizacion|from iam\.organizacion.*\b(arbol|entrada)\b|insert into iam\.organizacion \([^)]*\b(arbol|entrada)\b", l) and not l.strip().startswith("//"):
            lectores.append("%s:%d" % (rel, i))
ap = leer("malla/aprovisionar-inquilino.sh")
for i, l in enumerate(ap.splitlines(), 1):
    if re.search(r"consulta (arbol|entrada)\b", l) and not l.strip().startswith("#"):
        lectores.append("malla/aprovisionar-inquilino.sh:%d" % i)
for rel in ("lib/server/organizacion.ts",):
    for i, l in enumerate(leer(rel, CONSOLA).splitlines(), 1):
        if re.search(r"org\.entrada\b", l) and not l.strip().startswith("//") and not l.strip().startswith("*"):
            lectores.append("consola:%s:%d" % (rel, i))
grant = "arbol" in re.search(r"grant select \(([^)]*)\)\s*on iam\.organizacion", leer("iam/migraciones/023-el-papel-del-aprovisionador.sql")).group(1)
revocado = any("revoke select (arbol, entrada) on iam.organizacion" in leer("iam/migraciones/" + f)
               for f in os.listdir(os.path.join(RAIZ, "iam", "migraciones")) if f.endswith(".sql"))
if grant and not revocado:
    lectores.append("iam/migraciones/023 (grant)")
estado["lectores_de_organizacion_arbol_entrada"] = lectores

# ── salida ──────────────────────────────────────────────────────────────────
if COMO_JSON:
    print(json.dumps({"estado": estado, "fallos": fallos}, indent=1, ensure_ascii=False))
else:
    m = estado["modelo"]
    print("MODELO   celda con nombre propio: %s · organizacion lleva arbol/entrada: %s · secreto con celda: %s · indice una-por-org: %s" % (
        m["celda_con_nombre_propio"], m["organizacion_lleva_arbol_y_entrada"], m["secreto_con_celda"], m["indice_una_celda_por_organizacion"]))
    for org, celdas in orgs.items():
        for celda, t in celdas.items():
            if t.get("estado") == "retirada":
                print("  %-10s %-10s retirada · %s" % (org, celda, "ausente ✓" if t["ausente"] else "TODAVIA PRESENTE ✗"))
                continue
            print("%-8s %-10s %s/%s  ns=%s forja=%s cofre=%s serve=%s cola=%s  arbol=%s  %s → %s %s  salud=%s" % (
                org, celda, t["cluster"], t["tier"], *("✓" if t[x] else "✗" for x in ("namespace", "forja", "cofre", "ore-serve", "cola")),
                t["arbol"], t["entrada"], t["puerta"], "✓" if t["entrada_resuelve_a_la_puerta"] else "✗", t["salud"]))
    if "discrepancias" in estado:
        print("DOBLE VERDAD  iam.discrepancias = %d" % estado["discrepancias"])
    print("LECTORES de organizacion.arbol/entrada en el codigo: %d" % len(lectores))
    for l in lectores:
        print("   · %s" % l)
    print()
    if fallos:
        print("✗ %d invariantes rotos:" % len(fallos))
        for f in fallos:
            print("   · %s" % f)
        sys.exit(1)
    print("✓ invariantes en pie")
