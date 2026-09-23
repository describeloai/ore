"""0037 ② · LA TUBERÍA, MEDIDA ANTES DE CAVARLA.

El tramo ① encendió lo que ya estaba dentro del navegador (json y typescript).
Los otros tres lenguajes de la casa —python, java, sql— necesitan un servidor
de verdad al otro lado, y entre el editor y ese servidor hace falta un canal.
Antes de cavarlo se mide QUÉ PIDE un servicio de lenguaje, QUÉ AGUANTA lo que
hay, y POR DÓNDE puede entrar.

  §1  EL CANAL DE HOY          cómo habla la consola con el puesto, con las
                               constantes leídas del código, no de memoria.
  §2  LO QUE PIDE UN SERVICIO  cuántos mensajes y cuántos bytes mueve un
                               servicio de lenguaje mientras se teclea, medido
                               en el navegador con el de Monaco (el mismo
                               motor de TypeScript que usa VS Code).
  §3  POR DÓNDE ENTRA          cómo llega hoy la consola a `ore-serve`, y qué
                               permite y qué no permite ese camino.
  §4  LA PUERTA                qué balanceador hay delante y cuánto deja vivir
                               a una conexión.
  §5  LO QUE FALTARÍA          para un websocket, pieza por pieza.

    python pruebas-de-fuego/medida-la-tuberia.py
"""
import os
import re
import sys

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
AQUI = os.path.dirname(os.path.abspath(__file__))
RAIZ = os.path.dirname(AQUI)
CONSOLA = "C:/rubix-platform"

# ─────────────────────────────────────────────────────────────────────────────
# Lo medido en el navegador el 2026-09-23, con Monaco 0.55.1 y el servicio de
# TypeScript (el motor de VS Code), envolviendo `Worker.postMessage` y el
# `message` de vuelta para CONTARLOS. Fichero de 122 líneas / 3849 bytes, 32
# caracteres tecleados uno cada 60 ms. No se mide aquí porque haría falta un
# navegador; se anota de dónde sale, que es lo que permite discutirlo.
#
#   sonda: crear el modelo, teclear carácter a carácter, contar mensajes y bytes
# ─────────────────────────────────────────────────────────────────────────────
TECLEO = {
    "lineas": 122, "bytes_fichero": 3849,
    "caracteres": 32, "segundos": 3.5,
    "mensajes": 296, "bytes": 62216,
}
AUTOCOMPLETADO = {"mensajes": 8, "bytes": 84125, "ms": 2508}


def titulo(t):
    print("\n" + t)
    print("  " + "─" * (len(t) - 2))


def leer(*partes):
    return open(os.path.join(*partes), encoding="utf-8", errors="replace").read()


def constante(texto, nombre):
    m = re.search(nombre + r"[^=]*=\s*Duration::from_secs\((\d+)\)", texto)
    if m:
        return int(m.group(1))
    m = re.search(nombre + r"[^=]*=\s*(\d+)", texto)
    return int(m.group(1)) if m else None


def seccion_canal_de_hoy():
    titulo("  §1 EL CANAL DE HOY (leído del código, no de memoria)")
    http = leer(RAIZ, "crates", "ore-entrada", "src", "http.rs")
    puestos = leer(RAIZ, "crates", "ore-serve", "src", "puestos.rs")
    agente = leer(RAIZ, "puesto", "python", "agente.py")
    conexiones = constante(http, "const CONEXIONES: usize")
    plazo = constante(http, "const PLAZO")
    espera = constante(puestos, "const ESPERA")
    latido = constante(puestos, "const SIN_LATIDO")
    m = re.search(r"plazo=(\d+)", agente)
    print("     ore-entrada/http.rs  · HTTP/1.1 escrito a mano sobre `std::net`")
    print("       conexiones a la vez: %s   ← CADA UNA ES UN HILO; la que sobra recibe 503" % conexiones)
    print("       plazo de lectura y de escritura: %s s" % plazo)
    sin_keepalive = re.search(r"no hay\s*(?://!\s*)?`keep-alive`", http) is not None
    print("       keep-alive: %s" % ("NO — una conexión atiende UNA petición y se cierra"
                                     if sin_keepalive else "sí"))
    print("     ore-serve/puestos.rs · la espera larga")
    print("       el servidor retiene la respuesta hasta %s s (por debajo del plazo)" % espera)
    print("       el agente pregunta con plazo de cliente de %s s y reintenta a los 5 s"
          % (m.group(1) if m else "?"))
    print("       sin latido durante %s s, el puesto está perdido" % latido)
    print("     ⇒ mientras espera, la conexión del agente OCUPA UNA de las %s." % conexiones)
    print("       Un puesto vivo = una plaza tomada para siempre en su inquilino.")


