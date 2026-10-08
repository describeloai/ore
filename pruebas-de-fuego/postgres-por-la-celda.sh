#!/usr/bin/env bash
# 0058 P4·6 · POSTGRES POR LA CELDA: `/v1/postgres/…` en `ore-serve`.
#
# El binario de verdad, con un `ore-iam` y un `ore-postgres` de mentira. Lo que se
# fija aquí es el CABLEADO de quién puede (las pruebas del crate fijan la tabla de
# rutas → potestad):
#
#   · crear es de `postgres:crear`, y el `dueno` lo pone ORE (el de `quien`), no
#     el cuerpo; a `ore-postgres` llega con el token de la celda;
#   · dentro de un proyecto, su dueño sí; otro, sólo con `postgres:gestionar`
#     (y sin ella, 403 sin que `ore-postgres` vea nada); los roles, `postgres:usar`;
#   · desde un puesto (un agente), nada: 403, ni leer;
#   · sin `ore-iam`, 503; sin `ore-postgres`, 503.
set -uo pipefail
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
PUERTO="${PUERTO:-8961}"
TMP="$(mktemp -d)"
PIDS=""
falla() { echo "✗ $*" >&2; [ -s "$TMP/serve.txt" ] && tail -20 "$TMP/serve.txt" >&2; exit 1; }
dice() { echo "  · $*"; }
limpiar() { for p in $PIDS; do kill "$p" 2>/dev/null; done; sleep 0.2; rm -rf "$TMP"; }
trap limpiar EXIT
buscar() {
  local n
  for n in ${ORE_BIN:+"$ORE_BIN/$1" "$ORE_BIN/$1.exe"} \
           "$RAIZ/target/release/$1" "$RAIZ/target/release/$1.exe" \
           "$RAIZ/target/debug/$1"   "$RAIZ/target/debug/$1.exe"; do
    [ -x "$n" ] && { echo "$n"; return 0; }
  done
  return 1
}
SERVE="$(buscar ore-serve)" || falla "no hay binario de \`ore-serve\` — cargo build -p ore-serve"
PY=$(command -v python3 || command -v python) || falla "hace falta python"

# ── ore-iam de mentira: pertenecen admin, ana y beto; `gestionar`, sólo admin ──
cat > "$TMP/iam.py" <<'PYCODE'
import json, sys
from http.server import BaseHTTPRequestHandler, HTTPServer
MIEMBROS = {"persona:admin", "persona:ana", "persona:beto"}
TODOS = {"organizacion:leer", "postgres:ver", "postgres:crear", "postgres:usar"}
n = [0]
class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def do_POST(self):
        c = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))) or b"{}")
        s = self.headers.get("Ore-Sujeto")
        with open(sys.argv[2], "a") as f:
            f.write(json.dumps({"camino": self.path, "sujeto": s, "cuerpo": c}) + "\n")
        if self.path == "/access/v1/evaluation":
            n[0] += 1
            a = c["action"]["name"]
            ok = s in MIEMBROS and (a in TODOS or (a == "postgres:gestionar" and s == "persona:admin"))
            r = {"decision": ok, "context": {"id": "dec_%d" % n[0], "version": "v", "vale": 0}}
            if not ok:
                r["context"]["motivo"] = "no tienes `%s` en esta organización" % a
            codigo = 200
        elif self.path == "/access/v1/quien":
            r, codigo = {"handle": s.split(":", 1)[1]}, 200
        elif self.path == "/access/v1/eventos":
            r, codigo = {"id": c.get("id")}, 201
        else:
            r, codigo = {"error": "no"}, 404
        b = json.dumps(r).encode()
        self.send_response(codigo); self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(b))); self.end_headers(); self.wfile.write(b)
HTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
PYCODE

