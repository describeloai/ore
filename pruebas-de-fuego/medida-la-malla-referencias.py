# -*- coding: utf-8 -*-
"""La malla, nutrida de lo publicado — y traducida a nuestra escala.

No se disena una malla desde cero cuando hay gente que lleva nueve anos
haciendolo y lo ha escrito. Esto recoge SEIS referencias publicadas, lo que
cada una aporta, y —la parte que importa— **que se traduce a un proyecto con
32 vCPU y que no**.

  A. LAS SEIS REFERENCIAS   quien, y que problema resolvio
  B. LO QUE SE TRADUCE      con el argumento, no por imitacion
  C. LO QUE NO SE TRADUCE   y por que copiarlo seria cargo cult
  D. NUESTRA VENTAJA        lo que `ore` tiene y sus servicios no
  E. EL TECHO DE HOY        lo que la cuota decide por nosotros
"""
import textwrap


def parrafo(t, sangria="     ", ancho=72):
    for l in textwrap.wrap(t, ancho):
        print("%s%s" % (sangria, l))


print("== la malla, medida contra lo publicado ==")

# -- A -------------------------------------------------------------------------
print()
print("A - LAS SEIS REFERENCIAS")
print()
REF = [
    ("Palantir · Rubix",
     "palantir.com/docs/foundry/architecture-center/rubix",
     "NINGUN NODO VIVE MAS DE 48 HORAS. Compute efimero e inmutable, con un "
     "«pipeline multidimensional de drenaje y terminacion con seleccion de "
     "nodo dirigida por politica». La efimeridad no es higiene: OBLIGA a que "
     "todo servicio tolere morir, y eso se comprueba solo"),
    ("Palantir · Cilium sobre Rubix",
     "blog.palantir.com/hardening-palantirs-kubernetes-infrastructure-with-cilium",
     "red segura POR DEFECTO, y todos los recursos de Kubernetes etiquetados "
     "por su ROL y su FUNCION. La politica se escribe contra la etiqueta, no "
     "contra la IP ni el nombre"),
    ("Palantir · Apollo",
     "blog.palantir.com/palantir-apollo-orchestration-constraint-based-continuous-deployment",
     "despliegue continuo POR RESTRICCIONES: las restricciones se codifican "
     "DENTRO del software y se validan antes de aplicar un cambio. Es el patron "
     "operador de Kubernetes mas una cosa — reconocer las interdependencias del "
     "sistema distribuido. Y «Compute Modules»: traer tu propio contenedor a la "
     "malla gestionada"),
    ("Google · GKE enterprise multi-tenancy",
     "cloud.google.com/kubernetes-engine/docs/best-practices/enterprise-multitenancy",
     "un namespace por tenant (~10.000 de techo practico), deny-all de entrada "
     "y abrir despues, `ResourceQuota` por namespace, Workload Identity "
     "Federation para no gestionar claves, Policy Controller en admision, y "
     "reparto de coste por namespace y etiqueta hacia BigQuery"),
    ("Kubernetes SIG · Kueue",
     "kueue.sigs.k8s.io",
     "cola de Jobs con CUOTA y jerarquia de reparto justo. Decide cuando un Job "
     "espera y cuando arranca, junto al autoscaler. Funciona sobre Autopilot y "
     "sobre Standard con node auto-provisioning"),
    ("Loft · vCluster",
     "vcluster.com",
     "un plano de control VIRTUAL por tenant, invisible para el tenant, "
     "corriendo como carga dentro de un namespace del cluster real. Mas barato "
     "que un cluster por tenant y mas aislado que un namespace"),
]
for q, url, que in REF:
    print("   · %s" % q)
    print("     %s" % url)
    parrafo(que, "       ")
    print()

# -- B -------------------------------------------------------------------------
print()
print("B - LO QUE SE TRADUCE, Y CON QUE ARGUMENTO")
print()
SI = [
    ("nodos efimeros con ciclo forzado", "Rubix",
     "y a nuestra escala es MAS facil, no menos: `ore` no es un servicio, es un "
     "proceso que lee un arbol y contesta. Un nodo que muere a mitad de un job "
     "es un job que se reintenta — no hay estado que drenar. Lo que a Palantir "
     "le costo rearquitecturar sus servicios, aqui viene de fabrica"),
    ("politica de red contra ETIQUETAS", "Rubix · Cilium",
     "porque nuestra frontera ya esta dibujada: `ore-core` y `ore-view` no "
     "abren una conexion y los drivers si. Son dos clases de carga con dos "
     "politicas, y la etiqueta lo dice — el que planifica NO necesita salida a "
     "internet, el que lee un origen si"),
    ("Kueue desde el dia uno", "SIG Batch",
     "y con 32 vCPU la cola importa MAS, no menos: sin ella, tres `discover` "
     "concurrentes contra un BigQuery grande saturan el cluster entero y el "
     "cuarto usuario ve un timeout en vez de una espera"),
    ("Workload Identity Federation", "GKE",
     "cero claves en el cluster. Encaja con lo que el sustrato ya hace: la URL "
     "del origen viaja por stdin y nunca por argv ni por el documento"),
    ("`ResourceQuota` y coste por etiqueta", "GKE",
     "porque «cuanto cuesta este tenant» tiene que ser contestable ANTES de "
     "que haya tenants, no despues"),
    ("el tenant es una ETIQUETA en todo, desde el principio", "vCluster",
     "aunque hoy sea un namespace. Es lo que permite migrar a plano de control "
     "virtual sin reescribir: si nada asume «un solo arbol», cambiar el "
     "mecanismo de aislamiento es una decision de operacion y no de producto"),
]
for q, de, por in SI:
    print("   · %-42s [%s]" % (q, de))
    parrafo(por, "       ")
    print()

