# -*- coding: utf-8 -*-
"""
LA COPIA EN LA CELDA, DE VERDAD (0027 P1 I5): una base estandar sobre olist en `demo`, el
Job `copiar-<resumen>` corriendo en `t-demo`, la copia en el bucket del inquilino, el
informe en el arbol, y la segunda pasada leyendo 0 filas del origen.

No es un banco: habla con `demo.ore.paladio.io` con el token del AGENTE de la celda (client
credentials; el secreto sale de Secret Manager y no se imprime), con `kubectl` contra
`t-demo`, y con `gcloud storage` contra el bucket. Lo que la persona ve es su consola (la base
con «Standard database», las tablas «Copied into this cluster», la ficha con las filas).

Fases, en orden — cada una deja escrito lo que vio:

  crear     POST /paquetes {name, source, only: <tres tablas pequenas>, type: standard}
            → 200, type standard, N tablas a copiar, y «NO encolado: el conducto espera al dueño»
  dueno     POST /paquetes/<base>/decisiones {dueno/<base>: team:demo}
            → conduits.yaml nace y AHORA se encola el Job `copiar-<h>` en t-demo/trabajo
  esperar   el Job en t-demo: Flux lo crea, Kueue lo admite, corre, termina (o falla, y se dice)
  ver       GET /paquetes/<base>/copias → estado copiada, filas, digest, copiado_por, cuando;
            GET /paquetes → copias {declaradas, copiadas}; los objetos en el bucket
  segunda   otra base estandar con UNA tabla mas → un Job con las cuatro vistas: las tres de
            antes «al-dia» (0 filas leidas del origen), la nueva copiada
  limpiar   (no borra las bases: son la aceptacion; solo dice que no queda nada corriendo)

Uso:  python pruebas-de-fuego/la-copia-en-demo.py <fase> [demo] [--base=olist_copia]
"""
import json
import os
import subprocess
import sys
import time

FASE = next((a for a in sys.argv[1:] if not a.startswith("--")), "ver")
CELDA = next((a for a in sys.argv[2:] if not a.startswith("--")), "demo")
BASE = next((a.split("=", 1)[1] for a in sys.argv if a.startswith("--base=")), "olist_copia")
PROYECTO = "project-8853a180-450d-47be-b83"
IDP = "https://login.paladio.io/realms/rubix"
GCLOUD = "gcloud.cmd" if os.name == "nt" else "gcloud"
NS = "t-" + CELDA
BUCKET = "gs://%s-%s-copia" % (PROYECTO, NS)
# Tres tablas pequenas de olist: 71, 3 095 y 32 951 filas (la aceptacion no es de volumen).
TABLAS = ["olist.product_category_name_translation", "olist.sellers", "olist.products"]
# La cuarta, para la segunda pasada.
CUARTA = "olist.order_payments"


def sh(cmd):
    r = subprocess.run(cmd, shell=True, capture_output=True, text=True, encoding="utf-8")
    return r.stdout.strip() if r.returncode == 0 else ""


def kubectl(*args):
    r = subprocess.run(["kubectl", *args], capture_output=True, text=True, encoding="utf-8")
    return r.stdout


def token_del_agente():
    cli = sh("%s secrets versions access latest --secret=%s-agente-cliente --project=%s" % (GCLOUD, NS, PROYECTO))
    sec = sh("%s secrets versions access latest --secret=%s-agente-secreto --project=%s" % (GCLOUD, NS, PROYECTO))
    if not cli or not sec:
        sys.exit("no se pudo leer la credencial del agente de %s" % NS)
    r = subprocess.run(["curl", "-s", "-m", "10", "-X", "POST", IDP + "/protocol/openid-connect/token", "-d", "grant_type=client_credentials",
                        "-d", "client_id=" + cli, "--data-urlencode", "client_secret=" + sec], capture_output=True, text=True, encoding="utf-8")
    return json.loads(r.stdout)["access_token"]


def serve(metodo, camino, tok, cuerpo=None):
    args = ["curl", "-s", "-m", "120", "-o", "-", "-w", "\n%{http_code} %{time_total}", "-X", metodo,
            "https://%s.ore.paladio.io%s" % (CELDA, camino), "-H", "authorization: Bearer " + tok]
    if cuerpo is not None:
        args += ["-H", "content-type: application/json", "-d", json.dumps(cuerpo)]
    r = subprocess.run(args, capture_output=True, text=True, encoding="utf-8")
    cuerpo, _, cod = (r.stdout or "").rpartition("\n")
    partes = cod.split()
    return (partes[0] if partes else "000"), (partes[1] if len(partes) > 1 else "?"), cuerpo.strip()


def hora():
    return time.strftime("%H:%M:%S")


def fuente(tok):
    cod, _, c = serve("GET", "/paquetes", tok)
    if cod != "200":
        sys.exit("GET /paquetes → %s %s" % (cod, c[:200]))
    enteros = [p for p in json.loads(c)["packages"] if not p.get("scoped") and p.get("source")]
    if not enteros:
        sys.exit("no hay ningun paquete de fuente entera del que inducir")
    return enteros[0]["name"]


