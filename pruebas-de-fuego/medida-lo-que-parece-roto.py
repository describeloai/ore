#!/usr/bin/env python3
"""
MEDIDA · lo que hace que una base nueva parezca rota durante minutos
(18 de septiembre)

W1 contesta: `standard_postgre_3.products` devolvió 20 filas en 1,6 s desde
la copia. Pero entre «alta de una base» y esa respuesta pasaron NUEVE minutos
en los que la consola decía «The copy is not made yet» con un botón de copiar.
Cuatro sospechas, una medida cada una, todas sobre el clúster real:

  §1  EL AVISO   la forja del inquilino no avisa a Flux: ¿existe el webhook?
                 ¿entrega? ¿quién lo bloquea? Y cuánto tarda hoy un empujón a
                 la cola en ser un Job (commit en `trabajo` → artefacto de Flux)
  §2  EL FRÍO    el pool `jobs-p` arranca de cero por cada Job: cuánto pasa
                 entre crear el Job y que su contenedor trabaje, y cuánto de
                 eso es el nodo, la imagen y los `initContainers`
  §3  EL PANEL   la ventana en la que `ask` contesta 409 «no está hecha»
                 mientras la copia ESTÁ en la cola o corriendo — y qué sabe el
                 servidor en ese momento que no dice
  §4  EL BUNDLE  la cabecera del recibo lleva el digest del árbol ENTERO: qué
                 commits lo cambian (y por tanto invalidan TODOS los recibos)
                 aunque no toquen la vista copiada

Uso:
  python pruebas-de-fuego/medida-lo-que-parece-roto.py --inquilino victor [--horas 6]

Hace falta `kubectl` con el contexto de la malla y `gcloud` con sesión. Corre
`ore compile` DENTRO del pod de `ore-serve` del inquilino (es donde está el
testigo de la forja); no saca ningún secreto: del webhook sólo el host.
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


def fila(k, v, nota=""):
    print("  %-46s %-22s %s" % (k, v, nota))


def sh(*args, entrada=None):
    # En bytes, no en texto: en Windows el modo texto pone retorno de carro a
    # cada salto de linea y el guion que va por stdin le llega al pod con CR.
    exe = shutil.which(args[0]) or args[0]
    r = subprocess.run((exe,) + tuple(args[1:]), input=entrada.encode("utf-8") if entrada else None,
                       capture_output=True)
    return r.stdout.decode("utf-8", "replace")


def k(*args, ns=None):
    a = ["kubectl"] + (["-n", ns] if ns else []) + list(args)
    return sh(*a)


def kexec(ns, pod, guion, contenedor=None):
    a = ["kubectl", "exec", "-i", "-n", ns, pod] + (["-c", contenedor] if contenedor else []) + ["--", "sh", "-s"]
    return sh(*a, entrada=guion)


def fecha(s):
    s = s.replace("Z", "+00:00")
    return dt.datetime.fromisoformat(s)


def segundos(a, b):
    return int((fecha(b) - fecha(a)).total_seconds())


# ── §1 · el aviso ──────────────────────────────────────────────────────────


def seccion_1(inq, ns, horas):
    print("\n§1 · EL AVISO — ¿la forja de `%s` avisa a Flux?" % inq)
    # ① Quién puede llamar al receptor (la NetworkPolicy que 17-el-aviso recorta).
    np_ = json.loads(k("get", "networkpolicy", "allow-webhooks", "-o", "json", ns="flux-system") or "{}")
    desde = []
    for regla in np_.get("spec", {}).get("ingress", []):
        for f in regla.get("from", []):
            sel = f.get("namespaceSelector", {}).get("matchLabels", {})
            desde.append(sel.get("kubernetes.io/metadata.name") or json.dumps(sel))
    fila("receptor: entrada permitida desde", ", ".join(desde) or "(nadie)",
         "" if ns in desde else "← `%s` NO está: la forja del inquilino no lo alcanza" % ns)
    # ② Los webhooks que hay en cada forja, y sus entregas (sqlite de Forgejo).
    consulta = (
        "sqlite3 /data/gitea/gitea.db \"select w.id, w.is_active, substr(w.url,1,60), "
        "(select count(*) from hook_task t where t.hook_id=w.id), "
        "(select coalesce(sum(is_succeed),0) from hook_task t where t.hook_id=w.id), "
        "(select substr(response_content,1,400) from hook_task t where t.hook_id=w.id order by delivered desc limit 1) "
        "from webhook w;\"\n"
    )
    for forja_ns, quien in ((ns, "forja del inquilino"), ("forja", "forja de la plataforma")):
        salida = kexec(forja_ns, "forja-0", consulta, contenedor="forgejo").strip()
        filas_ = [l for l in salida.splitlines() if "|" in l]
        fila("%s (%s/forja-0): webhooks" % (quien, forja_ns), str(len(filas_)),
             "" if filas_ else "← ninguno: los empujones no avisan a nadie")
        for l in filas_:
            wid, activo, url, entregas, ok, ultima = (l.split("|", 5) + [""] * 6)[:6]
            host = re.sub(r"^https?://([^/]*)/.*$", r"\1", url) or "(url vacía)"
            m = re.search(r'"status":(\d+)', ultima or "")
            causa = ""
            if "deadline" in (ultima or ""):
                causa = "context deadline exceeded (nadie contesta: red)"
            elif m:
                causa = "último HTTP %s" % m.group(1)
            fila("  hook %s → %s" % (wid, host), "%s entregas · %s ok" % (entregas, ok), causa)
    # ③ El aprovisionador: qué pone en /puesto, y qué dijo del hook.
    cj = k("get", "cronjob", "aprovisionador", "-o", "json", ns="ore-system")
    pone = sorted(set(re.findall(r"--out-file=/puesto/([a-z-]+)", cj)))
    fila("aprovisionador: deja en /puesto", ", ".join(pone),
         "" if "receptor-url" in pone else "← sin `receptor-url`: RECEPTOR=\"\" y el hook se pide con url vacía")
    jobs_ap = json.loads(k("get", "jobs", "-o", "json", ns="ore-system") or "{}").get("items", [])
    completos = [j["metadata"]["name"] for j in jobs_ap
                 if j["metadata"]["name"].startswith("aprovisionador-") and j.get("status", {}).get("succeeded")]
    for ultimo in sorted(completos)[::-1]:
        log = k("logs", "job/%s" % ultimo, ns="ore-system")
        if ("APROVISIONAR `%s`" % inq) not in log:
            continue
        bloque = log.split("APROVISIONAR `%s`" % inq)[-1]
        m = re.search(r"avisara a Flux en cada empujon · (\d+)", bloque)
        fila("  última pasada (%s): hook de `%s`" % (ultimo, inq), "HTTP %s" % (m.group(1) if m else "?"),
             "422 y lo marca ✓: la idempotencia tapa el fallo" if m and m.group(1) == "422" else "")
        break
    # ④ Sin aviso: cuánto tarda un empujón a la cola en ser artefacto de Flux.
    print("  — commit en `trabajo` → artefacto de Flux (últimas %d h; sin aviso es el sondeo de 5 min) —" % horas)
    commits = kexec(ns, "forja-0",
                    "cd /data/git/repositories/%s/trabajo.git && git -c safe.directory='*' log --format='%%cI %%s' -40\n" % ns,
                    contenedor="forgejo")
    ts = [(l.split(" ", 1)[0], l.split(" ", 1)[1][:48]) for l in commits.splitlines() if l[:4].isdigit()]
    logs = k("logs", "deploy/source-controller", "--since=%dh" % horas, ns="flux-system")
    arte = []
    for l in logs.splitlines():
        if '"name":"trabajo-%s"' % inq in l and "stored artifact for commit" in l:
            m = re.search(r'"ts":"([^"]+)"', l)
            if m:
                arte.append(m.group(1))
    arte.sort()
    desde_ = fecha(arte[0]) if arte else None
    lat = []
    for t, msg in reversed(ts):
        if desde_ is None or fecha(t) < desde_:
            continue
        sig = next((a for a in arte if fecha(a) >= fecha(t)), None)
        if sig:
            lat.append(segundos(t, sig))
            fila("  %s %s" % (t[11:19], msg), "%d s" % lat[-1])
    if lat:
        fila("  latencia commit → artefacto", "media %d s · máx %d s" % (sum(lat) // len(lat), max(lat)),
             "con el aviso serían segundos (17-el-aviso lo midió: ~2 min de ciclo, 90 % el nodo)")


# ── §2 · el frío ───────────────────────────────────────────────────────────


def seccion_2(inq, ns, horas):
    print("\n§2 · EL FRÍO — el pool de Jobs arranca de cero")
    pool = sh("gcloud", "container", "node-pools", "describe", "jobs-p", "--cluster", "ore-mesh", "--zone",
              "europe-west1-b", "--format=json")
    try:
        p = json.loads(pool)
        a = p.get("autoscaling", {})
        fila("jobs-p", "%s · spot=%s" % (p["config"]["machineType"], p["config"].get("spot", False)),
             "min %s · max %s · taint %s" % (a.get("minNodeCount", 0), a.get("maxNodeCount"),
                                            ",".join(t["key"] for t in p["config"].get("taints", []))))
    except (ValueError, KeyError):
        fila("jobs-p", "?", pool.strip()[:80])
    perfil = sh("gcloud", "container", "clusters", "describe", "ore-mesh", "--zone", "europe-west1-b",
                "--format=value(autoscaling.autoscalingProfile)").strip()
    fila("perfil de autoescalado", perfil, "OPTIMIZE_UTILIZATION baja el nodo en cuanto sobra")
    ev = json.loads(k("get", "events", "-o", "json", ns=ns) or "{}").get("items", [])
    por_pod = {}
    for e in ev:
        o = e.get("involvedObject", {})
        n = o.get("name", "")
        if o.get("kind") != "Pod" or not re.match(r"(copiar|catalogo)-", n):
            continue
        t = e.get("lastTimestamp") or e.get("eventTime") or ""
        por_pod.setdefault(n, []).append((t, e.get("reason", ""), e.get("message", "")))
    jobs = {j["metadata"]["name"]: j for j in json.loads(k("get", "jobs", "-o", "json", ns=ns) or "{}").get("items", [])}
    print("  — por Job: creado → nodo pedido → programado → contenedor de trabajo → terminado —")
    frios = []
    for pod, evs in sorted(por_pod.items(), key=lambda x: min(t for t, _, _ in x[1])):
        evs.sort()
        job = jobs.get(pod.rsplit("-", 1)[0])
        if not job:
            continue
        creado = job["metadata"]["creationTimestamp"]
        fin = job.get("status", {}).get("completionTime")
        pedido = next((t for t, r, _ in evs if r == "TriggeredScaleUp"), None)
        programado = next((t for t, r, _ in evs if r == "Scheduled"), None)
        arranques = [t for t, r, _ in evs if r == "Started"]
        # Los eventos viven una hora: un Job más viejo no se puede medir.
        if not programado or not arranques:
            fila("  %s" % pod.rsplit("-", 1)[0], "eventos caducados", "(los eventos viven una hora)")
            continue
        trabajo = arranques[-1] if arranques else None
        pulls = [re.search(r"in ([\d.]+)s", m) for _, r, m in evs if r == "Pulled"]
        pull = max((float(m.group(1)) for m in pulls if m), default=0)
        partes = []
        if pedido:
            partes.append("nodo %ds" % segundos(pedido, programado) if programado else "nodo ?")
        else:
            partes.append("nodo ya estaba")
        if programado and trabajo:
            partes.append("imagen+init %ds (pull %.0fs)" % (segundos(programado, trabajo), pull))
        total = segundos(creado, trabajo) if trabajo else None
        if total is not None:
            frios.append((bool(pedido), total))
        fila("  %s" % pod.rsplit("-", 1)[0], "%ss hasta trabajar" % (total if total is not None else "?"),
             " · ".join(partes) + (" · trabajo %ds" % segundos(trabajo, fin) if trabajo and fin else ""))
    f = [t for p, t in frios if p]
    c = [t for p, t in frios if not p]
    if f:
        fila("  en frío (nodo pedido)", "media %d s" % (sum(f) // len(f)), "%d Job(s)" % len(f))
    if c:
        fila("  en caliente (nodo ya estaba)", "media %d s" % (sum(c) // len(c)), "%d Job(s)" % len(c))


# ── §3 · el panel ──────────────────────────────────────────────────────────


def seccion_3(inq, ns, pod_serve):
    print("\n§3 · EL PANEL — cuánto dura «not made yet» mientras la copia está en marcha")
    guion = (
        "W=/tmp/medida-panel; rm -rf $W\n"
        "export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=http.extraheader\n"
        "GIT_CONFIG_VALUE_0=\"Authorization: token $(cat /testigo/forja)\"; export GIT_CONFIG_VALUE_0\n"
        "git clone -q http://forja.%s.svc.cluster.local:3000/%s/ontologia.git $W && cd $W || exit 1\n"
        "git log --format='COMMIT %%cI %%s' --name-only -60\n"
        "cd /; rm -rf $W\n" % (ns, ns)
    )
    # Cada commit con sus ficheros: el alta de una base se reconoce por el
    # paquete que estrena, y su informe por el `copias/<paquete>_*.json` que trae.
    commits, actual = [], None
    for l in kexec(ns, pod_serve, guion).splitlines():
        if l.startswith("COMMIT "):
            _, t, msg = l.split(" ", 2)
            actual = {"t": t, "msg": msg, "ficheros": []}
            commits.append(actual)
        elif l.strip() and actual is not None:
            actual["ficheros"].append(l.strip())
    commits.reverse()
    ventanas = []
    for i, c in enumerate(commits):
        if not c["msg"].startswith("alta de una base"):
            continue
        paquetes = {f.split("/")[1] for f in c["ficheros"] if f.startswith("packages/") and f.count("/") >= 2}
        if not paquetes:
            continue
        paq = sorted(paquetes)[0]
        fin = next((d for d in commits[i + 1:]
                    if any(f.startswith("copias/%s_" % paq) for f in d["ficheros"])), None)
        retirada = next((d for d in commits[i + 1:] if d["msg"].startswith("retirar la base `%s`" % paq)), None)
        if fin and (not retirada or fecha(fin["t"]) < fecha(retirada["t"])):
            ventanas.append(segundos(c["t"], fin["t"]))
            fila("  %s %s → informe %s" % (c["t"][11:19], paq[:22], fin["t"][11:19]), "%d s" % ventanas[-1],
                 "todo ese tiempo `ask` → 409 «no está hecha» + botón Copy")
        elif retirada:
            fila("  %s %s" % (c["t"][11:19], paq[:22]), "retirada a las %s" % retirada["t"][11:19], "sin informe antes de irse")
        else:
            fila("  %s %s" % (c["t"][11:19], paq[:22]), "sin informe todavía")
    if ventanas:
        fila("  ventana media", "%d s" % (sum(ventanas) // len(ventanas)), "%d alta(s) con informe" % len(ventanas))
    # Qué sabe el servidor en esa ventana (por código, no por medida): la cola.
    cola = kexec(ns, "forja-0",
                 "cd /data/git/repositories/%s/trabajo.git && git -c safe.directory='*' ls-tree --name-only HEAD | grep '^48-'\n" % ns,
                 contenedor="forgejo").split()
    fila("  la cola hoy tiene", ", ".join(cola) or "(ningún 48-)",
         "`ore-serve` la clona para `estado` de una FUENTE; para una vista no la mira")


# ── §4 · el bundle ─────────────────────────────────────────────────────────


def seccion_4(inq, ns, pod_serve):
    print("\n§4 · EL BUNDLE — qué commits invalidan TODOS los recibos")
    guion = (
        "W=/tmp/medida-bundle; rm -rf $W\n"
        "export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=http.extraheader\n"
        "GIT_CONFIG_VALUE_0=\"Authorization: token $(cat /testigo/forja)\"; export GIT_CONFIG_VALUE_0\n"
        "git clone -q http://forja.%s.svc.cluster.local:3000/%s/ontologia.git $W && cd $W || exit 1\n"
        "for c in $(git log --format=%%h -30 | tac); do\n"
        "  git checkout -q $c 2>/dev/null\n"
        "  ore compile . > /tmp/c.json 2>/dev/null\n"
        "  B=$(grep -o '\"bundle\": *\"sha256:[0-9a-f]*\"' /tmp/c.json | head -1 | grep -o 'sha256:[0-9a-f]\\{10\\}')\n"
        "  N=$(git diff --name-only $c~1 $c 2>/dev/null | tr '\\n' ' ')\n"
        "  echo \"$c|$(git log -1 --format=%%cI $c)|${B:-?}|$(git log -1 --format=%%s $c | cut -c1-40)|$N\"\n"
        "done\n"
        "cd /; rm -rf $W /tmp/c.json\n" % (ns, ns)
    )
    lineas = [l.split("|", 4) for l in kexec(ns, pod_serve, guion).splitlines() if l.count("|") >= 4]
    previo, cambian, iguales, por_que = None, 0, 0, {}
    for c, t, b, msg, ficheros in lineas:
        if b == "?":
            fila("  %s %s" % (t[11:19], msg), "no compila", "(antes de la 0027)")
            continue
        if previo is not None:
            cambia = b != previo
            cambian += cambia
            iguales += not cambia
            clase = "copias/*.json" if ficheros.strip() and all(f.startswith("copias/") for f in ficheros.split()) else \
                    "ontology.config.yaml" if ficheros.split() == ["ontology.config.yaml"] else "documentos de un paquete"
            por_que.setdefault(clase, [0, 0])[0 if cambia else 1] += 1
            fila("  %s %s" % (t[11:19], msg), b, ("CAMBIA · " if cambia else "igual  · ") + clase)
        previo = b
    fila("  commits que cambian el bundle", "%d de %d" % (cambian, cambian + iguales),
         "cada uno deja SIN recibo a todas las vistas del árbol")
    for clase, (si, no) in por_que.items():
        fila("    %s" % clase, "cambia %d · igual %d" % (si, no))
    print("  — la cabecera del recibo (materializar.rs::cabecera): bundle · clave · conducto · esquema · plan · testigo —")
    print("    plan+esquema+clave ya nombran lo que se copia; bundle = SHA-256(árbol entero ‖ versión OOS ‖ lock)")


def main():
    a = sys.argv[1:]
    inq = a[a.index("--inquilino") + 1] if "--inquilino" in a else "victor"
    horas = int(a[a.index("--horas") + 1]) if "--horas" in a else 6
    ns = "t-%s" % inq
    pod_serve = k("get", "pods", "-l", "ore.dev/rol=control", "-o", "jsonpath={.items[0].metadata.name}", ns=ns).strip()
    if not pod_serve:
        sys.exit("sin pod de ore-serve en %s" % ns)
    print("MEDIDA · lo que parece roto · inquilino `%s` · %s" % (inq, dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%d %H:%MZ")))
    seccion_1(inq, ns, horas)
    seccion_2(inq, ns, horas)
    seccion_3(inq, ns, pod_serve)
    seccion_4(inq, ns, pod_serve)


if __name__ == "__main__":
    main()
