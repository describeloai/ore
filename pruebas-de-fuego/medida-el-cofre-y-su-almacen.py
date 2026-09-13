# -*- coding: utf-8 -*-
"""Medida: el cofre, sus verbos y donde guarda — para decidir con la 0024 si el
material se queda central o viaja con la celda.

La pregunta que la medida de acoplamiento dejo en «decidir»: el cofre corre EN el
inquilino (`t-<n>`) y guarda EN la base central de `iam`. Lo que hacen los que
viven de BYOC (Redpanda, WarpStream) es lo contrario: el material vive en la
cuenta del plano de datos y nunca sale de ella; el plano de control guarda
metadatos. Antes de escribir la decision, esto mide cuanto hay que mover y que
invariante se rompe al moverlo.

Se corre desde fuera, con `kubectl` y `gcloud` apuntando al cluster:

    PYTHONIOENCODING=utf-8 python pruebas-de-fuego/medida-el-cofre-y-su-almacen.py [demo]

Secciones:
  A  los verbos del cofre y que SQL hace cada uno sobre `cofre.*`
  B  quien mas toca `cofre.*` (codigo, migraciones, guiones)
  C  donde corre, donde guarda, con que llave, y por que camino de red
  D  lo que hay guardado hoy (por organizacion, clases, versiones, concesiones)
  E  que cruza de plano, cotejado con lo que hace el sector — y el veredicto
"""
import json
import os
import re
import subprocess
import sys

INQ = sys.argv[1] if len(sys.argv) > 1 else "demo"
NS = "t-" + INQ
RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def correr(*args, ok=True):
    # `shell=True` porque `gcloud` es un `.cmd` en Windows y `CreateProcess` no lo resuelve.
    r = subprocess.run(list(args), capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=90,
                       shell=(args[0] == "gcloud"))
    if r.returncode and ok:
        print("   !! %s → %s" % (" ".join(args[:4]), (r.stderr or r.stdout).strip()[:160]))
    return r.stdout


def kj(*args):
    s = correr("kubectl", *args, "-o", "json")
    return json.loads(s) if s.strip() else {}


def sql(q):
    # La base central: `idp-db` en `identidad`. El papel `keycloak` es el
    # superusuario que la 020 nombra; para MEDIR vale, para operar no.
    return correr("kubectl", "-n", "identidad", "exec", "idp-db-0", "--", "psql", "-U", "keycloak", "-d", "iam", "-qtAc", q)


def leer(rel):
    with open(os.path.join(RAIZ, rel), encoding="utf-8") as f:
        return f.read()


def titulo(t):
    print()
    print(t)
    print("-" * len(t))


hallazgos = []


def anota(clase, que, det=""):
    hallazgos.append((clase, que, det))


# ── A ───────────────────────────────────────────────────────────────────────
titulo("A - LOS VERBOS DEL COFRE, Y QUE SQL HACE CADA UNO SOBRE `cofre.*`")
rutas = leer("crates/ore-cofre/src/rutas.rs")
mapa = re.search(r"pub fn mapa\(.*?\]\s*\n", rutas, re.S).group(0)
verbos = re.findall(r'\("(GET|POST|PUT|DELETE|PATCH)", "([^"]+)", (\w+)\)', mapa)
for m, ruta, con in verbos:
    print("   %-6s %-45s %s" % (m, ruta, "con testigo" if con == "con" else "abierta"))
print()
# Que hace cada `fn` con las tablas del cofre. Se parte el fichero por funcion.
fns = re.split(r"\n    fn (\w+)\(", rutas)
sql_por_fn = {}
for i in range(1, len(fns), 2):
    nombre, cuerpo = fns[i], fns[i + 1]
    ops = set()
    for op, tabla in re.findall(r"\b(insert into|update|delete from|from|join)\s+(cofre\.\w+)", cuerpo):
        ops.add(("escribe" if op in ("insert into", "update", "delete from") else "lee", tabla))
    if ops:
        sql_por_fn[nombre] = sorted(ops)
        print("   %-10s %s" % (nombre, " · ".join("%s %s" % o for o in sorted(ops))))
