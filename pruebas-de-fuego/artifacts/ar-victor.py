"""ORE 0074 AR1·5 + AR2·4 · LA MEDIDA EN VICTOR: Maven y PyPI de punta a punta, en producción.

    §0  victor sirve el commit que se pide (`ore.dev/commit`)
    §1  dos registros por la API (`formats: [maven, pypi]`), en `main`
    §2  el token de publicar, emitido por `ore-serve` con `ore-iam` (y sus instrucciones)
    §3  Maven por el túnel, desde un Docker local: deploy-file pequeño y de 60 MB; 409, 401, 403, 400
    §4  PyPI por el túnel: `uv publish` (rueda y sdist) y `twine upload`; 409; el índice de la celda
    §5  las capas de la JVM y de Python, en una rama propia: declaran la librería, `POST /entorno`
        las encola, el Job lleva el token de leer, y el informe dice `de_artefactos`; y una versión
        sin publicar → error, sin buscarla en Central ni en PyPI

    python pruebas-de-fuego/artifacts/ar-victor.py [commit-esperado]

Como `salud/vara.py`: el agente de la celda (`client_credentials`, secretos en GCP) y `curl -K -`:
ningún token en la línea de órdenes ni en un fichero, ni se imprime. Lo que publica son librerías de
prueba (`com.banco:motor-riesgo`, `banco-riesgo`): inmutables, se quedan (AR5 retira).
"""
import json
import os
import re
import subprocess
import sys
import time
import urllib.parse

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
NS = "t-victor"
PROYECTO = "project-8853a180-450d-47be-b83"
API = "https://victor.ore.paladio.io"
IDP = "https://login.paladio.io/realms/rubix"
GCLOUD = "gcloud.cmd" if os.name == "nt" else "gcloud"
B, S = os.environ.get("AR_BASE", "ar_medida"), os.environ.get("AR_SCHEMA", "libs")
R, R2 = "banco", "otro"
ESPERADO = sys.argv[1] if len(sys.argv) > 1 else ""
MAVEN_IMG, PY_IMG = "maven:3.9-eclipse-temurin-21", "python:3.13-slim"
fallos = []


def sh(args, entrada=None):
    r = subprocess.run(args, input=entrada, capture_output=True, text=True, encoding="utf-8", errors="replace",
                       env=dict(os.environ, MSYS_NO_PATHCONV="1"))
    return r.returncode, (r.stdout + r.stderr).strip()


def dice(m):
    print("  ✓ " + m)


def falla(m):
    print("  ✗ " + m)
    fallos.append(m)


_tok = [None, 0]


def token():
    if _tok[0] and time.time() - _tok[1] < 200:
        return _tok[0]
    cli = sh([GCLOUD, "secrets", "versions", "access", "latest", "--secret=%s-agente-cliente" % NS, "--project=" + PROYECTO])[1]
    sec = sh([GCLOUD, "secrets", "versions", "access", "latest", "--secret=%s-agente-secreto" % NS, "--project=" + PROYECTO])[1]
    cuerpo = urllib.parse.urlencode({"grant_type": "client_credentials", "client_id": cli, "client_secret": sec})
    _, o = sh(["curl", "-s", "-m", "20", "-K", "-"], 'url = "%s/protocol/openid-connect/token"\ndata = "%s"\n' % (IDP, cuerpo))
    try:
        _tok[0], _tok[1] = json.loads(o)["access_token"], time.time()
    except (ValueError, KeyError):
        raise SystemExit("el IdP no dio token al agente: %s" % o[:160])
    return _tok[0]


def serve(metodo, camino, cuerpo=None, cabeceras=(), crudo=None):
    conf = 'url = "%s%s"\nrequest = "%s"\nheader = "authorization: Bearer %s"\n' % (API, camino, metodo, token())
    for k, v in cabeceras:
        conf += 'header = "%s: %s"\n' % (k, v)
    if cuerpo is not None:
        conf += 'header = "content-type: application/json"\ndata = %s\n' % json.dumps(json.dumps(cuerpo))
    elif crudo is not None:
        conf += "data-binary = %s\n" % json.dumps(crudo)
    r = subprocess.run(["curl", "-s", "-m", "120", "-w", "\n%{http_code}", "-K", "-"], input=conf.encode(), capture_output=True)
    b, _, cod = r.stdout.rpartition(b"\n")
    try:
        j = json.loads(b) if b.strip() else {}
    except ValueError:
        j = {"_texto": b.decode(errors="replace")[:400]}
    return (int(cod) if cod.strip().isdigit() else 0), j


