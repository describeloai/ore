# -*- coding: utf-8 -*-
"""MEDIDA · qué separa a la consola de `ore-iam`, en todo su espectro.

No se escribe nada hasta saber esto. La consola (`C:\\rubix-platform`) habla hoy
con `admin/` —la Admin API legacy, en el clúster viejo, contra el Cloud SQL
viejo— y ahí no vamos a volver. La pregunta no es «¿cuántos endpoints faltan?»,
que es la fácil, sino qué de lo que hay que decidir NO es código.

Se mide sobre los ficheros, no sobre lo que recuerdo:

    C:\\rubix-platform\\lib\\server\\query.ts     lo que la consola pide
    C:\\rubix-platform\\components\\governance    lo que las pantallas leen
    C:\\rubix-platform\\lib\\banco\\modo.ts       el banco, y sus tres interruptores
    C:\\Rubix\\admin\\src\\server.ts              lo que contesta hoy
    C:\\ORE\\crates\\ore-iam\\src\\rutas.rs       lo que contestaríamos
    C:\\ORE\\malla\\61-realms.yaml                si el token siquiera entraría

    uso:  python pruebas-de-fuego/medida-la-consola-contra-ore-iam.py
"""
import json
import pathlib
import re
import sys

try:
    sys.stdout.reconfigure(encoding="utf-8")
except AttributeError:
    pass

CONSOLA = pathlib.Path(r"C:\rubix-platform")
PLATAFORMA = pathlib.Path(r"C:\Rubix")
ORE = pathlib.Path(r"C:\ORE")

hallazgos = []


def titulo(n, t):
    print("\n" + "=" * 78)
    print("%s · %s" % (n, t))
    print("=" * 78)


def leer(p):
    try:
        return p.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return ""


def cita(t, marca, antes=0, despues=3):
    """Devuelve las lineas alrededor de la primera aparicion de `marca`."""
    ls = t.splitlines()
    for i, l in enumerate(ls):
        if marca in l:
            return "\n".join(ls[max(0, i - antes):i + despues])
    return ""


# ═══ ① LA SUPERFICIE QUE LA CONSOLA PIDE ═══════════════════════════════════
titulo("①", "LO QUE LA CONSOLA PIDE, extraido de `lib/server/query.ts`")

q = leer(CONSOLA / "lib" / "server" / "query.ts")

# ⚠️ Una sola expresion por linea se dejaba `revocar-invitacion`, que va en un
#   bloque de varias con un comentario en medio. Se parte por entradas y se lee
#   cada una entera: una medida que cuenta de menos es peor que ninguna.
pide = []
# Una expresion no basta: unas entradas caben en una linea y otras llevan un
# comentario en medio. Se cuentan LLAVES, que es lo unico que no depende del
# formato — y una medida que cuenta de menos es peor que ninguna.
pide = []
entrada, hondo = None, 0
for linea in q.splitlines():
    if entrada is None:
        m = re.match(r"\s{2}'?([a-z-]+)'?:\s*\{", linea)
        if not m:
            continue
        entrada, hondo = [m.group(1), linea], 0
    else:
        entrada[1] += "\n" + linea
    hondo += linea.count("{") - linea.count("}")
    if hondo > 0:
        continue
    nombre, cuerpo = entrada
    entrada = None
    met = re.search(r"metodo:\s*'(\w+)'", cuerpo)
    pla = re.search(r"plano:\s*'(\w+)'", cuerpo)
    rut = re.search(r"ruta:\s*(?:\([^)]*\)\s*=>\s*)?[`']([^`'$]+)", cuerpo)
    if met and pla and rut:
        pide.append((nombre, met.group(1), rut.group(1), pla.group(1)))

control = [x for x in pide if x[3] == "control"]
dato = [x for x in pide if x[3] != "control"]
for n, m, r, _ in sorted(control):
    print("  control   %-22s %-6s %s" % (n, m, r))
for n, m, r, _ in sorted(dato):
    print("  dato      %-22s %-6s %s" % (n, m, r))
print("\n  ⇒ %d del plano de CONTROL (las que van al 8082) · %d del de datos"
      % (len(control), len(dato)))
hallazgos.append("la consola pide %d cosas al plano de control" % len(control))

# ═══ ② LO QUE `ore-iam` SIRVE ══════════════════════════════════════════════
titulo("②", "LO QUE `ore-iam` SIRVE HOY, de `rutas.rs::mapa()`")

r = leer(ORE / "crates" / "ore-iam" / "src" / "rutas.rs")
sirve = re.findall(r'\("(GET|POST|GET \| POST)",\s*"([^"]+)"', r)
for m, ruta in sirve:
    print("  %-6s %s" % (m, ruta))
print("\n  ⇒ %d rutas" % len(sirve))

# ═══ ③ EL COTEJO ═══════════════════════════════════════════════════════════
titulo("③", "EL COTEJO, endpoint por endpoint")

