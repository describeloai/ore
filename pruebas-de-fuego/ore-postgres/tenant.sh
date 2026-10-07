#!/usr/bin/env bash
# tenant.sh — un tenant con su timeline `main` en el pageserver (lo que hará el storage_controller
# en P2), o una rama de él:
#   tenant.sh               → crea tenant + main; guarda `tenant` y `main` en $ORE_PG_TRABAJO
#   tenant.sh rama <nombre> [lsn]  → timeline hija de main en <lsn> (o el último); guarda `rama-<nombre>`
source "$(dirname "$0")/entorno.sh"
hex() { python -c "import secrets;print(secrets.token_hex(16))"; }

if [ "${1:-}" = rama ]; then
  T=$(leer tenant) && M=$(leer main) || exit 1
  R=$(hex); LSN=${3:+,\"ancestor_start_lsn\":\"$3\"}
  r=$(pageserver POST /v1/tenant/$T/timeline/ "{\"new_timeline_id\":\"$R\",\"ancestor_timeline_id\":\"$M\"$LSN,\"pg_version\":17}")
  case $r in *"$R"*) ;; *) echo "✗ rama: $r"; exit 1;; esac
  guardar "rama-$2" "$R"; echo "rama $2 = $R (de main ${3:-en su último LSN})"
  exit 0
fi

T=$(hex); M=$(hex)
# generación 1 (en C4 el reenganche usa la 2: es lo que reparte el storage_controller)
pageserver PUT /v1/tenant/$T/location_config '{"mode":"AttachedSingle","generation":1,"tenant_conf":{}}' >/dev/null
until [ "$(pageserver GET /v1/tenant/$T | python -c "import sys,json;print(json.load(sys.stdin)['state']['slug'])" 2>/dev/null)" = Active ]; do sleep 0.5; done
r=$(pageserver POST /v1/tenant/$T/timeline/ "{\"new_timeline_id\":\"$M\",\"pg_version\":17}")
case $r in *"$M"*) ;; *) echo "✗ timeline: $r"; exit 1;; esac
guardar tenant "$T"; guardar main "$M"
echo "tenant $T · main $M"