def docker(img, guion, env):
    args = ["docker", "run", "--rm", "-i"]
    for k in env:
        args += ["-e", k]  # el valor, del entorno de este proceso: no va en la línea de órdenes
    r = subprocess.run(args + [img, "bash", "-s"], input=guion, capture_output=True, text=True, encoding="utf-8",
                       errors="replace", env=dict(os.environ, MSYS_NO_PATHCONV="1", **env))
    return r.returncode, r.stdout + r.stderr


# ── §0 ────────────────────────────────────────────────────────────────────────
print("§0 · lo que victor sirve")
_, commit = sh(["kubectl", "-n", NS, "get", "deploy", "ore-serve", "-o", "jsonpath={.spec.template.metadata.annotations.ore\\.dev/commit}"])
print("  ore-serve en victor: %s" % commit)
if ESPERADO and not commit.startswith(ESPERADO[:8]):
    raise SystemExit("  ✗ victor no sirve %s todavía: espera al despliegue (ci-verde-no-es-desplegado)" % ESPERADO)

# ── §1 ────────────────────────────────────────────────────────────────────────
print("§1 · los registros, por la API, en main")
for r in (R, R2):
    c, j = serve("PUT", "/documentos/ArtifactRegistry/%s/%s/%s" % (B, S, r), {"spec": {"formats": ["maven", "pypi"]}})
    if c in (200, 201):
        dice("%s.%s.%s (maven, pypi) → %d" % (B, S, r, c))
    else:
        falla("crear %s: %d %s" % (r, c, json.dumps(j)[:300]))
if fallos:
    raise SystemExit("sin registros no hay medida (¿main protegida para el agente? ¿la base existe?)")

# ── §2 ────────────────────────────────────────────────────────────────────────
print("§2 · el token de publicar (ore-serve pregunta a ore-iam)")
tokens = {}
for r in (R, R2):
    c, j = serve("POST", "/artifact-registries/%s/%s/%s/token" % (B, S, r))
    if c == 201 and j.get("token", "").startswith("ore_art_"):
        tokens[r] = j["token"]
        dice("token de %s → 201 (%d s; maven %s, pypi %s)" % (r, j.get("expires_in", 0), bool(j.get("pom_xml")), bool(j.get("pypi"))))
    else:
        falla("token de %s: %d %s" % (r, c, json.dumps(j)[:300]))
if len(tokens) < 2:
    raise SystemExit("sin tokens no se publica (un 403 aquí es la potestad `artefacto:publicar` del agente)")

U_MVN = "%s/artifacts/%s/%s/%s/maven" % (API, B, S, R)
U_MVN2 = "%s/artifacts/%s/%s/%s/maven" % (API, B, S, R2)
U_PY = "%s/artifacts/%s/%s/%s/pypi/" % (API, B, S, R)
U_PY2 = "%s/artifacts/%s/%s/%s/pypi/" % (API, B, S, R2)

