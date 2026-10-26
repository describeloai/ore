#!/usr/bin/env python3
"""Toda migración de `iam/migraciones/` está en la lista de `iam/kustomization.yaml`, y la lista no
nombra ninguna que no exista.

⛔ El fallo que caza no se ve al empujar: la lista es la que Flux usa para el `ConfigMap`
`iam-migraciones`, y una migración que no está en ella no llega nunca al Job `migrar-iam`. La 053
(las potestades de los artefactos, 0074) pasó así un día en `main` sin aplicarse, y `ore-iam`
contestaba «potestad desconocida» al pedir un token de publicar."""
import os
import re
import sys

RAIZ = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
IAM = os.path.join(RAIZ, "iam")

en_disco = {f"migraciones/{f}" for f in os.listdir(os.path.join(IAM, "migraciones")) if f.endswith(".sql")}
with open(os.path.join(IAM, "kustomization.yaml"), encoding="utf-8") as f:
    en_lista = set(re.findall(r"^\s*-\s*(migraciones/\S+\.sql)\s*$", f.read(), re.M))

faltan = sorted(en_disco - en_lista)
sobran = sorted(en_lista - en_disco)
for m in faltan:
    print(f"⛔ {m} no está en iam/kustomization.yaml: no llegaría al ConfigMap ni a migrar-iam")
for m in sobran:
    print(f"⛔ iam/kustomization.yaml nombra {m}, que no existe")
if faltan or sobran:
    sys.exit(1)
print(f"✓ las {len(en_disco)} migraciones de iam/ están en la lista del ConfigMap")
