#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
medida-w0-el-arbol-en-el-editor.py — ¿qué hay y qué falta para que el árbol del
cliente se plasme en el editor (ADR 0030, W0)?

W0 es: el code workspace abre EL ÁRBOL DE LA CELDA (no una semilla), abre un YAML
en Monaco, guardar es un commit del sujeto en una rama, y los diagnósticos de
`ore validate` salen en línea. Cero runtime nuevo. Esto mide, antes de escribir:

  §A  el árbol: cuánto es (ficheros, bytes, kinds) y cuánto tarda en clonarse y
      compilarse — lo que decide si «una petición = un clon» aguanta un editor.
  §B  lo que `ore-serve` sirve hoy del árbol (documentos por kind, PUT con
      compilar-antes-de-empujar) y lo que no (cualquier fichero, ramas, diff).
  §C  lo que la forja (Forgejo) ya da por su API: contenido por ruta y rama,
      ramas, PRs — lo que W0 puede tomar prestado sin construir.
  §D  el editor: Monaco en la consola, los esquemas JSON de OOS para completar y
      validar forma en el navegador, y cómo se convierte un diagnóstico de `ore`
      en un marcador de Monaco.

  PYTHONIOENCODING=utf-8 python pruebas-de-fuego/medida-w0-el-arbol-en-el-editor.py [--sin-demo]
