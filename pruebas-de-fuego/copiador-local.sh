#!/bin/sh
# ═══════════════════════════════════════════════════════════════════════════
# LA PASADA DEL COPIADOR, EN LOCAL (0073 §9, fase H)
#
#   docker run --rm -v "$PWD":/src alpine:3.20 sh -c \
#     'apk add -q git python3 >/dev/null && sh /src/pruebas-de-fuego/copiador-local.sh'
#
# `copiar-mantener.sh` y `copiar-worker.sh` de verdad, contra una forja `file://`,
# un `gcloud` y un `ore` de mentira (el `ore materialize` falso escribe el puntero
# de cada vista con un retardo, y una vista con `mala` en el nombre sale en error).
# Los punteros viven en el catálogo (0079 ·5g·3): el `ore` de mentira los guarda en
# `$CATALOGO_FALSO` y `ore puntero` los lee de ahí, que es lo que la pasada pregunta.
# Comprueba lo que la fase H promete y lo que la pasada ya hacía.
# ═══════════════════════════════════════════════════════════════════════════
set -u
MALLA=${MALLA:-/src/malla}
R=$(mktemp -d)
export PATH="$R/bin:$PATH" PUESTO="$R/puesto" TRABAJO="$R/trabajo" FORJA_URL="file://$R/forja.git"
export ORE_CATALOGO_ESQUEMA=t_prueba CATALOGO_FALSO="$R/catalogo"
export COFRE_URL=http://cofre.invalido SECRETOS=prueba ORGANIZACION=o EMISOR=http://idp.invalido \
  DIRECCION=http://idp.invalido REALM=o
FALLOS=0
bien() { echo "  ✓ $1"; }
falla() { echo "  ✗ $1"; FALLOS=$((FALLOS + 1)); }

# ── lo de mentira ───────────────────────────────────────────────────────────
mkdir -p "$R/bin"
cat > "$R/bin/gcloud" <<'EOF'
#!/bin/sh
# gcloud secrets versions access latest --secret=X --out-file=F
for a in "$@"; do case "$a" in --out-file=*) echo "valor-de-mentira" > "${a#--out-file=}";; esac; done
echo "$@" >> "$GCLOUD_LLAMADAS"
EOF
cat > "$R/bin/curl" <<'EOF'
#!/bin/sh
# El proveedor de identidad y el cofre: sólo la fuente `pg1` tiene secreto.
W="" U=""
for a in "$@"; do case "$a" in -w) W=1 ;; http*) U="$a" ;; esac; done
case "$U" in
  *openid-configuration) echo '{"issuer":"http://idp.invalido/realms/o"}' ;;
  */token) echo '{"access_token":"testigo-de-mentira"}' ;;
  */secretos/fuente-pg1) printf '{"valor":"postgres://u:clave@h/db"}'; [ -n "$W" ] && printf '\n200' ;;
  *) printf '{}'; [ -n "$W" ] && printf '\n404' ;;
esac
EOF
cat > "$R/bin/ore" <<'EOF'
#!/bin/sh
# `ore materialize . --vista v…` y `ore puntero <v>`; lo demás se apunta y no hace nada.
# Una vista `…secreta` lee la fuente `pg1`, y `…cerrada` la fuente `otra`: su
# credencial la pide al gancho, como `lector::url`.
if [ "$1" = puntero ]; then cat "$CATALOGO_FALSO/$2.json" 2>/dev/null; exit 0; fi
[ "$1" = materialize ] || { echo "$1" >> "$ORE_LLAMADAS"; exit 0; }
shift
VISTAS="" INFORME=""
while [ $# -gt 0 ]; do
  case "$1" in
    --vista) VISTAS="$VISTAS $2"; shift ;;
    --informe) INFORME="$2"; shift ;;
  esac
  shift
done
echo "$VISTAS" >> "$ORE_LLAMADAS"
for v in $VISTAS; do
  echo "$v"
  sleep "${RETARDO:-1}"
  P="$CATALOGO_FALSO/$v.json"
  mkdir -p "$(dirname "$P")"
  F=""
  case "$v" in *secreta*) F=pg1 ;; *cerrada*) F=otra ;; esac
  if [ -n "$F" ]; then
    if [ "$(sh "$ORE_CREDENCIAL" "$F")" = "postgres://u:clave@h/db" ]; then
      printf '{\n  "estado": "copiada",\n  "filas": 1\n}\n' > "$P"
    else
      printf '{\n  "estado": "error",\n  "motivo": "sin credencial de %s"\n}\n' "$F" > "$P"
    fi
    continue
  fi
  case "$v" in
    *mala*) printf '{\n  "estado": "error",\n  "motivo": "la fuente no contesta"\n}\n' > "$P"; echo "  error" ;;
    *) printf '{\n  "estado": "copiada",\n  "filas": 3,\n  "bytes": 30\n}\n' > "$P"; echo "  copiada" ;;
  esac
