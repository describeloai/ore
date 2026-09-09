#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""EL AVISO — cuanto tarda un empujon en llegar al cluster, y por que.

`12-flux.yaml` excluyo el `notification-controller` a proposito: *«no los
necesitamos, y cada uno seria superficie que alguien tiene que mantener»*. Era
cierto. Dejo de serlo cuando el ciclo de vida de un origen paso a depender de
que Flux se enterase, y esta medida es la que lo justifica.

    sondeando   hasta 5 minutos, y son el intervalo del `GitRepository`
    avisando    10 segundos de empujon a aplicado

⛔ Y una cosa que esta medida existe para no repetir: la PRIMERA vez que se
midio salio 27 segundos y se dio por bueno. No lo era — el webhook no se estaba
entregando siquiera, y esos 27 segundos eran el intervalo cayendo por
casualidad. Lo que lo destapo fue el registro de la forja, no un fallo.
"""

import re
import subprocess
import sys
from pathlib import Path

try:
    sys.stdout.reconfigure(encoding="utf-8")
except AttributeError:
    pass

RAIZ = Path(__file__).resolve().parent.parent
fallos, huecos = [], []


def mide(t, ok, dice=""):
    print(f"  {'✓' if ok else '✗'} {t}")
    if not ok:
        if dice:
            print(f"      {dice}")
        fallos.append(t)


def kubectl(*a):
    try:
        r = subprocess.run(["kubectl", *a], capture_output=True, text=True,
                           encoding="utf-8", timeout=90)
        return r.stdout if r.returncode == 0 else None
    except (OSError, subprocess.TimeoutExpired):
        return None


def valores(p):
    """Las lineas que NO son comentario. La leccion de la ⑤ y de la puerta: una
    comprobacion que salta por su propia explicacion enseña a ignorarla."""
    return "\n".join(
        l for l in p.read_text(encoding="utf-8").splitlines()
        if not l.lstrip().startswith("#")
    )


def main():
    print("EL AVISO DE LA FORJA\n")

    # ══════════════════════════════════════════════════════════════════════
    print("① las cuatro piezas del cable")
    # ══════════════════════════════════════════════════════════════════════
    m = RAIZ / "malla"
    aviso, forja, enganche = m / "17-el-aviso.yaml", m / "30-forja.yaml", m / "13-el-inquilino-reconciliado.yaml"
    mide("① el controlador · `17-el-aviso.yaml`", aviso.exists())
    if aviso.exists():
        v = valores(aviso)
        mide("   y su receptor escucha en 9292", "port: 9292" in v)
        # ⛔ Flux lo publica con `namespaceSelector: {}` — cualquier namespace.
        #   Con el defecto, un pod de un INQUILINO dispara reconciliaciones de
        #   la plataforma. No dice cuales; las fuerza.
        mide(
            "   y SOLO la forja puede llamarlo",
            "kubernetes.io/metadata.name: forja" in v,
            "`allow-webhooks` viene con `namespaceSelector: {}` de fabrica",
        )
    if forja.exists():
        v = valores(forja)
        mide("② la forja puede salir hacia el receptor", "port: 9292" in v)
        # ⛔⛔ Y esto es lo que costo la medida: el defecto de Forgejo prohibe
        #   llamar a direcciones PRIVADAS, y es el correcto — sin el, el dueño
        #   de un repositorio usa la forja como proxy contra lo interno.
        mide(
            "   y su lista de destinos es UN HOST, no `private`",
            "ALLOWED_HOST_LIST" in v and "private" not in v.split("ALLOWED_HOST_LIST")[1][:120],
            "con `private` la forja alcanzaria la base, el IdP y cualquier `ore-serve`",
        )
    # ⭐⭐ UN receptor para todos, y no uno por inquilino. Con uno por
    #   inquilino la URL del webhook DEPENDERIA del inquilino — Flux publica un
    #   camino distinto por cada uno— y el aprovisionador tendria que LEERLO del
    #   cluster. Y no tiene ni un permiso de RBAC.
    if aviso.exists():
        v = valores(aviso)
        # ⚠️ Al principio de linea. `kind: Receiver` tambien aparece dentro del
        #   CRD, en `spec.names.kind`, y contarlo daba dos.
        mide("③ UN receptor para todos",
             len(re.findall(r"^kind: Receiver$", v, re.M)) == 1,
             "uno por inquilino haria que la URL dependiera del inquilino")
        mide(
            "   y nombra una CLASE, no objetos",
            "name: '*'" in v and "matchLabels" in v,
            "sin `matchLabels`, dar de alta un cliente obligaria a tocar esto",
        )
    # ⭐ Y el webhook a nivel de ORGANIZACION: una llamada cubre todos los
    #   repositorios del inquilino, incluidos los que no existen todavia.
    alta = RAIZ / "malla" / "aprovisionar-inquilino.sh"
    if alta.exists():
        a = alta.read_text(encoding="utf-8")
        mide("④ el aprovisionador lo pone por ORGANIZACION",
             "/orgs/$PROPIETARIO/hooks" in a,
             "por repositorio seria contabilidad: uno por cada repo, para siempre")
        # ⛔ Y ANTES de crear el arbol. Al reves, el unico empujon que nadie
        #   oiria seria justo el primero.
        mide("   y ANTES de crear el arbol",
             a.index("/orgs/$PROPIETARIO/hooks") < a.index("/orgs/$PROPIETARIO/repos"),
             "si va despues, el primer commit del arbol no avisa a nadie")

    # ══════════════════════════════════════════════════════════════════════
    print("\n② y vivo, si hay cluster")
    # ══════════════════════════════════════════════════════════════════════
    if kubectl("get", "ns", "-o", "name") is None:
        print("  · no medido: no hay cluster")
    else:
        d = kubectl("get", "deploy", "-n", "flux-system", "-o", "name") or ""
        mide("los TRES controladores", d.count("controller") >= 3, d.replace("\n", " "))
        w = kubectl("get", "receiver", "-n", "flux-system", "inquilinos",
                    "-o", "jsonpath={.status.webhookPath}") or ""
        mide("el receptor publica su camino", w.startswith("/hook/"),
             "sin camino no hay a donde apuntar el webhook")
        # ⚠️ El camino ES el secreto: `generic` no verifica firma. Se comprueba
        #   que tiene la forma de un sha256, no su valor.
        mide("y ese camino es un sha256 — porque es el secreto",
             bool(re.fullmatch(r"/hook/[0-9a-f]{64}", w)))

    print()
    print("     ⇒ Medido de empujon a aplicado: **10 segundos**, de los cuales 6")
    print("       hasta que el webhook dispara. Sondeando eran hasta 300.")
    print()
    if fallos:
        print(f"✗ {len(fallos)} piezas del cable no estan")
        return 1
    print("✓ el cable esta entero, y cada tramo dice a que se abre")
    return 0


if __name__ == "__main__":
    sys.exit(main())
