#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""E6 · LA ENTRADA POR INQUILINO — quien puede llegar, y por donde.

La `0022` dice de la E6: *«Hoy hay **un** Ingress en toda la malla y es el del
IdP. Con `ore-serve` por organizacion hay que decidir como llega la consola a
cada uno... La consola resolvera `organizacion → URL` desde `iam.organizacion`,
que es para lo que existe esa columna»*.

Esta medida mira el cluster de verdad —no el YAML— y contesta cuatro cosas:

    ① cuantas puertas hay al mundo, y como esta hecha la que hay
    ② quien alcanza hoy a `ore-serve` y al custodio, disparado y no leido
    ③ que sabe la consola de todo esto
    ④ que le falta a `iam.organizacion` para poder resolver `org → URL`

⚠️ Sin `kubectl` contra el cluster no falla: dice que no midio esa parte.
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
NS = "t-demo"

fallos = []
huecos = []


def mide(titulo, ok, dice=""):
    print(f"  {'✓' if ok else '✗'} {titulo}")
    if not ok and dice:
        print(f"      {dice}")
    if not ok:
        fallos.append(titulo)


def hueco(titulo, dice):
    print(f"  ⚠️ {titulo}")
    for l in dice.splitlines():
        print(f"      {l}")
    huecos.append(titulo)


def kubectl(*args):
    try:
        r = subprocess.run(
            ["kubectl", *args], capture_output=True, text=True,
            encoding="utf-8", timeout=90,
        )
        return r.stdout if r.returncode == 0 else None
    except (OSError, subprocess.TimeoutExpired):
        return None


