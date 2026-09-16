# -*- coding: utf-8 -*-
"""MEDIDA · la celda llega al gateway sin que nadie toque nada, antes de E2 (0027).

E0 lo midió con la regla aplicada a mano y E1 dejó los verbos; E2 es que la
plataforma lo lleve a donde vive lo que se obedece: la plantilla, `malla/` y el
realm. Esto mide, con lo que hay hoy, dónde cae cada pieza y qué falta:

    A  la regla       qué políticas tiene la celda, dónde se rinden, y si la de clase está
    B  la firewall    la otra mitad de la regla: existe en GCP, y si `malla/` la describe
    C  el realm       qué lleva hoy el token de agente y qué espera el gateway
    D  el gateway     la máquina `modelos`, su backend, su tenant
    E  la imagen      qué `ore-serve` corre la celda y si trae `--modelos` (I3)
    F  la aceptación  los pasos de E2 y qué los bloquea hoy

    uso:  PYTHONIOENCODING=utf-8 python pruebas-de-fuego/medida-la-celda-llega-al-gateway.py [celda]

Es un METRO: no escribe nada (acuña un token de agente para mirarlo, y lo tira).
"""
import base64
import json
import os
import re
import subprocess
import sys

CELDA = next((a for a in sys.argv[1:] if not a.startswith("--")), "victor")
NS = "t-" + CELDA
RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PROYECTO = "project-8853a180-450d-47be-b83"
MODELOS = "10.10.0.100"
IDP = "https://login.paladio.io/realms/rubix"


def leer(rel):
    with open(os.path.join(RAIZ, rel), encoding="utf-8") as f:
        return f.read()


def correr(*args):
    r = subprocess.run(list(args), capture_output=True, text=True, encoding="utf-8", env={**os.environ, "MSYS_NO_PATHCONV": "1"})
    return r.returncode, (r.stdout or ""), (r.stderr or "")


def sh(cmd):
    r = subprocess.run(cmd, shell=True, capture_output=True, text=True, encoding="utf-8")
    return r.stdout.strip() if r.returncode == 0 else ""


def kj(*args):
    rc, out, _ = correr("kubectl", *args, "-o", "json")
    return json.loads(out) if rc == 0 and out else {}


def fila(k, v):
    print("     %-48s %s" % (k, v))


print()
print("  ═══ LA CELDA LLEGA AL GATEWAY · antes de E2 · %s ═══" % CELDA)

# ── A · la regla ─────────────────────────────────────────────────────────────
print("\n  A · LA REGLA DE CLASE")
pol = kj("get", "netpol", "-n", NS).get("items", [])
nombres = sorted(p["metadata"]["name"] for p in pol)
fila("NetworkPolicy en la celda", "%d: %s" % (len(nombres), ", ".join(nombres)))
fila("`salida-al-modelo` hoy", "puesta" if "salida-al-modelo" in nombres else "NO (E0 la aplicó a mano y la retiró)")
t11 = leer("malla/11-el-inquilino.yaml")
fila("dónde se rinde la red de la celda", "`11-el-inquilino.yaml` (plantilla `demo`, %d NetworkPolicy), por `gen-inquilino.py` a cada celda" % t11.count("kind: NetworkPolicy"))
fila("MODELOS en la plantilla de red", "sí" if MODELOS in t11 else "NO: E2 añade `salida-al-modelo` a 11 — `driver` → %s/32:8000 y `control` → %s/32:9000" % (MODELOS, MODELOS))
gen = leer("malla/gen-inquilino.py")
fila("MODELOS como constante nombrada", "sí, `gen-inquilino.py` (⑬ coteja `--modelos` en 40)" if 'MODELOS = "%s"' % MODELOS in gen else "NO")
fila("lo que E2 comprueba (como ⑫)", "que 11 lleve `cidr: MODELOS/32` con 8000 para `driver` y 9000 para `control`, y nada más hacia esa IP")