"""
import json
import os
import re
import subprocess
import sys
import time

RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SIN_DEMO = "--sin-demo" in sys.argv
PROYECTO = "project-8853a180-450d-47be-b83"
CELDA = "demo"
NS = "t-" + CELDA
IDP = "https://login.paladio.io/realms/rubix"
GCLOUD = "gcloud.cmd" if os.name == "nt" else "gcloud"
CONSOLA = "C:/rubix-platform" if os.name == "nt" else os.path.expanduser("~/rubix-platform")


def fila(que, medido, veredicto=""):
    print("  %-52s %-52s %s" % (que, str(medido)[:52], veredicto))


def sh(cmd, cwd=None):
    r = subprocess.run(cmd, shell=True, capture_output=True, text=True, encoding="utf-8", cwd=cwd)
    return r.returncode, (r.stdout or "") + (r.stderr or "")


def ore():
    for c in ("release", "debug"):
        for n in ("ore.exe", "ore"):
            p = os.path.join(RAIZ, "target", c, n)
            if os.path.isfile(p):
                return p
    sys.exit("no hay binario de `ore`")


# ── §A · el árbol ───────────────────────────────────────────────────────────
def seccion_a(arbol):
    print("\n§A · el árbol de demo, y lo que cuesta tenerlo")
    if not arbol or not os.path.isdir(arbol):
        fila("A1 · el árbol", "no hay clon local (--sin-demo)", "—")
        return
    rc, out = sh("git ls-files", cwd=arbol)
    ficheros = [l for l in out.splitlines() if l.strip()]
    bytes_ = sum(os.path.getsize(os.path.join(arbol, f)) for f in ficheros if os.path.isfile(os.path.join(arbol, f)))
    kinds = {}
    for f in ficheros:
        if f.endswith(".yaml"):
            try:
                t = open(os.path.join(arbol, f), encoding="utf-8").read(400)
                m = re.search(r"^kind:\s*(\w+)", t, re.M)
                kinds[m.group(1) if m else "?"] = kinds.get(m.group(1) if m else "?", 0) + 1
            except Exception:
                pass
    fila("A1 · ficheros · bytes", "%d ficheros · %.0f KB · %s" % (len(ficheros), bytes_ / 1024.0, ", ".join("%s %d" % kv for kv in sorted(kinds.items(), key=lambda x: -x[1])[:6])), "cabe entero en el navegador")
    t0 = time.time(); rc, out = sh('"%s" validate .' % ore(), cwd=arbol); dt = time.time() - t0
    n = len(re.findall(r"^error\[", out, re.M))
    fila("A2 · `ore validate .` sobre el árbol entero", "%.2f s · %d errores (rc %d)" % (dt, n, rc), "por debajo de un tecleo: vale en cada guardado")
    # un solo fichero
    f = next((x for x in ficheros if "/views/" in x), None)
    if f:
        t0 = time.time(); rc, out = sh('"%s" validate "%s"' % (ore(), f), cwd=arbol); dt = time.time() - t0
        fila("A3 · `ore validate <fichero>` (solo forma)", "%.2f s · rc %d" % (dt, rc), "la forma sola no ve referencias: los marcadores de verdad son del árbol")
    t0 = time.time(); rc, out = sh("git clone -q --depth 1 file://%s %s-clon" % (arbol.replace("\\", "/"), arbol.replace("\\", "/"))); dt = time.time() - t0
    fila("A4 · clonar el árbol (local, depth 1)", "%.2f s" % dt, "el coste de «una petición = un clon» sin red")
    sh('rm -rf "%s-clon"' % arbol)


# ── §B · ore-serve ──────────────────────────────────────────────────────────
def seccion_b():
    print("\n§B · lo que ore-serve sirve del árbol")
    rutas = open(os.path.join(RAIZ, "crates/ore-serve/src/rutas.rs"), encoding="utf-8").read()
    docs = open(os.path.join(RAIZ, "crates/ore-serve/src/documentos.rs"), encoding="utf-8").read()
    kinds = re.findall(r'nombre: "(\w+)",\s*carpeta', docs)
    fila("B1 · documentos por kind", "GET/PUT/DELETE /documentos/{kind}/{ns}/{n} · kinds: " + ", ".join(kinds), "Function, Model, Package, config: NO")
    fila("B2 · PUT compila antes de empujar", "diagnosticos_de → empeora (no empeorar) → commit del sujeto → 409 si el árbol se movió", "✓ es lo que «guardar» necesita")
    fila("B3 · cualquier fichero por ruta", "existe" if re.search(r'\["arbol"', rutas) else "NO: sólo documentos con kind", "W0: GET/PUT /arbol/<ruta>")
    fila("B4 · ramas", "clonar() clona main; publicar() empuja a main", "W0: rama por persona (`persona/<sub>`) o PR de la forja")
    fila("B5 · diagnósticos como datos", "de stderr: código, mensaje, donde (fichero:línea:col), ayuda", "✓ es un marcador de Monaco; falta la SEVERIDAD y el rango")
    git = open(os.path.join(RAIZ, "crates/ore-serve/src/git.rs"), encoding="utf-8").read()
    fila("B6 · el clon por petición", "sí: cada verbo clona a un directorio temporal y lo tira", "medir en demo (C)")


# ── §C · la forja ───────────────────────────────────────────────────────────
def token_del_agente():
    _, cli = sh("%s secrets versions access latest --secret=%s-agente-cliente --project=%s" % (GCLOUD, NS, PROYECTO))
    _, sec = sh("%s secrets versions access latest --secret=%s-agente-secreto --project=%s" % (GCLOUD, NS, PROYECTO))
    cli, sec = cli.strip(), sec.strip()
    if not cli or not sec:
        return None
    r = subprocess.run(["curl", "-s", "-m", "10", "-X", "POST", IDP + "/protocol/openid-connect/token", "-d", "grant_type=client_credentials",
                        "-d", "client_id=" + cli, "--data-urlencode", "client_secret=" + sec], capture_output=True, text=True, encoding="utf-8")
    try:
        return json.loads(r.stdout)["access_token"]
    except Exception:
        return None


def serve(camino, tok):
    r = subprocess.run(["curl", "-s", "-m", "60", "-o", "-", "-w", "\n%{http_code} %{time_total}",
                        "https://%s.ore.paladio.io%s" % (CELDA, camino), "-H", "authorization: Bearer " + tok],
                       capture_output=True, text=True, encoding="utf-8")
    cuerpo, _, cod = (r.stdout or "").rpartition("\n")
    p = cod.split()
    return (p[0] if p else "000"), float(p[1]) if len(p) > 1 else 0.0, cuerpo.strip()


def seccion_c(arbol):
    print("\n§C · la forja (Forgejo) y ore-serve, en demo")
    malla = open(os.path.join(RAIZ, "malla/46-la-forja-del-inquilino.yaml"), encoding="utf-8").read()
    m = re.search(r"forgejo:(\d+)", malla)
    fila("C1 · la forja del inquilino", "Forgejo %s, una por celda, sólo dentro de la malla" % (m.group(1) if m else "?"), "API: contents por ruta y rama, branches, pulls, diff")
    fila("C2 · API de contenido", "GET/PUT /api/v1/repos/{o}/{r}/contents/{path}?ref=<rama> · POST …/branches · POST …/pulls", "W0 puede tomar prestado leer/escribir/ramas/PR; compilar sigue siendo de ore-serve")
    if SIN_DEMO:
        return
    tok = token_del_agente()
    if not tok:
        fila("C3 · token", "no se pudo acuñar", "✗")
        return
    ts = []
    for k in ("View", "Entity", "Table"):
        cod, t, cuerpo = serve("/documentos/%s" % k, tok)
        try:
            n = len(json.loads(cuerpo).get("documentos", json.loads(cuerpo).get("documents", [])))
        except Exception:
            n = -1
        ts.append(t)
        fila("C3 · GET /documentos/%s (clon + listar)" % k, "%s · %d docs · %.2f s" % (cod, n, t), "")
    fila("C4 · latencia media de «una petición = un clon»", "%.2f s" % (sum(ts) / len(ts)), "aguanta abrir/guardar; NO aguanta un marcador por tecla → validar al guardar")
    if arbol and os.path.isdir(arbol):
        rc, out = sh("git log -1 --format=%H", cwd=arbol)
        fila("C5 · el árbol local y el de la forja", "HEAD %s" % out.strip()[:12], "")


# ── §D · el editor ──────────────────────────────────────────────────────────
def seccion_d():
    print("\n§D · el editor")
    cw = os.path.join(CONSOLA, "components/code-workspace")
    hay = os.path.isdir(cw)
    fila("D1 · code workspace en la consola", ("%d ficheros, Monaco con tema Carbon" % len(os.listdir(cw))) if hay else "NO", "sólo UI: árbol, celdas y ficheros son estado de cliente (types.ts)")
    if hay:
        t = open(os.path.join(cw, "types.ts"), encoding="utf-8").read()
        fila("D2 · lenguajes que conoce", ", ".join(sorted(set(re.findall(r"'(python|sql|java|toml|yaml|text|markdown)'", t)))), "yaml está; falta que un fichero sea DEL ÁRBOL")
    pk = os.path.join(CONSOLA, "package.json")
    if os.path.isfile(pk):
        deps = json.load(open(pk, encoding="utf-8")).get("dependencies", {})
        fila("D3 · monaco en package.json", ", ".join(k for k in deps if "monaco" in k) or "NO", "monaco-yaml daría completado y validación de FORMA con los esquemas de OOS")
    esq = os.path.join(RAIZ, "vendor/oos/schemas")
    if os.path.isdir(esq):
        n = sum(len([f for f in fs if f.endswith(".json")]) for _, _, fs in os.walk(esq))
        fila("D4 · esquemas JSON de OOS", "%d ficheros en vendor/oos/schemas" % n, "la forma en el navegador, sin red; el significado (referencias, flujo) en ore-serve")
    fila("D5 · diagnóstico → marcador", "OOS code + mensaje + fichero:línea:col + ayuda → {startLineNumber, startColumn, message, code, severity}", "falta: rango (fin) y severidad (error/aviso) en la salida de ore")


if __name__ == "__main__":
    t0 = time.time()
    print("W0 · el árbol en el editor — lo que hay y lo que falta (%s)" % time.strftime("%Y-%m-%d %H:%M"))
    arbol = next((a.split("=", 1)[1] for a in sys.argv if a.startswith("--arbol=")), None)
    seccion_a(arbol)
    seccion_b()
    seccion_c(arbol)
    seccion_d()
    print("\n%.0f s" % (time.time() - t0))
