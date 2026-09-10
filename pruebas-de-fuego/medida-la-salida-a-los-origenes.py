# -*- coding: utf-8 -*-
"""MEDIDA · lo que cuesta que un Job pueda leer un origen que NO es de Google.

Lo que lo dispara, medido en vivo el 2026-09-10 con un Postgres de verdad —Neon,
en AWS— dado de alta desde la consola:

    ore-read-postgres, en un pod del `default-pool`   ✓ leyo la base:
                                                        olist.customers, …
    el MISMO driver, en el Job de catalogo            ✗ «no se pudo conectar»

⭐⭐ Y la diferencia no es el driver ni la credencial ni la `NetworkPolicy`: es
  DONDE corre. El Job aterriza en `jobs-p`, que no tiene IP publica, y no hay
  Cloud NAT en el proyecto. No hay ruta a internet, punto.

Se mide contra el arbol y contra lo que la nube contesto, con la procedencia de
cada numero escrita al lado.

    uso:  python pruebas-de-fuego/medida-la-salida-a-los-origenes.py
"""
import pathlib
import re
import sys

try:
    sys.stdout.reconfigure(encoding="utf-8")
    sys.stderr.reconfigure(encoding="utf-8")
except AttributeError:
    pass

RAIZ = pathlib.Path(__file__).resolve().parent.parent
fallos = []


def leer(p):
    f = RAIZ / p
    return f.read_text(encoding="utf-8") if f.exists() else ""


def titulo(t):
    print("\n" + "═" * 74)
    print(t)
    print("═" * 74)


# ══════════════════════════════════════════════════════════════════════════
titulo("① LO MEDIDO EN LA NUBE, CON SU ORDEN AL LADO")
# ══════════════════════════════════════════════════════════════════════════
HECHOS = [
    ("subred ore-mesh-europe-west1", "10.10.0.0/20 · privateIpGoogleAccess: True",
     "gcloud compute networks subnets describe"),
    ("pool default-pool", "IP externa 34.156.50.255 — sale a internet por si mismo",
     "gcloud compute instances list"),
    ("pool jobs-p", "enablePrivateNodes: True — sin IP externa",
     "gcloud container node-pools list"),
    ("Cloud Routers", "NINGUNO ⇒ no hay Cloud NAT en el proyecto",
     "gcloud compute routers list"),
    ("cuota IN_USE_ADDRESSES", "1 en uso de 4, en europe-west1",
     "gcloud compute regions describe"),
    ("el Job de catalogo", "corrio en gke-ore-mesh-jobs-p-… y salio con 69",
     "kubectl get pods -o custom-columns=NODO"),
]
for que, dice, orden in HECHOS:
    print("  %-30s %s" % (que, dice))
    print("  %-30s   ← %s" % ("", orden))

print("""
  ⇒ BigQuery funciona y Neon no, y ahora se ve por que: Private Google Access
    deja que una VM SIN IP publica alcance **las APIs de Google**, y nada mas.
    Un Postgres en AWS no es una API de Google.""")

# ══════════════════════════════════════════════════════════════════════════
titulo("② ⛔ POR QUE EL POOL ES PRIVADO — y no es lo que yo dije")
# ══════════════════════════════════════════════════════════════════════════
#
# Escribi que anadir NAT «revierte una propiedad que este arbol construyo a
# proposito», dando por hecho que el pool se hizo privado para confinar a los
# lectores de origenes. `malla/README.md` dice otra cosa, y lo dice entero.
readme = leer("malla/README.md")
citas = [
    ("quitó un techo que nadie había contado",
     "el motivo REAL: cada nodo con IP publica gasta una direccion de "
     "`IN_USE_ADDRESSES`, que esta en 4. Un nodo de sistema mas tres de jobs eran "
     "exactamente cuatro, y el cluster no podia crecer «aunque hubiera 32 vCPU libres»"),
    ("salir a algo que",
     "y este dia estaba PREVISTO, con su ejemplo: «un Postgres de un cliente»"),
]
for patron, dice in citas:
    ok = bool(re.search(patron, readme))
    print("  %s %s" % ("✓" if ok else "✗", dice))
    if not ok:
        fallos.append("`malla/README.md` ya no dice lo que esta medida cita: %s" % patron)

print("""
  ⇒ El pool es privado POR CUOTA, no por confinamiento. Que ademas confinara era
    un efecto lateral —real, y del que nos aprovechamos— pero no el motivo.

  ⭐ Y eso cambia la decision entera: Cloud NAT da salida SIN devolverle una IP
    publica a cada nodo, asi que **conserva el motivo por el que el pool es
    privado** y quita el efecto lateral. No deshace lo que se construyo.""")

# ══════════════════════════════════════════════════════════════════════════
titulo("③ QUE PROTEGE HOY, Y QUE SEGUIRIA PROTEGIENDO")
# ══════════════════════════════════════════════════════════════════════════
#
# Lo que queda en pie despues de NAT no es una promesa: son ficheros.
driver = leer("malla/20-driver.yaml")
puertos = re.findall(r"port:\s*(\d+)\s*\}", driver)
privadas = re.findall(r"-\s*(10\.0\.0\.0/8|172\.16\.0\.0/12|192\.168\.0\.0/16|169\.254\.0\.0/16)", driver)
print("  `salida-del-driver` deja salir por: %s" % ", ".join(sorted(set(puertos))))
print("  y NUNCA hacia: %s" % ", ".join(sorted(set(privadas))))
if "5432" not in puertos or "443" not in puertos:
    fallos.append("`salida-del-driver` ya no abre 443 y 5432: la medida habla de otro fichero")
