#!/usr/bin/env bash
# P4·4 · roles y bases (ADR 0058). Hecho cuando: un rol creado por el API se conecta; regenerar su
# contraseña invalida la vieja.
#
# Todo por el API como una celda, y la conexión por la overlay desde `cliente-overlay` (el lado del
# proxy) con el rol y la contraseña que dio el API. La contraseña sólo existe en la respuesta: aquí se
# guarda en una variable y no se imprime.
#
#   p44.sh [celda]      (demo por defecto)
export ORE_PG_COMPUTO=vm
source "$(dirname "$0")/entorno.sh"
A=${1:-demo}
P="p44-$(date +%s | tail -c 6)"
PRUEBA=p44
source "$(dirname "$0")/celdas.sh"
abrir_celdas "$A"
R="/v1/postgres/proyectos/$P/ramas/main"
# como ROL CLAVE BASE IP SQL → lo que contesta (o el error)
como() { k exec cliente-overlay -- env PGPASSWORD="$2" PGCONNECT_TIMEOUT=5 \
  psql -h "$4" -p 5432 -U "$1" -d "$3" -Atc "$5" 2>&1 | tail -1; }

echo "── $A crea $P con dueño user:p44: su rol, su contraseña (una vez) y su base"
mapfile -t L < <(en "$A" "pide POST /v1/postgres/proyectos '{\"id\":\"$P\",\"dueno\":\"user:p44\"}'")
espera 202 "crear" "${L[0]}"
OP=$(campo "${L[0]}" operacion id); ROL=$(campo "${L[0]}" rol nombre); CLAVE=$(campo "${L[0]}" rol contrasena)
[ "$ROL" = p44 ] && [ ${#CLAVE} = 32 ] && echo "  ✓ rol $ROL, con una contraseña de 32" || { echo "  ✗ rol: «$ROL»"; fallos=$((fallos+1)); }
mapfile -t L < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OP" "pide GET $R/endpoints/principal" "pide GET $R/roles")
DIR=$(campo "${L[1]}" direccion)
[ "$(campo "${L[0]}" estado)" = hecha ] && echo "  ✓ listo; principal en $DIR" || { echo "  ✗ ${L[0]:0:300}"; exit 1; }
case "${L[2]}" in *SCRAM*|*"$CLAVE"*) echo "  ✗ la lista de roles enseña el verificador o la contraseña"; fallos=$((fallos+1));;
  *) echo "  ✓ la lista de roles no enseña ni verificador ni contraseña";; esac

echo "── entrar con él, en su base"
[ "$(como "$ROL" "$CLAVE" "$P" "$DIR" 'select current_user')" = "$ROL" ] && echo "  ✓ entra como $ROL en $P" \
  || { echo "  ✗ no entra: $(como "$ROL" "$CLAVE" "$P" "$DIR" 'select 1')"; fallos=$((fallos+1)); }
[ "$(como "$ROL" "$CLAVE" "$P" "$DIR" 'create table t (n int); insert into t values (1); select count(*) from t')" = 1 ] \
  && echo "  ✓ y es dueño: crea una tabla y escribe" || { echo "  ✗ no puede escribir en su base"; fallos=$((fallos+1)); }
case "$(como "$ROL" "mala" "$P" "$DIR" 'select 1')" in 1) echo "  ✗ entra con cualquier contraseña"; fallos=$((fallos+1));;
  *) echo "  ✓ con otra contraseña, no";; esac

echo "── un rol más, en caliente (compute_ctl /configure, sin reiniciar)"
mapfile -t L < <(en "$A" "pide POST $R/roles '{\"nombre\":\"app\"}'")
espera 202 "crear app" "${L[0]}"; OP=$(campo "${L[0]}" operacion id); CLAVE_APP=$(campo "${L[0]}" rol contrasena)
mapfile -t L < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OP")
[ "$(campo "${L[0]}" estado)" = hecha ] || { echo "  ✗ ${L[0]:0:300}"; fallos=$((fallos+1)); }
[ "$(como app "$CLAVE_APP" postgres "$DIR" 'select current_user')" = app ] && echo "  ✓ app entra" \
  || { echo "  ✗ app no entra: $(como app "$CLAVE_APP" postgres "$DIR" 'select 1')"; fallos=$((fallos+1)); }

echo "── regenerar la de app: la vieja deja de entrar"
mapfile -t L < <(en "$A" "pide POST $R/roles/app/contrasena")
espera 202 "regenerar" "${L[0]}"; OP=$(campo "${L[0]}" operacion id); NUEVA=$(campo "${L[0]}" rol contrasena)
mapfile -t L < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OP")
case "$(como app "$CLAVE_APP" postgres "$DIR" 'select 1')" in 1) echo "  ✗ la vieja sigue entrando"; fallos=$((fallos+1));;
  *) echo "  ✓ la vieja, no";; esac
[ "$(como app "$NUEVA" postgres "$DIR" 'select current_user')" = app ] && echo "  ✓ la nueva, sí" \
  || { echo "  ✗ la nueva no entra"; fallos=$((fallos+1)); }

echo "── borrar el rol app (en caliente) y el proyecto"
mapfile -t L < <(en "$A" "pide DELETE $R/roles/app")
OP=$(campo "${L[0]}" operacion id)
mapfile -t L < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OP")
case "$(como app "$NUEVA" postgres "$DIR" 'select 1')" in 1) echo "  ✗ app sigue entrando"; fallos=$((fallos+1));;
  *) echo "  ✓ app ya no existe";; esac
mapfile -t L < <(en "$A" "pide DELETE /v1/postgres/proyectos/$P")
OP=$(campo "${L[0]}" operacion id)
mapfile -t L < <(en "$A" "hasta_hecha /v1/postgres/operaciones/$OP")
[ "$(campo "${L[0]}" estado)" = hecha ] && echo "  ✓ borrado" || { echo "  ✗ ${L[0]:0:300}"; fallos=$((fallos+1)); }

echo
[ $fallos = 0 ] && echo "P4·4 ✓ todo" || { echo "P4·4 ✗ $fallos fallos"; exit 1; }
