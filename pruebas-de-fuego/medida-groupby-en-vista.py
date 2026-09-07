# -*- coding: utf-8 -*-
"""`groupBy` en `View`: que enciende exactamente, y que hay que escribir.

La frase de la medida anterior era «es la unica que no construye maquina: la
maquina esta». Cierto y a medias. Esto la parte en dos:

  A. LO QUE SE ENCIENDE       funcion a funcion, y que hace cada una
  B. LO QUE SE GOBIERNA       el linaje indirecto, y quien ya lo esperaba
  C. LO QUE HAY QUE ESCRIBIR  y el descubrimiento: `groupBy` solo no basta
  D. EL PRECIO DE `fields`    quien asume hoy que es un renombre
  E. EL CORTE
"""
import pathlib
import re
import textwrap

RAIZ = pathlib.Path(r"C:\ORE")
VIEW = RAIZ / "crates/ore-view/src"


def parrafo(t, sangria="     ", ancho=70):
    for l in textwrap.wrap(t, ancho):
        print("%s%s" % (sangria, l))


def produccion(p):
    """El fichero SIN su `mod tests`. Lo que se enciende es codigo de
    produccion; contar dentro de las pruebas seria contar el termometro."""
    t = p.read_text(encoding="utf-8", errors="replace")
    i = t.find("#[cfg(test)]")
    return t[:i] if i > 0 else t


def funciones_con(t, patron):
    """Las funciones cuyo cuerpo menciona el patron. Los metodos van indentados
    dentro de un `impl`, asi que el ancla admite sangria: la primera version la
    exigia en columna 0 y dio «solo en pruebas» sobre tres piezas que si lo
    tocan en produccion."""
    fns = [(m.group(1), m.start())
           for m in re.finditer(r"^\s*(?:pub(?:\([a-z]+\))? )?(?:const )?fn ([a-z_0-9]+)",
                                t, re.M)]
    fns.append(("<fin>", len(t)))
    out = []
    for (n, a), (_, b) in zip(fns, fns[1:]):
        if re.search(patron, t[a:b]) and n not in out:
            out.append(n)
    return out


print("== `groupBy` en `View`: que enciende ==")

# -- A -------------------------------------------------------------------------
print()
print("A - LO QUE SE ENCIENDE")
print()
parrafo("`vista.rs` es la UNICA costura entre un documento y el IR del motor, y "
        "hoy fabrica exactamente tres nodos: `Lee`, `Filtra` y `Proyecta`. Todo "
        "lo que responda a un `Agrupa` esta escrito y probado, y ninguna prueba "
        "lo alcanza desde un documento: lo construyen a mano.")
print()
QUE_HACE = {
    "plan": "la identidad del plan agregado: forma canonica RFC 8785, digesto, ida y vuelta a JSON",
    "schema": "el TIPO de salida. `cuenta` da Integer sin columna; `prom` da Decimal y no Integer —«el primer sitio por donde se pierde un decimal»—; y un agregado sin campo es `AgregadoSinCampo`",
    "lineage": "las dos aristas de OpenLineage que hoy no puede producir ningun documento: `AGGREGATION` directa, y `GROUP_BY` INDIRECTA — la clave de grupo decide que filas se agregan juntas",
    "view_matcher": "el Check 4: contestar una consulta desde una copia agregada de grano MAS FINO, enrollando. Y `AVG` no se enrolla",
    "delta_compiler": "mantener un agregado incremental: un acumulador por grupo para suma y cuenta, el multiconjunto entero para min y max —no son invertibles bajo baja— y la media RECHAZADA con su motivo",
    "capabilities": "que se puede BAJAR al origen con `aggregatePushdown`, y que columnas hacen falta arriba",
    "catalog": "reescribir el plan cuando la hoja cambia de sitio",
}
total = 0
for p in sorted(VIEW.glob("*.rs")):
    t = produccion(p)
    fns = funciones_con(t, r"Agrupa|Agregado|Agregacion")
    if not fns:
        continue
    total += len(fns)
    print("   · %s — %d funcion(es)" % (p.stem, len(fns)))
    print("       %s" % ", ".join("`%s`" % f for f in fns))
    if p.stem in QUE_HACE:
        parrafo(QUE_HACE[p.stem], "       ")
    print()
print("   %d funciones de produccion, ninguna alcanzable desde un documento." % total)

