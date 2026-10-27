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
# Comprueba lo que la fase H promete y lo que la pasada ya hacía.
# ═══════════════════════════════════════════════════════════════════════════
set -u
MALLA=${MALLA:-/src/malla}
R=$(mktemp -d)
export PATH="$R/bin:$PATH" PUESTO="$R/puesto" TRABAJO="$R/trabajo" FORJA_URL="file://$R/forja.git"
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
# Sólo `ore materialize . --vista v… --informe DIR`; lo demás se apunta y no hace nada.
# Una vista `…secreta` lee la fuente `pg1`, y `…cerrada` la fuente `otra`: su
# credencial la pide al gancho, como `lector::url`.
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
  P="$INFORME/$(echo "$v" | tr . /).json"
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
en_la_forja() { git --git-dir="$R/forja.git" show "main:datasets/$1.json" 2>/dev/null; }

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
  en_la_forja "$v" | grep -q copiada && bien "$v en la forja" || falla "$v no está en la forja"
done
en_la_forja c/s/mala | grep -q error && bien "c/s/mala en la forja, en error" || falla "c/s/mala no está en la forja"
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

rm -rf "$R"
[ "$FALLOS" = 0 ] && { echo "todo en verde"; exit 0; }
echo "$FALLOS fallo(s)"
exit 1
