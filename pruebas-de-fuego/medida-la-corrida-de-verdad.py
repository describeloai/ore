#!/usr/bin/env python3
"""LA CORRIDA DE VERDAD — lo que la prueba en seco no podia destapar.

El aprovisionador vivio una iteracion entera probado **en seco** y parecia
bueno. La primera corrida de verdad destapo tres defectos en diez minutos, y
los tres comparten forma: *una linea verde encima de algo que no paso*.

    ① `curl -o /tmp/r` dentro de la forja, cuyo sistema de ficheros es de solo
       lectura. Salio con **23** en las tres llamadas — y el llamante
       redirigia a `/dev/null` y no miraba. «✓ organizacion y repositorio»;
    ② `correr "el secreto …" -- true`, que no es una orden: `command not
       found`, y la salida siguio como si nada;
    ③ `git init` sobre lo recien rendido. La primera pasada empuja; la segunda
       muere con «rejected — fetch first», y el guion imprime «✓ prueba» al
       final igual.

⇒ El defecto de fondo no es ninguno de los tres: es que **el guion no
  comprobaba**. Y en seco eso es invisible por construccion, porque en seco no
  hay nada que comprobar.

Esta medida no corre el aprovisionador —no puede: escribe en Google, en la
forja y en GitHub—. Lee los dos guiones y comprueba las cuatro propiedades que
la corrida enseño, para que no vuelvan.
"""

import re
import sys
from pathlib import Path

try:
    sys.stdout.reconfigure(encoding="utf-8")
except AttributeError:
    pass

RAIZ = Path(__file__).resolve().parent.parent
ALTA = RAIZ / "malla" / "aprovisionar-inquilino.sh"
BAJA = RAIZ / "malla" / "desaprovisionar-inquilino.sh"

fallos = []


def mide(titulo, ok, dice):
    print(f"  {'✓' if ok else '✗'} {titulo}")
    if not ok:
        print(f"      {dice}")
        fallos.append(titulo)


def main():
    if not ALTA.exists() or not BAJA.exists():
        print("✗ faltan los guiones")
        return 1
    alta = ALTA.read_text(encoding="utf-8")
    baja = BAJA.read_text(encoding="utf-8")

    print("LA CORRIDA DE VERDAD\n")

    # ── ① Nadie escribe dentro del pod de la forja ────────────────────────
    #
    # Su sistema de ficheros es de solo lectura, y con razon. Un `-o` a
    # cualquier ruta que no sea `/dev/null` vuelve a dar el 23.
    print("① nada se escribe dentro del pod de la forja")
    for f, s in (("aprovisionar", alta), ("desaprovisionar", baja)):
        malos = [o for o in re.findall(r"curl[^\n]*?-o (\S+)", s) if o != "/dev/null"]
        mide(f"{f}: `curl -o` solo a /dev/null", not malos, f"escribe en {malos}")

    # ── ② Ninguna llamada a la forja queda sin mirar ──────────────────────
    #
    # ⭐ La propiedad no es «que funcione»: es que si NO funciona, se note. Un
    #   `>/dev/null` detras de una llamada a la API se traga el codigo.
    print("\n② ninguna llamada a la forja se tira a la basura")
    sin_mirar = re.findall(r"(?:forja_api|\bapi) [A-Z]+[^\n]*>/dev/null", alta + baja)
    mide("no hay `forja_api … >/dev/null`", not sin_mirar, f"{len(sin_mirar)} llamadas ciegas")
    mide(
        "y el codigo HTTP se comprueba en la funcion",
        "2??|409|422" in alta,
        "`forja_api` no distingue exito de fallo",
    )

    # ── ③ El empuje converge, no ocurre ───────────────────────────────────
    print("\n③ el repositorio de instancia converge en vez de ocurrir una vez")
    mide(
        "no hay `git init` sobre lo rendido",
        # ⚠️ Contar `git init` no vale: aparece en la nota que explica el
        #   defecto. Lo que se mide es que NO se inicialice sobre lo rendido —
        #   que era la forma exacta del fallo— y que SI se clone lo que hay.
        "git clone" in alta and not re.search(r'cd "\$TMP/rendido"[^\n]*git init', alta),
        "un `git init` y un `push` no sobreviven a la segunda pasada",
    )
    mide(
        "y se borra lo que el renderizador ya no emite",
        "-name '*.yaml' -delete" in alta,
        "un fichero retirado de la plantilla se quedaria, y Flux lo obedeceria",
    )

    # ── ④ Todo lo que se crea se puede deshacer ───────────────────────────
    #
    # ⭐⭐ Y esta es la que importa de verdad. Un aprovisionador sin su inverso
    #   solo se corre EN SERIO una vez — y por eso se prueba en seco, y por eso
    #   los defectos de arriba vivieron una iteracion entera.
    print("\n④ cada cosa que se crea tiene quien la borre")
    inverso = {
        "la clave (el permiso)": "kms keys remove-iam-policy-binding",
        "las cuentas de Google": "iam service-accounts delete",
        "el enlace del driver": "svc.id.goog[$NS/driver]",
        "el secreto del almacen": "secrets delete",
        "el repositorio de la forja": "/repos/$ARBOL",
        "la organizacion de la forja": "/orgs/$PROPIETARIO",
        "el usuario de la forja": "user delete --username serve-",
        "el repositorio de instancia": "gh repo delete",
    }
    for que, marca in inverso.items():
        mide(que, marca in baja, f"`{marca}` no aparece en el desaprovisionador")

    # ⚠️ Y lo que NO se deshace tiene que estar DICHO, no simplemente ausente.
    print("\n  ⚠️ lo que no se deshace, y se dice:")
    for que, marca in (
        ("la fila de `iam.organizacion`", "LA FILA"),
        ("la clave de KMS, que Google no deja borrar", "no permite borrar una clave"),
        ("los `Secret` y el namespace, que nunca creo", "nunca los"),
    ):
        mide(que, marca in baja, "no esta dicho en la cabecera del desaprovisionador")

    print()
    if fallos:
        print(f"✗ {len(fallos)} propiedades rotas")
        return 1
    print("✓ las cuatro propiedades que la corrida de verdad enseño siguen puestas")
    return 0


if __name__ == "__main__":
    sys.exit(main())