# -- B -------------------------------------------------------------------------
print()
print("B - LO QUE SE GOBIERNA, y quien ya lo esperaba")
print()
lin = produccion(VIEW / "lineage.rs")
print("   Las clases de arista que el linaje ya distingue:")
for m in re.finditer(r'Clase::(Directo|Indirecto)\((\w+)::(\w+)\) => "(\w+)"', lin):
    marca = "  <- solo desde un `Agrupa`" if m.group(4) in ("AGGREGATION", "GROUP_BY") else ""
    print("     %-10s %s%s" % (m.group(1), m.group(4), marca))
print()
GOB = [
    ("`GROUP_BY` es INDIRECTA", "crates/ore-view/src/lineage.rs",
     "y por eso la comprobacion de flujo implicito ya la mira: la clave de "
     "grupo influye en el resultado sin aparecer en el. Es el mismo mecanismo "
     "que ya niega una vista que recorta por `nationalId` y expone solo `id`"),
    ("el desclasificador `aggregate`", "crates/ore-core/src/flow.rs",
     "y su umbral `minGroupSize`, sin el cual es `OOS4007`. Hoy solo lo puede "
     "declarar una politica; una vista que agrega es el sujeto que le faltaba"),
    ("`OOS5016`", "crates/ore-core/src/code.rs",
     "bajar el `minGroupSize` ya es un cambio rompedor declarado"),
    ("la accion `aggregate` de Cedar", "crates/ore-core/src/cedar_schema.rs",
     "una de las cuatro, junto a `read`, `export` e `invoke`"),
]
for q, d, por in GOB:
    print("   · %s" % q)
    parrafo(por, "       ")
    print("       %s" % d)
    print()

# -- C -------------------------------------------------------------------------
print()
print("C - LO QUE HAY QUE ESCRIBIR, Y EL DESCUBRIMIENTO")
print()
parrafo("`Nodo::Agrupa` lleva DOS cosas: `por` —las claves— y `agregados` —un "
        "`Agregacion { funcion, sobre }` por columna de salida—. `groupBy` "
        "aporta la primera. La segunda no tiene donde escribirse.")
print()
compilador = (RAIZ / "crates/ore-cli/src/vista.rs").read_text(encoding="utf-8",
                                                              errors="replace")
m = re.search(r"\.map\(\|\(campo, en_fuente\)\| ([^\n]+)\)\n", compilador)
print("   Como se compila `fields` hoy:")
print("     %s" % (m.group(1).strip() if m else "(campo.clone(), Expr::campo(en_fuente))"))
print()
parrafo("Un renombre, y nada mas: `Expr::campo(en_fuente)`. Para pedir "
        "`total: sum(importe)` o `n: count()`, `fields` tiene que dejar de ser "
        "«alias -> columna». Y eso choca de frente con la guarda que ya existe:")
print()
vist = (RAIZ / "crates/ore-core/src/vistas.rs").read_text(encoding="utf-8", errors="replace")
for m in re.finditer(r"(CampoCalculado|ConstruccionDesconocida) \{ vista: String, (\w+): String \}", vist):
    print("     NoInvertible::%s" % m.group(1))
print()
parrafo("`CampoCalculado` dice literalmente que un campo que sale de CALCULARLO "
        "no se puede escribir al reves. Asi que el dia que `fields` admita un "
        "agregado, la guarda de invertibilidad pasa de ser una prueba de "
        "laboratorio a negarle la escritura a un documento real — que es "
        "exactamente para lo que se escribio, y hoy nadie la llama.")
print()
PASOS = [
    ("`groupBy` en el vocabulario", "`document.rs` + `view.schema.json`",
     "dos sitios, y su deriva la arbitra la suite de conformidad — no hay "
     "prueba que los compare, es la decision de ADR 0002"),
    ("clasificarlo", "`vistas.rs`",
     "el censo se cae hasta que `groupBy` este en `NEUTRAS` o en "
     "`INVERTIBLES`. Y no es ninguna de las dos: pide una tercera"),
    ("un agregado en `fields`", "el que cuesta",
     "es un cambio de forma, no una clave mas: hoy el valor es un nombre de "
     "columna y pasaria a ser un nombre O una llamada"),
    ("compilar a `Agrupa`", "`vista.rs`",
     "entre `Filtra` y `Proyecta`, que es donde el algebra lo pone"),
    ("decir si romper", "`diff.rs`",
     "anadir o cambiar una clave de grupo cambia QUE FILAS salen. `OOS5016` ya "
     "existe para el umbral; esto es otra pregunta"),
]
print("   %-32s %s" % ("paso", "donde"))
print("   " + "-" * 66)
for q, d, _ in PASOS:
    print("   %-32s %s" % (q, d))