COTEJO = [
    ("GET  /organization/members", "⛔ NO EXISTE",
     "listar la `pertenencia` de una organizacion. No hay ruta."),
    ("GET  /organization/roles", "⚠ OTRA FORMA",
     "el suyo devuelve {catalogo:{porDefecto,porRol}, asignaciones}. El catalogo "
     "son POTESTADES por rol; el nuestro es una escalera con `que_puede` en prosa."),
    ("GET  /organization/invitations", "✓ EXISTE",
     "con la organizacion en el camino, no en el token. Ver ④a."),
    ("GET  /organization/activity", "⚠ OTRA COSA",
     "el suyo lee `rubix.acceso`: accesos a DATOS, particionado, 400 dias, "
     "servidas/consideradas. `iam.huella` son actos ADMINISTRATIVOS. Ver ④c."),
    ("POST /organization/roles", "⛔ NO EXISTE",
     "investir a alguien ya dentro. Hoy `pertenencia.rol` SOLO lo escribe `admitir`."),
    ("POST /organization/invitations", "✓ EXISTE", "misma salvedad de la organizacion."),
    ("DEL  /organization/invitations/{id}", "⛔ NO EXISTE",
     "`006` tiene `revocada_en` y `revoco`, y ningun verbo las usa."),
]
for ruta, estado, nota in COTEJO:
    print("  %-38s %s" % (ruta, estado))
    print("      %s" % nota)
faltan = sum(1 for _, e, _ in COTEJO if e.startswith("⛔"))
distintas = sum(1 for _, e, _ in COTEJO if e.startswith("⚠"))
print("\n  ⇒ %d hay · %d con otra forma · %d no existen"
      % (len(COTEJO) - faltan - distintas, distintas, faltan))
hallazgos.append("%d verbos nuevos, %d cambios de forma" % (faltan, distintas))

# ═══ ④ LO QUE NO ES CODIGO: TRES DECISIONES ════════════════════════════════
titulo("④", "LAS TRES DECISIONES, y ninguna se resuelve escribiendo mas rutas")

print("""
  ⓐ ⭐⭐ LA ORGANIZACION: ¿del token, o del camino?

     En `admin/` sale del TOKEN, y su motivo esta escrito:""")
print("       " + cita(leer(CONSOLA / "lib" / "server" / "roles.ts"),
                       "salen del token", antes=1, despues=2).strip()[:300])
print("""
     En `ore-iam` va en el CAMINO —`/organizaciones/{org}/…`— porque una
     persona puede pertenecer a varias y `GET /organizaciones` devuelve todas
     las suyas.

     ⛔ No se arregla con un `if`. O el token lleva la organizacion —y entonces
       una persona con dos necesita DOS sesiones— o la consola aprende a elegir
       organizacion y a llevarla en cada llamada. Es la diferencia entre una
       consola de un cliente y una consola de una plataforma.

  ⓑ EL CATALOGO DE POTESTADES

     Su `GET /organization/roles` devuelve el catalogo junto a las asignaciones,
     y por un motivo que sigue siendo bueno: *«si la interfaz tuviera su propia
     copia de las potestades, habria dos descripciones de lo mismo»*.

     ⛔ Nosotros NO tenemos catalogo de potestades. `iam.rol` es una escalera de
       cuatro con una frase cada uno. La pantalla de roles pinta «que anade cada
       rol», y eso aqui no existe: habria que inventarlo o cambiar la pantalla.

  ⓒ LA ACTIVIDAD NO ES LA HUELLA

     `rubix.acceso`   quien consulto QUE DATOS · particionado · 400 dias
                      servidas/consideradas · se suelta por particion
     `iam.huella`     quien hizo QUE ACTO administrativo · sin retencion escrita

     ⛔ Pintar una en el sitio de la otra es mentir en una pantalla que se llama
       auditoria. O se decide que la actividad de la consola son los actos
       administrativos —y se dice—, o esa pantalla se queda fuera.
""")
hallazgos.append("3 decisiones de forma que no son codigo")

# ═══ ⑤ EL VOCABULARIO CABLEADO EN LAS PANTALLAS ════════════════════════════
titulo("⑤", "EL VOCABULARIO, cableado en la consola")

viejos = ["ACCOUNTADMIN", "USERADMIN", "SECURITYADMIN"]
sitios = {}
for p in (CONSOLA / "components").rglob("*.tsx"):
    t = leer(p)
    n = sum(t.count(v) for v in viejos)
    if n:
        sitios[str(p.relative_to(CONSOLA))] = n
for p in (CONSOLA / "lib").rglob("*.ts"):
    t = leer(p)
    n = sum(t.count(v) for v in viejos)
    if n:
        sitios[str(p.relative_to(CONSOLA))] = n
for f, n in sorted(sitios.items(), key=lambda x: -x[1]):
    print("  %-52s %d menciones" % (f, n))
print("\n  ⇒ los tres roles viejos aparecen %d veces en %d ficheros: listas cerradas,"
      % (sum(sitios.values()), len(sitios)))
print("    colores por rol y prosa por rol. Nuestros cuatro (`lector` `miembro`")
print("    `administrador` `dueno`) NO son los mismos ni en numero ni en eje.")
hallazgos.append("%d menciones del vocabulario viejo en %d ficheros" % (sum(sitios.values()), len(sitios)))

