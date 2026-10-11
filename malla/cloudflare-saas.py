#!/usr/bin/env python3
# ═══════════════════════════════════════════════════════════════════════════
# EL NOMBRE DE UNA CELDA EN CLOUDFLARE (ORE 0060 C2·4)
#
#   python3 malla/cloudflare-saas.py asegurar <host>   # que exista, con su certificado
#   python3 malla/cloudflare-saas.py quitar   <host>   # que no exista
#
# Las entradas de las celdas (`<celda>.ore.paladio.io`) llegan a la malla por el
# túnel de Cloudflare: el DNS es un CNAME a `entrada.ontoforge.org`, y Cloudflare
# sólo sirve el nombre si está dado de alta como *custom hostname* de esa zona
# (Cloudflare for SaaS, 100 gratis). Esto es ese alta.
#
# ⭐ Validación por HTTP, sin TXT: el paso ⑧ del aprovisionador ya escribió el CNAME
#   (`<entrada> CNAME <puerta>` → `entrada.ontoforge.org`), y con el CNAME puesto
#   Cloudflare verifica el nombre y emite el certificado solo (~1,5 min, medido en
#   C2·0). Y sin TXT bajo el nombre no hay trampa del comodín: un TXT en
#   `_acme-challenge.X` convierte a X en nodo y el comodín deja de cubrirlo (así
#   cayó `iam` el 2026-10-11).
#
# ⛔ Las tres respuestas del aprovisionador, por el código de salida:
#     0  estaba ya            3  se dio de alta / se quitó
#     2  NO SE SABE (red, permisos, la API): no se toca nada y se dice por qué
#     4  sin token: este clúster no sirve por el túnel; se dice y no es un fallo
#
# El token (`cloudflare-saas` en Secret Manager, sólo «SSL and Certificates: Edit»
# sobre `ontoforge.org`) llega en el fichero de CF_SAAS_TOKEN_FILE; la zona, en CF_ZONA.
# ═══════════════════════════════════════════════════════════════════════════
import json
import os
import sys
import urllib.error
import urllib.parse
import urllib.request

API = "https://api.cloudflare.com/client/v4"


def llamar(metodo, ruta, token, cuerpo=None):
    req = urllib.request.Request(
        API + ruta, method=metodo,
        data=json.dumps(cuerpo).encode() if cuerpo is not None else None,
        headers={"Authorization": f"Bearer {token}", "Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=30) as r:
        d = json.load(r)
    if not d.get("success"):
        raise RuntimeError(f"la API dice que no: {d.get('errors')}")
    return d["result"]


def main():
    if len(sys.argv) != 3 or sys.argv[1] not in ("asegurar", "quitar"):
        print(__doc__ or "uso: cloudflare-saas.py asegurar|quitar <host>", file=sys.stderr)
        return 2
    accion, host = sys.argv[1], sys.argv[2].strip().rstrip(".").lower()
    fichero, zona = os.environ.get("CF_SAAS_TOKEN_FILE", ""), os.environ.get("CF_ZONA", "")
    if not fichero or not os.path.exists(fichero) or not zona:
        print("sin token de Cloudflare o sin CF_ZONA: el nombre no se da de alta en el túnel")
        return 4
    token = open(fichero).read().strip()
    try:
        hay = llamar("GET", f"/zones/{zona}/custom_hostnames?hostname={urllib.parse.quote(host)}", token)
        if accion == "asegurar":
            if hay:
                h = hay[0]
                print(f"{host}: {h['status']}, certificado {(h.get('ssl') or {}).get('status')}")
                return 0
            h = llamar("POST", f"/zones/{zona}/custom_hostnames", token, {
                "hostname": host,
                "ssl": {"method": "http", "type": "dv", "settings": {"min_tls_version": "1.2"}}})
            print(f"{host}: dado de alta ({h['id'][:8]}), Cloudflare lo valida por el CNAME")
            return 3
        if not hay:
            print(f"{host}: no estaba")
            return 0
        llamar("DELETE", f"/zones/{zona}/custom_hostnames/{hay[0]['id']}", token)
        print(f"{host}: quitado")
        return 3
    except (urllib.error.URLError, RuntimeError, KeyError, ValueError) as e:
        cuerpo = ""
        if isinstance(e, urllib.error.HTTPError):
            cuerpo = e.read().decode("utf-8", "replace")[:200]
        print(f"{host}: no se sabe — {e} {cuerpo}".strip(), file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
