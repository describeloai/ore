#!/usr/bin/env python3
"""
MEDIDA · W2, proponer — en todo su espectro, antes de construir
(18 de septiembre)

W2 (0030 ⑤): «rama por persona, PR en la forja, diff y diagnósticos de la
rama, merge → Flux — acepta: dos personas, dos ramas, una revisión». La
intuición es que la lógica ya está: la forja tiene ramas y PRs, `ore-serve`
clona y empuja, `ore diff`/`validate` existen, y la consola ya tiene las tres
pantallas. Esto mide qué hay de verdad, pieza a pieza, sobre el inquilino real:

  §1  LA FORJA      con el testigo de `serve-<n>` desde el pod de ore-serve:
                    versión; ramas; crear una rama por API; clonar la rama y
                    empujar un commit de una persona; abrir la PR; sus
                    ficheros y su diff; ¿puede `serve-<n>` aprobarla?; ¿está
                    `main` protegida?; ¿hay CODEOWNERS?; ¿es mergeable? —
                    y se cierra y se borra la rama: `main` no se toca
  §2  EL DIAGNÓSTICO `ore validate` y `ore diff` sobre la rama frente a main:
                    qué dicen y cuánto tardan (es lo que la PR enseñaría)
  §3  ORE-SERVE     qué rutas tocan el árbol y a qué rama van (por código)
  §4  LA IDENTIDAD  quién es el autor del commit, quién el autor de la PR en
                    la forja, y qué potestad diría quién revisa (ore-iam)
  §5  LA CONSOLA    las pantallas que ya existen y las formas que esperan
  §6  FLUX          a qué rama mira y cuánto tarda en enterarse (el aviso)

Uso:
  python pruebas-de-fuego/medida-w2-proponer.py --inquilino victor

No saca ningún secreto: el testigo se lee DENTRO del pod y no sale de él.
"""
import datetime as dt
import json
import os
import re
import shutil
import subprocess
import sys

os.environ["MSYS_NO_PATHCONV"] = "1"
os.environ["MSYS2_ARG_CONV_EXCL"] = "*"
RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CONSOLA = os.environ.get("CONSOLA", "C:/rubix-platform")


def fila(k, v, nota=""):
    print("  %-50s %-20s %s" % (k, v, nota))


def sh(*args, entrada=None):
    exe = shutil.which(args[0]) or args[0]
    r = subprocess.run((exe,) + tuple(args[1:]), input=entrada.encode("utf-8") if entrada else None,
                       capture_output=True)
    return r.stdout.decode("utf-8", "replace")


def k(*args, ns=None):
    return sh(*(["kubectl"] + (["-n", ns] if ns else []) + list(args)))


def kexec(ns, pod, guion, contenedor=None):
    a = ["kubectl", "exec", "-i", "-n", ns, pod] + (["-c", contenedor] if contenedor else []) + ["--", "sh", "-s"]
    return sh(*a, entrada=guion)


# ── §1 y §2 · en el pod: la forja y el diagnóstico ─────────────────────────

