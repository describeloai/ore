#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""① · EL APROVISIONADOR SIN CREDENCIALES DE CLUSTER — y lo que hoy si tiene.

La cabecera de `aprovisionar-inquilino.sh` dice su decision en una frase:

    ⭐⭐ El aprovisionador NO tiene credenciales de cluster.

Esta medida comprueba si eso es verdad hoy, cuanto de lo que usa es EVITABLE, y
que queda que no lo sea. No lee la intencion: cuenta llamadas y dispara contra
la forja de verdad.

    ① lo que el guion usa hoy, contado
    ② lo que ese acceso ALCANZA — que es el argumento entero
    ③ cuanto sobra: la forja por HTTP, probado
    ④ y lo que no sobra, con su nombre
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
ALTA = RAIZ / "malla" / "aprovisionar-inquilino.sh"
BAJA = RAIZ / "malla" / "desaprovisionar-inquilino.sh"

fallos, huecos = [], []


def mide(t, ok, dice=""):
    print(f"  {'✓' if ok else '✗'} {t}")
    if not ok:
        if dice:
            for l in dice.splitlines():
                print(f"      {l}")
        fallos.append(t)


def hueco(t, dice):
    print(f"  ⚠️ {t}")
    for l in dice.splitlines():
        print(f"      {l}")
    huecos.append(t)


def kubectl(*a):
    try:
        r = subprocess.run(["kubectl", *a], capture_output=True, text=True,
                           encoding="utf-8", timeout=120)
        return r.stdout if r.returncode == 0 else None
    except (OSError, subprocess.TimeoutExpired):
        return None


