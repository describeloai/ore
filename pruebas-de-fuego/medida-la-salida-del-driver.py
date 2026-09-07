# -*- coding: utf-8 -*-
"""Cloud NAT y la politica de salida del driver, medidos antes de escribirlos.

La regla de la malla es «se niega por defecto, tambien la salida», y el driver
que lee un origen es la EXCEPCION CON NOMBRE. Escribir esa excepcion parecia
una linea de YAML y la medida dice tres cosas que la cambian: que NAT hoy no
hace falta, que la excepcion no se puede escribir por NOMBRE, y que hay un
techo de cuatro nodos que nadie habia contado.

  A. LA SALIDA QUE YA HAY    y por que Cloud NAT no es el bloqueo
  B. LO QUE LA POLITICA PUEDE DECIR   y lo que no
  C. EL /30 QUE LO RESUELVE  y lo que compra de gobierno
  D. EL TECHO DE CUATRO      la cuota que decide el tamaño del cluster
  E. Y `kubectl logs` ROTO   un agujero abierto, dicho y no tapado
"""
import textwrap


def parrafo(t, sangria="     ", ancho=72):
    for l in textwrap.wrap(t, ancho):
        print("%s%s" % (sangria, l))


print("== la salida del driver, medida ==")

# -- A -------------------------------------------------------------------------
print()
print("A - LA SALIDA QUE YA HAY")
print()
print("   %-34s %s" % ("nodos con IP publica", "los dos"))
print("   %-34s %s" % ("cluster privado", "NO — `enablePrivateNodes` vacio"))
print("   %-34s %s" % ("Private Google Access en la subred", "si"))
print("   %-34s %s" % ("Cloud NAT", "ninguno"))
print()
parrafo("**Cloud NAT no hace falta hoy**, y decirlo ahorra construir una pieza "
        "que no resuelve nada: los nodos tienen IP publica, asi que la salida a "
        "internet ya existe. NAT sirve para el dia que los nodos sean PRIVADOS "
        "—una IP de salida estable, ninguna entrante— que es mejor postura y es "
        "otra decision.")
print()
parrafo("Asi que el bloqueo no es la red: es la POLITICA. `t-demo` niega toda "
        "salida salvo DNS, y esta medido de la forma mas tonta y mas "
        "convincente — una sonda que empezaba con `apk add curl` se quedo "
        "colgada hasta el timeout. Ni el gestor de paquetes sale.")

# -- B -------------------------------------------------------------------------
print()
print("B - LO QUE LA POLITICA PUEDE DECIR, Y LO QUE NO")
print()
print("   CRDs de red en el cluster:")
for q, hay in [("networkpolicies (networking.k8s.io/v1)", True),
               ("ciliumendpoints, ciliumidentities, ciliumnodes", True),
               ("ciliumnetworkpolicies", False)]:
    print("     %-3s %s" % ("si " if hay else "NO ", q))
print()
parrafo("Y ahi esta el hallazgo. GKE Dataplane V2 trae el DATAPATH de Cilium "
        "—`anetd` corriendo, `ADVANCED_DATAPATH` en el cluster— pero **no "
        "expone `CiliumNetworkPolicy`**. Sin ese CRD no hay reglas por FQDN.")
print()
parrafo("Con `NetworkPolicy` de Kubernetes a secas, una regla de salida se "
        "escribe contra un CIDR o contra un selector de pods. **No existe «deja "
        "salir a `bigquery.googleapis.com`»**: el nombre no es expresable. Y "
        "Google publica 145 prefijos para sus rangos, que ademas cambian.")
print()
parrafo("Esto contradice a medias lo que escribi en la malla —«la politica se "
        "escribe contra la ETIQUETA, no contra la IP»—. Es verdad para la "
        "ENTRADA, donde el selector de pods es la etiqueta. Para la SALIDA "
        "hacia fuera del cluster, hoy solo hay IP.")

