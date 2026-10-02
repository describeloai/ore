#!/usr/bin/env bash
# LA SONDA DEL CORREO (0048, deuda 3) — ¿acepta el relay lo que manda el IdP?
#
#     bash identidad/sonda-correo.sh
#
# Habla SMTP con `smtp-relay.gmail.com:587` desde un pod en `identidad`, en el mismo
# nodo que el IdP (sale por la misma IP), con el remitente de `realm.mjs`, y CORTA
# ANTES DE `DATA`: no se envía nada. Sale 0 si el relay acepta remitente y
# destinatario, y 1 si no, con la IP de salida y la respuesta del relay.
#
# ⭐ Por qué en el nodo del IdP: el relay autoriza POR IP DE ORIGEN, y la IP de salida
#   depende del nodo (los privados salen por el NAT `salida-a-origenes`; uno con IP
#   pública, por la suya). Una sonda en otro nodo mediría otra cosa.
#
# Es lo que decide `HAY_CORREO` en `identidad/ore.mjs`.
set -euo pipefail
NS=identidad
NODO=$(kubectl -n "$NS" get pod idp-0 -o jsonpath='{.spec.nodeName}')
IMAGEN=europe-west1-docker.pkg.dev/project-8853a180-450d-47be-b83/ore/ore-drivers:main
POD=sonda-correo-$$

trap 'kubectl -n "$NS" delete pod "$POD" --wait=false >/dev/null 2>&1 || true' EXIT

kubectl -n "$NS" run "$POD" --restart=Never --image="$IMAGEN" --quiet \
  --overrides="{\"spec\":{\"nodeName\":\"$NODO\"}}" --command -- python3 -c '
import smtplib, sys, urllib.request
try: print("ip de salida:", urllib.request.urlopen("https://api.ipify.org", timeout=10).read().decode())
except Exception as e: print("ip de salida: ?", e)
try:
    s = smtplib.SMTP("smtp-relay.gmail.com", 587, timeout=20)
    s.ehlo("login.paladio.io"); s.starttls(); s.ehlo("login.paladio.io")
    m = s.mail("no-reply@paladio.io"); print("MAIL FROM:", m[0], m[1].decode()[:200])
    r = s.rcpt("victor@paladio.io") if m[0] == 250 else (0, b"")
    if r[0]: print("RCPT TO:", r[0], r[1].decode()[:200])
    try: s.rset(); s.quit()
    except Exception: pass
    print("cortado antes de DATA: no se ha enviado nada")
    sys.exit(0 if m[0] == 250 and r[0] in (250, 251) else 1)
except Exception as e:
    print("el relay cortó:", e); sys.exit(1)
' >/dev/null

FASE=""
for _ in $(seq 1 30); do
  FASE=$(kubectl -n "$NS" get pod "$POD" -o jsonpath='{.status.phase}' 2>/dev/null || true)
  case "$FASE" in Succeeded|Failed) break ;; esac
  sleep 3
done
kubectl -n "$NS" logs "$POD"
[ "$FASE" = Succeeded ] && echo "✓ el relay acepta lo que manda el IdP" || { echo "✗ el relay NO acepta (fase: ${FASE:-?})" >&2; exit 1; }