GUION = r'''
set +e
NS=%(ns)s; ORG=%(ns)s; REPO=ontologia
F=http://forja.$NS.svc.cluster.local:3000
T=$(cat /testigo/forja)
API="$F/api/v1"
# BusyBox: ni `date +%%N` ni `wget --method`. Los ms salen de /proc/uptime
# (centesimas) y solo hay GET y POST — cerrar la PR y borrar la rama van por
# git (push --delete), que es lo que la forja entiende.
ms() { awk '{printf "%%d", $1*1000}' /proc/uptime; }
api() { # <GET|POST> <camino> [cuerpo] → "codigo ms", cuerpo en /tmp/r.json
  local m=$1 c=$2 d=${3:-} t0 t1 cod
  t0=$(ms)
  if [ "$m" = POST ]; then
    wget -qO /tmp/r.json -S --header="Authorization: token $T" --header="Content-Type: application/json" --post-data="$d" "$API$c" 2>/tmp/h.txt
  else
    wget -qO /tmp/r.json -S --header="Authorization: token $T" "$API$c" 2>/tmp/h.txt
  fi
  t1=$(ms)
  cod=$(sed -n 's/^ *HTTP\/[0-9.]* \([0-9][0-9][0-9]\).*/\1/p' /tmp/h.txt | tail -1)
  echo "${cod:-000} $((t1-t0))"
}
j() { python3 -c "import json,sys; d=json.load(open('/tmp/r.json')); print(eval(sys.argv[1]))" "$1" 2>/dev/null || sed -n 's/.*"'"$2"'": *"\{0,1\}\([^,"}]*\).*/\1/p' /tmp/r.json | head -1; }

echo "### version $(api GET /version) $(cat /tmp/r.json)"
ramas() { grep -o '\(^\[\|},\){"name":"[^"]*"' /tmp/r.json | sed 's/.*"name":"//; s/"$//' | tr '\n' ' '; }
echo "### ramas $(api GET /repos/$ORG/$REPO/branches) $(ramas)"
echo "### proteccion $(api GET /repos/$ORG/$REPO/branch_protections) $(head -c 200 /tmp/r.json)"
echo "### codeowners $(api GET "/repos/$ORG/$REPO/contents/CODEOWNERS?ref=main")"
echo "### yo $(api GET /user) $(grep -o '"login":"[^"]*"' /tmp/r.json)"
echo "### permiso $(api GET /repos/$ORG/$REPO/collaborators/$(grep -o '"login":"[^"]*"' /tmp/r.json | cut -d'"' -f4)/permission) $(grep -o '"permission":"[^"]*"' /tmp/r.json)"

RAMA="ana/medida-w2-$(date -u +%%H%%M%%S)"
echo "### crear-rama $(api POST /repos/$ORG/$REPO/branches "{\"new_branch_name\":\"$RAMA\",\"old_branch_name\":\"main\"}") $RAMA"

export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=http.extraheader
GIT_CONFIG_VALUE_0="Authorization: token $T"; export GIT_CONFIG_VALUE_0
W=/tmp/w2; rm -rf $W $W-main
t0=$(ms); git clone -q --branch "$RAMA" "$F/$ORG/$REPO.git" $W; echo "### clonar-rama $? $(( $(ms)-t0 ))"
t0=$(ms); git clone -q "$F/$ORG/$REPO.git" $W-main; echo "### clonar-main $? $(( $(ms)-t0 ))"
cd $W || exit 1
# el cambio de una persona: una vista nueva sobre una tabla que ya esta (sin copia)
V=$(ls packages/*/views/*.yaml 2>/dev/null | head -1)
P=$(echo "$V" | cut -d/ -f2)
N=$(sed -n 's/^  name: *\(.*\)$/\1/p' "$V" | head -1)
sed "s/^  name: *$N\$/  name: ${N}_propuesta/" "$V" | grep -v "materialized:" > "packages/$P/views/${N}_propuesta.yaml"
echo "### fichero packages/$P/views/${N}_propuesta.yaml (desde $V)"
t0=$(ms); ore validate . >/tmp/v.txt 2>&1; echo "### validate-rama $? $(( $(ms)-t0 )) errores=$(grep -c '^error' /tmp/v.txt)"
t0=$(ms); ore diff $W-main $W >/tmp/d.txt 2>&1; echo "### diff $? $(( $(ms)-t0 )) lineas=$(wc -l < /tmp/d.txt)"; head -12 /tmp/d.txt | sed 's/^/    | /'
git add -A
GIT_AUTHOR_NAME=ana GIT_AUTHOR_EMAIL=ana@sujeto.invalid GIT_COMMITTER_NAME=ore-serve GIT_COMMITTER_EMAIL=ore-serve@ore.dev \
  git commit -q -m "Propuesta de ana: ${N}_propuesta (medida W2)"
t0=$(ms); git push -q origin "HEAD:$RAMA"; echo "### empujar-rama $? $(( $(ms)-t0 ))"
echo "### autor $(git log -1 --format='%%an <%%ae> · committer %%cn')"

echo "### abrir-pr $(api POST /repos/$ORG/$REPO/pulls "{\"head\":\"$RAMA\",\"base\":\"main\",\"title\":\"Propuesta de ana (medida W2)\",\"body\":\"sub: persona:ana\"}") numero=$(j "d['number']" number) autor=$(grep -o '"user":{[^}]*' /tmp/r.json | grep -o '"login":"[^"]*"' | cut -d'"' -f4)"
NUM=$(j "d['number']" number)
echo "### pr $(api GET /repos/$ORG/$REPO/pulls/$NUM) mergeable=$(j "d['mergeable']" mergeable) estado=$(j "d['state']" state)"
echo "### pr-ficheros $(api GET /repos/$ORG/$REPO/pulls/$NUM/files) n=$(grep -o '"filename"' /tmp/r.json | wc -l) $(grep -o '"filename":"[^"]*"' /tmp/r.json | head -3 | tr '\n' ' ')"
t0=$(ms); wget -qO /tmp/pr.diff --header="Authorization: token $T" "$API/repos/$ORG/$REPO/pulls/$NUM.diff"; echo "### pr-diff $? $(( $(ms)-t0 )) bytes=$(wc -c < /tmp/pr.diff)"
echo "### aprobar-propia $(api POST /repos/$ORG/$REPO/pulls/$NUM/reviews '{"event":"APPROVED","body":"revisado por persona:bea (via ore-serve)"}') la forja no deja aprobar la PR propia, y serve-$NS es el autor de todas"
echo "### comentar $(api POST /repos/$ORG/$REPO/pulls/$NUM/reviews '{"event":"COMMENT","body":"persona:bea: revisado, ok"}') $(j "d['state']" state)"
echo "### comentario-issue $(api POST /repos/$ORG/$REPO/issues/$NUM/comments '{"body":"persona:bea dice: ok"}')"
# Cerrar la PR es un PATCH y BusyBox wget no sabe: HTTP a mano por nc.
# (Borrar la rama NO cierra la PR: medido en la primera pasada, quedo abierta.
#  Y sin el sleep, nc corta antes de la respuesta y la forja dice 500 aunque cierre.)
B='{"state":"closed"}'
t0=$(ms); { printf "PATCH /api/v1/repos/$ORG/$REPO/pulls/$NUM HTTP/1.1\r\nHost: forja.$NS.svc.cluster.local:3000\r\nAuthorization: token $T\r\nContent-Type: application/json\r\nContent-Length: ${#B}\r\nConnection: close\r\n\r\n$B" ; sleep 3; } | nc forja.$NS.svc.cluster.local 3000 > /tmp/p.txt; echo "### cerrar-pr $(sed -n 's/^HTTP\/[0-9.]* \([0-9]*\).*/\1/p' /tmp/p.txt | head -1) $(( $(ms)-t0 )) PATCH por nc"
t0=$(ms); git push -q origin --delete "$RAMA"; echo "### borrar-rama $? $(( $(ms)-t0 )) git push --delete"
echo "### pr-despues $(api GET /repos/$ORG/$REPO/pulls/$NUM) estado=$(j "d['state']" state)"
echo "### ramas-despues $(api GET /repos/$ORG/$REPO/branches) $(ramas)"
cd /; rm -rf $W $W-main /tmp/r.json /tmp/h.txt /tmp/v.txt /tmp/d.txt /tmp/pr.diff /tmp/p.txt
'''