# ── §3 ────────────────────────────────────────────────────────────────────────
print("§3 · Maven por el túnel")
c, o = docker(MAVEN_IMG, r'''
set -u; cd /tmp
ajustes() { printf '<settings><servers><server><id>ore</id><username>publish</username><password>%s</password></server></servers></settings>' "$1"; }
ajustes "$PUB" > pub.xml; ajustes "$PUB2" > otro.xml; ajustes "" > sin.xml
D=org.apache.maven.plugins:maven-deploy-plugin:3.1.3:deploy-file
mkdir -p src/com/banco/riesgo out
printf "package com.banco.riesgo;\npublic class Calculadora { public static int riesgo() { return 42; } }\n" > src/com/banco/riesgo/Calculadora.java
javac -d out src/com/banco/riesgo/Calculadora.java && jar cf motor-riesgo-1.0.0.jar -C out .
pub() { # ajustes url version fichero artefacto
  mvn -B -q -s $1 $D -Durl=$2 -DrepositoryId=ore -Dfile=$4 -DgroupId=com.banco -DartifactId=$5 -Dversion=$3 -Dpackaging=jar > log 2>&1 && echo ok || (grep -o "status code: [0-9]*" log | head -1 || tail -2 log)
}
t0=$(date +%s%N); echo "PEQUENO $(pub pub.xml $U 1.0.0 motor-riesgo-1.0.0.jar motor-riesgo) $(( ($(date +%s%N)-t0)/1000000 ))ms"
head -c 60000000 /dev/urandom > g.bin; mkdir -p g && mv g.bin g/ && jar cf0 grande.jar -C g .
t0=$(date +%s%N); echo "GRANDE $(pub pub.xml $U 1.0.0 grande.jar grande) $(( ($(date +%s%N)-t0)/1000000 ))ms $(stat -c %s grande.jar)B"
echo "REPUBLICAR $(pub pub.xml $U 1.0.0 motor-riesgo-1.0.0.jar motor-riesgo)"
echo "SNAPSHOT $(pub pub.xml $U 1.1.0-SNAPSHOT motor-riesgo-1.0.0.jar motor-riesgo)"
echo "SINTOKEN $(pub sin.xml $U 1.2.0 motor-riesgo-1.0.0.jar motor-riesgo)"
echo "OTROTOKEN $(pub otro.xml $U 1.2.0 motor-riesgo-1.0.0.jar motor-riesgo)"
echo "OTROREGISTRO $(pub otro.xml $U2 9.9.9 motor-riesgo-1.0.0.jar motor-riesgo)"
''', {"PUB": tokens[R], "PUB2": tokens[R2], "U": U_MVN, "U2": U_MVN2})
res = dict(re.findall(r"^([A-Z]+) (.*)$", o, re.M))
esperado = {"PEQUENO": "ok", "GRANDE": "ok", "REPUBLICAR": "status code: 409", "SNAPSHOT": "status code: 400",
            "SINTOKEN": "status code: 401", "OTROTOKEN": "status code: 403", "OTROREGISTRO": "status code: 409"}
for k, v in esperado.items():
    got = res.get(k, "(sin respuesta)")
    (dice if got.startswith(v) else falla)("%s → %s" % (k.lower(), got))
if c and not res:
    print(o[-800:])

# ── §4 ────────────────────────────────────────────────────────────────────────
print("§4 · PyPI por el túnel")
c, o = docker(PY_IMG, r'''
set -u; cd /tmp
pip install -q --root-user-action=ignore --disable-pip-version-check uv==0.12.23 twine >/dev/null 2>&1
apt-get -qq update >/dev/null 2>&1; apt-get -qq install -y curl >/dev/null 2>&1
export UV_NO_PROGRESS=1
build() { rm -rf p dist && mkdir -p p/src/banco_riesgo && printf 'X = 42\n' > p/src/banco_riesgo/__init__.py
  printf '[project]\nname = "Banco_Riesgo"\nversion = "%s"\nrequires-python = ">=3.10"\n[build-system]\nrequires = ["uv_build>=0.9,<0.13"]\nbuild-backend = "uv_build"\n' "$1" > p/pyproject.toml
  (cd p && uv build -q --out-dir ../dist); }
build 1.0
t0=$(date +%s%N); uv publish --publish-url "$U" --username __token__ --password "$PUB" dist/* > log 2>&1 && r=ok || r="$(tail -1 log)"; echo "UVPUBLISH $r $(( ($(date +%s%N)-t0)/1000000 ))ms"
cp -r dist d10; build 1.1
twine upload --non-interactive --disable-progress-bar --repository-url "$U" -u __token__ -p "$PUB" dist/*.whl > log 2>&1 && r=ok || r="$(tail -1 log)"; echo "TWINE $r"
uv publish --publish-url "$U" --username __token__ --password "$PUB" d10/*.whl > log 2>&1 && r=entro || r="$(grep -o 409 log | head -1)"; echo "REPUBLICAR $r"
uv publish --publish-url "$U2" --username __token__ --password "$PUB2" dist/*.whl > log 2>&1 && r=entro || r="$(grep -o 409 log | head -1)"; echo "OTROREGISTRO $r"
c=$(curl -s -o i.html -w '%{http_code}' -u "__token__:$PUB" "$IDX"); echo "INDICE $c $(grep -c '#sha256=' i.html)"
''', {"PUB": tokens[R], "PUB2": tokens[R2], "U": U_PY, "U2": U_PY2, "IDX": API + "/artifacts/pypi/simple/banco-riesgo/"})
res = dict(re.findall(r"^([A-Z]+) (.*)$", o, re.M))
esperado = {"UVPUBLISH": "ok", "TWINE": "ok", "REPUBLICAR": "409", "OTROREGISTRO": "409", "INDICE": "200 3"}
for k, v in esperado.items():
    got = res.get(k, "(sin respuesta)")
    (dice if got.startswith(v) else falla)("%s → %s" % (k.lower(), got))
