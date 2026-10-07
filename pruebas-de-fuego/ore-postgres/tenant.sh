#!/usr/bin/env bash
# tenant.sh — un tenant con su timeline `main`, o una rama de él. Desde P2·4, por el
# storage_controller (con el token admin), que reparte las generaciones y se lo cuenta a `avisos`:
#   tenant.sh                       → crea tenant + main; guarda `tenant` y `main` en $ORE_PG_TRABAJO
#   tenant.sh rama <nombre> [lsn]   → timeline hija de main en <lsn> (o el último); guarda `rama-<nombre>`
#   tenant.sh borrar                → borra el tenant (y con él su prefijo en GCS, P2·5)
source "$(dirname "$0")/entorno.sh"
hex() { python -c "import secrets;print(secrets.token_hex(16))"; }
# el controller contesta al crear el tenant antes de que esté Active: se reintenta (P2·1)
reintentar() { local r; for i in $(seq 1 20); do r=$("$@"); case $r in *timeline_id*) echo "$r"; return 0;; esac; sleep 3; done; echo "✗ $r" >&2; return 1; }

case "${1:-}" in
  rama)
    T=$(leer tenant) && M=$(leer main) || exit 1
    R=$(hex); LSN=${3:+,\"ancestor_start_lsn\":\"$3\"}
    reintentar controlador POST /v1/tenant/$T/timeline "{\"new_timeline_id\":\"$R\",\"ancestor_timeline_id\":\"$M\"$LSN,\"pg_version\":17}" >/dev/null || exit 1
    guardar "rama-$2" "$R"; echo "rama $2 = $R (de main ${3:-en su último LSN})" ;;
  borrar)
    # P2·6: el controller borra lo del pageserver (y su prefijo en GCS), pero NO el WAL de los
    # safekeepers mientras no los gestione (--timelines-onto-safekeepers, que exige 3 zonas):
    # quedaba huérfano (317 MB). Se le pide a cada safekeeper: local y GCS. Es lo que hará P4.
    T=$(leer tenant) || exit 1
    for i in $(seq 1 30); do r=$(controlador DELETE /v1/tenant/$T); case $r in *NotFound*) break;; esac; sleep 2; done
    SKT=""; [ "$ORE_PG_AUTH" = si ] && SKT=$(cat "$(secreto almacen-jwt safekeeperdata)")
    for i in 0 1 2; do api "safekeeper-$i.$ORE_PG_NS.svc.cluster.local" 7676 DELETE "/v1/tenant/$T" "" "$SKT" >/dev/null; done
    echo "tenant $T borrado (controller + 3 safekeepers)" ;;
  *)
    T=$(hex); M=$(hex)
    controlador POST /v1/tenant "{\"new_tenant_id\":\"$T\"}" >/dev/null
    reintentar controlador POST /v1/tenant/$T/timeline "{\"new_timeline_id\":\"$M\",\"pg_version\":17}" >/dev/null || exit 1
    guardar tenant "$T"; guardar main "$M"
    echo "tenant $T · main $M" ;;
esac