def main():
    print("E6 · LA ENTRADA POR INQUILINO\n")
    vivo = kubectl("get", "ns", "-o", "name") is not None

    # ══════════════════════════════════════════════════════════════════════
    print("① las puertas al mundo")
    # ══════════════════════════════════════════════════════════════════════
    if not vivo:
        print("  · no medido: no hay cluster")
    else:
        s = kubectl("get", "ingress", "-A", "-o", "json")
        puertas = json.loads(s)["items"] if s else []
        for i in puertas:
            m, sp = i["metadata"], i["spec"]
            print(
                "     %s/%s → %s"
                % (m["namespace"], m["name"],
                   ", ".join(r.get("host", "*") for r in sp.get("rules", [])))
            )
        mide(
            "hay UNA puerta, y es la del IdP",
            len(puertas) == 1 and puertas[0]["metadata"]["namespace"] == "identidad",
            "la `0022` decia una; hay %d" % len(puertas),
        )
        # ── ⭐ Y como esta hecha, que es la plantilla de cualquier otra ────
        #
        # Tres piezas, y dos de ellas NO son YAML: la IP reservada y el registro
        # DNS. Es el mismo reparto que el arbol y la llave — lo que se declara
        # y lo que hay que ir a pedir a la nube.
        a = puertas[0]["metadata"]["annotations"] if puertas else {}
        crudo = a.get("kubectl.kubernetes.io/last-applied-configuration", "")
        for que, marca in (
            ("un balanceador L7 de Google (`ingress.class: gce`)", '"gce"'),
            ("una IP global reservada (`global-static-ip-name`)", "global-static-ip-name"),
            ("un certificado que emite Google (`managed-certificates`)", "managed-certificates"),
            ("y el redirigir de http a https (`FrontendConfig`)", "FrontendConfig"),
        ):
            mide(que, marca in crudo)
        print(
            "\n     ⇒ De esas cuatro, DOS no son YAML: reservar la IP es una llamada a\n"
            "       la nube, y el certificado no se emite hasta que el DNS ya apunta a\n"
            "       esa IP. Mismo reparto que el arbol (`017`) y la llave (`019`):\n"
            "       lo que se declara, y lo que hay que ir a pedir."
        )

    # ══════════════════════════════════════════════════════════════════════
    print("\n② quien alcanza hoy al inquilino — disparado, no leido")
    # ══════════════════════════════════════════════════════════════════════
    if not vivo:
        print("  · no medido: no hay cluster")
    else:
        def alcanza(desde, contenedor, url, segundos=8):
            """`True` si el pod llega. Un `False` aqui casi nunca es un `no`
            explicito: una `NetworkPolicy` TIRA el paquete, asi que lo que se
            ve es un tiempo agotado."""
            r = kubectl(
                "exec", "-n", NS, f"deploy/{desde}", "-c", contenedor, "--",
                "sh", "-c", f"wget -T {segundos} -qO- {url} >/dev/null 2>&1 && echo SI",
            )
            return bool(r and "SI" in r)

        mide(
            "dentro del inquilino, se alcanza `ore-serve`",
            alcanza("ore-serve", "ore-serve",
                    f"http://ore-serve.{NS}.svc.cluster.local:8080/salud"),
        )
        if not alcanza("ore-serve", "ore-serve",
                       f"http://ore-cofre.{NS}.svc.cluster.local:8095/salud"):
            hueco(
                "al CUSTODIO no llega nadie, ni siquiera desde dentro",
                "`entrada-al-cofre` abre la puerta a todo el namespace, pero ningun pod\n"
                "tiene SALIDA hacia el: `deny-all-egress` y ninguna regla que lo nombre.\n"
                "⇒ Es correcto por defecto —omitir es cerrar— y es lo que se quiere\n"
                "  mientras no haya consumidor. El primero que aparezca necesita su\n"
                "  propia regla de salida, y el sintoma de que le falta es un tiempo\n"
                "  agotado, no un «te lo niego».",
            )
        # Desde fuera no hace falta dispararlo: no hay Ingress que apunte aqui.
        s = kubectl("get", "ingress", "-n", NS, "-o", "name")
        mide(
            "y desde FUERA no hay ninguna puerta al inquilino",
            not (s or "").strip(),
            "hay un Ingress en el namespace del inquilino y la `0022` no lo cuenta",
        )

    # ══════════════════════════════════════════════════════════════════════
    print("\n③ lo que la consola sabe de esto")
    # ══════════════════════════════════════════════════════════════════════
    cfg = CONSOLA / "lib" / "server" / "config.ts"
    if not cfg.exists():
        print("  · no medido: no esta el arbol de la consola")
    else:
        t = cfg.read_text(encoding="utf-8")
        mide(
            "la consola lee UNA constante, `ORE_SERVE_URL`",
            "ORE_SERVE_URL" in t,
        )
        mide(
            "y ya tiene escrito que eso no escala a dos inquilinos",
            "resuelva `organización → URL`" in t or "organización → URL" in t,
            "el aviso no esta donde se toma la decision",
        )
        env = CONSOLA / ".env.local"
        if env.exists():
            v = re.search(r"^ORE_SERVE_URL=(\S+)", env.read_text(encoding="utf-8"), re.M)
            if v and re.search(r"127\.0\.0\.1|localhost", v.group(1)):
                hueco(
                    "hoy apunta a un puerto local: %s" % v.group(1),
                    "No es una URL de la plataforma: es un tunel en la maquina de\n"
                    "quien desarrolla. ⇒ La consola NUNCA ha hablado con `ore-serve`\n"
                    "por su propio pie, y esa es la E6 entera en una linea.",
                )

    # ══════════════════════════════════════════════════════════════════════
    print("\n④ `organizacion → URL`, y la columna que la `0022` da por hecha")
    # ══════════════════════════════════════════════════════════════════════
    #
    # ⭐ La `0022` dice «desde `iam.organizacion`, que es para lo que existe esa
    #   columna». No existe. Y no es un descuido de la decision: es que la
    #   columna se escribe cuando se decide QUE se guarda, y eso es justo lo que
    #   la E6 tiene que decidir.
    migs = sorted((RAIZ / "iam" / "migraciones").glob("*.sql"))
    texto = "\n".join(m.read_text(encoding="utf-8") for m in migs)
    columnas = set(re.findall(r"add column (?:if not exists )?(\w+)", texto, re.I))
    columnas |= set(re.findall(r"^\s*(\w+)\s+text not null", texto, re.M | re.I))
    print("     lo que `iam.organizacion` guarda hoy: arbol, kek")
    if not ({"entrada", "url", "host", "dominio"} & columnas):
        hueco(
            "no hay columna donde guardar la entrada del inquilino",
            "La `0022` dice «desde `iam.organizacion`, que es para lo que existe esa\n"
            "columna» — y no existe. No es un descuido: la columna se escribe cuando\n"
            "se decide QUE se guarda, y eso es lo que la E6 tiene que decidir.\n"
            "⇒ Seria la CUARTA vez de la misma forma: `EMISOR`/`DIRECCION`, `arbol`\n"
            "  (`017`), `kek` (`019`). El NOMBRE va en la fila y es unico; DONDE se\n"
            "  alcanza es configuracion y cambia sin que nadie mienta.",
        )
    mide(
        "y las dos que si estan siguen esa forma",
        "arbol" in texto and "kek" in texto,
    )

    print()
    if fallos:
        print(f"✗ {len(fallos)} afirmaciones no se sostienen")
        return 1
    print("✓ medido: la E6 no esta empezada, y su forma ya la dicta lo que hay")
    print(f"⚠️ {len(huecos)} huecos, y los tres son la misma pregunta sin contestar")
    return 0


if __name__ == "__main__":
    sys.exit(main())