escriben = {t for ops in sql_por_fn.values() for (o, t) in ops if o == "escribe"}
tablas = sorted({t for ops in sql_por_fn.values() for (_, t) in ops})
print()
print("   tablas/vistas del cofre que el codigo toca: %s" % ", ".join(tablas))
print("   verbos que escriben: %s" % ", ".join(n for n, ops in sql_por_fn.items() if any(o == "escribe" for o, _ in ops)))
# Un verbo existe si hay una `fn` con ese nombre, no si la palabra sale en un comentario.
sin = [v for v in ("rotar", "retirar", "borrar") if not re.search(r"\n    fn %s\(" % v, rutas)]
print("   verbos que NO existen: %s" % ", ".join(sin))
anota("mover", "%d verbos con testigo, %d escriben (`emitir`), %d leen (`listar`, `resolver`); no hay rotar ni retirar" % (
    sum(1 for _, _, c in verbos if c == "con"), 1, 2),
      "es el tamano del almacen a abstraer: dos tablas y una vista, tres funciones")

# La invariante que importa: `resolver` pregunta por el material Y por la
# concesion en UNA consulta. Si el material se va a otro sitio, son dos.
una = re.search(r"from cofre\.secreto s\s+join cofre\.vigente v.*?join iam\.concesion_viva c", rutas, re.S)
if una:
    print()
    print("   ⚠️ `resolver` junta `cofre.vigente` e `iam.concesion_viva` en UNA consulta")
    print("      («asi no hay un camino en el que el codigo sepa que el secreto existe")
    print("       antes de saber si quien pregunta puede»). Con el material fuera de la")
    print("      base, esa consulta son DOS pasos: preguntar a iam, y luego leer.")
    anota("rompe", "la invariante «una sola consulta» de `resolver` (material + concesion en un `join`)",
          "con el material fuera de Postgres el orden lo pone el codigo: PRIMERO la concesion, DESPUES el almacen — y un test que lo cobre")

# ── B ───────────────────────────────────────────────────────────────────────
titulo("B - QUIEN MAS TOCA `cofre.*`")
r = subprocess.run(["git", "grep", "-l", "-E", r"cofre\.(secreto|material|vigente)", "--",
                    "crates", "iam", "malla", "pruebas-de-fuego", "docs"], capture_output=True, text=True, cwd=RAIZ)
for f in r.stdout.split():
    t = leer(f)
    if f.startswith("crates/") and not f.startswith("crates/ore-cofre"):
        # ¿Codigo de otro crate que lea el esquema? Solo si esta fuera de un comentario.
        vivo = [l for l in t.splitlines() if re.search(r"cofre\.(secreto|material|vigente)", l) and not l.strip().startswith("//")]
        print("   %-60s %s" % (f, "CODIGO" if vivo else "solo comentario"))
        if vivo:
            anota("mover", "`%s` lee `cofre.*` desde otro crate" % f, "\n".join(vivo[:3]))
    else:
        print("   %-60s %s" % (f, "propio" if f.startswith("crates/ore-cofre") else "migracion/guion/prosa"))
papeles = leer("iam/migraciones/020-el-cofre-y-sus-permisos.sql")
print()
print("   papeles de la 020: ore_iam → todo `iam`, nada de `cofre`; ore_cofre → todo `cofre`,"
      " y de `iam` SOLO leer concesion y llave")
print("   ⇒ ya HOY la separacion iam/cofre es de papeles de Postgres, no de bases. Mover el")
print("     material fuera no cambia quien puede: cambia por DONDE llega el cofre a iam.")

# ── C ───────────────────────────────────────────────────────────────────────
titulo("C - DONDE CORRE, DONDE GUARDA, CON QUE LLAVE, Y POR QUE CAMINO")
d = kj("-n", NS, "get", "deploy", "ore-cofre")
c = d["spec"]["template"]["spec"]["containers"][0]
args = c.get("args", [])
lugar = args[args.index("--lugar") + 1] if "--lugar" in args else "?"
print("   corre en    %s · serviceAccount `%s` · nodo pool %s" % (
    NS, d["spec"]["template"]["spec"].get("serviceAccountName"),
    (d["spec"]["template"]["spec"].get("nodeSelector") or {}).get("ore.dev/pool", "cualquiera")))
u = correr("gcloud", "secrets", "versions", "access", "latest", "--secret=cofre-url").strip()
host = u.split("@")[-1].split("/")[0] if u else "?"
print("   guarda en   postgres://***@%s  (de `cofre-url` en el almacen; init `traer-la-base`)" % host)
print("   llave       KMS `--lugar %s`, por organizacion (`iam.organizacion.kek`):" % lugar)
for l in sql("select id, kek from iam.organizacion order by kek").splitlines():
    if l.strip():
        oid, kek = l.split("|")
        print("               %-36s %s" % (oid, kek))
llaves = correr("gcloud", "kms", "keys", "list", "--keyring=ore", "--location=" + lugar,
                "--format=value(name.basename(),primary.state,versionTemplate.protectionLevel)")
