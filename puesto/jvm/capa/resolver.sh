#!/bin/sh
# ═══════════════════════════════════════════════════════════════════════════
# RESOLVER LA CAPA DE LA JVM (ADR 0037 ③c) — dentro de `capa-jvm:1`
#
# Lo llama `54-la-capa-jvm.yaml`. Espera el árbol ya clonado y deja, en
# `$TRABAJO`: `deps.txt`, `digest.txt`, `jars/` y `informe.json`. No sube nada
# y no toca git: eso lo hace el contenedor que tiene la nube y el testigo, y
# esta imagen no los tiene A PROPÓSITO —lo único que necesita es salir a
# Central—.
#
# ⚠️ NO FALLA NUNCA (sale 0). Si Maven no puede resolver, el informe dice
#   `estado: error` con el motivo, que es lo que la consola sabe enseñar. Un
#   contenedor que se cae dejaría al que espera mirando un «pendiente» eterno.
#
#   $1  el árbol clonado        $2  el alcance (o vacío: la celda)
# ═══════════════════════════════════════════════════════════════════════════
set -eu
ARBOL="$1"
ALCANCE="${2:-}"
TRABAJO="${TRABAJO:-/trabajo}"
CAPA="${CAPA:-}"
# ⭐ El repositorio local de Maven vive EN EL JOB y muere con él (24 MB en el
#   caso medido). Compartirlo entre inquilinos es una optimización con su
#   propia pregunta de aislamiento, y no se responde aquí.
REPO="$TRABAJO/m2"
# ⭐ A VERSIÓN FIJA, y no es manía: este plugin es código que corre en nuestro
#   Job. Sin fijarlo, el Job ejecuta lo que Central tenga ese día.
PLUGIN="org.apache.maven.plugins:maven-dependency-plugin:3.8.1"
JAVA="${JAVA_HOME:-/opt/java/openjdk}/bin/java"
CLASES="/opt/ore/capa"

mkdir -p "$TRABAJO" "$REPO"

# ── 1 · la declaración: los pom del alcance, como los lee `ore-serve` ───────
"$JAVA" -cp "$CLASES" Capa declarar "$ARBOL" "$ALCANCE" "$TRABAJO"
DIGEST=$(cat "$TRABAJO/digest.txt")
if [ -z "$DIGEST" ]; then
  echo "· el árbol no declara dependencias de la JVM: nada que resolver"
  exit 0
fi
if [ -n "$CAPA" ] && [ "$DIGEST" != "$CAPA" ]; then
  echo "⚠️ se pidió $CAPA y el árbol declara $DIGEST (cambió entre medias): se resuelve lo que hay"
fi

# ── 2 · el pom que Maven resuelve: lo declarado y, detrás, lo de la imagen ──
#   (con el repositorio de la celda delante de Central si hay Artifacts: ADR 0074)
"$JAVA" -cp "$CLASES" Capa pom "$TRABAJO" /opt/ore/provisto.txt

# ── 2b · los registros de artefactos de la celda (ADR 0074 AR1·4) ──────────
# `ore-serve` firmó un token de LEER al rendir este Job (`ORE_ARTEFACTOS_TOKEN`,
# 1 h, toda la celda) y la plantilla trae la dirección (`ORE_ARTEFACTOS_URL`).
# ⭐ El `settings.xml` va en $HOME (/tmp de ESTE contenedor) y no en $TRABAJO:
#   el token no tiene por qué llegar al contenedor que sube.
# ⭐ El espejo `maven-default-http-blocker` se anula SÓLO para ORE: desde Maven
#   3.8.1 un repositorio `http://` se bloquea EN SILENCIO y todo sale de Central
#   (medido en AR0 P4); `ore-serve` es `http://` dentro de la celda.
AJUSTES=""
case "${ORE_ARTEFACTOS_TOKEN:-}" in
  ore_art_*)
    if [ -n "${ORE_ARTEFACTOS_URL:-}" ]; then
      AJUSTES="$HOME/settings-ore.xml"
      umask 077
      printf '<settings><mirrors><mirror><id>maven-default-http-blocker</id><mirrorOf>ninguno</mirrorOf><url>http://0.0.0.0/</url></mirror></mirrors><servers><server><id>ore</id><username>capa</username><password>%s</password></server></servers></settings>\n' \
        "$ORE_ARTEFACTOS_TOKEN" > "$AJUSTES"
      umask 022
    fi;;