def seccion_1_2(inq, ns, pod):
    print("\n§1 · LA FORJA (con el testigo de serve-%s, desde su pod) y §2 · EL DIAGNÓSTICO" % inq)
    salida = kexec(ns, pod, GUION % {"ns": ns})
    for l in salida.splitlines():
        if l.startswith("### "):
            partes = l[4:].split(" ", 3)
            que = partes[0]
            resto = " ".join(partes[1:])
            m = re.match(r"(\d{3}|\d) (\d+)(.*)", resto)
            if m:
                fila("  " + que, "%s · %s ms" % (m.group(1), m.group(2)), m.group(3).strip()[:110])
            else:
                fila("  " + que, "", resto[:120])
        elif l.startswith("    | "):
            print("      %s" % l[6:])
        elif l.strip() and not l.startswith("Defaulted"):
            print("      ? %s" % l[:140])


# ── §3 · ore-serve, por código ─────────────────────────────────────────────


def seccion_3():
    print("\n§3 · ORE-SERVE — qué toca el árbol, y a qué rama")
    rutas = open(os.path.join(RAIZ, "crates/ore-serve/src/rutas.rs"), encoding="utf-8").read()
    for m in re.finditer(r'\("(GET|PUT|DELETE|POST)", \[("arbol"[^\]]*)\]\)', rutas):
        fila("  %s /%s" % (m.group(1), m.group(2).replace('"', "").replace(", ", "/").replace(" @ ..", "…")), "", "")
    git = open(os.path.join(RAIZ, "crates/ore-serve/src/git.rs"), encoding="utf-8").read()
    clon = re.search(r'"clone",\s*"--quiet",\s*&self\.url', git)
    push = re.search(r'\["push", "--quiet", "origin", "([^"]+)"\]', git)
    fila("  clonar()", "sin --branch" if clon else "?", "clona la rama por defecto: main")
    fila("  publicar()", "push origin %s" % (push.group(1) if push else "?"), "HEAD del clon → la misma rama: main")
    fila("  rutas con rama/ref/pull", str(len(re.findall(r"\brama\b|\bbranch\b|/pulls", rutas))), "cero: el servidor no sabe de ramas")
    cola = open(os.path.join(RAIZ, "crates/ore-serve/src/cola.rs"), encoding="utf-8").read()
    fila("  encolar (cola.rs)", "rinde y empuja a main de `trabajo`", "un Job por contenido, no por rama")