def main():
    print("① EL APROVISIONADOR SIN CREDENCIALES DE CLUSTER\n")
    alta = ALTA.read_text(encoding="utf-8")
    baja = BAJA.read_text(encoding="utf-8")
    codigo = "\n".join(
        l for l in (alta + "\n" + baja).splitlines() if not l.lstrip().startswith("#")
    )

    # ══════════════════════════════════════════════════════════════════════
    print("① lo que el guion usa hoy — contado, no recordado")
    # ══════════════════════════════════════════════════════════════════════
    usos = re.findall(r"kubectl (\w+)[^\n]*?(-n \"?\$?\{?\w[\w-]*)", codigo)
    for verbo, donde in usos:
        print(f"     kubectl {verbo:<7} {donde.replace(chr(34), '')}")
    mide(
        "la cabecera afirma que no tiene credenciales de cluster",
        "NO tiene credenciales de cl" in alta,
    )
    # ⚠️ Hueco y no fallo, a proposito. Esto es lo que la ① viene a cerrar, y
    #   una medida en rojo permanente enseña a ignorarla — que es justo la
    #   leccion que este arbol lleva aprendiendo toda la sesion. El dia que el
    #   Job exista, la lista de arriba queda vacia y esto se calla solo.
    if usos:
        hueco(
            "…y usa %d llamadas a `kubectl`" % len(usos),
            "`kubectl exec` ES una credencial de cluster, y de las gordas.\n"
            "Lo que falta no es reescribir el guion: es CORRERLO EN OTRO SITIO.",
        )

    # ══════════════════════════════════════════════════════════════════════
    print("\n② lo que ese acceso ALCANZA, que es el argumento entero")
    # ══════════════════════════════════════════════════════════════════════
    #
    # ⛔ `pods/exec` sobre `idp-db-0` no es «leer una fila»: es un `psql` como
    #   superusuario dentro del pod de la base. El guion lo usa para leer
    #   `arbol` y `kek`; con el mismo permiso se lee cualquier otra cosa.
    if kubectl("get", "ns", "-o", "name") is None:
        print("  · no medido: no hay cluster")
    else:
        for que, sql in (
            ("el material cifrado del cofre", "select count(*) from cofre.material"),
            ("todas las personas", "select count(*) from iam.persona"),
            ("todas las concesiones", "select count(*) from iam.concesion"),
        ):
            r = kubectl("exec", "-n", "identidad", "idp-db-0", "--",
                        "psql", "-U", "keycloak", "-d", "iam", "-tAc", sql)
            if r is not None:
                print(f"     con el MISMO exec se lee: {que} ({r.strip()})")
        hueco(
            "el guion tiene HOY mas poder del que su propio diseño prohibe",
            "La `0022` rechazo que el aprovisionador creara los `Secret` porque eso\n"
            "exige permisos de ambito de cluster sobre `Secret`, y entonces «quien\n"
            "puede crear el de un inquilino puede leer el de todos».\n"
            "⇒ `pods/exec` sobre la base de identidad es ESTRICTAMENTE MAS que eso:\n"
            "  no lee los secretos de un namespace, lee el censo entero — personas,\n"
            "  concesiones y el material del cofre. La propiedad no esta rota en el\n"
            "  diseño: esta rota en COMO SE CORRE.",
        )

    # ══════════════════════════════════════════════════════════════════════
    print("\n③ cuanto de eso SOBRA — la forja entera, y esta probado")
    # ══════════════════════════════════════════════════════════════════════
    #
    # ⭐ Tres de los cinco `exec` son contra la forja, y los tres tienen
    #   equivalente por HTTP. Se probo contra la forja de verdad con un usuario
    #   de usar y tirar:
    #
    #       POST   /admin/users                 201   crear el usuario
    #       POST   /users/{u}/tokens            201   acuñar SU testigo (basica)
    #       DELETE /admin/users/{u}?purge=true  204   y borrarlo
    #
    #   ⛔ El segundo es el que parecia imposible: no hay endpoint de admin para
    #     acuñar el testigo de otro. Pero el aprovisionador CREA a ese usuario,
    #     asi que la contraseña la pone el — y con basica puede pedirlo en su
    #     nombre. El `exec` era una comodidad, no una necesidad.
    swagger = kubectl("exec", "-n", "forja", "forja-0", "--", "sh", "-c",
                      "curl -sS http://localhost:3000/swagger.v1.json")
    if swagger:
        for camino, metodo in (
            ("/admin/users", "post"),
            ("/users/{username}/tokens", "post"),
            ("/admin/users/{username}", "delete"),
            ("/users/{username}/tokens/{token}", "delete"),
        ):
            hay = f'"{camino}"' in swagger
            mide(f"{metodo.upper():<6} {camino}", hay,
                 "no existe en esta version de la forja")
        print(
            "\n     ⇒ Y el ultimo arregla algo que esta sesion dio por imposible: se dijo\n"
            "       que Forgejo no sabia revocar un testigo ni por CLI ni por API, y hubo\n"
            "       que ir a su SQLite. Si sabe — con basica, en nombre de su dueño."
        )

    # ══════════════════════════════════════════════════════════════════════
    print("\n④ y lo que NO sobra, con su nombre")
    # ══════════════════════════════════════════════════════════════════════
    if kubectl("get", "ns", "-o", "name") is not None:
        papeles = kubectl("exec", "-n", "identidad", "idp-db-0", "--", "psql",
                          "-U", "keycloak", "-d", "iam", "-tAc",
                          "select string_agg(rolname, ' ') from pg_roles "
                          "where rolname like 'ore%'") or ""
        print("     papeles de la base: " + papeles.strip())
        if "aprovision" not in papeles:
            hueco(
            "falta un papel estrecho para leer la fila del inquilino",
            "solo estan `ore_iam` y `ore_cofre`. `ore_iam` tiene privilegio sobre 68\n"
            "tablas; el aprovisionador necesita CUATRO COLUMNAS de una. La `020` ya\n"
            "hizo este reparto una vez —dos papeles para que la separacion fuera un\n"
            "`grant`— y aqui falta el tercero.",
        )
        # ── ⛔⛔ Y EL HALLAZGO QUE REENCUADRA TODO ────────────────────────
        k = kubectl("get", "kustomization", "-A", "--no-headers") or ""
        gobernados = [l.split()[1] for l in k.splitlines() if l.split()]
        print("     lo que Flux gobierna: " + (", ".join(gobernados) or "(nada)"))
        if not any("malla" in g or "plataforma" in g for g in gobernados):
            hueco(
                "`malla/` NO esta bajo GitOps — solo el inquilino lo esta",
                "Y eso reencuadra los dos «pasos a mano» que se le achacaban al\n"
                "aprovisionador. El enganche (`13-…`) y la clave de despliegue se\n"
                "aplican a mano no porque el aprovisionador no pueda: es que **la\n"
                "plataforma entera se aplica a mano**. La E3 hizo la mitad del\n"
                "trabajo —el inquilino— y la otra mitad no se nombro.\n"
                "⇒ Con `malla/` reconciliado, dar de alta a un inquilino seria UN\n"
                "  COMMIT, y el `13-…` dejaria de ser un paso para ser un diff.",
            )

    # La clave de despliegue: esta si es irreductible, y conviene decirlo.
    hueco(
        "la clave de despliegue de Flux no puede venir del almacen",
        "`source-controller` la lee de etcd, asi que crearla exige `create secret`\n"
        "en `flux-system`. Es el UNICO permiso de cluster que el aprovisionador no\n"
        "puede quitarse cambiando de sitio.\n"
        "⇒ Y tiene salida, pero es una decision: UNA clave de maquina con lectura\n"
        "  sobre todos los `inquilino-*` en vez de una por inquilino. No empeora el\n"
        "  aislamiento —los compartimentos son NUESTROS y Flux ya es `cluster-admin`\n"
        "  sobre todos— y convierte un paso por cliente en un paso una sola vez.",
    )

    print()
    if fallos:
        print(f"✗ {len(fallos)} afirmaciones no se sostienen")
    print(f"⚠️ {len(huecos)} huecos · y el de `malla/` es mas grande que el ①")
    return 1 if fallos else 0


if __name__ == "__main__":
    sys.exit(main())
