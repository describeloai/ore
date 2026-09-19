#!/usr/bin/env bash
# GOOGLE EN PRIVADO (0031 W3.1): `*.googleapis.com` → `private.googleapis.com`
#
# ── Por qué ─────────────────────────────────────────────────────────────────
# Una NetworkPolicy abre direcciones, no nombres. Para que un puesto alcance
# el bucket de copias y NADA MÁS, `storage.googleapis.com` tiene que resolver a
# un rango que se pueda nombrar: `private.googleapis.com` son cuatro
# direcciones (199.36.153.8/30) que sirven todas las APIs de Google desde
# dentro de la VPC, sin salir a internet (Private Google Access, que la subred
# `ore-mesh-europe-west1` ya tiene puesto). Es el «paso 2» que `20-driver.yaml`
# dejó escrito como promesa el 2026-09-10.
#
# ── Qué hace ────────────────────────────────────────────────────────────────
#   una zona DNS PRIVADA `googleapis.com.` visible sólo desde la VPC `ore-mesh`,
#   con `private.googleapis.com A 199.36.153.8-11` y
#   `*.googleapis.com CNAME private.googleapis.com.`
#
# ── Lo que cambia para todos ────────────────────────────────────────────────
#   TODO lo que resuelva `*.googleapis.com` desde la VPC (los nodos, ore-serve,
#   los Jobs del driver, la máquina de modelos) pasa a ir por el rango privado.
#   Con Private Google Access funciona igual; sin él, dejaría de funcionar —por
#   eso se comprueba primero y el guion se niega si no está.
#   `*.pkg.dev` (las imágenes) no es `googleapis.com`: no lo toca.
#
# Uso:  bash malla/72-google-en-privado.sh [--seco]
set -u

PROYECTO="project-8853a180-450d-47be-b83"
RED="ore-mesh"
SUBRED="ore-mesh-europe-west1"
LUGAR="europe-west1"
ZONA="googleapis-en-privado"
RANGO="199.36.153.8/30"           # = 21-el-puesto.yaml, cotejado abajo

SECO=""; [ "${1:-}" = "--seco" ] && SECO="1"
RAIZ="$(cd "$(dirname "$0")/.." && pwd)"
GCLOUD=$(command -v gcloud.cmd || command -v gcloud) || { echo "✗ hace falta gcloud" >&2; exit 1; }

hecho() { echo "  ✓ $*"; }
ya()    { echo "  · $* — ya estaba"; }
haria() { echo "  ~ $*"; }
falla() { echo "✗ $*" >&2; exit 1; }
g() { "$GCLOUD" "$@" --project "$PROYECTO" --quiet 2>/dev/null; }

echo "GOOGLE EN PRIVADO${SECO:+   (EN SECO: no se escribe nada)}"

# ── 0 · el rango es el de la política, y la subred tiene acceso privado ──────
grep -q "cidr: $RANGO" "$RAIZ/malla/21-el-puesto.yaml" || falla "21-el-puesto.yaml no abre $RANGO: la zona resolvería a un rango que la política tira"
hecho "$RANGO es el que abre 21-el-puesto.yaml"
PGA=$(g compute networks subnets describe "$SUBRED" --region "$LUGAR" --format='value(privateIpGoogleAccess)')
[ "$PGA" = "True" ] || falla "la subred $SUBRED no tiene Private Google Access ($PGA): con la zona, NADIE llegaría a Google. No se toca nada."
hecho "Private Google Access en $SUBRED"

# ── 1 · la zona privada ─────────────────────────────────────────────────────
TIENE=$(g dns managed-zones describe "$ZONA" --format='value(dnsName,visibility)')
case "$TIENE" in
  "googleapis.com."*private*) ya "la zona $ZONA (googleapis.com., privada)" ;;
  "") if [ -n "$SECO" ]; then haria "crear la zona privada $ZONA para googleapis.com. en $RED"; else
        g dns managed-zones create "$ZONA" --dns-name=googleapis.com. --visibility=private --networks="$RED" \
          --description "0031 W3.1: *.googleapis.com por private.googleapis.com, para que una NetworkPolicy pueda abrir Google sin abrir internet" \
          && hecho "zona $ZONA creada" || falla "no se pudo crear la zona"; fi ;;
  *) falla "la zona $ZONA existe y no es lo esperado: $TIENE" ;;
esac

# ── 2 · los dos registros ───────────────────────────────────────────────────
registro() { # <nombre> <tipo> <ttl> <datos...>
  local N="$1" T="$2" TTL="$3"; shift 3
  local VIVO
  VIVO=$(g dns record-sets describe "$N" --type "$T" --zone "$ZONA" --format='value(rrdatas)' | tr ';' ' ')
  if [ -n "$VIVO" ]; then
    ya "$N $T → $VIVO"
  elif [ -n "$SECO" ]; then
    haria "$N $T → $*"
  else
    g dns record-sets create "$N" --type "$T" --ttl "$TTL" --zone "$ZONA" --rrdatas="$(IFS=,; echo "$*")" \
      && hecho "$N $T → $*" || falla "no se pudo crear $N $T"
  fi
}
[ -n "$SECO" ] && [ -z "$TIENE" ] && { haria "private.googleapis.com. A → 199.36.153.8 .9 .10 .11"; haria "*.googleapis.com. CNAME → private.googleapis.com."; exit 0; }
registro private.googleapis.com. A 300 199.36.153.8 199.36.153.9 199.36.153.10 199.36.153.11
registro '*.googleapis.com.' CNAME 300 private.googleapis.com.

# ── 3 · y se comprueba desde dentro: lo que resuelve un pod ────────────────
# (se deja a `medida-w3-el-puesto.py`, que lo mide con el rol `puesto`)
echo "  → comprobar desde un pod: python pruebas-de-fuego/medida-w3-el-puesto.py (internet debe decir «no»; la copia debe bajar)"