done
EOF
chmod +x "$R/bin/gcloud" "$R/bin/ore" "$R/bin/curl"

# El IdP y `ore-serve` de `cerrar-ejecucion.py`: el testigo, y cada cierre a un fichero.
mkdir -p /usr/local/lib/ore "$R/cierres"
cp "$MALLA/cerrar-ejecucion.py" /usr/local/lib/ore/cerrar-ejecucion.py
cat > "$R/servidor.py" <<'EOF'
import http.server, json, os, sys
class S(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        cuerpo = self.rfile.read(int(self.headers.get("content-length", 0)))
        if self.path.endswith("/token"):
            r = {"access_token": "testigo-del-agente"}
        else:
            id_ = self.path.split("/")[2]
            open(os.path.join(sys.argv[2], id_ + ".json"), "wb").write(cuerpo)
            r = {"id": id_, "written": True}
        b = json.dumps(r).encode()
        self.send_response(200); self.send_header("content-length", str(len(b))); self.end_headers()
        self.wfile.write(b)
    def log_message(self, *a): pass
http.server.HTTPServer(("127.0.0.1", int(sys.argv[1])), S).serve_forever()
EOF
python3 "$R/servidor.py" 8099 "$R/cierres" & SERVIDOR=$!
export ORE_SERVE=http://127.0.0.1:8099 DIRECCION=http://127.0.0.1:8099
sleep 1
export GCLOUD_LLAMADAS="$R/gcloud.log" ORE_LLAMADAS="$R/ore.log"

git init -q --bare -b main "$R/forja.git"
git clone -q "$FORJA_URL" "$R/semilla" 2>/dev/null
( cd "$R/semilla" && git config user.email s@x && git config user.name s \
  && printf 'datasources: []\n' > ontology.config.yaml && mkdir -p datasets && touch datasets/.keep \
  && git add -A && git commit -qm semilla && git push -q origin HEAD:main )
git -C "$R/forja.git" branch -q otra main

pasada() { # <id> <vistas>  → la salida en $R/p<id>.log
  sh "$MALLA/copiar-worker.sh" "$1" "$2" > "$R/p$1.log" 2>&1
}
en_el_catalogo() { cat "$CATALOGO_FALSO/$(echo "$1" | tr / .).json" 2>/dev/null; }

echo "1 · el mantenimiento: testigos y espejo"
sh "$MALLA/copiar-mantener.sh" > "$R/m.log" 2>&1 || falla "copiar-mantener.sh salió con error: $(cat "$R/m.log")"
[ -s "$PUESTO/forja" ] && [ -s "$PUESTO/agente-secreto" ] && bien "testigos en el puesto" || falla "faltan testigos"
[ -d "$TRABAJO/espejo/.git" ] && bien "espejo clonado" || falla "no hay espejo: $(cat "$R/m.log")"
sh "$MALLA/copiar-mantener.sh" > "$R/m.log" 2>&1 && grep -q "espejo al día" "$R/m.log" \
  && bien "la segunda vez lo trae al día" || falla "segunda vez: $(cat "$R/m.log")"

echo "2 · dos pasadas a la vez sobre el espejo, sin renovar testigos"
: > "$GCLOUD_LLAMADAS"
pasada 1 b.s.uno,b.s.dos & A=$!
pasada 2 c.s.tres,c.s.mala & B=$!
wait $A; wait $B
for p in 1 2; do
  grep -q "árbol del espejo" "$R/p$p.log" && bien "pasada $p: árbol del espejo" || falla "pasada $p no usó el espejo: $(cat "$R/p$p.log")"
  grep -q "renovando los testigos" "$R/p$p.log" && falla "pasada $p renovó testigos frescos" || true
done
[ -s "$GCLOUD_LLAMADAS" ] && falla "las pasadas llamaron a gcloud" || bien "ninguna pasada llamó a gcloud"
for v in b/s/uno b/s/dos c/s/tres; do
  en_el_catalogo "$v" | grep -q copiada && bien "$v en el catálogo" || falla "$v no está en el catálogo"
done
en_el_catalogo c/s/mala | grep -q error && bien "c/s/mala en el catálogo, en error" || falla "c/s/mala no está en el catálogo"
grep -q "@@ b.s.uno listo" "$R/p1.log" && grep -q "@@ b.s.dos listo" "$R/p1.log" && bien "@@ listo de la pasada 1" \
  || falla "faltan @@ listo: $(grep @@ "$R/p1.log")"
grep -q "@@ c.s.mala fallo la fuente no contesta" "$R/p2.log" && bien "@@ fallo con su motivo" \
  || falla "falta @@ fallo: $(grep @@ "$R/p2.log")"
N=$(git -C "$TRABAJO/espejo" worktree list | wc -l)
[ "$N" = 1 ] && bien "sin worktrees que sobren" || falla "quedan worktrees: $(git -C "$TRABAJO/espejo" worktree list)"
ls -d "$TRABAJO"/p-* >/dev/null 2>&1 && falla "quedan directorios de pasada: $(ls "$TRABAJO")" || bien "sin directorios de pasada"

[ "$(grep -c . "$ORE_LLAMADAS")" = 2 ] && bien "una llamada a ore materialize por pasada, ningún ore view" \
  || falla "llamadas a ore: $(cat "$ORE_LLAMADAS")"

echo "2b · las credenciales, por el gancho"
: > "$ORE_LLAMADAS"
pasada 5 e.s.secreta,e.s.cerrada
grep -q "@@ e.s.secreta listo" "$R/p5.log" && bien "la credencial llega por el gancho" \
  || falla "gancho: $(cat "$R/p5.log")"
grep -q "@@ e.s.cerrada fallo sin credencial de otra" "$R/p5.log" && bien "sin secreto, falla ese dataset y no los demás" \
  || falla "sin secreto: $(grep @@ "$R/p5.log")"
grep -q "clave" "$R/p5.log" && falla "el valor de la credencial salió al registro" || bien "el valor no sale al registro"
grep -q "fuente-otra" "$R/p5.log" && bien "el motivo del custodio, en el registro" || falla "falta el motivo del custodio"

echo "2c · los punteros salen mientras ore copia, y cada paso dura lo suyo"
export EJECUCION=copiador-6 RETARDO=2
pasada 6 f.s.uno,f.s.dos,f.s.tres
unset EJECUCION RETARDO
A=$(grep -n "@@ f.s.uno listo" "$R/p6.log" | cut -d: -f1)
B=$(grep -nx "f.s.tres" "$R/p6.log" | cut -d: -f1)
[ -n "$A" ] && [ -n "$B" ] && [ "$A" -lt "$B" ] && bien "el primero se publica antes de que empiece el tercero" \
  || falla "publicar mientras copia: @@ en $A, f.s.tres en $B: $(cat "$R/p6.log")"
N=$(ls "$CATALOGO_FALSO"/f.s.*.json 2>/dev/null | wc -l)
[ "$N" = 3 ] && bien "los $N punteros de la pasada, en el catálogo" || falla "sólo $N puntero(s) de la pasada"
python3 - "$R/cierres/copiador-6.json" <<'EOF' && bien "el cierre lleva tres pasos ok con su duración" || falla "el cierre: $(cat "$R/cierres/copiador-6.json" 2>&1)"
import json, sys
c = json.load(open(sys.argv[1]))
assert c["status"] == "ok", c["status"]
d = {p["name"]: p for p in c["steps"]}
assert set(d) == {"f.s.uno", "f.s.dos", "f.s.tres"}, d
for p in d.values():
    assert p["status"] == "ok", p
    assert 1500 <= p.get("duration_ms", 0) <= 4000, p
EOF
grep -q "testigo-del-agente" "$R/cierres/copiador-6.json" && falla "el testigo del agente salió en el cierre" \
  || bien "el testigo del agente no sale en el cierre"

echo "3 · sin espejo, el clon de siempre"
rm -rf "$TRABAJO/espejo"
pasada 3 d.s.cuatro
grep -q "árbol clonado" "$R/p3.log" && grep -q "@@ d.s.cuatro listo" "$R/p3.log" \
  && bien "clona y copia" || falla "sin espejo: $(cat "$R/p3.log")"

echo "4 · con los testigos caducados, la pasada los renueva"
touch -d "2000-01-01" "$PUESTO/forja"
pasada 4 d.s.cinco
grep -q "renovando los testigos" "$R/p4.log" && grep -q "@@ d.s.cinco listo" "$R/p4.log" \
  && bien "los renueva y copia" || falla "testigos caducados: $(cat "$R/p4.log")"

ls -a "$R/trabajo" | grep -q "estado" && falla "queda el estado de alguna pasada: $(ls -a "$R/trabajo")" \
  || bien "sin estado de pasadas al terminar"
kill "$SERVIDOR" 2>/dev/null
rm -rf "$R"
[ "$FALLOS" = 0 ] && { echo "todo en verde"; exit 0; }
echo "$FALLOS fallo(s)"
exit 1