# -- C -------------------------------------------------------------------------
print()
print("C - LO QUE NO SE TRADUCE, Y COPIARLO SERIA CARGO CULT")
print()
NO = [
    ("multi-nube desde el principio", "Rubix corre en AWS, Azure, GCP, Oracle "
     "y on-prem con las mismas caracteristicas operativas. Eso es una exigencia "
     "de SUS clientes, y pagarla ahora seria abstraer sobre una sola nube — el "
     "coste de la abstraccion sin ninguno de sus beneficios"),
    ("vCluster hoy", "es la respuesta correcta para el aislamiento y con 32 "
     "vCPU no cabe: cada plano de control virtual consume. Se deja PREPARADO "
     "por la via de la etiqueta, y se enciende cuando haya con que"),
    ("GKE Sandbox / gVisor", "es para codigo NO CONFIABLE de usuario. Hoy la "
     "unica carga es `ore`, que escribimos nosotros. El dia que la plataforma "
     "ejecute una funcion de un cliente —el plano KINETICO— pasa a ser "
     "obligatorio, y ese dia esta escrito en el mapa"),
    ("un cluster por entorno", "GKE recomienda un proyecto de administracion "
     "por cluster y VPC compartida con proyecto anfitrion por entorno. Es "
     "correcto y es de una organizacion con equipos separados. Con un proyecto "
     "y una persona, la separacion por namespace dice lo mismo y cuesta cero"),
]
for q, por in NO:
    print("   · %s" % q)
    parrafo(por, "       ")
    print()

# -- D -------------------------------------------------------------------------
print()
print("D - LO QUE NOSOTROS TENEMOS Y SUS SERVICIOS NO")
print()
parrafo("Apollo codifica las restricciones DENTRO del software y las valida "
        "antes de aplicar un cambio. Es exactamente la figura que este arbol "
        "ya tiene — y sobre otro sujeto:")
print()
print("   %-22s %-26s %s" % ("", "Apollo", "ORE"))
print("   " + "-" * 74)
for a, b, c in [
    ("sujeto", "el despliegue", "el SIGNIFICADO"),
    ("la restriccion", "en que orden se actualiza", "que revela una agregacion"),
    ("cuando se valida", "antes de aplicar", "al COMPILAR, sin abrir nada"),
    ("que dice al romper", "no despliegues todavia", "que eje rompe y que bump exige"),
]:
    print("   %-22s %-26s %s" % (a, b, c))
print()
parrafo("No compiten: se componen. Un despliegue por restricciones que ademas "
        "sepa que el cambio de una vista es CONSUMER breaking es una cosa que "
        "hoy no existe — y las dos mitades estan escritas, una por ellos y otra "
        "por nosotros.")
print()
parrafo("Y «Compute Modules» —traer tu contenedor a la malla gestionada— es la "
        "forma que `ore` YA tiene: 12 de 14 crates son hermeticos, el binario "
        "contesta desde el arbol de ficheros y lo que toca la red vive en "
        "subprocesos con la URL por stdin. Es un Job, no un servicio, y esa es "
        "la unica propiedad que una malla de compute efimero de verdad exige.")

# -- E -------------------------------------------------------------------------
print()
print("E - EL TECHO DE HOY, QUE DECIDE MAS QUE EL GUSTO")
print()
print("   %-22s %-10s %s" % ("cuota", "hoy", "que impide"))
print("   " + "-" * 74)
TECHO = [
    ("PREEMPTIBLE_CPUS", "0", "el pool Spot — hasta 91% de descuento, 30 s de aviso"),
    ("IN_USE_ADDRESSES", "4", "un Gateway + Cloud NAT y no queda margen"),
    ("CPUS / N2_CPUS", "32", "regional a 3 zonas deja ~10 vCPU utiles por zona"),
    ("SSD_TOTAL_GB", "250", "poco para discos de nodo generosos"),
    ("NVIDIA_L4 / T4", "0", "nada de inferencia — plazo de semanas al pedirlo"),
]
for q, v, i in TECHO:
    print("   %-22s %-10s %s" % (q, v, i))
print()
parrafo("Las dos primeras son las que cambian el DISENO y no solo el tamano. "
        "Sin Spot, la malla no tiene su clase de nodo barata y desechable, que "
        "es justo la que un Job de `ore` quiere; y con cuatro IPs no hay "
        "arquitectura de red que crecer. Pedirlas no es optimizar: es lo que "
        "separa un cluster de juguete de un sustrato que escala.")
