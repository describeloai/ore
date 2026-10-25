#!/usr/bin/env bash
# ORE 0074 AR2 · lo que corre dentro del contenedor de Python de `ar2-local.sh`: uv y twine de verdad.
# Espera PUBLICAR, OTRO (token de otro registro), MAVEN (token de un registro sin pypi), LEER y ORE.
set -u
fallos=0; falla() { echo "  ✗ $*"; fallos=$((fallos+1)); }; dice() { echo "  ✓ $*"; }
pip install -q --root-user-action=ignore --disable-pip-version-check uv==0.12.23 twine 2>&1 | grep -v "^$" | tail -2
apt-get -qq update >/dev/null 2>&1 && apt-get -qq install -y curl >/dev/null 2>&1
export UV_NO_PROGRESS=1 UV_PYTHON_DOWNLOADS=never
cd /tmp
P="$ORE/artifacts/banco/riesgo/py/pypi/"

proyecto() { # version -> dist/ de `uv build` de banco_riesgo con X = 42
  rm -rf p dist && mkdir -p p/src/banco_riesgo
  printf 'X = 42\n' > p/src/banco_riesgo/__init__.py
  printf '[project]\nname = "Banco_Riesgo"\nversion = "%s"\nrequires-python = ">=3.10"\n\n[build-system]\nrequires = ["uv_build>=0.9,<0.13"]\nbuild-backend = "uv_build"\n' "$1" > p/pyproject.toml
  (cd p && uv build -q --out-dir ../dist) || return 1
}

