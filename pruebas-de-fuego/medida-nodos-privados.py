# -*- coding: utf-8 -*-
"""Nodos privados, Cloud NAT y la politica de salida — las tres a la vez.

Se miden juntas porque **la respuesta de cada una cambia la de las otras**, y
medirlas por separado da tres decisiones que no encajan. La conclusion es que
dos de las tres cuestan menos de lo que parecia y la tercera cuesta mas.

  A. LO QUE SE PUEDE HACER SIN RECREAR NADA
  B. NAT: PARA QUE SI, Y PARA QUE NO
  C. LA POLITICA: EL /30 Y LO QUE HAY QUE MONTAR PARA MERECERLO
  D. LO QUE CUESTA
  E. EL ORDEN, Y LO QUE DESBLOQUEA CADA PASO
"""
import textwrap


def parrafo(t, sangria="     ", ancho=72):
    for l in textwrap.wrap(t, ancho):
        print("%s%s" % (sangria, l))


print("== nodos privados, NAT y la salida — medido ==")

# -- A -------------------------------------------------------------------------
print()
print("A - LO QUE SE PUEDE HACER SIN RECREAR NADA")
print()
print("   %-46s %s" % ("`--enable-private-nodes` en `node-pools create`", "SI"))
print("   %-46s %s" % ("`--enable-private-nodes` en `clusters update`", "SI"))
print()
parrafo("Es la mejor noticia de esta medida y no la esperaba: **un pool se "
        "puede crear privado sin tocar el cluster**. Asi que la migracion no es "
        "«recrear el cluster» —que es lo que yo daba por hecho— es crear el "
        "pool nuevo, mover la carga y borrar el viejo. Y el pool de jobs escala "
        "a cero, asi que ni siquiera hay carga que mover.")

# -- B -------------------------------------------------------------------------
print()
print("B - NAT: PARA QUE SI, Y PARA QUE NO")
print()
parrafo("Aqui esta el hallazgo que ahorra trabajo. **Private Google Access "
        "—que ya esta encendido en la subred— deja que una VM SIN IP publica "
        "alcance las APIs de Google.** Y casi todo lo que esta malla necesita "
        "es una API de Google:")
print()
DESTINOS = [
    ("Artifact Registry — bajar `ore` y `ore-drivers`", "*.pkg.dev", "Google · PGA"),
    ("BigQuery — el driver leyendo un origen", "bigquery.googleapis.com", "Google · PGA"),
    ("las credenciales de Workload Identity", "el servidor de metadatos", "local al nodo"),
    ("logs y metricas", "logging/monitoring.googleapis.com", "Google · PGA"),
    ("un Postgres del cliente en internet", "un host cualquiera", "NAT · NO es Google"),
    ("una imagen de Docker Hub", "registry-1.docker.io", "NAT · NO es Google"),
]
print("   %-46s %s" % ("que necesita salir", "por donde"))
print("   " + "-" * 74)
for q, _, v in DESTINOS:
    print("   %-46s %s" % (q, v))
print()
parrafo("**Cuatro de las seis no necesitan NAT.** Con nodos privados y PGA, la "
        "malla arranca, se descarga las imagenes y consulta BigQuery sin un "
        "gateway de NAT. NAT hace falta para lo que NO es de Google — un "
        "Postgres del cliente, una imagen base de otro registro— y eso es real "
        "pero llega despues.")
print()
parrafo("Y hay un efecto de segundo orden que decide mas que el coste: sin IP "
        "publica **el nodo deja de consumir `IN_USE_ADDRESSES`**. La cuota es "
        "4, cada nodo publico se come una, y con un sistema mas tres de jobs el "
        "cluster topaba en cuatro nodos. Hacerlos privados quita ese techo "
        "**sin esperar a que nadie apruebe una cuota**.")

# -- C -------------------------------------------------------------------------
print()
print("C - LA POLITICA: EL /30, Y LO QUE HAY QUE MONTAR PARA MERECERLO")
print()
parrafo("`NetworkPolicy` de Kubernetes solo sabe de CIDR —no hay "
        "`CiliumNetworkPolicy` en Dataplane V2— asi que la excepcion del driver "
        "se escribe con direcciones. Las opciones son dos y no se parecen:")
print()
print("   %-30s %-14s %s" % ("", "tamaño", "que exige"))
print("   " + "-" * 74)
print("   %-30s %-14s %s" % ("los rangos publicos", "145 prefijos",
                             "mantenerlos al dia; cambian"))
print("   %-30s %-14s %s" % ("restricted.googleapis.com", "199.36.153.4/30",
                             "una zona de DNS privada"))