print()
for q, _, por in PASOS:
    print("   · %s" % q)
    parrafo(por, "       ")
    print()

# -- C bis ---------------------------------------------------------------------
print()
print("C bis - LO QUE CONTESTA EL MOTOR, CORRIDO DE VERDAD")
print()
parrafo("Cuatro planes construidos a mano contra `schema::esquema`, "
        "`lineage::linaje` y `delta_compiler::motivos`. La corrida entera esta "
        "en el mensaje del commit; lo que queda aqui es la tabla y sus dos "
        "sorpresas.")
print()
print("   %-22s %-9s %-40s %s" % ("forma", "tipo", "linaje de la salida", "increm."))
print("   " + "-" * 84)
CORRIDA = [
    ("`groupBy` y nada mas", "-", "Directo(Identidad)", "si"),
    ("`count()`", "Integer", "Indirecto(Agrupacion)", "si"),
    ("`sum(total)`", "Integer", "Indirecto(Agrupacion) + Directo(Agregacion)", "si"),
    ("`avg(total)`", "Decimal", "Indirecto(Agrupacion) + Directo(Agregacion)", "NO"),
]
for f, ti, li, inc in CORRIDA:
    print("   %-22s %-9s %-40s %s" % (f, ti, li, inc))
print()
for q in [
    "AGRUPAR SIN AGREGADOS NO ENCIENDE EL GOBIERNO. Es un `SELECT DISTINCT` y "
    "su linaje sale `Directo(Identidad)`: la arista INDIRECTA aparece cuando "
    "hay un agregado influido por la agrupacion, no por agrupar. El paso "
    "barato es real y NO es el que ejerce el flujo implicito",
    "Y UNA CORRECCION MIA. Dije que «la media se parte en suma y cuenta». No: "
    "`motivos` devuelve `Promedio { nombre }` y la RECHAZA. Lo de «SUMA y "
    "CUENTA aparte» describe el estado que un almacen de produccion "
    "necesitaria, no una reescritura que el compilador haga. `avg` no se "
    "mantiene incrementalmente: se dice por que",
]:
    parrafo("· " + q, "     ")
    print()

# -- D -------------------------------------------------------------------------
print()
print("D - EL PRECIO DE TOCAR `fields`")
print()
lectores = []
for p in sorted((RAIZ / "crates").rglob("src/*.rs")):
    t = produccion(p)
    if re.search(r'section\("fields"\)|columnas_de\(\w+, "fields"\)', t):
        lectores.append(str(p.relative_to(RAIZ)).replace("\\", "/"))
print("   Quien lee `fields` y asume que su valor es un nombre de columna:")
for l in lectores:
    print("     %s" % l)
print()
print("   %d lectores." % len(lectores))
print()
parrafo("Ninguno se rompe por anadir `groupBy`. Todos hay que mirarlos el dia "
        "que el valor de `fields` pueda no ser una columna — y por eso son dos "
        "iteraciones y no una.")

# -- E -------------------------------------------------------------------------
print()
print("E - EL CORTE")
print()
parrafo("La medida anterior decia «enciende algo ya construido» y era verdad a "
        "medias: enciende %d funciones de produccion, y para encenderlas hacen "
        "falta DOS cambios de vocabulario, no uno. El barato es `groupBy`; el "
        "caro es que `fields` deje de ser un renombre — y el gobierno solo se "
        "enciende con el caro." % total)
print()
for q in [
    "`groupBy` SOLO ES DEDUPLICAR, y funciona: esquema, linaje e incremental, "
    "los tres. Pero su linaje sale DIRECTO, asi que no ejerce el gobierno. Es "
    "un peldano de verdad y no es el que importa",
    "EL PRIMER PASO QUE IMPORTA ES `count()`. Es el unico agregado que no "
    "necesita columna —`sobre: None`—, sale `Integer`, se mantiene "
    "incrementalmente, y ES el que hace aparecer la arista INDIRECTA que la "
    "comprobacion de flujo mira. Enciende el gobierno con el minimo de "
    "vocabulario",
    "PERO `count()` YA CAMBIA LA FORMA DE `fields`: el valor deja de ser un "
    "nombre de columna. No hay paso que encienda el gobierno sin tocarla, y "
    "eso es lo que esta medida vino a averiguar",
    "LA ESCRITURA SE QUEDA FUERA, y por primera vez de forma OBSERVABLE: "
    "`invertible` deja de ser un laboratorio el dia que un documento real la "
    "haga decir que no",
]:
    parrafo("· " + q, "     ")
    print()
