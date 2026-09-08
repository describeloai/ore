#!/usr/bin/env bash
# Dos controladores reservaban 800m de un nodo de 1930m sin hacer nada.
#
# ── Por qué esto es un script y no un manifiesto ────────────────────────────
#
# Los dos Deployments son AJENOS —vienen de los manifiestos de Kueue y del
# operador de Keycloak, aplicados desde sus URLs—. Un YAML nuestro que los
# redeclarara sería un YAML que se queda viejo el día que se actualice
# cualquiera de los dos, y que además tendría que repetir campos que no son
# nuestros para poder tocar el que sí. Un `patch` toca lo que toca.
#
# ── Lo que se midió ─────────────────────────────────────────────────────────
#
# Al intentar levantar el IdP, todo quedó `Pending` con «Insufficient cpu». El
# nodo estable estaba al **96%** de CPU RESERVADA, y de esos:
#
#     kueue-controller-manager   500m     un controlador que admite Jobs
#     keycloak-operator          300m     un operador con un CR
#     kube-dns                   270m
#     anetd                      205m
#
# Las requests son RESERVAS, no consumo: esos 800m no se estaban gastando, se
# estaban apartando. Y apartados por dos controladores que en este clúster
# reconcilian un puñado de objetos.
#
# ── ⚠️ El atasco que esto provoca, y cómo se sale ───────────────────────────
#
# Bajar la request no basta: el rollout crea el pod NUEVO antes de matar al
# viejo, y el nuevo tampoco cabe porque el viejo sigue reservando. Los dos
# quedan `Pending` y el despliegue se queda quieto sin decir por qué.
#
# ⇒ Hay que **borrar el pod viejo** para que el nuevo entre. Se dice porque el
#   síntoma —«el rollout no avanza»— no menciona en ninguna parte que la causa
#   sea el propio rollout.
set -eu

echo "== antes =="
kubectl describe node -l cloud.google.com/gke-nodepool=default-pool \
  | sed -n '/Allocated resources/,/hugepages/p' | grep cpu

kubectl -n kueue-system patch deployment kueue-controller-manager --type=json \
  -p '[{"op":"replace","path":"/spec/template/spec/containers/0/resources/requests/cpu","value":"150m"}]'
kubectl -n identidad patch deployment keycloak-operator --type=json \
  -p '[{"op":"replace","path":"/spec/template/spec/containers/0/resources/requests/cpu","value":"100m"}]'

# Y el empujón que rompe el atasco. `--wait=false`: el ReplicaSet los recrea
# con la request nueva en cuanto hay hueco.
kubectl -n kueue-system delete pod -l control-plane=controller-manager --wait=false || true
kubectl -n identidad delete pod -l app.kubernetes.io/name=keycloak-operator --wait=false || true

kubectl -n kueue-system rollout status deploy/kueue-controller-manager --timeout=300s
kubectl -n identidad rollout status deploy/keycloak-operator --timeout=300s

echo "== despues =="
kubectl describe node -l cloud.google.com/gke-nodepool=default-pool \
  | sed -n '/Allocated resources/,/hugepages/p' | grep cpu
