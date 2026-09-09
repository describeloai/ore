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
    ④ que guarda `iam.organizacion` para resolver `org → URL`
    ⑤ la puerta compartida, y por que todavia no esta aplicada

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
    mide(
        "existe la columna `entrada` (`022`)",
        "entrada" in columnas,
        "la `0022` la daba por hecha y no estaba: la escribe la `022`",
    )
    mide(
        "es unica — dos inquilinos no comparten puerta",
        "organizacion_entrada_unica" in texto,
        "sin esto el segundo se queda con el trafico del primero",
    )
    mide(
        "y guarda un HOST, no una URL",
        "organizacion_entrada_forma" in texto,
        "un esquema aqui seria carretera metida dentro de la identidad",
    )
    # ⚠️ La `019` puso `kek` `not null` y `fundar` no la escribia: fundar una
    #   organizacion de cero reventaba, y en el cluster no se veia porque las
    #   que habia se rellenaron en la migracion. Se mira aqui para que la misma
    #   forma no vuelva a costar lo mismo.
    mide(
        "y `fundar` la escribe, como escribe `arbol` y `kek`",
        "entrada" in (RAIZ / "crates" / "ore-iam" / "src" / "fundar.rs").read_text(
            encoding="utf-8"
        ),
        "la `019` puso `kek` not null sin que `fundar` la escribiera, y lo cazo CI",
    )

    # ══════════════════════════════════════════════════════════════════════
    print("\n⑤ la puerta compartida, que es la opcion `b`")
    # ══════════════════════════════════════════════════════════════════════
    puerta = RAIZ / "malla" / "14-la-puerta.yaml"
    ruta = RAIZ / "malla" / "43-la-entrada.yaml"
    mide("existe `malla/14-la-puerta.yaml`", puerta.exists())
    mide("existe `malla/43-la-entrada.yaml`", ruta.exists())
    # ⛔ Los VALORES, no la prosa. La primera version miraba el fichero entero y
    #   fallo por el comentario que explica POR QUE no hay `certificateRefs` —
    #   una comprobacion que salta por su propia explicacion ensena a
    #   ignorarla. Es la misma leccion que la ⑤ de `gen-inquilino.py`.
    def valores(p):
        return "\n".join(
            l for l in p.read_text(encoding="utf-8").splitlines()
            if not l.lstrip().startswith("#")
        )

    if puerta.exists() and ruta.exists():
        g = valores(puerta)
        r = valores(ruta)
        # ⭐ La propiedad entera de un balanceador compartido: quien puede
        #   colgarse lo dice LA PUERTA, no la ruta.
        mide(
            "solo los namespaces de inquilino pueden colgarse",
            "from: Selector" in g and "ore.dev/rol: cargas" in g,
            "sin esto, cualquier namespace se queda con el hostname de otro",
        )
        mide("un solo balanceador, no uno por cliente", g.count("kind: Gateway") == 1)
        mide("y el puerto 80 no existe", "port: 80" not in g)
        mide(
            "el certificado NO vive en un `Secret`",
            "certificateRefs" not in g and "certmap" in g,
            "un comodin en etcd es la llave privada de todos los inquilinos",
        )
        mide(
            "la ruta abre `ore-serve` y NO el custodio",
            "name: ore-serve" in r and "ore-cofre" not in r,
        )
        mide(
            "y el inquilino deja entrar al balanceador",
            "130.211.0.0/22" in r and "35.191.0.0/16" in r,
            "sin esto la ruta queda `Accepted` y el balanceador da 502 sin registro",
        )
    if vivo:
        clases = kubectl("get", "gatewayclass", "-o", "name") or ""
        mide(
            "y el cluster tiene la clase que la puerta nombra",
            "gke-l7-global-external-managed" in clases,
            "Gateway API no esta habilitado: `--gateway-api=standard`",
        )
        if "puerta" not in (kubectl("get", "gateway", "-A", "-o", "name") or ""):
            hueco(
                "la puerta todavia NO esta aplicada, y es a proposito",
                "El certificado comodin esta en `AUTHORIZING`: espera un CNAME en el DNS\n"
                "de `paladio.io`, que no esta en el Cloud DNS de este proyecto. Aplicarla\n"
                "ahora levanta un balanceador que cobra desde el primer minuto y contesta\n"
                "un error de certificado.\n"
                "⇒ Lo declarado es la verdad y el mundo converge; lo nuevo es que quien\n"
                "  tiene que mover ficha no somos nosotros.",
            )

    print()
    if fallos:
        print(f"✗ {len(fallos)} afirmaciones no se sostienen")
        return 1
    print("✓ medido: la E6 esta escrita entera, y espera UN registro DNS")
    print(f"⚠️ {len(huecos)} huecos, y los tres son la misma pregunta sin contestar")
    return 0


if __name__ == "__main__":
    sys.exit(main())
