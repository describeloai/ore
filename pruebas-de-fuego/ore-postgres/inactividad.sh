#!/usr/bin/env bash
# D0b·3 · la señal del escalado a cero: GET /status de compute_ctl (last_active), con y sin consultas.
read P IP < /c/tmp/d0b/vm-pod.txt
TOK=$(python -c "
import jwt,json,time
kid=json.load(open('C:/tmp/d0b/config.json'))['compute_ctl_config']['jwks']['keys'][0]['kid']
print(jwt.encode({'compute_id':'pg-d0','exp':int(time.time())+3600}, open('C:/tmp/d0b/jwt-prueba.pem').read(), algorithm='EdDSA', headers={'kid':kid}))")
st() { kubectl -n d0-neon exec cliente -- bash -c "exec 3<>/dev/tcp/$IP/3080; printf 'GET /status HTTP/1.0\r\nHost: x\r\nAuthorization: Bearer $TOK\r\n\r\n' >&3; timeout 3 cat <&3" 2>/dev/null | tail -1; }
q() { kubectl -n d0-neon exec cliente -- env PGPASSWORD=cloud_admin psql -h $IP -p 55433 -U cloud_admin -d postgres -Atqc "$1" >/dev/null; }
echo "$(date -u +%T) status: $(st)"
sleep 1
echo "$(date -u +%T) tras una consulta: $(st)"
sleep 90
echo "$(date -u +%T) tras 75 s sin consultas: $(st)"