print("   en KMS      %s" % " · ".join("%s(%s,%s)" % tuple(l.split("\t")) for l in llaves.splitlines() if l.strip()))
print()
pol = kj("-n", NS, "get", "networkpolicy", "salida-del-cofre")
print("   red         `salida-del-cofre` deja salir a:")
for e in pol.get("spec", {}).get("egress", []):
    puertos = ",".join(str(p.get("port")) for p in e.get("ports", []))
    for to in e.get("to", []):
        if "ipBlock" in to:
            print("               %-6s → %s" % (puertos, to["ipBlock"]["cidr"]))
        elif "namespaceSelector" in to:
            print("               %-6s → namespace %s" % (puertos, json.dumps(to["namespaceSelector"].get("matchLabels"))))
print()
print("   ⇒ el proceso vive en el inquilino; el material cifrado vive en `identidad`; la llave")
print("     vive en KMS del proyecto de la plataforma. Tres sitios para un secreto.")
if "identidad" in host:
    anota("cruza", "el material CIFRADO vive en la base central (`%s`), y el cofre del inquilino la alcanza por 5432" % host,
          "en dedicado/BYOC eso es abrir la base central a otro cluster; ninguno de los medidos lo hace")
anota("cruza", "la llave (KEK `ore/%s`) vive en el KMS del proyecto de la plataforma" % INQ,
      "en BYOC la llave tiene que ser suya: la 0024 ya lo nombra como E5")

# ── D ───────────────────────────────────────────────────────────────────────
titulo("D - LO QUE HAY GUARDADO HOY")
for l in sql("select o.nombre, count(s.id), count(*) filter (where s.retirado_en is not null), string_agg(distinct s.clase, ',') "
             "from iam.organizacion o left join cofre.secreto s on s.organizacion = o.id group by 1 order by 1").splitlines():
    if l.strip():
        n, cnt, ret, clases = l.split("|")
        print("   %-8s %2s secretos · %s retirados · clases: %s" % (n, cnt, ret, clases or "—"))
vers = sql("select count(*), max(version), max(length(cifrado)), min(length(cifrado)) from cofre.material").strip().split("|")
print("   material: %s filas · version maxima %s · cifrado entre %s y %s bytes" % (vers[0], vers[1], vers[3], vers[2]))
con = sql("select rol, count(*) from iam.concesion_viva where recurso like 'secreto/%' group by 1 order by 1")
print("   concesiones vivas sobre `secreto/*`: %s" % " · ".join("%s=%s" % tuple(l.split("|")) for l in con.splitlines() if l.strip()))
print()
print("   quien escribe: `ore-serve` al dar de alta una fuente (POST /secretos, clase `conexion`)")
print("   quien lee:     el Job de catalogo (GET /secretos/{nombre}) con el agente del inquilino")
anota("mover", "%s secretos vivos, todos `conexion`, una version cada uno; %s bytes como mucho" % (
    sql("select count(*) from cofre.secreto where retirado_en is null").strip(), vers[2]),
      "cabe de sobra en un Secret Manager (64 KiB por version) y la migracion es un bucle")

# ── E ───────────────────────────────────────────────────────────────────────
titulo("E - QUE CRUZA DE PLANO, COTEJADO CON EL SECTOR")
print("""
   Lo que hacen los que viven de BYOC (medido en su documentacion, 2026-09-13):
     Redpanda   «static secrets in AWS Secrets Manager or GCP Secret Manager, and
                 those secrets never leave the data plane account or network»
     WarpStream «only metadata is transferred from your environment to WarpStream's»

   Lo nuestro hoy, pieza a pieza:
     en claro     SOLO en memoria del pod del cofre, en el inquilino (KMS por stdin/stdout)  ✓
     cifrado      en Postgres CENTRAL, alcanzado por SQL desde el inquilino               ✗
     llave        en KMS del proyecto de la PLATAFORMA, una por organizacion              ✗ (E5)
     quien puede  en `iam.concesion`, central — y eso es METADATO, que SI va central      ✓
""")

orden = ["mover", "rompe", "cruza"]
for clase in orden:
    los = [h for h in hallazgos if h[0] == clase]
    if not los:
        continue
    print("   [%s]" % clase.upper())
    for _, que, det in los:
        print("     · %s" % que)
        if det:
            for l in det.splitlines():
                print("       %s" % l)
print()
print("  => Mover el material a la celda es: 2 tablas + 1 vista, 3 funciones, %s filas, y UNA" % vers[0])
print("     invariante que pasa de un `join` a un orden en el codigo. La llave es otra etapa (E5).")
print("     Lo que NO se mueve: `iam.concesion`. Quien puede sigue siendo del plano de control.")