# -- C -------------------------------------------------------------------------
print()
print("C - EL /30 QUE LO RESUELVE, Y LO QUE COMPRA")
print()
print("   %-28s %-20s %s" % ("", "rango", "que sirve"))
print("   " + "-" * 74)
print("   %-28s %-20s %s" % ("private.googleapis.com", "199.36.153.8/30", "todas las APIs de Google"))
print("   %-28s %-20s %s" % ("restricted.googleapis.com", "199.36.153.4/30", "solo las que soportan VPC-SC"))
print("   %-28s %-20s %s" % ("(alternativa)", "145 prefijos", "los rangos publicos, y cambian"))
print()
parrafo("Private Google Access ya esta encendido en la subred, asi que "
        "cualquier trafico a esos dos /30 sale por la red de Google sin pasar "
        "por internet. **La excepcion del driver son CUATRO DIRECCIONES**, no "
        "una lista que se pudre.")
print()
parrafo("Y `restricted` es la que yo elegiria, porque no es solo mas estrecha: "
        "sirve **unicamente las APIs que soportan Controles de Servicio de "
        "VPC**, que es el conjunto sobre el que se puede impedir la "
        "exfiltracion. Un driver que solo alcanza `restricted.googleapis.com` "
        "no puede escribir a un bucket de otro proyecto aunque alguien le meta "
        "las credenciales.")
print()
parrafo("Postgres es el otro caso y es mas facil: un `host:puerto` concreto, "
        "que en `NetworkPolicy` es un `/32` y un puerto. Lo interesante es de "
        "DONDE sale ese dato — de `datasources` del `OntologyConfig`, que es un "
        "documento del arbol. La politica de salida de un tenant es DERIVABLE "
        "de su ontologia, no algo que alguien escriba a mano en YAML de "
        "Kubernetes.")

# -- D -------------------------------------------------------------------------
print()
print("D - EL TECHO DE CUATRO, QUE NADIE HABIA CONTADO")
print()
print("   IN_USE_ADDRESSES   limite 4   en uso 2  (dos nodos, dos IPs)")
print()
parrafo("Cada nodo con IP publica consume una direccion de la cuota regional. "
        "Con el limite en 4: **un nodo de sistema mas tres de jobs son "
        "exactamente cuatro**, y el cluster no puede crecer mas aunque la cuota "
        "de CPU lo permita — hay 32 vCPU y solo caben cuatro nodos.")
print()
parrafo("La peticion de cuota de IPs ya esta presentada, y ahora se entiende "
        "que no era una comodidad. Y hay un remedio que no depende de nadie: "
        "**nodos privados mas Cloud NAT**. Sin IP publica no consumen la cuota, "
        "salen todos por la IP del NAT, y de paso desaparece la superficie "
        "entrante. Es la misma pieza que en (A) no hacia falta y aqui se gana "
        "sola.")

# -- E -------------------------------------------------------------------------
print()
print("E - Y UN AGUJERO QUE SALIO MIDIENDO, Y NO ESTA TAPADO")
print()
parrafo("`kubectl logs` sobre un pod del pool de jobs da `dial tcp "
        "10.10.0.x:10250: i/o timeout`. Funciono la primera vez —el `ore "
        "--version` de la imagen se leyo— y despues no. Se probo:")
print()
for q, r in [
    ("una regla del endpoint publico del master al nodo", "no arregla"),
    ("la regla `-vms` que ya existe, 10.10.0.0/20 -> nodos", "ya cubre 10250"),
    ("konnectivity-agent", "1 replica, Running, en el pool de sistema"),
    ("los dos nodos", "misma etiqueta de red, los dos con IP publica"),
]:
    print("   %-52s %s" % (q, r))
print()
parrafo("No esta resuelto y no lo tapo con una explicacion que no he "
        "comprobado. Lo que si se sabe: **es de la VPC custom** —la por defecto "
        "trae reglas automaticas que aqui hay que escribir— y no afecta a que "
        "los Jobs corran, solo a leerles la salida. Se saca por el mensaje de "
        "terminacion, que no pasa por el kubelet.")
print()
parrafo("Un cluster donde no se pueden leer los logs no es operable, asi que "
        "esto va antes que la politica de salida en cualquier orden sensato.")