def crear(tok, nombre, tablas):
    src = fuente(tok)
    print("  %s POST /paquetes {name: %s, source: %s, only: %s, type: standard}" % (hora(), nombre, src, tablas))
    cod, t, c = serve("POST", "/paquetes", tok, {"name": nombre, "source": src, "only": tablas, "type": "standard"})
    print("  → %s en %s s" % (cod, t))
    j = json.loads(c) if c.startswith("{") else {}
    for k in ("type", "copias", "encolado", "conducto", "quedan"):
        if k in j:
            print("    %s: %s" % (k, json.dumps(j[k], ensure_ascii=False)))
    if cod != "200":
        print("    " + c[:600])
        sys.exit(1)


def dueno(tok, nombre):
    print("  %s POST /paquetes/%s/decisiones {dueno/%s: team:%s}" % (hora(), nombre, nombre, CELDA))
    cod, t, c = serve("POST", "/paquetes/%s/decisiones" % nombre, tok, {"answers": {"dueno/" + nombre: "team:" + CELDA}})
    print("  → %s en %s s" % (cod, t))
    j = json.loads(c) if c.startswith("{") else {}
    for k in ("copias", "encolado", "conducto", "quedan"):
        if k in j:
            print("    %s: %s" % (k, json.dumps(j[k], ensure_ascii=False)))
    if cod != "200":
        print("    " + c[:600])
        sys.exit(1)


def esperar(plazo=1500):
    print("  %s esperando un Job copiar-* en %s (Flux → Kueue → pod; ~2 min de arranque)" % (hora(), NS))
    visto = set()
    fin = time.time() + plazo
    while time.time() < fin:
        salida = kubectl("get", "jobs", "-n", NS, "-o", "json")
        try:
            jobs = json.loads(salida)["items"]
        except Exception:
            jobs = []
        copias = [j for j in jobs if j["metadata"]["name"].startswith("copiar-")]
        for j in copias:
            n = j["metadata"]["name"]
            st = j.get("status", {})
            estado = "succeeded" if st.get("succeeded") else "failed" if st.get("failed") else "running" if st.get("active") else "pending"
            clave = (n, estado)
            if clave not in visto:
                visto.add(clave)
                print("  %s %s · %s" % (hora(), n, estado))
            if estado in ("succeeded", "failed"):
                sel = "job-name=" + n
                print("  ── log de %s ──" % n)
                log = kubectl("logs", "-n", NS, "-l", sel, "--tail=60", "--all-containers=true")
                for l in log.splitlines()[-60:]:
                    print("    " + l)
                return estado == "succeeded"
        time.sleep(20)
    print("  ✗ ningun Job copiar-* termino en %d s" % plazo)
    return False


def ver(tok, nombre):
    cod, _, c = serve("GET", "/paquetes/%s/copias" % nombre, tok)
    print("  GET /paquetes/%s/copias → %s" % (nombre, cod))
    if cod == "200":
        for v in json.loads(c)["copias"]:
            cp = v.get("copia", {})
            print("    %-40s %-10s filas=%-7s leidas=%-7s bytes=%-9s %s %s" % (
                v["view"], cp.get("estado", "?"), cp.get("filas", "-"), cp.get("leidas", "-"), cp.get("bytes", "-"),
                cp.get("copiado_por", ""), cp.get("cuando", "")))
    cod, _, c = serve("GET", "/paquetes", tok)
    if cod == "200":
        for p in json.loads(c)["packages"]:
            if p["name"] == nombre:
                print("  GET /paquetes[%s]: type=%s copias=%s tablas=%s modeladas=%s" % (
                    nombre, p.get("type"), p.get("copias"), p.get("tablas"), p.get("modeladas")))
    print("  el bucket %s:" % BUCKET)
    lista = sh("%s storage ls -l %s/ore/v1/** --project=%s" % (GCLOUD, BUCKET, PROYECTO))
    n = 0
    for l in lista.splitlines():
        if "ore/v1/" in l:
            n += 1
            print("    " + l.strip())
    print("    %d objeto(s)" % n)


def limpiar():
    pods = kubectl("get", "pods", "-n", NS, "--field-selector=status.phase=Running", "-o", "name")
    corriendo = [p for p in pods.splitlines() if "copiar-" in p]
    print("  pods copiar-* corriendo en %s: %d" % (NS, len(corriendo)))
    print("  las bases de la aceptacion se quedan (son la aceptacion); nada de pago corre")


if __name__ == "__main__":
    print("\n  ═══ P1 I5 · la copia en %s · fase %s ═══" % (CELDA, FASE))
    if FASE == "esperar":
        sys.exit(0 if esperar() else 1)
    if FASE == "limpiar":
        limpiar()
        sys.exit(0)
    tok = token_del_agente()
    if FASE == "crear":
        crear(tok, BASE, TABLAS)
    elif FASE == "dueno":
        dueno(tok, BASE)
    elif FASE == "ver":
        ver(tok, BASE)
    elif FASE == "segunda":
        crear(tok, BASE + "2", [CUARTA])
        dueno(tok, BASE + "2")
    else:
        sys.exit("fase desconocida: " + FASE)
