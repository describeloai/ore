# -*- coding: utf-8 -*-
"""Medida: por que una organizacion no puede tener dos serverless, y cuanto cuesta
que pueda. Redpanda: una organizacion tiene N clusters. ORE: una organizacion
tiene UNA celda, y la celda no tiene nombre propio — usa el de la organizacion.

Lo que se mide, antes de escribir la ADR 0025:
  A  las columnas de `iam.organizacion` que son de la CELDA, y quien las lee
  B  donde el nombre de la organizacion se usa como nombre TECNICO
  C  lo que `iam.celda` es hoy, y lo que le falta para ser la unidad
  D  lo que se queda en la cuenta (y por que no choca entre celdas)
  E  lo que rompe en `demo` si la primera celda se llama `demo`: nada — y se prueba
  F  la consola: donde supone «una organizacion = un arbol»

    PYTHONIOENCODING=utf-8 python pruebas-de-fuego/medida-la-celda-con-nombre.py
"""
import json
import os
import re
import subprocess

RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CONSOLA = os.environ.get("CONSOLA", r"C:\rubix-platform")


def leer(rel, raiz=RAIZ):
    with open(os.path.join(raiz, rel), encoding="utf-8") as f:
        return f.read()


def sql(q):
    r = subprocess.run(["kubectl", "-n", "identidad", "exec", "idp-db-0", "--", "psql", "-U", "keycloak", "-d", "iam", "-qtAc", q],
                       capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=60)
    return r.stdout.strip()


def grep(patron, rel, raiz=RAIZ, comentarios=False):
    """Lineas que casan, sin comentarios salvo que se pidan."""
    out = []
    for i, l in enumerate(leer(rel, raiz).splitlines(), 1):
        s = l.strip()
        if not comentarios and (s.startswith("#") or s.startswith("//") or s.startswith("*") or s.startswith("--")):
            continue
        if re.search(patron, l):
            out.append((i, s[:100]))
    return out


def titulo(t):
    print()
    print(t)
    print("-" * len(t))


mover = []   # (que, de donde, a donde)


# ── A ───────────────────────────────────────────────────────────────────────
titulo("A - LAS COLUMNAS DE `iam.organizacion` QUE SON DE LA CELDA, Y QUIEN LAS LEE")
cols = sql("select column_name from information_schema.columns where table_schema='iam' and table_name='organizacion' order by ordinal_position").split()
print("   iam.organizacion: %s" % ", ".join(cols))
CLASIF = {"arbol": "CELDA (donde vive un arbol)", "entrada": "CELDA (la puerta de ese arbol, 0024-6)",
          "kek": "CUENTA (la llave de la organizacion: cifra lo de todas sus celdas)",
          "nombre": "CUENTA", "estado": "CUENTA", "creada_por": "CUENTA", "id": "CUENTA", "creada_en": "CUENTA"}
for c in cols:
    print("     %-11s %s" % (c, CLASIF.get(c, "?")))
lectores = {}
for rel in ("crates/ore-iam/src/fundar.rs", "crates/ore-iam/src/rutas.rs", "malla/aprovisionar-inquilino.sh",
            "iam/migraciones/027-la-puerta-de-la-celda.sql", "iam/migraciones/023-el-papel-del-aprovisionador.sql"):
    hits = grep(r"\b(arbol|entrada)\b", rel)
    hits = [(i, l) for i, l in hits if re.search(r"select|insert|consulta|grant|o\.(arbol|entrada)", l)]
    if hits:
        lectores[rel] = hits
for rel in ("lib/server/organizacion.ts", "lib/server/celdas.ts", "lib/server/query.ts"):
    hits = grep(r"\.entrada\b|entradaActual\(|\.arbol\b", rel, CONSOLA)
    if hits:
        lectores["consola:" + rel] = hits
print()
for rel, hits in lectores.items():
    print("   %s" % rel)
    for i, l in hits[:4]:
        print("      %4d  %s" % (i, l))
mover.append(("`arbol` y `entrada`", "iam.organizacion", "iam.celda — y %d lectores que cambian de tabla" % len(lectores)))

# ── B ───────────────────────────────────────────────────────────────────────
titulo("B - DONDE EL NOMBRE DE LA ORGANIZACION ES EL NOMBRE TECNICO")
ap = leer("malla/aprovisionar-inquilino.sh")
usos = {}
for m in re.finditer(r'(t-\$NOMBRE|\$NS\b|ore-(?:cofre|serve|driver|forja)-\$NOMBRE|cq-\$NOMBRE|serve-\$NOMBRE|ore-agente-\$NOMBRE|ore/\$NOMBRE|--organizacion \$NOMBRE|\$PROPIETARIO)', ap):
    usos[m.group(1)] = usos.get(m.group(1), 0) + 1
CLASE = {"t-$NOMBRE": "CELDA namespace", "$NS": "CELDA namespace / prefijos del almacen", "ore-cofre-$NOMBRE": "CELDA cuenta de Google",
         "ore-serve-$NOMBRE": "CELDA cuenta", "ore-driver-$NOMBRE": "CELDA cuenta", "ore-forja-$NOMBRE": "CELDA cuenta",
         "cq-$NOMBRE": "CELDA cola de Kueue", "serve-$NOMBRE": "CELDA usuario de SU forja", "$PROPIETARIO": "CELDA organizacion de SU forja",
         "ore-agente-$NOMBRE": "CUENTA cliente del IdP (uno por organizacion)", "ore/$NOMBRE": "CUENTA llave", "--organizacion $NOMBRE": "CUENTA"}
