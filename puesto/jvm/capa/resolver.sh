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
"$JAVA" -cp "$CLASES" Capa pom "$TRABAJO" /opt/ore/provisto.txt

# ── 3 · resolver ───────────────────────────────────────────────────────────
# `-C`  política ESTRICTA de sumas: un jar que no cuadre con su `.sha1` de
#       Central rompe la resolución en vez de colarse.
# `-B`  sin color ni progreso, que esto lo lee un registro.
# `-U`  nada de metadatos rancios; no hay SNAPSHOT aquí, pero tampoco caché.
# `includeScope=runtime` es LA PIEZA: lo que la imagen pone va `provided`, y
#       `provided` no se copia. Medido en `medida-la-capa-de-la-jvm.py` §7.
mkdir -p "$TRABAJO/jars"
ESTADO=lista
# ⛔ Y SIN `-q`. Lo parecía una limpieza y era perder LO ÚNICO que dice que
#    hubo un choque: con `-q`, el «must be unique … 2.19.0 vs 2.18.2» de Maven
#    no se imprime, y el informe se queda sin el aviso que la consola enseña.
#    `--no-transfer-progress` quita el ruido de las descargas y deja los avisos.
if mvn -B -C -U --no-transfer-progress -f "$TRABAJO/pom.xml" -Dmaven.repo.local="$REPO" \
       "$PLUGIN:copy-dependencies" \
       -DincludeScope=runtime -DoutputDirectory="$TRABAJO/jars" \
       -Dmdep.useRepositoryLayout=false -Dmdep.overWriteReleases=true \
       > "$TRABAJO/mvn.log" 2>&1; then
  # El conjunto exacto, para el lock del informe. Si esto falla no se pierde
  # la capa: se pierde el lock, y el informe lo dirá por estar vacío.
  mvn -B -C --no-transfer-progress -f "$TRABAJO/pom.xml" -Dmaven.repo.local="$REPO" \
      "$PLUGIN:list" -DincludeScope=runtime -DoutputFile="$TRABAJO/lista.txt" \
      >> "$TRABAJO/mvn.log" 2>&1 || echo "⚠️ el lock no se pudo listar"
  # ⭐ Y LO QUE EL REPOSITORIO QUERRÍA SIN CONTENEDOR (0037 ③c · d): no baja
  #   nada, sólo resuelve `pom-suyo.xml` para poder decir qué versión pedía.
  #   Cruzar esto con lo que la imagen pone da el aviso del choque SIEMPRE,
  #   también cuando lo arrastra una dependencia de otra —que es el caso que
  #   Maven resuelve en silencio—. Si falla, se pierde el aviso y no la capa.
  mvn -B -C --no-transfer-progress -f "$TRABAJO/pom-suyo.xml" -Dmaven.repo.local="$REPO" \
      "$PLUGIN:list" -DincludeScope=runtime -DoutputFile="$TRABAJO/lista-suya.txt" \
      >> "$TRABAJO/mvn.log" 2>&1 || echo "⚠️ no se pudo saber qué versiones pedía el repositorio"
else
  ESTADO=error
  echo "✗ Maven no pudo resolver: $(tail -c 600 "$TRABAJO/mvn.log")"
fi

# ── 4 · el informe: lo resuelto, sus sumas y sus avisos ────────────────────
"$JAVA" -cp "$CLASES" Capa informe "$TRABAJO" "$ESTADO"
exit 0
