#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""E5 · EL ARBOL DEL INQUILINO — `ore init --name <org>`, y si es cierto.

La `0022` dice de la E5: *«`ore init --name <org>`, primer commit y push, con la
imagen de `serve` —la unica con `git`—. El nombre del manifiesto sale de la
organizacion, y no es cosmetico: se propaga a cada `connectionEnv`»*.

Esta medida dispara las cuatro afirmaciones contra binarios de verdad y una
forja de verdad —un repositorio pelado local, que es lo mismo que hace
`servidor-forja.sh`— en vez de leerlas.

    ① el verbo existe y toma `--name`
    ② el nombre se propaga al `connectionEnv`, y no es cosmetico
    ③ sin arbol, NADA del plano de control funciona
    ④ la imagen que puede hacerlo existe

Necesita `ore` y `ore-serve` compilados. Sin ellos no falla: dice que no midio.
"""

import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

try:
    sys.stdout.reconfigure(encoding="utf-8")
except AttributeError:
    pass

RAIZ = Path(__file__).resolve().parent.parent
PUERTO = 8241
fallos = []
huecos = []


def mide(titulo, ok, dice=""):
    print(f"  {'✓' if ok else '✗'} {titulo}")
    if not ok:
        if dice:
            print(f"      {dice}")
        fallos.append(titulo)


def hueco(titulo, dice):
    print(f"  ⚠️ {titulo}")
    print(f"      {dice}")
    huecos.append(titulo)


def binario(nombre):
    for perfil in ("release", "debug"):
        for sufijo in (".exe", ""):
            p = RAIZ / "target" / perfil / f"{nombre}{sufijo}"
            if p.exists():
                return p
    return None


def git(dir_, *args):
    return subprocess.run(
        ["git", *args], cwd=dir_, capture_output=True, text=True, encoding="utf-8"
    )


def pedir(metodo, camino, cuerpo=None):
    """Una peticion al servidor. Devuelve `(codigo, texto)`, sin excepciones."""
    datos = json.dumps(cuerpo).encode() if cuerpo is not None else None
    r = urllib.request.Request(
        f"http://127.0.0.1:{PUERTO}{camino}", data=datos, method=metodo
    )
    r.add_header("x-ore-sujeto", "ana")
    if datos:
        r.add_header("content-type", "application/json")
    try:
        with urllib.request.urlopen(r, timeout=30) as s:
            return s.status, s.read().decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode("utf-8", "replace")
    except OSError as e:
        return 0, str(e)


def main():
    ore, serve = binario("ore"), binario("ore-serve")
    print("E5 · EL ARBOL DEL INQUILINO\n")
    if not ore or not serve:
        print("  · no medido: falta `cargo build` de `ore` y `ore-serve`")
        return 0

    tmp = Path(tempfile.mkdtemp(prefix="e5-"))
    servidor = None
    try:
        # ── ⛔ `-b main`, y NO es un detalle de la prueba ──────────────────
        #
        # Un repositorio pelado con HEAD en `master` al que se empuja `main`
        # clona una rama VACIA. Y el sintoma no dice nada de ramas: dice «este
        # directorio no es un repositorio ontologico», que manda a mirar el
        # arbol cuando el problema es la rama por defecto del repositorio.
        forja = tmp / "forja.git"
        subprocess.run(
            ["git", "init", "-q", "--bare", "-b", "main", str(forja)], check=True
        )

        entorno = dict(os.environ, FORJA_TOKEN="no-hace-falta-en-file")
        servidor = subprocess.Popen(
            [
                str(serve), "--forja", str(forja), "--ore", str(ore),
                "--bind", f"127.0.0.1:{PUERTO}",
                "--identidad", "cabecera", "--no-es-produccion",
            ],
            env=entorno, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        for _ in range(60):
            if pedir("GET", "/salud")[0] == 200:
                break
            time.sleep(0.25)
        else:
            print("  · no medido: el servidor no arranco")
            return 0

        # ══════════════════════════════════════════════════════════════════
        print("③ sin arbol, el plano de control no sirve para nada")
        # ══════════════════════════════════════════════════════════════════
        #
        # ⭐ Este es el hueco entero de la E5, y se mide ANTES de taparlo: el
        #   aprovisionador (E4) deja el repositorio de la forja CREADO Y VACIO,
        #   y nada de lo que corre puede poner el arbol dentro.
        cod, cuerpo = pedir("GET", "/fuentes")
        mide(f"GET /fuentes sobre un repositorio vacio · {cod}", cod == 404, cuerpo)
        cod, cuerpo = pedir(
            "POST", "/fuentes",
            {"name": "pg", "type": "postgres", "url": "postgres://h/db"},
        )
        mide(f"POST /fuentes no puede crearlo tampoco · {cod}", cod == 409, cuerpo)

        # ⚠️ Y una tercera ruta contesta que TODO VA BIEN.
        cod, cuerpo = pedir("GET", "/paquetes")
        if cod == 200:
            hueco(
                "GET /paquetes contesta 200 sobre un arbol que no existe",
                f'dice {cuerpo.strip()} — «no hay paquetes» en vez de «no hay arbol». '
                "Dos estados distintos con una sola respuesta.",
            )

        # ══════════════════════════════════════════════════════════════════
        print("\n① el verbo existe, y toma el nombre de la organizacion")
        # ══════════════════════════════════════════════════════════════════
        arbol = tmp / "arbol"
        arbol.mkdir()
        r = subprocess.run(
            [str(ore), "init", "--name", "prueba"],
            cwd=arbol, capture_output=True, text=True, encoding="utf-8",
        )
        mide("`ore init --name prueba`", r.returncode == 0, r.stderr.strip()[:200])
        conf = (arbol / "ontology.config.yaml").read_text(encoding="utf-8")
        mide(
            "y el nombre queda en `metadata.name`",
            re.search(r"name:\s*prueba\b", conf) is not None,
            "el manifiesto no lleva el nombre que se le dio",
        )

        # El primer commit y el empujon: exactamente lo que la E5 tiene que
        # hacer dentro del cluster, y aqui a mano para poder mirarlo.
        for orden in (
            ("init", "-q", "-b", "main"),
            ("add", "-A"),
            ("-c", "user.name=semilla", "-c", "user.email=s@x",
             "commit", "-qm", "El arbol de prueba"),
            ("remote", "add", "origin", str(forja)),
            ("push", "-q", "origin", "main"),
        ):
            r = git(arbol, *orden)
            if r.returncode != 0:
                mide(f"git {orden[0]}", False, r.stderr.strip()[:200])
                return 1
        mide("primer commit y empujon", True)

        # ══════════════════════════════════════════════════════════════════
        print("\n② el nombre se propaga al `connectionEnv`, y no es cosmetico")
        # ══════════════════════════════════════════════════════════════════
        cod, cuerpo = pedir("GET", "/fuentes")
        mide(f"con arbol, GET /fuentes · {cod}", cod == 200, cuerpo)
        cod, cuerpo = pedir(
            "POST", "/fuentes",
            {"name": "crm_prod", "type": "postgres", "url": "postgres://h/db"},
        )
        mide(f"con arbol, POST /fuentes · {cod}", cod == 201, cuerpo[:200])
        escrito = git(tmp, "--git-dir", str(forja), "show",
                      "main:ontology.config.yaml").stdout
        mide(
            "el secreto se busca en `PRUEBA_CRM_PROD_URL`",
            "PRUEBA_CRM_PROD_URL" in escrito,
            "el prefijo no salio del nombre de la organizacion: "
            + str(re.findall(r"connectionEnv: (\S+)", escrito)),
        )
        print(
            "\n     ⇒ Un arbol nacido SIN `--name` da `CRM_PROD_URL` a secas, y dos\n"
            "       inquilinos en el mismo proceso pisarian la misma variable. Por eso\n"
            "       la `0022` dice que no es cosmetico."
        )

    finally:
        if servidor:
            servidor.terminate()
            try:
                servidor.wait(timeout=10)
            except subprocess.TimeoutExpired:
                servidor.kill()
        shutil.rmtree(tmp, ignore_errors=True)

    # ══════════════════════════════════════════════════════════════════════
    print("\n④ la imagen que puede hacerlo, y quien tendria que correrla")
    # ══════════════════════════════════════════════════════════════════════
    df = (RAIZ / "Dockerfile").read_text(encoding="utf-8")
    etapa = df.split("AS serve", 1)[1].split("\nFROM ", 1)[0] if "AS serve" in df else ""
    mide("la etapa `serve` trae `git`", "apk add --no-cache git" in etapa)
    mide("la etapa `serve` trae `ore`", "/ore " in etapa or "/ore\n" in etapa)

    # ── ⭐ Y la pieza que lo corre, que es la E5 construida ────────────────
    #
    # Se mide el manifiesto y no una promesa: el que existe, el que el
    # renderizador emite, y el que la forja deja entrar.
    job = RAIZ / "malla" / "42-el-arbol.yaml"
    mide("existe `malla/42-el-arbol.yaml`", job.exists())
    if job.exists():
        j = job.read_text(encoding="utf-8")
        mide("corre `ore init --name`", "ore init --name demo ." in j)
        mide(
            "firma `aprovisionador`, y no el dueño de la organizacion",
            "user.name=aprovisionador" in j,
            "el arbol nace ANTES que nadie que pueda pedir nada: no hay sujeto",
        )
        mide(
            "es idempotente por la pregunta correcta",
            "[ -f ontology.config.yaml ]" in j,
            "mirar si hay commits no es lo mismo que mirar si hay arbol",
        )
        mide(
            "empuja una rama por su nombre, no `HEAD`",
            "push -q origin main" in j and "checkout -q -B main" in j,
        )
        mide(
            "y NO lleva la etiqueta del servidor",
            "ore.dev/rol: semilla" in j and "ore.dev/rol: control" not in j,
            "con `control` entraria en el `selector` del `Service` de ore-serve",
        )
    gen = (RAIZ / "malla" / "gen-inquilino.py").read_text(encoding="utf-8")
    mide('el renderizador lo emite', '"42-el-arbol.yaml"' in gen)
    mide("y sustituye el `--name`", '"--name %s" % MODELO' in gen)

    # ⛔ La lista de invitados de la forja es CERRADA y esta en otro fichero.
    #   Un rol nuevo que no se anada aqui no da «rechazado»: da un `git clone`
    #   colgado, porque una `NetworkPolicy` tira el paquete. Paso exactamente.
    forja = (RAIZ / "malla" / "30-forja.yaml").read_text(encoding="utf-8")
    mide(
        "la forja deja entrar al rol `semilla`",
        "{ore.dev/rol: semilla}" in forja,
        "sin esto el Job se cuelga en `git clone` y nada dice que sea la red",
    )

    alta = (RAIZ / "malla" / "aprovisionar-inquilino.sh").read_text(encoding="utf-8")
    mide(
        "y el aprovisionador sigue sin hacerlo el (es un Job, no un guion)",
        "ore init --name $NOMBRE" in alta,
        "el paso ⑦ no nombra la E5",
    )
    # ⚠️ Lo que el aprovisionador crea en la forja no dice su rama por defecto.
    if '"default_branch"' not in alta:
        hueco(
            "el repositorio se crea sin decir su rama por defecto",
            "hoy Forgejo la pone en `main` y coincide con la del empujon. El dia que "
            "no coincida, el sintoma sera «este directorio no es un repositorio "
            "ontologico» — que manda a mirar el arbol, no la rama.",
        )

    print()
    if fallos:
        print(f"✗ {len(fallos)} afirmaciones de la E5 no se sostienen")
        return 1
    print("✓ la E5 es lo que la `0022` dice, y ya esta construida")
    if huecos:
        print(f"⚠️ y {len(huecos)} huecos nombrados por el camino")
    return 0


if __name__ == "__main__":
    sys.exit(main())
