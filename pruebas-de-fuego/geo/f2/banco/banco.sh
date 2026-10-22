#!/usr/bin/env bash
# ORE 0072 F2·1·3 · El laboratorio de punta a punta de las teselas: el banco de C2·4 (ore-serve,
# el S3 de mentira, el árbol `geo`, la identidad de cabecera) MÁS `ore-teselas` en 127.0.0.1:8097,
# y ore-serve hablándole como en la celda (`ORE_TESELAS`). ore-serve escucha en 0.0.0.0:8080 para
# que el navegador del anfitrión llegue (`-p 8080:8080`). Lo lanza `medir.sh`.
set -euo pipefail
R=/repo
T=/tmp/banco
B=$R/target/release
rm -rf $T && mkdir -p $T
cd $T

# el S3 de mentira
python3 $R/pruebas-de-fuego/de-mentira.py s3 9000 > $T/s3.log 2>&1 &
until grep -q listo $T/s3.log 2>/dev/null; do sleep 0.2; done

# el árbol
mkdir -p arbol/packages/geo
cat > arbol/ontology.config.yaml <<'Y'
apiVersion: oos.dev/v1alpha1
kind: OntologyConfig
metadata: { name: demo, version: 0.1.0 }
Y
cat > arbol/packages/geo/package.yaml <<'Y'
apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: geo, version: 1.0.0, status: active, domain: geo }
spec: { owner: team:geo }
Y
git -C arbol init -q -b main && git -C arbol -c user.name=b -c user.email=b@x add -A && git -C arbol -c user.name=b -c user.email=b@x commit -q -m arbol

# la cola, con las plantillas del inquilino (como el banco del catálogo)
git init -q --bare -b main cola.git
python3 $R/malla/gen-inquilino.py demo --a $T/rendido > /dev/null
mkdir semilla && cp $T/rendido/plantilla-puesto.txt $T/rendido/plantilla-capa.txt $T/rendido/plantilla-capa-jvm.txt semilla/
git -C semilla init -q -b main && git -C semilla -c user.name=b -c user.email=b@x add -A \
  && git -C semilla -c user.name=b -c user.email=b@x commit -q -m plantilla && git -C semilla push -q $T/cola.git HEAD:main

# el motor de teselas (C3), como en la celda: un proceso aparte
ORE_TESELAS_PUERTO=8097 $B/ore-teselas > $T/teselas.log 2>&1 &
for i in $(seq 1 100); do curl -sf http://127.0.0.1:8097/salud >/dev/null && break; sleep 0.1; done

# ore-serve
export ORE_TESELAS=127.0.0.1:8097
export ORE_STORE=r2 ORE_R2_S3_ENDPOINT=http://127.0.0.1:9000 ORE_R2_BUCKET=copia ORE_R2_ACCESS_KEY_ID=de \
  ORE_R2_SECRET_ACCESS_KEY=mentira LAGO_URL=s3://copia FORJA_TOKEN=no-hace-falta PATH=$B:$PATH
$B/ore-serve --repo $T/arbol --ore $B/ore --bind 0.0.0.0:8080 --cola file://$T/cola.git \
  --identidad cabecera --no-es-produccion --organizacion demo > $T/serve.log 2>&1 &
for i in $(seq 1 100); do curl -sf http://127.0.0.1:8080/salud >/dev/null && break; sleep 0.25; done
curl -sf http://127.0.0.1:8080/salud >/dev/null || { tail -30 $T/serve.log; exit 1; }
echo "banco listo"
sleep infinity