# ── ore-postgres de mentira: `ventas` es de ana; apunta lo que le llega ──
cat > "$TMP/pg.py" <<'PYCODE'
import json, sys
from http.server import BaseHTTPRequestHandler, HTTPServer
class H(BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def _r(self):
        largo = int(self.headers.get("Content-Length", 0) or 0)
        c = self.rfile.read(largo).decode() if largo else ""
        with open(sys.argv[2], "a") as f:
            f.write(json.dumps({"metodo": self.command, "camino": self.path,
                                "auth": self.headers.get("Authorization"), "cuerpo": c}) + "\n")
        if self.command == "GET" and self.path == "/v1/postgres/proyectos/ventas":
            r, codigo = {"id": "ventas", "dueno": "user:ana", "tenant": "t"}, 200
        elif self.command == "GET" and self.path.startswith("/v1/postgres/proyectos/"):
            r, codigo = {"error": "no hay ningún proyecto así"}, 404
        elif self.command == "GET":
            r, codigo = {"proyectos": []}, 200
        else:
            r, codigo = {"operacion": {"id": "op-1", "estado": "en-curso"}}, 202
        b = json.dumps(r).encode()
        self.send_response(codigo); self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(b))); self.end_headers(); self.wfile.write(b)
    do_GET = do_POST = do_DELETE = _r
HTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
PYCODE

IAM=$((PUERTO + 1)); PG=$((PUERTO + 2)); SIN_IAM=$((PUERTO + 3)); SIN_PG=$((PUERTO + 4))
"$PY" "$TMP/iam.py" "$IAM" "$TMP/iam.log" >/dev/null 2>&1 & PIDS="$PIDS $!"
"$PY" "$TMP/pg.py" "$PG" "$TMP/pg.log" >/dev/null 2>&1 & PIDS="$PIDS $!"
git init -q "$TMP/repo" && git -C "$TMP/repo" -c user.email=a@b -c user.name=a commit -q --allow-empty -m 0
printf 'token-de-la-celda' > "$TMP/iam.tok"
printf 'token-pg-de-la-celda' > "$TMP/pg.tok"
servir() { # <puerto> <acceso> <ore-postgres> <log>
  ORE_POSTGRES_DIRECCION="$3" ORE_POSTGRES_TESTIGO="$TMP/pg.tok" \
    "$SERVE" --repo "$TMP/repo" --bind "127.0.0.1:$1" --identidad cabecera --no-es-produccion \
    --organizacion demo --acceso "$2" --acceso-testigo "$TMP/iam.tok" >"$4" 2>&1 &
  PIDS="$PIDS $!"
}
servir "$PUERTO" "127.0.0.1:$IAM" "127.0.0.1:$PG" "$TMP/serve.txt"
servir "$SIN_IAM" "127.0.0.1:1" "127.0.0.1:$PG" "$TMP/serve-sin-iam.txt"
servir "$SIN_PG" "127.0.0.1:$IAM" "127.0.0.1:1" "$TMP/serve-sin-pg.txt"
for _ in $(seq 1 60); do
  curl -s -o /dev/null "http://127.0.0.1:$IAM/" && curl -s -o /dev/null "http://127.0.0.1:$PG/"     && curl -s -o /dev/null "http://127.0.0.1:$PUERTO/salud" && curl -s -o /dev/null "http://127.0.0.1:$SIN_IAM/salud" \
    && curl -s -o /dev/null "http://127.0.0.1:$SIN_PG/salud" && break; sleep 0.25
done
pide() { # <puerto> <metodo> <camino> <quien> [cuerpo]
  curl -s -o "$TMP/r.json" -w '%{http_code}' -X "$2" -H "authorization: Bearer $4" \
    -H 'content-type: application/json' ${5:+-d "$5"} "http://127.0.0.1:$1/v1/postgres$3"
}
ultimo_pg() { tail -1 "$TMP/pg.log" 2>/dev/null; }
cuantos_pg() { wc -l < "$TMP/pg.log" 2>/dev/null || echo 0; }

# ① crear: el dueño es el de `quien`, no el del cuerpo; con el token de la celda
c=$(pide "$PUERTO" POST /proyectos persona:ana '{"id":"ventas","dueno":"user:mallory"}')
[ "$c" = 202 ] || falla "① crear: $c $(cat "$TMP/r.json")"
"$PY" -c 'import json,sys; e=json.loads(sys.argv[1]); c=json.loads(e["cuerpo"]); assert e["metodo"]=="POST" and e["camino"]=="/v1/postgres/proyectos" and c=={"id":"ventas","dueno":"user:ana"} and e["auth"]=="Bearer token-pg-de-la-celda", e' "$(ultimo_pg)" \
  || falla "① a ore-postgres no llegó el dueño de quien crea con el token de la celda: $(ultimo_pg)"
