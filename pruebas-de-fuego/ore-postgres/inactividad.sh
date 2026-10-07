#!/usr/bin/env bash
# D0b·3 · la señal del escalado a cero: GET /status de compute_ctl (last_active), con y sin
# consultas. B.5: cloud_admin NO cuenta y las consultas muy cortas pueden no verse (muestreo de
# 500 ms) ⇒ la actividad se hace con un rol de aplicación y pg_sleep.
source "$(dirname "$0")/entorno.sh"
IP=$(leer "pod-$ORE_PG_VM") || exit 1
TOK=$(python "$AQUI/especificacion.py" token "$ORE_PG_VM")
st() { k exec cliente -- bash -c "exec 3<>/dev/tcp/$IP/3080; printf 'GET /status HTTP/1.0\r\nHost: x\r\nAuthorization: Bearer $TOK\r\n\r\n' >&3; timeout 3 cat <&3" 2>/dev/null | tail -1; }
q "$IP" "do \$\$ begin if not exists (select from pg_roles where rolname='app') then create role app login password 'app'; end if; end \$\$" >/dev/null
echo "$(date -u +%T) status: $(st)"
k exec cliente -- env PGPASSWORD=app psql -h "$IP" -p 55433 -U app -d postgres -Atqc "select pg_sleep(5)" >/dev/null
echo "$(date -u +%T) tras 5 s de actividad de app: $(st)"
sleep 90
echo "$(date -u +%T) tras 90 s sin consultas: $(st)"
