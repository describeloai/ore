#!/usr/bin/env bash
# P8·4a en producción (ADR 0058): lo que tarda `GET …/proyectos/{p}/almacenamiento` (lo que ocupa cada
# rama, del pageserver) con 1 rama y con 10, al lado de `GET …/ramas`. Medido DENTRO del pod de la celda
# (curl, time_total): el plano y el controller, sin la red de este PC. El Overview lo pide cada minuto.
#
#   p84.sh     (unos 3 min; `pg` despierto)
PRUEBA=p84
source "$(dirname "$0")/comun.sh"
A=demo

# Cinco veces cada camino, en un pod: «t CAMINO s1 s2 …» (segundos).
tiempos() {
  local c pasos=()
  for c in "$@"; do
    pasos+=("echo \"t $c \$(for i in 1 2 3 4 5; do curl -s -o /dev/null -w '%{time_total} ' -H \"Authorization: Bearer \$T\" \"\$URL$c\"; done)\"")
  done
  en "$A" "${pasos[@]}" | grep '^t '
}
p50ms() { percentil 0.5 $(for s in "$@"; do python -c "print(round(float('$s')*1000))"; done); }
mide() {   # mide ETIQUETA → números de almacenamiento y ramas
  local l
  while read -r _ c v; do
    case "$c" in
      */almacenamiento) numero "p84_almacenamiento_${1}_ms" "$(p50ms $v)"; echo "    ($c: $v s)";;
      */ramas) numero "p84_ramas_${1}_ms" "$(p50ms $v)"; echo "    ($c: $v s)";;
    esac
  done < <(tiempos "/v1/postgres/proyectos/$P/almacenamiento" "/v1/postgres/proyectos/$P/ramas")
}

echo "── la instancia"
P="p84-$(date +%s | tail -c 5)"
crea "$A" "$P" >/dev/null || exit 1
echo "  · $P"

echo "── con una rama"
mide 1rama
l=$(en "$A" "pide GET /v1/postgres/proyectos/$P/almacenamiento" | grep -E '^[0-9]{3} ')
case "$l" in 200*'"id":"main","logico":'*'"retencion_pitr":"1 day"'*) bien "main con su tamaño y la retención";; *) mal "almacenamiento: ${l:0:300}";; esac

echo "── nueve ramas más"
crear='for i in 1 2 3 4 5 6 7 8 9; do l=$(pide POST /v1/postgres/proyectos/'"$P"'/ramas "{\"id\":\"r$i\",\"padre\":\"main\"}"); op=$(echo "$l" | grep -o "\"id\":\"[^\"]*\"" | head -1 | cut -d\" -f4); case "$(hasta_hecha "/v1/postgres/operaciones/$op")" in *\"hecha\":true*) echo HECHA;; *) echo "r$i: $l";; esac; done'
# ⚠️ No se cuentan aquí: `kubectl run -i` pierde lo que el pod escribe antes de engancharse, y las
#   primeras salen deprisa. Lo que dice si están es la lista de abajo (diez, cada una con su tamaño).
en "$A" "$crear" >/dev/null

echo "── con diez ramas"
mide 10ramas
l=$(en "$A" "pide GET /v1/postgres/proyectos/$P/almacenamiento" | grep -E '^[0-9]{3} ')
n=$(printf '%s' "$l" | grep -o '"logico":' | wc -l)
[ "$n" = 10 ] && bien "diez ramas, cada una con su tamaño" || mal "con tamaño: $n de 10: ${l:0:300}"

borra "$A" "$P"; echo "  · $P borrado"
echo
[ $fallos = 0 ] && echo "P8·4a (producción) ✓" || { echo "P8·4a (producción) ✗ $fallos fallos"; exit 1; }