echo "1 · publicar"
proyecto 1.0 || falla "uv build 1.0"
if uv publish --publish-url "$P" --username __token__ --password "$PUBLICAR" dist/* > log 2>&1; then
  dice "uv publish de $(ls dist | tr '\n' ' ')→ rueda y sdist (D-A15)"
else falla "uv publish: $(tail -3 log)"; fi
cp -r dist dist-1.0
proyecto 1.1 || falla "uv build 1.1"
if twine upload --non-interactive --disable-progress-bar --repository-url "$P" -u __token__ -p "$PUBLICAR" dist/*.whl > log 2>&1; then
  dice "twine upload de la rueda 1.1"
else falla "twine: $(tail -3 log)"; fi

echo "2 · lo que no entra"
uv publish --publish-url "$P" --username __token__ --password "$PUBLICAR" dist-1.0/*.whl > log 2>&1 \
  && falla "republicar entró" || { grep -q "409" log && dice "republicar la rueda 1.0 → 409 ($(grep -o 'File already exists' log | head -1))" || falla "republicar: $(tail -2 log)"; }
proyecto 1.2 >/dev/null
W=$(ls dist/*.whl)
c=$(curl -s -o r.txt -w '%{http_code}' -F ':action=file_upload' -F name=Banco_Riesgo -F version=1.2 -F "content=@$W" "$P")
[ "$c" = 401 ] && dice "sin token → 401" || falla "sin token: $c"
c=$(curl -s -o r.txt -w '%{http_code}' -u "__token__:$OTRO" -F ':action=file_upload' -F name=Banco_Riesgo -F version=1.2 -F "content=@$W" "$P")
[ "$c" = 403 ] && dice "token de otro registro → 403" || falla "otro registro: $c $(cat r.txt)"
c=$(curl -s -o r.txt -w '%{http_code}' -u "__token__:$MAVEN" -F ':action=file_upload' -F name=Banco_Riesgo -F version=1.2 -F "content=@$W" "$ORE/artifacts/banco/riesgo/libs/pypi/")
[ "$c" = 400 ] && grep -q "does not take .pypi." r.txt && dice "un registro sin pypi en sus formats → 400" || falla "registro sin pypi: $c $(cat r.txt)"
c=$(curl -s -o r.txt -w '%{http_code}' -u "__token__:$PUBLICAR" -F ':action=file_upload' -F name=Banco_Riesgo -F version=1.2 \
  -F "sha256_digest=$(printf '0%.0s' $(seq 64))" -F "content=@$W" "$P")
[ "$c" = 400 ] && grep -q "upload was altered" r.txt && dice "sha256_digest que no casa → 400" || falla "suma: $c $(cat r.txt)"
c=$(curl -s -o r.txt -w '%{http_code}' -u "__token__:$PUBLICAR" -F ':action=file_upload' -F name=Banco_Riesgo -F version=1.2 \
  -F "content=@$W" -F tarde=x "$P")
[ "$c" = 400 ] && dice "un campo detrás del fichero → 400 ($(grep -o 'not arrive whole' r.txt))" || falla "fichero no último: $c $(cat r.txt)"
c=$(curl -s -o r.txt -w '%{http_code}' -u "__token__:$PUBLICAR" -F ':action=file_upload' -F name=otro-nombre -F version=1.2 -F "content=@$W" "$P")
[ "$c" = 400 ] && dice "un name que no es el del fichero → 400" || falla "name distinto: $c $(cat r.txt)"

echo "3 · el índice simple"
c=$(curl -s -o idx.html -D h.txt -w '%{http_code}' -u "capa:$LEER" "$ORE/artifacts/pypi/simple/banco-riesgo/")
n=$(grep -c '#sha256=' idx.html)
[ "$c" = 200 ] && grep -qi '^content-type: text/html' h.txt && [ "$n" = 3 ] \
  && dice "…/pypi/simple/banco-riesgo/ → 200 text/html con 3 ficheros y su #sha256 (sin 1.2: lo que falló no quedó)" || falla "índice: $c, $n enlaces; $(cat idx.html)"
grep -q 'data-requires-python="&gt;=3.10"' idx.html && dice "con data-requires-python" || falla "sin data-requires-python"
c=$(curl -s -o /dev/null -D h.txt -w '%{http_code}' -u "capa:$LEER" "$ORE/artifacts/pypi/simple/Banco_Riesgo")
[ "$c" = 301 ] && grep -qi '^location: /artifacts/pypi/simple/banco-riesgo/' h.txt && dice "Banco_Riesgo → 301 al normalizado" || falla "redirige: $c"
c=$(curl -s -o /dev/null -w '%{http_code}' -u "capa:$LEER" "$ORE/artifacts/pypi/simple/requests/")
[ "$c" = 404 ] && dice "un nombre de ningún registro → 404" || falla "nombre ajeno: $c"
c=$(curl -s -o /dev/null -w '%{http_code}' -u "__token__:$OTRO" "$ORE/artifacts/pypi/simple/banco-riesgo/")
[ "$c" = 404 ] && dice "un token que no puede leer ese registro → 404, no 403 (AR2·0)" || falla "token de otro registro al leer: $c"
c=$(curl -s -o /dev/null -w '%{http_code}' -u "capa:ore_art_00.00" "$ORE/artifacts/pypi/simple/banco-riesgo/")
[ "$c" = 401 ] && dice "un token que no vale → 401" || falla "token falso: $c"

echo "4 · resolver como la capa (uv lock, ORE delante de PyPI)"
rm -rf c && mkdir c && cd c
printf '[project]\nname = "capa"\nversion = "0"\nrequires-python = ">=3.10"\ndependencies = ["banco-riesgo==1.0.0"]\n\n[tool.uv]\npackage = false\nno-build = true\n' > pyproject.toml
export UV_INDEX="ore=$ORE/artifacts/pypi/simple/" UV_INDEX_ORE_USERNAME=capa UV_INDEX_ORE_PASSWORD="$LEER"
if uv lock > ../log 2>&1; then
  grep -A2 'name = "banco-riesgo"' uv.lock | grep -q 'version = "1.0"' && dice "banco-riesgo==1.0.0 resuelve a la 1.0 publicada (PEP 440)" || falla "versión: $(grep -A3 banco-riesgo uv.lock)"
  grep -q "ar2-ore:8080" uv.lock && dice "de ORE" || falla "no salió de ORE"
  grep -q "ore_art_" uv.lock && falla "el token quedó en el lock" || dice "el lock no lleva el token"
  uv export -q --format pylock.toml --no-emit-project -o pylock.toml && uv pip install -q --no-deps --target ../t -r pylock.toml > ../log 2>&1 \
    && [ "$(cd .. && PYTHONPATH=t python -c 'import banco_riesgo; print(banco_riesgo.X)')" = 42 ] \
    && dice "instalada desde ORE: import banco_riesgo → 42" || falla "instalar: $(tail -3 ../log)"
else falla "uv lock: $(tail -4 ../log)"; fi
sed -i 's/==1.0.0/==9.9/' pyproject.toml
uv lock > ../log 2>&1 && falla "==9.9 resolvió (debía fallar)" || dice "banco-riesgo==9.9 (no publicada) falla sin buscarla en PyPI"
cd ..
unset UV_INDEX UV_INDEX_ORE_USERNAME UV_INDEX_ORE_PASSWORD

echo "5 · el nombre es de un registro"
proyecto 2.0 >/dev/null
uv publish --publish-url "$ORE/artifacts/banco/riesgo/py2/pypi/" --username __token__ --password "$OTRO" dist/*.whl > log 2>&1 \
  && falla "banco-riesgo entró en py2" || { grep -q 409 log && dice "banco-riesgo en banco.riesgo.py2 → 409 (D-A13)" || falla "otro registro: $(tail -2 log)"; }

echo; [ $fallos -eq 0 ] && echo "AR2 dentro: todo en verde" || echo "AR2 dentro: $fallos fallo(s)"
exit $fallos
