#!/usr/bin/env python3
"""
MEDIDA · el historial de un fichero (W2, «Version history») — ¿ya tenemos
versiones nativas? (19 de septiembre)

La consola tiene el modal de *Version history* (lista de versiones con fecha
y autor, el texto de la elegida, *Restore*, *Compare*) con versiones DE
EJEMPLO. La pregunta es si hace falta inventar algo o si git ya lo tiene:

  §1  en el árbol del inquilino: cuántas versiones tiene cada fichero
      (`git log --follow -- ruta`), quién es el autor (la persona, el Job),
      y cuánto cuesta preguntarlo (tras el clon de ~1 s)
  §2  qué expone ore-serve hoy de eso (`commit_de`: el ÚLTIMO commit del
      fichero, en GET /arbol/{ruta}) y qué falta (la lista y una versión)
  §3  qué espera el modal (la forma `Version {id, cuando, autor, texto}`)

Uso:  python pruebas-de-fuego/medida-w2-historial.py --inquilino victor
"""
import datetime as dt
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
    print("  %-52s %-18s %s" % (k, v, nota))


def sh(*args, entrada=None):
    exe = shutil.which(args[0]) or args[0]
    r = subprocess.run((exe,) + tuple(args[1:]), input=entrada.encode("utf-8") if entrada else None, capture_output=True)
    return r.stdout.decode("utf-8", "replace")


def kexec(ns, pod, guion):
    return sh("kubectl", "exec", "-i", "-n", ns, pod, "--", "sh", "-s", entrada=guion)


GUION = r'''
NS=%(ns)s; W=/tmp/hist; rm -rf $W
export GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=http.extraheader
GIT_CONFIG_VALUE_0="Authorization: token $(cat /testigo/forja)"; export GIT_CONFIG_VALUE_0
ms() { awk '{printf "%%d", $1*1000}' /proc/uptime; }
t0=$(ms); git clone -q http://forja.$NS.svc.cluster.local:3000/$NS/ontologia.git $W; echo "### clon $(( $(ms)-t0 ))"
cd $W || exit 1
echo "### ficheros $(git ls-files | wc -l) commits $(git rev-list --count HEAD)"
for f in $(git ls-files | grep -v "^copias/" | head -400); do echo "$(git log --follow --format=%%H -- "$f" | wc -l) $f"; done | sort -rn > /tmp/v.txt
echo "### versiones-por-fichero $(awk '{s+=$1; if($1>m)m=$1} END{print s/NR" media · "m" max"}' /tmp/v.txt)"
echo "### con-mas-de-una $(awk '$1>1' /tmp/v.txt | wc -l) de $(wc -l < /tmp/v.txt)"
head -3 /tmp/v.txt | sed 's/^/### top /'
F=$(head -1 /tmp/v.txt | cut -d" " -f2-)
t0=$(ms); git log --follow --format='%%H%%x1f%%an%%x1f%%cn%%x1f%%cI%%x1f%%s' -- "$F" > /tmp/l.txt; echo "### log-un-fichero $(( $(ms)-t0 )) $(wc -l < /tmp/l.txt) versiones de $F"
echo "### autores $(cut -d$'\x1f' -f2 /tmp/l.txt | sort | uniq -c | sort -rn | head -4 | awk '{print $2"×"$1}' | tr '\n' ' ')"
echo "### committers $(cut -d$'\x1f' -f3 /tmp/l.txt | sort | uniq -c | sort -rn | head -3 | awk '{print $2"×"$1}' | tr '\n' ' ')"
H=$(head -1 /tmp/l.txt | cut -d$'\x1f' -f1)
t0=$(ms); git show "$H:$F" > /tmp/s.txt; echo "### show-una-version $(( $(ms)-t0 )) $(wc -c < /tmp/s.txt) bytes"
echo "### mensajes $(cut -d$'\x1f' -f5 /tmp/l.txt | head -3 | tr '\n' '|' | cut -c1-150)"
cd /; rm -rf $W /tmp/v.txt /tmp/l.txt /tmp/s.txt
'''


def main():
    a = sys.argv[1:]
    inq = a[a.index("--inquilino") + 1] if "--inquilino" in a else "victor"
    ns = "t-%s" % inq
    pod = sh("kubectl", "get", "pods", "-n", ns, "-l", "ore.dev/rol=control", "-o", "jsonpath={.items[0].metadata.name}").strip()
    print("MEDIDA · el historial de un fichero · `%s` · %s" % (inq, dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%d %H:%MZ")))
    print("\n§1 · EN EL ÁRBOL (git, en el pod de ore-serve)")
    for l in kexec(ns, pod, GUION % {"ns": ns}).splitlines():
        if l.startswith("### "):
            partes = l[4:].split(" ", 1)
            v = partes[1] if len(partes) > 1 else ""
            m = re.match(r"(\d+) (.*)", v)
            if partes[0] in ("clon", "log-un-fichero", "show-una-version") and m:
                fila("  " + partes[0], "%s ms" % m.group(1), m.group(2)[:100])
            else:
                fila("  " + partes[0], "", v[:120])

    print("\n§2 · ORE-SERVE hoy")
    doc = open(os.path.join(RAIZ, "crates/ore-serve/src/documentos.rs"), encoding="utf-8").read()
    fila("  commit_de(raiz, fichero)", "sí", "el ÚLTIMO commit del fichero {hash, autor, fecha} en GET /arbol/{ruta}")
    rutas = open(os.path.join(RAIZ, "crates/ore-serve/src/rutas.rs"), encoding="utf-8").read()
    fila("  GET /arbol/historia/{ruta}", "sí" if '["arbol", "historia"' in rutas else "no", "la lista de versiones: `git log --follow --format -- ruta`")
    fila("  GET /arbol/version/{hash}/{ruta}", "sí" if '["arbol", "version"' in rutas else "no", "el texto de una: `git show hash:ruta`")
    fila("  restaurar", "no hace falta", "es un PUT con el texto viejo: un commit nuevo, con la persona")
    _ = doc

    print("\n§3 · LA CONSOLA")
    p = os.path.join(CONSOLA, "components/code-workspace/VersionHistoryModal.tsx")
    t = open(p, encoding="utf-8").read() if os.path.exists(p) else ""
    fila("  VersionHistoryModal.tsx", "%d líneas" % t.count("\n"), "forma Version{id, cuando, autor, texto} · mocks %s · «todavía no» ×%d"
         % ("sí" if "VERSIONES_DE_EJEMPLO" in t or "De ejemplo" in t else "no", t.count("todavía no implementado")))
    fila("  lo que casa", "hash→id · %cI→cuando · %an→autor", "y el texto se pide al elegir una versión (git show), no todos de golpe")


if __name__ == "__main__":
    main()