def seccion_lo_que_pide():
    titulo("  §2 LO QUE PIDE UN SERVICIO DE LENGUAJE (medido en el navegador)")
    t = TECLEO
    por_seg = t["mensajes"] / t["segundos"]
    por_pulsacion = t["mensajes"] / t["caracteres"]
    print("     fichero de %d líneas · %d bytes; %d caracteres tecleados en %.1f s"
          % (t["lineas"], t["bytes_fichero"], t["caracteres"], t["segundos"]))
    print("       mensajes con el servicio: %d  → %.1f POR SEGUNDO, %.1f por pulsación"
          % (t["mensajes"], por_seg, por_pulsacion))
    print("       bytes: %d  (%d por mensaje — son mensajes PEQUEÑOS y MUCHOS)"
          % (t["bytes"], t["bytes"] / t["mensajes"]))
    a = AUTOCOMPLETADO
    print("     un solo autocompletado: %d mensajes · %d bytes · %d ms hasta que calla"
          % (a["mensajes"], a["bytes"], a["ms"]))
    http = leer(RAIZ, "crates", "ore-entrada", "src", "http.rs")
    conexiones = constante(http, "const CONEXIONES: usize")
    print("")
    print("     ⇒ CONTRA EL CANAL DE HOY: una petición por mensaje y sin keep-alive")
    print("       son %d conexiones nuevas en %.1f s, contra un servidor que admite %d"
          % (t["mensajes"], t["segundos"], conexiones))
    print("       a la vez y contesta 503 a la que sobra. Escribir UNA LÍNEA agotaría")
    print("       el presupuesto de conexiones del inquilino entero.")
    print("     ⇒ Y no es el ancho de banda: %d bytes en %.1f s son 18 kB/s. Es el"
          % (t["bytes"], t["segundos"]))
    print("       COSTE DE ABRIR: sin conexión persistente, cada mensaje paga un")
    print("       saludo de TCP (y de TLS, cruzando la puerta) para mover 210 bytes.")


def seccion_por_donde_entra():
    titulo("  §3 POR DÓNDE ENTRA (el camino de la consola a `ore-serve`)")
    sesion = leer(CONSOLA, "lib", "auth", "sesion.ts")
    http_only = "httpOnly: true" in sesion
    acciones = 0
    for raiz, _, fs in os.walk(os.path.join(CONSOLA, "app")):
        for f in fs:
            if f.endswith((".ts", ".tsx")) and "'use server'" in leer(raiz, f):
                acciones += 1
    propio = any(os.path.exists(os.path.join(CONSOLA, n)) for n in ("server.ts", "server.js", "server.mjs"))
    paquete = leer(CONSOLA, "package.json")
    ver = re.search(r'"next":\s*"([^"]+)"', paquete)
    print("     la credencial de la persona vive en una COOKIE FIRMADA y `httpOnly`: %s"
          % ("sí" if http_only else "no"))
    print("       (y está escrito por qué: «el JS de la página no la lee — ni el nuestro")
    print("        ni el que entre por un script»)")
    print("     ficheros con `'use server'`: %d — el navegador NO llama a `ore-serve`," % acciones)
    print("       llama a la consola, y la consola pone el testigo del lado del servidor")
    print("     Next %s, servidor propio: %s" % (ver.group(1) if ver else "?", "sí" if propio else "NO (`next start`)"))
    print("")
    print("     ⇒ UN WEBSOCKET DEL NAVEGADOR A `ore-serve` pediría poner el testigo en")
    print("       JavaScript, que es exactamente la decisión que la consola ya tomó al")
    print("       revés. Y uno contra la propia consola no se puede: el App Router")
    print("       entrega una `Request` web, sin socket que ascender, y aquí no hay")
    print("       servidor propio donde interceptarlo.")
    print("     ⇒ Lo que ese mismo camino SÍ permite, con la cookie y sin tocar nada:")
    print("       una RESPUESTA QUE NO TERMINA (streaming) desde un route handler.")


