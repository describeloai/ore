# pg0-de-verdad.sh (ADR 0065 PG0·5): el `ore-read-postgres` de la imagen de drivers contra un Postgres
# de verdad, DENTRO de la celda. La URL sale del cofre ($FUENTE) con la credencial del agente; nunca
# se imprime: sólo el host sin usuario ni contraseña y los NOMBRES de sus parámetros.
#   · lo que es: versión, wal_level, TLS de la sesión (psql con la misma URL)
#   · check con la URL tal cual y con cada sslmode
#   · catalogo: tiempo, relaciones y si trae filas
#   · el pooler (host «-pooler» en Neon, o $POOLER_PUERTO): check, catalogo, Arrow ×2, servir ×3
set -e
DICE=$(curl -sSf "$DIRECCION/realms/$REALM/.well-known/openid-configuration" | python3 -c 'import json,sys;print(json.load(sys.stdin)["issuer"])')
[ "$DICE" = "$EMISOR/realms/$REALM" ] || { echo "## falla: emisor inesperado"; exit 1; }
TOK=$(curl -sSf -X POST "$DIRECCION/realms/$REALM/protocol/openid-connect/token" -d grant_type=client_credentials -d "client_id=$(cat /puesto/agente-cliente)" -d "client_secret=$(cat /puesto/agente-secreto)" | python3 -c 'import json,sys;print(json.load(sys.stdin)["access_token"])')
C=$(curl -s -o /tmp/s.json -w '%{http_code}' -H "authorization: Bearer $TOK" "http://ore-cofre.t-victor.svc.cluster.local:8095/organizaciones/$ORGANIZACION/secretos/fuente-$FUENTE")
[ "$C" = 200 ] || { echo "## falla: el custodio contesto $C"; exit 1; }
U="$(python3 -c 'import json;print(json.load(open("/tmp/s.json"))["valor"])')"; rm -f /tmp/s.json
set +e
D=/opt/ore/conectores/ore-read-postgres; [ -x $D ] || D=$(command -v ore-read-postgres)
echo "driver: ${D:-NO ESTÁ}"
# La URL sin secreto: esquema, host, puerto, base y los nombres de los parámetros.
python3 - "$U" <<'PY'
import sys, urllib.parse as u
p = u.urlsplit(sys.argv[1])
print("url:", p.scheme, "host=" + (p.hostname or ""), "puerto=" + str(p.port or ""), "base=" + p.path,
      "parametros=" + ",".join(f"{k}={v}" if k in ("sslmode", "channel_binding") else k for k, v in u.parse_qsl(p.query)))
PY
con() { python3 - "$U" "$1" "$2" <<'PY'
import sys, urllib.parse as u
url, clave, valor = sys.argv[1:]
p = u.urlsplit(url)
q = [(k, v) for k, v in u.parse_qsl(p.query) if k != clave]
if valor != "-":
    q.append((clave, valor))
host = p.netloc
if clave == "host":           # sustituir el host (el pooler)
    q = [(k, v) for k, v in u.parse_qsl(p.query)]
    host = p.netloc.replace(p.hostname, valor)
print(u.urlunsplit((p.scheme, host, p.path, u.urlencode(q), "")))
PY
}
ms() { python3 -c "import time;print(int(time.time()*1000))"; }
corto() { tr '\n' ' ' | cut -c1-"${1:-220}"; }
export PGCONNECT_TIMEOUT=15
echo "servidor: $(psql "$U" -XAtq -F ' | ' -c "select version(), current_setting('wal_level'), (select ssl::text||' '||coalesce(version,'') from pg_stat_ssl where pid=pg_backend_pid()), current_setting('server_version_num')" 2>&1 | corto 300)"
echo "permisos: $(psql "$U" -XAtq -F ' | ' -c "select current_user, rolreplication, rolsuper, (select count(*) from pg_namespace where nspname !~ '^pg_' and nspname<>'information_schema') from pg_roles where rolname=current_user" 2>&1 | corto 200)"
echo "check tal cual: $(printf '{"url":"%s"}' "$U" | timeout 30 $D check 2>&1 | corto)"
for m in - disable prefer require verify-ca verify-full; do
  echo "check sslmode=$m: $(printf '{"url":"%s"}' "$(con sslmode $m)" | timeout 30 $D check 2>&1 | corto)"
done
s=$(ms); printf '%s' "$U" | timeout 120 $D catalogo vara >/tmp/c.json 2>/tmp/c.err; rc=$?; e=$(( $(ms) - s ))
python3 - "$rc" "$e" <<'PY'
import json, sys, collections
try:
    ts = json.load(open("/tmp/c.json"))["tables"]