if c and not res:
    print(o[-800:])

# ── §5 ────────────────────────────────────────────────────────────────────────
print("§5 · las capas, en una rama propia")
c, j = serve("POST", "/ramas", {"nombre": "ar-medida-%d" % time.time()})
rama = j.get("rama") if c == 201 else None
if not rama:
    raise SystemExit("  ✗ la rama: %d %s" % (c, json.dumps(j)[:300]))
dice("rama %s" % rama)
PY_RAIZ, JVM_RAIZ = "packages/%s/ar-py" % B, "packages/%s/ar-jvm" % B


def declarar(version_py, version_jvm):
    pyproject = '[project]\nname = "ar-py"\nversion = "0"\ndependencies = ["banco-riesgo==%s"]\n' % version_py
    pom = ("<project><modelVersion>4.0.0</modelVersion><groupId>x</groupId><artifactId>ar-jvm</artifactId><version>0</version>"
           "<dependencies><dependency><groupId>com.banco</groupId><artifactId>motor-riesgo</artifactId><version>%s</version>"
           "</dependency></dependencies></project>\n" % version_jvm)
    for ruta, texto in ((PY_RAIZ + "/pyproject.toml", pyproject), (JVM_RAIZ + "/pom.xml", pom)):
        c, j = serve("PUT", "/arbol/" + ruta, crudo=texto, cabeceras=[("x-ore-rama", rama)])
        if c not in (200, 201):
            falla("escribir %s: %d %s" % (ruta, c, json.dumps(j)[:200]))


def capa(entorno, raiz):
    camino = "/entorno" if entorno == "python" else "/entorno/jvm"
    c, j = serve("POST", camino, cabeceras=[("x-ore-rama", rama), ("x-ore-raiz", raiz)])
    if c not in (200, 202):
        return "encolar %d %s" % (c, json.dumps(j)[:200]), {}
    job = j.get("job")
    if job:
        _, env = sh(["kubectl", "-n", NS, "get", "job", job, "-o", "jsonpath={.spec.template.spec.initContainers[*].env[?(@.name=='ORE_ARTEFACTOS_TOKEN')].value}"])
        print("    %s · Job %s · token de leer en el Job: %s" % (entorno, job, "sí" if env.startswith("ore_art_") else "NO (%s)" % env[:20]))
    for _ in range(90):
        c, j = serve("GET", camino, cabeceras=[("x-ore-rama", rama), ("x-ore-raiz", raiz)])
        if j.get("estado") in ("lista", "error"):
            return j["estado"], j
        time.sleep(10)
    return "sin terminar en 15 min", j


declarar("1.0", "1.0.0")
for entorno, raiz, coord in (("python", PY_RAIZ, "banco-riesgo==1.0"), ("jvm", JVM_RAIZ, "com.banco:motor-riesgo:1.0.0")):
    t0 = time.time()
    e, j = capa(entorno, raiz)
    texto = json.dumps(j, ensure_ascii=False)
    if e == "lista" and coord in texto and "de_artefactos" in texto:
        dice("capa %s con %s → lista en %d s, de_artefactos lo dice" % (entorno, coord, time.time() - t0))
    else:
        falla("capa %s: %s · %s" % (entorno, e, texto[:600]))
declarar("2.0", "2.0.0")
for entorno, raiz in (("python", PY_RAIZ), ("jvm", JVM_RAIZ)):
    e, j = capa(entorno, raiz)
    (dice if e == "error" else falla)("capa %s con la 2.0 (no publicada) → %s: %s" % (
        entorno, e, (json.dumps(j.get("informe", j), ensure_ascii=False)[:240])))

print("\n%s · rama %s (se queda para mirarla)" % ("AR1·5 + AR2·4 en victor: todo en verde" if not fallos else "%d fallo(s)" % len(fallos), rama))
print("  queda a mano: la huella `artefacto:publicar` en iam.huella (cita la decisión del token) y el Build que importa")
sys.exit(1 if fallos else 0)