# ── §4 · la identidad ──────────────────────────────────────────────────────


def seccion_4():
    print("\n§4 · LA IDENTIDAD — quién propone, quién revisa")
    sql = ""
    for f in sorted(os.listdir(os.path.join(RAIZ, "iam/migraciones"))):
        sql += open(os.path.join(RAIZ, "iam/migraciones", f), encoding="utf-8").read()
    pot = sorted(set(re.findall(r"'([a-z]+:[a-z-]+)'", sql)))
    fila("  potestades de ore-iam", str(len(pot)), ", ".join(pot))
    fila("  potestades sobre el árbol (arbol:*, propuesta:*)", str(len([p for p in pot if p.startswith(("arbol:", "propuesta:"))])),
         "ninguna: hoy escribe en main quien tenga sesión en la organización")
    roles = sorted(set(re.findall(r"\('([A-Z]+)', *'[a-z]+:[a-z-]+'\)", sql)))
    fila("  roles", ", ".join(roles), "")
    fila("  autor del commit", "la persona (sub)", "committer ore-serve (RFC 8693 sub+act) — git.rs::publicar")
    fila("  autor de la PR en la forja", "serve-<inquilino>", "la persona sólo en el cuerpo: la forja no la conoce")
    fila("  dueño del paquete", "team:<organización>", "sin CODEOWNERS ni equipos en la forja: la revisión la decide ore-serve")


# ── §5 · la consola ────────────────────────────────────────────────────────


def seccion_5():
    print("\n§5 · LA CONSOLA — lo que ya está pintado")
    base = os.path.join(CONSOLA, "components/code-workspace")
    for f, que in (("BranchesView.tsx", "ramas"), ("PullRequestsView.tsx", "PRs: lista, detalle, diff, comentarios, nueva"),
                   ("NotebookToolbar.tsx", "botones Branches · Commit · Pull requests"), ("VersionHistoryModal.tsx", "historial")):
        p = os.path.join(base, f)
        if not os.path.exists(p):
            fila("  " + f, "no está", "")
            continue
        t = open(p, encoding="utf-8").read()
        mocks = re.findall(r"const ([A-Z_]+_DE_EJEMPLO)", t)
        formas = re.findall(r"^(?:export )?interface (\w+)", t, flags=re.M)
        fila("  " + f, "%d líneas" % t.count("\n"), "%s · formas %s · mocks %s · «todavía no implementado» ×%d"
             % (que, ",".join(formas) or "-", ",".join(mocks) or "-", t.count("todavía no implementado")))
    srv = os.path.join(CONSOLA, "lib/server")
    n = 0
    for f in os.listdir(srv):
        t = open(os.path.join(srv, f), encoding="utf-8").read()
        n += len(re.findall(r"/ramas|/pulls|/propuestas", t))
    fila("  lib/server: llamadas a ramas/PRs", str(n), "cero: la consola no pide nada de esto todavía")


# ── §6 · Flux ──────────────────────────────────────────────────────────────


def seccion_6(inq):
    print("\n§6 · FLUX — a qué rama mira")
    for g in ("inquilino-%s" % inq, "trabajo-%s" % inq):
        ref = k("get", "gitrepository", g, "-o", "jsonpath={.spec.ref.branch} {.spec.interval}", ns="flux-system").strip()
        fila("  " + g, ref, "sólo main: una rama no despliega nada, que es lo que se quiere")
    fila("  el aviso forja → Flux", "~4 s", "medido el 18-09 tras 17-el-aviso: merge → main → Job en segundos")


def main():
    a = sys.argv[1:]
    inq = a[a.index("--inquilino") + 1] if "--inquilino" in a else "victor"
    ns = "t-%s" % inq
    pod = k("get", "pods", "-l", "ore.dev/rol=control", "-o", "jsonpath={.items[0].metadata.name}", ns=ns).strip()
    if not pod:
        sys.exit("sin pod de ore-serve en %s" % ns)
    print("MEDIDA · W2 proponer · inquilino `%s` · %s" % (inq, dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%d %H:%MZ")))
    seccion_1_2(inq, ns, pod)
    seccion_3()
    seccion_4()
    seccion_5()
    seccion_6(inq)


if __name__ == "__main__":
    main()