dice "① crear: 202; a ore-postgres llega dueno=user:ana (no el del cuerpo) con el token de la celda"

# ② otro miembro, en un proyecto ajeno: 403, y ore-postgres sólo vio la ficha
antes=$(cuantos_pg)
c=$(pide "$PUERTO" DELETE /proyectos/ventas persona:beto)
[ "$c" = 403 ] && grep -q "postgres:gestionar" "$TMP/r.json" || falla "② beto borró lo de ana: $c $(cat "$TMP/r.json")"
[ "$(( $(cuantos_pg) - antes ))" = 1 ] && ultimo_pg | grep -q '"GET"' || falla "② a ore-postgres le llegó más que la ficha: $(tail -2 "$TMP/pg.log")"
c=$(pide "$PUERTO" POST /proyectos/ventas/ramas persona:beto '{"id":"dev"}')
[ "$c" = 403 ] || falla "② beto creó una rama en lo de ana: $c"
dice "② beto en lo de ana (borrar, una rama): 403 postgres:gestionar; ore-postgres sólo vio la ficha"

# ③ los roles son de `postgres:usar`: beto sí
c=$(pide "$PUERTO" POST /proyectos/ventas/ramas/main/roles persona:beto '{"nombre":"app"}')
[ "$c" = 202 ] && ultimo_pg | grep -q '"/v1/postgres/proyectos/ventas/ramas/main/roles"' || falla "③ roles: $c $(ultimo_pg)"
dice "③ un rol en lo de ana: beto sí (postgres:usar)"

# ④ el dueño gestiona lo suyo; admin, lo de cualquiera
c=$(pide "$PUERTO" POST /proyectos/ventas/ramas persona:ana '{"id":"dev"}')
[ "$c" = 202 ] || falla "④ ana no pudo en lo suyo: $c $(cat "$TMP/r.json")"
c=$(pide "$PUERTO" DELETE /proyectos/ventas persona:admin)
[ "$c" = 202 ] && ultimo_pg | grep -q '"DELETE"' || falla "④ admin no pudo borrar: $c $(cat "$TMP/r.json")"
grep -q '"postgres:gestionar"' "$TMP/iam.log" && dice "④ ana gestiona lo suyo; admin borra lo ajeno (postgres:gestionar)"

# ⑤ leer: cualquiera de la organización; de fuera, no
[ "$(pide "$PUERTO" GET /proyectos persona:beto)" = 200 ] || falla "⑤ beto no lee: $(cat "$TMP/r.json")"
c=$(pide "$PUERTO" GET /proyectos persona:intrusa)
[ "$c" = 403 ] || falla "⑤ una de fuera lee: $c"
dice "⑤ leer: beto 200; una de fuera, 403"

# ⑥ desde un puesto, nada (ni leer)
antes=$(cuantos_pg)
c=$(pide "$PUERTO" GET /proyectos agente:puesto/p1/a1)
[ "$c" = 403 ] && [ "$(cuantos_pg)" = "$antes" ] || falla "⑥ un puesto llegó a Postgres: $c $(cat "$TMP/r.json")"
dice "⑥ desde un puesto: 403, y ore-postgres no ve nada"

# ⑦ sin ore-iam, 503; sin ore-postgres, 503
c=$(pide "$SIN_IAM" POST /proyectos persona:ana '{"id":"otro"}')
[ "$c" = 503 ] || falla "⑦ sin ore-iam: $c $(cat "$TMP/r.json")"
c=$(pide "$SIN_PG" GET /proyectos persona:ana)
[ "$c" = 503 ] && grep -q reintentar "$TMP/r.json" || falla "⑦ sin ore-postgres: $c $(cat "$TMP/r.json")"
dice "⑦ sin ore-iam, 503; sin ore-postgres, 503 (reintentar)"

echo "✓ Postgres por la celda: crear con el dueño de ORE, el dueño o postgres:gestionar, roles con postgres:usar, nada desde un puesto, 503 sin quien decida o sin plano"