except Exception as x:
    print("catalogo: rc=%s %s ms · ilegible: %s" % (sys.argv[1], sys.argv[2], x)); sys.exit()
k = collections.Counter(t.get("kind") for t in ts)
tipos = collections.Counter(c.get("type", "∅") for t in ts for c in t.get("columns", []))
print("catalogo: rc=%s %s ms · %d relaciones %s · rows=%d rowsEstimate=%d · tipos %s" % (
    sys.argv[1], sys.argv[2], len(ts), dict(k), sum("rows" in t for t in ts),
    sum("rowsEstimate" in t for t in ts), dict(tipos.most_common(12))))
PY
echo "avisos del catalogo: $(corto 300 </tmp/c.err)"
TC=$(python3 -c '
import json
for t in json.load(open("/tmp/c.json"))["tables"]:
    if t.get("kind") == "table" and t.get("columns"):
        print(t["name"], t["columns"][0]["name"]); break
' 2>/dev/null)
T=${TC% *}; COL=${TC#* }
echo "tabla de prueba: ${T:-ninguna}"
pooler() { # URL ETIQUETA
  local P=$1
  echo "$2 · check: $(printf '{"url":"%s"}' "$P" | timeout 30 $D check 2>&1 | corto)"
  printf '%s' "$P" | timeout 120 $D catalogo vara >/tmp/p.json 2>/tmp/p.err
  echo "$2 · catalogo: rc=$? · $(python3 -c "import json;print(len(json.load(open('/tmp/p.json'))['tables']))" 2>/dev/null) relaciones · $(corto 160 </tmp/p.err)"
  [ -n "$T" ] || return
  for n in 1 2; do
    printf '{"url":"%s","objeto":"%s","proyeccion":{"c":"%s"},"formato":"arrow","limit":500}' "$P" "$T" "$COL" | timeout 60 $D leer >/tmp/a.bin 2>/tmp/a.err
    echo "$2 · leer arrow #$n: rc=$? $(wc -c </tmp/a.bin) B · $(corto 200 </tmp/a.err)"
  done
  for n in 1 2 3; do printf '{"id":"r%s","url":"%s","objeto":"%s","proyeccion":{"c":"%s"},"formato":"arrow","limit":200}\n' $n "$P" "$T" "$COL"; sleep 1; done \
    | timeout 60 $D servir >/tmp/s.bin 2>/tmp/s.err
  echo "$2 · servir ×3: rc=$? ok=$(grep -a -c '"fin":"ok"' /tmp/s.bin) · $(grep -a -o '"mensaje":"[^"]*"' /tmp/s.bin | sort | uniq -c | corto 300) $(corto 200 </tmp/s.err)"
  # Cuatro lectores a la vez, como en el laboratorio (un pooler en modo transacción).
  for k in 1 2 3 4; do
    ( for n in 1 2 3 4 5; do
        printf '{"id":"c%s-%s","url":"%s","objeto":"%s","proyeccion":{"c":"%s"},"formato":"arrow","limit":%s}\n' $k $n "$P" "$T" "$COL" $((100 * n))
        sleep 0.3
      done | timeout 90 $D servir >/tmp/c$k.bin 2>/tmp/c$k.err ) &
  done
  wait
  echo "$2 · 4 servir a la vez: ok=$(cat /tmp/c?.bin | grep -a -c '"fin":"ok"')/20 · $(cat /tmp/c?.bin | grep -a -o '"mensaje":"[^"]*"' | sort | uniq -c | corto 300)"
  rm -f /tmp/c?.bin /tmp/c?.err
}
H=$(python3 -c 'import sys,urllib.parse as u;print(u.urlsplit(sys.argv[1]).hostname or "")' "$U")
case "$H" in
  *-pooler.*)
    pooler "$U" "pooler (la fuente)"
    pooler "$(con host "$(echo "$H" | sed 's/-pooler\././')")" directo ;;
  *.neon.tech)
    pooler "$U" "directo (la fuente)"
    pooler "$(con host "$(echo "$H" | sed 's/^\([^.]*\)\./\1-pooler./')")" pooler ;;
  *.supabase.com|*.supabase.co)
    # Supavisor: sesión en el 5432, transacción en el 6543 (que la malla no deja salir: PG3).
    pooler "$U" "la fuente"
    for puerto in 5432 6543; do
      V=$(python3 -c 'import sys,urllib.parse as u;p=u.urlsplit(sys.argv[1]);print(u.urlunsplit((p.scheme,(p.netloc.rsplit(":",1)[0] if p.port else p.netloc)+":"+sys.argv[2],p.path,p.query,"")))' "$U" $puerto)
      pooler "$V" "puerto $puerto"
    done ;;
  *) pooler "$U" "la fuente" ;;
esac
echo "## ok"