esac

# ── 3 · resolver ───────────────────────────────────────────────────────────
# `-C`  política ESTRICTA de sumas: un jar que no cuadre con su `.sha1` de
#       Central rompe la resolución en vez de colarse.
# `-B`  sin color ni progreso, que esto lo lee un registro.
# `-U`  nada de metadatos rancios; no hay SNAPSHOT aquí, pero tampoco caché.
# `includeScope=runtime` es LA PIEZA: lo que la imagen pone va `provided`, y
#       `provided` no se copia. Medido en `medida-la-capa-de-la-jvm.py` §7.
mkdir -p "$TRABAJO/jars"
ESTADO=lista
# D-A6: lo que un registro de la celda tiene, sólo de ahí (`Capa artefactos`).
if [ -n "$AJUSTES" ] && ! "$JAVA" -cp "$CLASES" Capa artefactos "$TRABAJO"; then
  ESTADO=error
  cp "$TRABAJO/artefactos.txt" "$TRABAJO/mvn.log"
fi
MVN_AJUSTES=""
[ -n "$AJUSTES" ] && MVN_AJUSTES="-s $AJUSTES"
# ⛔ Y SIN `-q`. Lo parecía una limpieza y era perder LO ÚNICO que dice que
#    hubo un choque: con `-q`, el «must be unique … 2.19.0 vs 2.18.2» de Maven
#    no se imprime, y el informe se queda sin el aviso que la consola enseña.
#    `--no-transfer-progress` quita el ruido de las descargas y deja los avisos.
if [ "$ESTADO" = lista ] && mvn $MVN_AJUSTES -B -C -U --no-transfer-progress -f "$TRABAJO/pom.xml" -Dmaven.repo.local="$REPO" \
       "$PLUGIN:copy-dependencies" \
       -DincludeScope=runtime -DoutputDirectory="$TRABAJO/jars" \
       -Dmdep.useRepositoryLayout=false -Dmdep.overWriteReleases=true \
       > "$TRABAJO/mvn.log" 2>&1; then
  # El conjunto exacto, para el lock del informe. Si esto falla no se pierde
  # la capa: se pierde el lock, y el informe lo dirá por estar vacío.
  mvn $MVN_AJUSTES -B -C --no-transfer-progress -f "$TRABAJO/pom.xml" -Dmaven.repo.local="$REPO" \
      "$PLUGIN:list" -DincludeScope=runtime -DoutputFile="$TRABAJO/lista.txt" \
      >> "$TRABAJO/mvn.log" 2>&1 || echo "⚠️ el lock no se pudo listar"
  # ⭐ Y LO QUE EL REPOSITORIO QUERRÍA SIN CONTENEDOR (0037 ③c · d): no baja
  #   nada, sólo resuelve `pom-suyo.xml` para poder decir qué versión pedía.
  #   Cruzar esto con lo que la imagen pone da el aviso del choque SIEMPRE,
  #   también cuando lo arrastra una dependencia de otra —que es el caso que
  #   Maven resuelve en silencio—. Si falla, se pierde el aviso y no la capa.
  mvn $MVN_AJUSTES -B -C --no-transfer-progress -f "$TRABAJO/pom-suyo.xml" -Dmaven.repo.local="$REPO" \
      "$PLUGIN:list" -DincludeScope=runtime -DoutputFile="$TRABAJO/lista-suya.txt" \
      >> "$TRABAJO/mvn.log" 2>&1 || echo "⚠️ no se pudo saber qué versiones pedía el repositorio"
else
  ESTADO=error
  echo "✗ no se resolvió: $(tail -c 600 "$TRABAJO/mvn.log")"
fi

# ── 4 · el informe: lo resuelto, sus sumas y sus avisos ────────────────────
"$JAVA" -cp "$CLASES" Capa informe "$TRABAJO" "$ESTADO"
exit 0
