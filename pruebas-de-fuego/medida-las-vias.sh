#!/usr/bin/env bash
# 0047 · M7 · Las vías a los datos que no pasan por `ore-serve`.
#
# Cada cuenta de servicio del cluster con identidad de GCP (Workload Identity), qué cargas la
# usan, y qué puede sobre datos del proyecto: roles en el proyecto, en cada bucket y en cada
# secreto de Secret Manager. DE LECTURA: nombres de cuentas, buckets, secretos y roles; nunca
# un valor.
#
#     bash pruebas-de-fuego/medida-las-vias.sh
#
# Necesita el contexto del cluster `ore-mesh` y `gcloud` con el proyecto por defecto.
set -euo pipefail

PROYECTO=$(gcloud config get-value project 2>/dev/null)

echo "== 1 · cuenta del cluster -> cuenta de GCP"
kubectl get sa -A -o jsonpath='{range .items[*]}{.metadata.namespace}{"\t"}{.metadata.name}{"\t"}{.metadata.annotations.iam\.gke\.io/gcp-service-account}{"\n"}{end}' \
  | awk -F'\t' '$3!=""' | sort | column -t -s $'\t'

echo
echo "== 2 · qué cargas usan cada cuenta (Deployments, StatefulSets, CronJobs, Jobs vivos, Pods sueltos)"
for k in deploy statefulset cronjob; do
  kubectl get "$k" -A -o jsonpath='{range .items[*]}{.metadata.namespace}{"\t"}'"$k"'/{.metadata.name}{"\t"}{.spec.template.spec.serviceAccountName}{.spec.jobTemplate.spec.template.spec.serviceAccountName}{"\n"}{end}'
done | awk -F'\t' '$3!="" && $3!="default"' | sort | column -t -s $'\t'
echo "-- pods vivos por cuenta (cuenta los de Jobs y puestos, que nacen y mueren)"
kubectl get pods -A -o jsonpath='{range .items[*]}{.metadata.namespace}{"\t"}{.spec.serviceAccountName}{"\n"}{end}' \
  | awk -F'\t' '$2!="" && $2!="default"' | sort | uniq -c | sort -k2

echo
echo "== 3 · roles en el proyecto de las cuentas ore-*"
gcloud projects get-iam-policy "$PROYECTO" --flatten='bindings[].members' \
  --format='value(bindings.members,bindings.role)' \
  | grep 'serviceAccount:ore-' | sed 's/serviceAccount://; s/@[^ \t]*//' | sort | column -t

echo
echo "== 4 · buckets: quién de ore-* puede qué"
# La condición va entera: `solo-la-capa` (el puesto) dice más que su título.
for b in $(gcloud storage buckets list --format='value(name)'); do
  gcloud storage buckets get-iam-policy "gs://$b" --format=json \
    | python -c '
import json, sys
b = sys.argv[1]
for x in json.load(sys.stdin).get("bindings", []):
    for m in x["members"]:
        if m.startswith("serviceAccount:ore-"):
            c = x.get("condition", {}).get("expression", "")
            print(b, m.split(":")[1].split("@")[0], x["role"], c.replace("projects/_/buckets/" + b + "/objects/", ""), sep="\t")
' "$b"
done | sort | column -t -s $'\t'

echo
echo "== 5 · secretos de Secret Manager: quién de ore-* puede qué (nombres, nunca valores)"
for s in $(gcloud secrets list --format='value(name)' 2>/dev/null); do
  gcloud secrets get-iam-policy "$s" --flatten='bindings[].members' \
    --format='value(bindings.members,bindings.role)' 2>/dev/null \
    | grep 'serviceAccount:ore-' | sed "s/serviceAccount://; s/@[^ \t]*//; s/^/$s\t/" || true
done | sort | column -t -s $'\t'