print()
parrafo("El /30 no es gratis: para que el trafico VAYA ahi, "
        "`*.googleapis.com` tiene que RESOLVER a esas cuatro direcciones, y eso "
        "es una zona privada de Cloud DNS sobre la VPC. Sin ella, el nodo "
        "resuelve a las IP publicas de siempre —PGA las encamina igual de "
        "bien— y la politica volveria a necesitar los 145 prefijos.")
print()
parrafo("Vale la pena por lo que `restricted` es, no por lo que mide: sirve "
        "**solo las APIs que soportan Controles de Servicio de VPC**, que es el "
        "conjunto sobre el que se puede impedir la exfiltracion. Un driver que "
        "solo alcanza ese /30 no escribe a un bucket de otro proyecto aunque "
        "alguien le meta credenciales. Cuatro direcciones y una zona de DNS "
        "compran una frontera, no una lista.")
print()
print("   Y las dos mitades de la excepcion, por sujeto:")
print()
for q, por in [
    ("el pod que PLANIFICA", "nada. Ni una regla de salida — la imagen `ore` "
     "ni siquiera lleva cliente TLS, asi que la garantia es estructural y la "
     "politica solo la confirma"),
    ("el pod DRIVER contra BigQuery", "`199.36.153.4/30:443`, y el servidor de "
     "metadatos para las credenciales"),
    ("el pod DRIVER contra un Postgres", "el `/32` del origen y su puerto — y "
     "**ese dato sale de `datasources` del `OntologyConfig`**, no de que "
     "alguien lo escriba a mano en YAML de Kubernetes"),
]:
    print("   · %s" % q)
    parrafo(por, "       ")
    print()
parrafo("La tercera es la que importa a largo plazo: **la politica de salida "
        "de un tenant es DERIVABLE de su ontologia**. Es el mismo P2 de "
        "siempre —lo derivable no se declara— aplicado a la malla, y es donde "
        "el sustrato y el cluster dejan de ser dos cosas.")

# -- D -------------------------------------------------------------------------
print()
print("D - LO QUE CUESTA")
print()
H = 730
filas = [
    ("gateway de Cloud NAT", 0.0014 * 2 * H, "por VM asociada, con dos nodos"),
    ("IP estatica del NAT", 0.0049 * H, "una, y es la que sale a internet"),
    ("datos por el NAT", 0.0, "0,045 $/GB — lo de Google NO pasa por aqui"),
    ("zona privada de Cloud DNS", 0.20, "0,20 $/zona/mes"),
    ("nodos privados", 0.0, "no cuesta: deja de gastar una IP"),
]
print("   %-28s %10s   %s" % ("", "$/mes", ""))
print("   " + "-" * 74)
for q, c, n in filas:
    print("   %-28s %10.2f   %s" % (q, c, n))
print("   %-28s %10.2f" % ("TOTAL si se monta todo", sum(f[1] for f in filas)))
print()
parrafo("Menos de seis dolares al mes sobre los 56 que ya cuesta el cluster. "
        "El coste NO es la razon para posponerlo, y tampoco para hacerlo: la "
        "razon es que quita el techo de cuatro nodos y cierra la salida.")

# -- E -------------------------------------------------------------------------
print()
print("E - EL ORDEN, Y LO QUE DESBLOQUEA CADA PASO")
print()
PASOS = [
    ("1 · pool de jobs PRIVADO", "quita el techo de 4 nodos y la superficie "
     "entrante. El pool escala a cero, asi que no hay carga que mover: se crea "
     "el nuevo, se borra el viejo. PGA ya cubre bajar la imagen y BigQuery"),
    ("2 · la politica de salida CON LOS 145 PREFIJOS", "fea y funciona hoy. "
     "Cierra la salida de verdad sin montar DNS, y se sustituye por el /30 "
     "cuando toque. Empezar por lo bonito seria retrasar el cierre"),
    ("3 · zona privada de DNS + el /30", "sustituye la lista por cuatro "
     "direcciones y trae la frontera de VPC-SC"),
    ("4 · Cloud NAT", "cuando haga falta salir a algo que NO sea de Google — un "
     "Postgres del cliente. Hoy no hay ninguno"),
]
for q, por in PASOS:
    print("   %s" % q)
    parrafo(por, "       ")
    print()
parrafo("Y lo que esto cambia respecto a lo que yo propuse antes: **NAT deja "
        "de ser el paso uno**. Lo era porque di por hecho que un nodo privado "
        "necesita NAT para todo, y no: para lo de Google le basta Private "
        "Google Access, que ya esta encendido.")
