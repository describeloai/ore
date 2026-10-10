#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════════════
# GENERA `19-envoy-gateway.yaml` — Envoy Gateway, rendido y versionado (ORE 0060 C2·1)
#
#   bash malla/gen-envoy-gateway.sh            # reescribe 19-envoy-gateway.yaml
#
# ⭐ Como Flux (`12-flux.yaml`) y los CRDs de Keycloak: sin `helm-controller`, lo de
#   terceros entra en la malla como YAML rendido, y el diff de una subida de versión
#   se lee en el PR.
#
# ── ⛔ LA VERSIÓN NO ES LA ÚLTIMA, Y ES A PROPÓSITO ─────────────────────────────
#
# Los CRDs de Gateway API **los gestiona GKE** (`addonmanager.kubernetes.io/mode`,
# hoy `bundle-version: v1.5.0`, canal estándar). Envoy Gateway fija contra qué
# versión de Gateway API está probado: v1.8 → v1.5.1, v1.9 → v1.6.1. Con v1.9 el
# controlador escribiría campos que estos CRDs no conocen. ⇒ **v1.8**, y sube
# cuando GKE suba sus CRDs. Fin de soporte de la v1.8: 2026-11-08.
#
# ── Lo que se quita del chart, y por qué ─────────────────────────────────────
#
#   crds.enabled=false        sus CRDs de Gateway API pisarían los de GKE (y GKE
#                             los devolvería: «NewerRevision»). Los SUYOS
#                             (gateway.envoyproxy.io) se añaden aparte, abajo.
#   topologyInjector=false    un webhook de mutación más sobre los pods; dormida la
#                             malla, otra pieza que tiene que estar en pie antes
#                             que nada. No hace falta con una réplica.
#   ttl del certgen fuera     el Job de los certificados se borraría a los 30 s y
#                             Flux lo volvería a crear en cada reconciliación.
# ═══════════════════════════════════════════════════════════════════════════
set -euo pipefail

VERSION=v1.8.5
NS=envoy-gateway-system
AQUI=$(cd "$(dirname "$0")" && pwd)
SALIDA="$AQUI/19-envoy-gateway.yaml"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

helm pull oci://docker.io/envoyproxy/gateway-helm --version "$VERSION" --untar -d "$TMP" >/dev/null
CRDS="$TMP/gateway-helm/charts/crds/crds/generated"

{
  cat <<EOF
# ═══════════════════════════════════════════════════════════════════════════
# ENVOY GATEWAY $VERSION — GENERADO por \`malla/gen-envoy-gateway.sh\`. NO SE EDITA A MANO.
#
# El controlador de la puerta del túnel (ORE 0060 C2). Por qué esta versión y qué
# se le quita al chart: la cabecera del generador.
# ═══════════════════════════════════════════════════════════════════════════
apiVersion: v1
kind: Namespace
metadata:
  name: $NS
  labels:
    ore.dev/rol: puerta-del-tunel
EOF
  for f in "$CRDS"/gateway.envoyproxy.io_*.yaml; do
    printf -- '---\n'
    cat "$f"
  done
  helm template eg "$TMP/gateway-helm" \
    --namespace "$NS" \
    --set crds.enabled=false \
    --set topologyInjector.enabled=false \
    --set certgen.job.ttlSecondsAfterFinished=null \
    | sed -e '/^\s*helm.sh\/hook/d' -e '/^\s*"helm.sh\/hook/d'
} > "$SALIDA"

echo "escrito $SALIDA ($(grep -c '^kind:' "$SALIDA") objetos)"
grep '^kind:' "$SALIDA" | sort | uniq -c