# ── B · la firewall ──────────────────────────────────────────────────────────
print("\n  B · LA FIREWALL (la otra mitad)")
fw = sh("gcloud compute firewall-rules describe ore-modelos-desde-la-malla --project %s --format=json" % PROYECTO)
if fw:
    fw = json.loads(fw)
    fila("`ore-modelos-desde-la-malla` en GCP", "%s → tag %s · %s" % (",".join(fw.get("sourceRanges", [])), ",".join(fw.get("targetTags", [])), " ".join("%s:%s" % (a["IPProtocol"], ",".join(a.get("ports", []))) for a in fw.get("allowed", []))))
else:
    fila("`ore-modelos-desde-la-malla` en GCP", "✗ no existe")
addr = sh("gcloud compute addresses describe modelos --region europe-west1 --project %s --format=value(address,status)" % PROYECTO)
fila("la reserva `modelos`", addr.replace("\t", " · ") or "✗ no existe")
rc, out, _ = correr("git", "-C", RAIZ, "grep", "-l", "firewall-rules", "--", "malla", "docs")
fila("`malla/` describe la VPC (firewall, reserva, NAT)", "sí: %s" % out.strip().replace("\n", ", ") if out.strip() else "NO: la red de la máquina de modelos se creó a mano en E0 (Bastion docs/runs). E2 la escribe en `malla/` como guion idempotente, al lado de los `.sh`")

# ── C · el realm ─────────────────────────────────────────────────────────────
print("\n  C · EL REALM (lo que el token de agente lleva, y lo que el gateway espera)")
cli = sh("gcloud secrets versions access latest --secret=%s-agente-cliente --project=%s" % (NS, PROYECTO))
sec = sh("gcloud secrets versions access latest --secret=%s-agente-secreto --project=%s" % (NS, PROYECTO))
claims = {}
if cli and sec:
    r = subprocess.run(["curl", "-s", "-m", "10", "-X", "POST", IDP + "/protocol/openid-connect/token", "-d", "grant_type=client_credentials",
                        "-d", "client_id=" + cli, "--data-urlencode", "client_secret=" + sec], capture_output=True, text=True, encoding="utf-8")
    try:
        tok = json.loads(r.stdout)["access_token"]
        cuerpo = tok.split(".")[1]
        claims = json.loads(base64.urlsafe_b64decode(cuerpo + "=" * (-len(cuerpo) % 4)))
    except (ValueError, KeyError, IndexError):
        claims = {}
if claims:
    fila("`aud`", json.dumps(claims.get("aud")))
    fila("`azp` · `rubix_tipo` · `rubix_celda`", "%s · %s · %s" % (claims.get("azp"), claims.get("rubix_tipo"), claims.get("rubix_celda", "AUSENTE")))
    fila("vida", "%d s" % (claims.get("exp", 0) - claims.get("iat", 0)))
else:
    fila("token de agente", "✗ no se pudo acuñar (¿gcloud sin sesión, IdP caído?)")
fila("lo que el gateway acepta hoy (E0)", "`--oidc-audience ore-serve`, y la celda de `azp = ore-agente-<celda>` (0027 ② lo permite hasta que el realm emita `rubix_celda`)")
realm = leer("malla/gen-realm.py")
fila("`gen-realm.py` conoce `modelos`", "sí" if "modelos" in realm else "NO: E2 añade la audiencia `modelos` a los clientes de agente")
aprov = leer("malla/aprovisionar-inquilino.sh")
fila("el mapeador `rubix_celda` en el cliente de agente (⑦)", "sí" if "rubix_celda" in aprov else "NO: E2 lo añade en el paso ⑦ (`oidc-hardcoded-claim-mapper`, como `rubix_tipo`) — un cliente por celda, el claim es la celda")
fila("y entonces el gateway", "`--oidc-audience modelos`, y `rubix_celda` manda sobre `azp`")