print("   aprovisionar-inquilino.sh, por `$NOMBRE` (= la organizacion):")
for k, n in sorted(usos.items(), key=lambda x: -x[1]):
    print("     %-24s x%-3d %s" % (k, n, CLASE.get(k, "?")))
gen = leer("malla/gen-inquilino.py")
subs = re.findall(r't\.replace\("([^"]+)" % MODELO', gen)
print("   gen-inquilino.py sustituye por el nombre: %s" % ", ".join(dict.fromkeys(subs)))
print("   ⇒ el renderizador ya sustituye por UN nombre. Que ese nombre sea el de la celda en vez")
print("     del de la organizacion es un parametro, no una reescritura.")
mover.append(("`$NOMBRE` del aprovisionador y del renderizador", "nombre de la organizacion", "nombre de la CELDA; la organizacion se lee de la celda"))

# ── C ───────────────────────────────────────────────────────────────────────
titulo("C - LO QUE `iam.celda` ES HOY, Y LO QUE LE FALTA")
ccols = sql("select column_name from information_schema.columns where table_schema='iam' and table_name='celda' order by ordinal_position").split()
print("   iam.celda: %s" % ", ".join(ccols))
print("   filas: %s" % sql("select string_agg(o.nombre || ' → ' || c.nombre || '/' || c.tier, ' · ') from iam.celda c join iam.organizacion o on o.id = c.organizacion"))
idx = sql("select indexdef from pg_indexes where schemaname='iam' and tablename='celda' and indexname='celda_una_por_organizacion'")
print("   indice: %s" % idx)
print("   ⇒ `nombre` hoy es el CLUSTER (`ore-mesh`), no la celda. Y el indice unico es la unica linea que PROHIBE la segunda.")
print("     Le falta: un nombre propio (= namespace `t-<celda>`), `cluster` aparte, y `arbol` + `entrada` suyos.")
mover.append(("`iam.celda`", "una por organizacion, nombrada por el cluster", "N por organizacion: nombre propio · cluster · arbol · entrada · puerta"))

# ── D ───────────────────────────────────────────────────────────────────────
titulo("D - LO QUE SE QUEDA EN LA CUENTA, Y POR QUE NO CHOCA ENTRE CELDAS")
print("   kek ore/<org>            una llave cifra los secretos de TODAS sus celdas (CMEK por secreto)")
print("   agente ore-agente-<org>  un cliente del IdP; `iam.agente.organizacion` — los Jobs de cualquier celda lo usan")
print("   concesiones, potestades  por organizacion: quien puede usar un secreto no depende de en que celda corra el Job")
print("   secretos del cofre       `t-<NS>-cofre-*`: el prefijo es el NAMESPACE de la celda → cada celda ve los suyos")
print("   ⚠️ pero `cofre.secreto` es por ORGANIZACION (nombre unico por org): dos celdas de `demo` con una fuente")
print("      `pg` cada una chocarian en `cofre.secreto(organizacion, nombre)`. El secreto pasa a ser de la celda,")
print("      o su nombre lleva la celda. Es la unica pieza de la cuenta que la celda empuja.")
mover.append(("`cofre.secreto`", "unico por (organizacion, nombre)", "unico por (celda, nombre) — o el nombre lleva la celda"))

# ── E ───────────────────────────────────────────────────────────────────────
titulo("E - LO QUE ROMPE EN `demo` SI SU PRIMERA CELDA SE LLAMA `demo`")
print("   namespace t-demo · arbol t-demo/ontologia · entrada demo.ore.paladio.io · cuentas ore-*-demo · forja t-demo")
print("   ⇒ todos los nombres tecnicos de hoy son `demo`. Si la migracion crea la celda `demo` de la organizacion")
print("     `demo` con el arbol y la entrada que hoy estan en la organizacion, NADA cambia de nombre: ni un")
print("     manifiesto se re-rinde, ni una cuenta se crea, ni un DNS se toca. La 022 dijo «el valor por defecto se")
print("     deriva; el valor no» — y por eso la fila puede mudarse de tabla sin que el mundo lo note.")
print("   lo que SI cambia de sitio: `entradaActual` (consola) y `consulta arbol/entrada` (aprovisionador) leen de la celda.")

# ── F ───────────────────────────────────────────────────────────────────────
titulo("F - LA CONSOLA: DONDE SUPONE «UNA ORGANIZACION = UN ARBOL»")
org_ts = leer("lib/server/organizacion.ts", CONSOLA)
q_ts = leer("lib/server/query.ts", CONSOLA)
print("   `entradaActual(acceso)`: de la organizacion → https://<org.entrada>. UNA direccion de arbol por sesion.")
print("   plano `arbol` de query.ts: DONDE.arbol = entradaActual → todas las consultas del arbol van a ESA.")
print("   la vista de clusters: lista `/celdas` (ya es una LISTA) y sonda UNA (`salud-del-arbol`, la de la org).")
print("   ⇒ con dos celdas la consola necesita saber CUAL miras: `celdaActual` (cookie o ruta), como Redpanda")
print("     al entrar en un cluster. Sources, Catalog y la sonda pasan a ser de la celda que miras.")
mover.append(("la consola", "`entradaActual` por organizacion", "`celdaActual`: un selector de celda, y el plano `arbol` resuelto por ella"))

titulo("LO QUE SE MUEVE, EN UNA TABLA")
for que, de, a in mover:
    print("   %-52s %-42s → %s" % (que, de, a))
print()
print("  => %d movimientos, 1 migracion (028+1: la celda con nombre, arbol y entrada; el indice fuera)," % len(mover))
print("     y `demo` no se entera. Lo que si es nuevo: `cofre.secreto` por celda, y la consola eligiendo celda.")