if len(set(privadas)) < 4:
    fallos.append("`salida-del-driver` ya no excluye las cuatro redes privadas")

print("""
  ⭐ Cloud NAT es SOLO DE SALIDA. No abre ni un puerto entrante: nadie de fuera
    alcanza un nodo de `jobs-p` porque exista un NAT. Lo que cambia es que lo de
    dentro pueda iniciar una conexion, y eso ya lo filtra la politica de arriba.

  ⛔ Lo que SI se pierde, dicho sin adornar: hoy un driver comprometido no puede
    sacar datos a internet porque NO HAY RUTA. Con NAT podria, dentro de esos dos
    puertos y sin tocar redes privadas. Deja de ser imposible y pasa a ser
    filtrado — y filtrado por una regla que alguien puede editar.

  ⚠️ Y no hay forma de tenerlo y no tenerlo: **los datos del cliente viven en la
    nube**. Un lector de origenes que no puede salir a internet no es seguro:
    es inutil. Lo honesto es decir que se cambia una imposibilidad por un
    control, no fingir que no se cambia nada.""")

# ══════════════════════════════════════════════════════════════════════════
titulo("④ LO QUE (c) AÑADE SOBRE (a): LAS IPs FIJAS")
# ══════════════════════════════════════════════════════════════════════════
print("""  Cloud NAT sin mas reparte direcciones EFIMERAS: cambian, y del otro lado no se
  pueden poner en ninguna lista. Con direcciones reservadas:

    · Neon —y cualquier cliente— puede permitir SOLO esas IPs. Eso convierte
      «nuestro cluster puede conectarse» en «solo nuestro cluster puede», que es
      el control que un cliente pide antes de dar acceso a su base;
    · y el trafico de salida queda identificable desde fuera. Un abuso se ve y
      se corta en el destino, no solo aqui.

  ⚠️ Y cuesta cuota: una direccion reservada gasta de `IN_USE_ADDRESSES`, que
    esta en 1 de 4 — la MISMA cuota que hizo privado al pool. Con una IP de NAT
    quedan 2 libres.

  ⭐ Pero la cuenta sale bien y es la clave: una IP de NAT sirve a LOS TRES
    nodos de `jobs-p`. Antes, tres nodos publicos gastaban tres. Se recupera la
    salida gastando un tercio de lo que costaba tenerla.

  ⛔ Y una IP fija NO es autenticacion. Quien la vea desde fuera sabe de donde
    viene el trafico; no sabe QUE Job lo mando ni por cuenta de que inquilino.
    Eso lo dice la huella del custodio, y son dos preguntas distintas.""")

# ══════════════════════════════════════════════════════════════════════════
titulo("⑤ EL DINERO, Y LO QUE NO SE HA MEDIDO")
# ══════════════════════════════════════════════════════════════════════════
print("""  ⚠️ NO medido contra la Billing API, y por eso va aparte y con el aviso: Cloud
    NAT cobra por pasarela y hora MAS por GB procesado, y una IP reservada cobra
    solo si esta SIN USAR. Las tres cifras hay que sacarlas de la API antes de
    afirmarlas — este arbol ya rechazo una vez una cuenta escrita de memoria.

  ⭐ Lo que si se puede decir sin medir: el pool escala a cero, asi que la
    pasarela existe siempre pero el trafico solo ocurre cuando corre un Job de
    catalogo. El coste dominante sera el fijo de la pasarela, no los GB.""")

# ══════════════════════════════════════════════════════════════════════════
titulo("⑥ LO QUE HAY QUE CREAR, EN ORDEN")
# ══════════════════════════════════════════════════════════════════════════
print("""  ① una direccion reservada en europe-west1
  ② un Cloud Router en la VPC `ore-mesh`
  ③ un Cloud NAT en ese router, ACOTADO a la subred `10.10.0.0/20`, con la IP
     de ① y `--nat-custom-subnet-ip-ranges` en vez de «todas»

  ⭐ Y no hace falta acotarlo al pool: Cloud NAT solo lo usan las instancias SIN
    IP externa, y `default-pool` tiene la suya. Sale por NAT `jobs-p` y nadie
    mas, sin una regla que lo diga.

  ⚠️ Lo que eso arrastra y hay que escribir en su sitio: un nodo privado FUTURO
    en esa subred heredaria la salida sin que nadie lo decida. Hoy hay dos pools
    y se ve; con seis no. Va en el README, al lado de las tres reglas que no hay
    que deshacer.

  ⛔ Y esto NO es GitOps: son recursos de red del proyecto, como el papel de IAM
    del aprovisionador. Se crean con `gcloud` y se documentan — igual que
    `papel-del-aprovisionador.yaml`, que esta en `FUERA` con su motivo escrito.""")

print("\n" + "═" * 74)
if fallos:
    print("⛔ LA MEDIDA NO SE SOSTIENE:")
    for f in fallos:
        print("   · " + f)
    sys.exit(1)
print("✓ medida coherente: NAT conserva el motivo del pool privado y quita su efecto lateral")
