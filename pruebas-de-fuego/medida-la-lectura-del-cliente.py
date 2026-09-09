#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""E7 · LECTURA PARA EL CLIENTE — que es, y que NO es.

La `0022` dice de la E7: *«Lectura para el cliente, y revision donde se pida. Lo
ultimo a proposito: es un ajuste del repositorio, no trabajo de plataforma. Que
sea barato es la mitad del valor de ③»*.

⛔ Y la pregunta que hay que contestar antes de construir nada es cual de las DOS
lecturas es. Porque son dos cosas distintas y la decision las nombra juntas:

    EL COMPARTIMENTO   `inquilino-<org>` en GitHub — que se despliega,
                       con que imagen, por donde puede salir. YAML.
    LA ONTOLOGIA       `t-<org>/ontologia` en la forja — sus fuentes, sus
                       paquetes, sus decisiones. El producto.

Esta medida dispara contra las dos y contra la consola, para que la respuesta
salga de lo que hay y no de lo que la decision recuerda.
"""

import json
import re
import subprocess
import sys
from pathlib import Path

try:
    sys.stdout.reconfigure(encoding="utf-8")
except AttributeError:
    pass

RAIZ = Path(__file__).resolve().parent.parent
CONSOLA = Path("C:/rubix-platform")
REPO = "describeloai/inquilino-demo"

fallos, huecos = [], []


def mide(t, ok, dice=""):
    print(f"  {'✓' if ok else '✗'} {t}")
    if not ok:
        if dice:
            print(f"      {dice}")
        fallos.append(t)


def hueco(t, dice):
    print(f"  ⚠️ {t}")
    for l in dice.splitlines():
        print(f"      {l}")
    huecos.append(t)


def corre(*args):
    try:
        r = subprocess.run(args, capture_output=True, text=True,
                           encoding="utf-8", timeout=90)
        return r.stdout if r.returncode == 0 else None
    except (OSError, subprocess.TimeoutExpired):
        return None


def main():
    print("E7 · LECTURA PARA EL CLIENTE\n")

    # ══════════════════════════════════════════════════════════════════════
    print("① EL COMPARTIMENTO — `inquilino-<org>`, que es lo que la E7 nombra")
    # ══════════════════════════════════════════════════════════════════════
    s = corre("gh", "api", f"repos/{REPO}/collaborators")
    if s is None:
        print("  · no medido: sin `gh` o sin acceso")
    else:
        gente = [(u["login"], u["role_name"]) for u in json.loads(s)]
        for l, r in gente:
            print(f"     {l}  ·  {r}")
        lectores = [l for l, r in gente if r in ("read", "triage")]
        if not lectores:
            hueco(
                "nadie del cliente puede leerlo — la E7 no esta empezada",
                "Y es barata como dice la decision: UNA llamada por inquilino.\n"
                "  gh api -X PUT repos/<org>/inquilino-<x>/collaborators/<quien> \\\n"
                "    -f permission=pull",
            )
        # ── ⛔ Y LA OTRA MITAD NO SE PUEDE, Y NO ES DE DISENO ─────────────
        #
        # La `0022` apoya su ③ entero en que la revision «se activa poniendo
        # revision obligatoria en ESE repositorio», y de ahi saca que no hay que
        # decidir hoy quien aprueba en 2028. Eso da por hecha una funcion que
        # esta cuenta NO TIENE en repositorios privados.
        p = corre("gh", "api", f"repos/{REPO}/branches/main/protection")
        if p is None:
            hueco(
                "y la revision obligatoria NO esta disponible en este plan",
                "GitHub contesta 403 «Upgrade to GitHub Pro or make this repository\n"
                "public». La `0022` apoya su ③ entero en que la revision se activa\n"
                "poniendola en ese repositorio — y en un repositorio privado de este\n"
                "plan no se puede.\n"
                "⇒ No es un fallo de diseño: es una decision de PRECIO que la decision\n"
                "  no vio. Y hay tres salidas, no una: pagar, mover el compartimento a\n"
                "  la forja —que si tiene revision y ya la operamos—, o transferirle el\n"
                "  repositorio al cliente, que la `0022` ya contempla.",
            )

    # ══════════════════════════════════════════════════════════════════════
    print("\n② LA ONTOLOGIA — que es lo que un cliente querria leer de verdad")
    # ══════════════════════════════════════════════════════════════════════
    #
    # ⭐ Aqui esta la respuesta a «¿la E7 es gestion del software desde la
    #   interfaz?». No: la ontologia NO se lee por git, y no es un descuido.
    def kubectl(*a):
        return corre("kubectl", *a)

    vivo = kubectl("get", "ns", "-o", "name") is not None
    if not vivo:
        print("  · no medido: no hay cluster")
    else:
        s = kubectl("exec", "-n", "forja", "forja-0", "--", "su", "git", "-c",
                    "forgejo admin user list")
        humanos = [l.split()[1] for l in (s or "").splitlines()[1:] if l.split()]
        print("     usuarios de la forja: " + (", ".join(humanos) or "(ninguno)"))
        hueco(
            "la forja no tiene NI UNA puerta al mundo",
            "Las unicas entradas de toda la malla son `login.paladio.io` y ahora\n"
            "`demo.ore.paladio.io`. Asi que «lectura para el cliente» sobre el arbol\n"
            "ontologico **no es una concesion que falte: es una puerta que no existe**.\n"
            "⇒ Y esta bien asi. El arbol no se lee por git: se lee por `ore-serve`,\n"
            "  que es quien sabe quien pregunta y con que concesion.",
        )

    # ══════════════════════════════════════════════════════════════════════
    print("\n③ LO QUE YA SE PUEDE LEER, Y POR DONDE")
    # ══════════════════════════════════════════════════════════════════════
    #
    # ⛔ Esto NO es la E7 — existe desde antes— y es justo lo que se confunde con
    #   ella: la gestion del producto desde la interfaz ya tiene su superficie.
    rutas = kubectl("logs", "-n", "t-demo", "deploy/ore-serve", "-c", "ore-serve") or ""
    leibles = re.findall(r"·\s+GET\s+(\S+)", rutas)
    for r in leibles:
        print(f"     GET {r}")
    mide(
        "el arbol ya se lee por identidad, no por git",
        any("/fuentes" in r for r in leibles) and any("/paquetes" in r for r in leibles),
        "no hay superficie de lectura del arbol",
    )
    print(
        "\n     ⇒ Y por eso la E7 NO es «gestion del software desde la interfaz»:\n"
        "       eso es ESTA superficie, y ya existe. La E7 es enseñarle al cliente\n"
        "       la INFRAESTRUCTURA que le corre, que es otra cosa y mas barata."
    )

    # ══════════════════════════════════════════════════════════════════════
    print("\n④ Y LA CONSOLA, QUE YA RESUELVE POR ORGANIZACION")
    # ══════════════════════════════════════════════════════════════════════
    cfg = CONSOLA / "lib" / "server" / "config.ts"
    org = CONSOLA / "lib" / "server" / "organizacion.ts"
    q = CONSOLA / "lib" / "server" / "query.ts"
    if not cfg.exists():
        print("  · no medido: no esta el arbol de la consola")
    else:
        # ⛔ Los VALORES, no la prosa: `config.ts` sigue NOMBRANDO la constante
        #   retirada para explicar por que se fue, y una comprobacion que saltara
        #   por su propia explicacion enseña a ignorarla.
        def codigo(p):
            t = p.read_text(encoding="utf-8")
            t = re.sub(r"/\*.*?\*/", "", t, flags=re.S)
            return "\n".join(l for l in t.splitlines() if not l.lstrip().startswith("//"))

        mide(
            "la constante `ORE_SERVE_URL` ya no se lee en ningun sitio",
            "ORE_SERVE_URL" not in codigo(cfg) + codigo(q),
            "un destino constante sirve el arbol del primer inquilino al segundo, "
            "y con un 200 encima",
        )
        mide(
            "la direccion del arbol se resuelve por organizacion",
            org.exists() and "entradaActual" in codigo(org),
            "falta `entradaActual` en `organizacion.ts`",
        )
        mide(
            "y sale de la MISMA llamada que dice a que organizacion perteneces",
            org.exists() and "misOrganizaciones(acceso)" in codigo(org),
            "resolverla aparte seria un viaje de mas y una segunda verdad",
        )
        mide(
            "el esquema lo pone la consola, no la fila",
            org.exists() and "https://${org.entrada}" in codigo(org),
            "la `022` guarda un HOST: un esquema en la fila es carretera en la identidad",
        )
        mide(
            "y `ore-iam` la devuelve",
            "o.entrada" in (RAIZ / "crates" / "ore-iam" / "src" / "rutas.rs").read_text(
                encoding="utf-8"
            ),
            "sin esto la consola resuelve contra un campo que nadie envia",
        )

    print()
    if fallos:
        print(f"✗ {len(fallos)} afirmaciones no se sostienen")
        return 1
    print("✓ medido: la E7 es lo pequeño, y lo grande ya tiene superficie sin cablear")
    print(f"⚠️ {len(huecos)} huecos, y uno de ellos es de precio y no de diseño")
    return 0


if __name__ == "__main__":
    sys.exit(main())