# ═══ ⑥ ⭐ ¿ENTRARIA SIQUIERA EL TOKEN? ═════════════════════════════════════
titulo("⑥", "LA AUDIENCIA — y esto para el trabajo antes de empezarlo")

print("  La consola manda `Authorization: Bearer ${sesion.acceso}` — el token")
print("  del realm. `ore-iam servir` valida emisor Y AUDIENCIA.\n")

# Los realms por donde entra una consola: si a uno le falta el mapeador, el
# token de esa consola no vale para este plano.
CON_CONSOLA = ("rubix", "rubix-dev")
falta_audiencia = []

crudo = leer(ORE / "malla" / "61-realms.yaml")
trozos = crudo.split("\n---\n")
trozos[0] = trozos[0][trozos[0].index("{"):]
for t in trozos:
    if not t.strip():
        continue
    d = json.loads(t)
    realm = d["spec"]["realm"]
    clientes = {c.get("clientId"): c for c in realm.get("clients", [])}
    consola = clientes.get("rubix-consola")
    mapea = False
    if consola:
        mapea = any(m.get("name") == "audiencia-ore-serve"
                    for m in consola.get("protocolMappers", []) or [])
    print("  %-14s clientes: %-42s mapeador ore-serve en `rubix-consola`: %s"
          % (realm["realm"], ", ".join(sorted(clientes)) or "—",
             "SI" if mapea else "NO"))
    if realm["realm"] in CON_CONSOLA and not mapea:
        falta_audiencia.append(realm["realm"])

print("""
  ✅ ARREGLADO el 2026-09-08. Esto decia `SOLO_EN = "rubix"`, y el emisor con el
    que corre `ore-iam` —y contra el que entra la consola local— es `rubix-dev`.
    El token que la consola ya tenia se rechazaba con 401 «no es para nosotros»,
    y el sintoma mandaba a mirar las rutas, que estaban bien.

  ⚠️ Y el artefacto NO basta: un `KeycloakRealmImport` se salta un realm que ya
    existe. El realm vivo hubo que cambiarlo con `kcadm`. Esta comprobacion se
    queda viva para que la proxima vez no se olvide la mitad.""")
if falta_audiencia:
    hallazgos.append("⛔ AUDIENCIA AUSENTE en %s: todo dara 401" % ", ".join(falta_audiencia))
else:
    hallazgos.append("✅ la audiencia esta en los realms por donde entra una consola")

# ═══ ⑦ LA RED ══════════════════════════════════════════════════════════════
titulo("⑦", "POR DONDE SE LLEGA")

pol = leer(ORE / "malla" / "67-iam-servir.yaml")
print("  `ore-iam` vive en `identidad` y su NetworkPolicy admite:")
for m in re.findall(r"ore\.dev/(tenant|rol):\s*(\S+)", pol.split("entrada-a-ore-iam")[-1]):
    print("      %s: %s" % m)
print("""
  ⇒ Una consola en `localhost:3000` no es ninguno de los dos. Hoy solo se llega
    con `kubectl port-forward -n identidad svc/ore-iam 8090:8090`, que sirve
    para desarrollar y no es un despliegue.

  ⚠️ Publicarlo es OTRA decision, y no pequeña: administra personas. Su propia
    cabecera ya lo dice — *«Publicarlo es otra decision»*.""")

# ═══ ⑧ EL BANCO ════════════════════════════════════════════════════════════
titulo("⑧", "EL BANCO — lo que YA existe para ver las pantallas hoy")

modo = leer(CONSOLA / "lib" / "banco" / "modo.ts")
cond = re.findall(r"process\.env\.(\w+)\s*(!==|===)\s*'([^']*)'", modo)
for v, op, val in cond:
    print("  %-18s %s '%s'" % (v, op, val))
print("""
  ⭐ Tres interruptores y no uno, *«para que una variable copiada de un
    docker-compose no encienda esto sin que nadie lo lea»*. Con el banco activo
    las pantallas de gobierno pintan SIN nada detras — sesion falsa incluida.

  ⇒ Es la forma honesta de trabajar la interfaz mientras se escriben los verbos,
    y no compromete nada: los datos se llaman «(banco)» y los correos son
    `@invalido.paladio.io`.

  ⚠️ Y lo que el banco NO cubre, dicho por ellos: `catalogo` devuelve N-Triples
    y no pasa por ahi. Esa pantalla sigue sin poder pintarse.""")

# ═══ EL RESUMEN ════════════════════════════════════════════════════════════
titulo("⇒", "LO QUE SALE DE MEDIR")
for h in hallazgos:
    print("  · " + h)
print("""
  El orden que impone la medida, y no es el que parecia:

    0  LA AUDIENCIA en `rubix-dev`   ⛔ sin esto, todo lo demas da 401
    1  la organizacion: token o camino   ← decision, no codigo
    2  los tres verbos que son nuestros: miembros, investir, revocar invitacion
    3  el vocabulario en las pantallas
    4  la actividad                      ← decidir que significa, o dejarla fuera

  Y el banco permite empezar por el 3 sin esperar a ninguno de los otros.""")