# ── D · el gateway ───────────────────────────────────────────────────────────
print("\n  D · EL GATEWAY (la máquina `modelos`)")
vm = sh("gcloud compute instances describe modelos-e0 --zone europe-west1-b --project %s --format=value(status,machineType.basename(),networkInterfaces[0].networkIP)" % PROYECTO)
fila("`modelos-e0`", vm.replace("\t", " · ") or "✗ no existe")
if vm.startswith("RUNNING"):
    rc, out, _ = correr("kubectl", "run", "e2-sonda-%d" % (os.getpid() % 10000), "-n", "default", "--rm", "-i", "--restart=Never", "--quiet",
                        "--image=europe-west1-docker.pkg.dev/%s/ore/ore-drivers:main" % PROYECTO, "--command", "--", "sh", "-c",
                        "curl -s -m 6 http://%s:9000/admin/health; echo; curl -s -m 6 http://%s:9000/admin/tenants" % (MODELOS, MODELOS))
    for l in out.strip().splitlines()[:2]:
        fila("  desde la VPC", l[:150])
else:
    fila("  backend / tenants", "(parada: al arrancar vuelve con el vLLM de mentira como backend `eu-dc` y el tenant `%s` que E0/E1 dejaron)" % CELDA)
fila("lo que E2 decide de la máquina", "sigue en la `e2-micro` con el stub hasta la cuota G4 (0028 hito 3); un g1 `community` sólo para medir (b)")

# ── E · la imagen y la convergencia ──────────────────────────────────────────
print("\n  E · LA IMAGEN Y LA CONVERGENCIA (I3 en la celda)")
dep = kj("get", "deploy", "ore-serve", "-n", NS)
args = dep.get("spec", {}).get("template", {}).get("spec", {}).get("containers", [{}])[0].get("args", [])
fila("`ore-serve` de %s lleva `--modelos`" % CELDA, ("sí: %s" % args[args.index("--modelos") + 1]) if "--modelos" in args else "NO todavía: el convergedor lo rinde cuando Flux aplique la `malla` con 40 nuevo")
anot = dep.get("spec", {}).get("template", {}).get("metadata", {}).get("annotations", {})
commit = next((v for k, v in anot.items() if "commit" in k or "sha" in k), "?")
fila("commit que corre en el pod", str(commit)[:12])
malla = kj("get", "gitrepository", "malla", "-n", "flux-system").get("status", {}).get("artifact", {}).get("revision", "?")
fila("`malla` que Flux ve", malla.split(":")[-1][:12])
local = sh("git -C %s rev-parse HEAD" % RAIZ)[:12]
fila("HEAD local", local + (" (= lo que Flux ve)" if local and local in malla else " (Flux aún no lo ve: CI/Flux en camino)"))
rc, out, _ = correr("gh", "run", "list", "--repo", "describeloai/ore", "--limit", "1", "--json", "status,conclusion,headSha")
try:
    run = json.loads(out)[0]
    fila("CI del último push", "%s %s (%s)" % (run["status"], run.get("conclusion") or "", run["headSha"][:12]))
except (ValueError, IndexError, KeyError):
    fila("CI del último push", "(sin `gh`)")

# ── F · la aceptación ────────────────────────────────────────────────────────
print("\n  F · LA ACEPTACIÓN DE E2, Y QUÉ LA BLOQUEA HOY")
pasos = [
    ("`POST /modelos` desde `curl` a `https://%s.ore.paladio.io` con el token de agente → 201" % CELDA,
     "necesita: la imagen con I3 (CI), `--modelos` rendido (convergedor), la regla `control → 9000` y `perfiles.json` en la cola (I2 ✓), y la máquina encendida"),
    ("la primera llamada de un Job contesta en < 60 s sin tocar `kubectl`",
     "necesita: la regla `driver → 8000` en la plantilla (hoy a mano en E0: ≤ 4–6 s desde el POST)"),
    ("`DELETE /modelos/{n}` → el Job recibe 401",
     "el gateway ya lo hace (Bastion `9df1f32`: sin suscripción, 401 aunque el tenant siga)"),
    ("la celda de al lado, sin `Model`, → 401",
     "el gateway ya lo hace (E0 (c)); medirlo con el token de `demo` o `prueba`"),
    ("`--cotejar` de E0 limpio con la regla ya en la plantilla",
     "el informador no cambia; la regla no añade pods"),
]
for que, bloqueo in pasos:
    fila("· " + que, "")
    fila("  ", bloqueo)
print()