def seccion_la_puerta():
    titulo("  §4 LA PUERTA (cuánto deja vivir una conexión)")
    puerta = leer(RAIZ, "malla", "14-la-puerta.yaml")
    entrada = leer(RAIZ, "malla", "43-la-entrada.yaml")
    clase = re.search(r"gatewayClassName:\s*(\S+)", puerta)
    print("     clase de la puerta: %s" % (clase.group(1) if clase else "?"))
    print("     política de backend en la entrada del inquilino: %s"
          % ("hay GCPBackendPolicy" if "GCPBackendPolicy" in entrada else "NINGUNA → el valor por defecto"))
    print("     ⇒ En un balanceador de aplicación de Google, el plazo del backend es,")
    print("       para una conexión ascendida o en streaming, LO QUE VIVE LA CONEXIÓN,")
    print("       no lo que tarda una respuesta. Por defecto son 30 s: sin escribir esa")
    print("       política, el canal se cae cada medio minuto — y eso vale igual para")
    print("       un websocket que para un streaming.")
    sitio = leer(RAIZ, "malla", "51-el-puesto.yaml")
    hay_servicio = "kind: Service" in sitio
    m = re.search(r'ORE_SERVE, value: "([^"]+)"', sitio)
    print("     el puesto: %s" % ("tiene Service" if hay_servicio else "NO tiene Service — no hay quien lo llame"))
    print("       el agente sale a %s" % (m.group(1) if m else "?"))
    print("     ⇒ El agente NO cruza la puerta: habla dentro del clúster. El plazo de")
    print("       30 s es un problema del lado del navegador, no del lado del puesto.")


def seccion_lo_que_faltaria():
    titulo("  §5 LO QUE FALTARÍA PARA UN WEBSOCKET, PIEZA POR PIEZA")
    cierre = leer(RAIZ, "Cargo.lock")
    hay_sha1 = 'name = "sha1"' in cierre
    http = leer(RAIZ, "crates", "ore-entrada", "src", "http.rs")
    py = leer(RAIZ, "puesto", "python", "agente.py")
    node = leer(RAIZ, "puesto", "node", "agente.mjs")
    print("     el saludo pide SHA-1 y base64 sobre `Sec-WebSocket-Key`:")
    print("       sha1 en el cierre de dependencias: %s" % ("sí" if hay_sha1 else "NO ESTÁ"))
    print("     el servidor tendría que dejar de cerrar tras una petición:")
    print("       hoy `atender()` lee UNA y responde UNA — %s"
          % ("y lo dice en su cabecera: «cada conexión atiende una petición y se cierra»"
             if "atiende **una** petición" in http else "?"))
    print("     y tendría que saber trocear marcos, con la máscara del cliente.")
    print("     los agentes son de biblioteca estándar y nada más:")
    print("       python: %s" % ("urllib, sin cliente de websocket" if "urllib.request" in py else "?"))
    print("       node:   %s" % ("node:24 trae `WebSocket` global (cliente), así que ESE lado saldría gratis"
                                 if "node:" in node else "?"))
    print("     ⇒ Un streaming pide MENOS: no hay saludo que firmar, no hay marcos ni")
    print("       máscara, y el sentido de vuelta (del editor al servidor) ya sabe")
    print("       viajar como lo que hoy viaja: una petición con su cuerpo.")


def main():
    print("═══ 0037 ② · la tubería, medida")
    seccion_canal_de_hoy()
    seccion_lo_que_pide()
    seccion_por_donde_entra()
    seccion_la_puerta()
    seccion_lo_que_faltaria()
    return 0


if __name__ == "__main__":
    sys.exit(main())
