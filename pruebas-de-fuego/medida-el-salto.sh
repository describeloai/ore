#!/usr/bin/env bash
# 0047 · M2 · Frecuencia y coste del salto: cuántas preguntas haría `ore-serve` a `ore-iam`, y
# cuánto cuesta cada una.
#
#   1. LAS ESCRITURAS   (Q3) los commits de la forja de cada celda en N días, por día y por
#                       clase de autor (persona, agente, sistema), y las PR. Sólo cuentas.
#   2. EL VIAJE         (Q2) 500 peticiones a `ore-iam:8090/salud` desde un pod efímero con las
#                       etiquetas del informador, el único que hoy tiene camino. p50, p95, p99.
#   3. LA CONSULTA      (Q2) en la base: la consulta de `potestad::exige` y una fila de la
#                       huella, 1.000 veces cada una. La huella en una tabla temporal con sus
#                       índices: ni una fila ni un número de la secuencia de la de verdad.
#   4. LAS PETICIONES   (Q1, Q4) las líneas `acceso · …` de los `ore-serve` (M2.3), de Cloud
#                       Logging, clasificadas con la tabla de M1 (`medida-el-acceso.py --json`).
#
#     bash pruebas-de-fuego/medida-el-salto.sh [secciones] [--dias N] [--desde 2026-09-29T12:00:00Z]
#
#   secciones: cualquier combinación de 1 2 3 4 (por defecto, todas). Necesita el contexto del
#   cluster `ore-mesh` y `gcloud` con el proyecto por defecto. Crea y borra un pod en `t-demo`;
#   no toca nada más. No imprime ni personas, ni nombres de paquetes, ni tokens.
set -uo pipefail
export MSYS_NO_PATHCONV=1   # Git Bash: que no reescriba los caminos de kubectl
gcloud() { env -u MSYS_NO_PATHCONV gcloud "$@"; }   # y gcloud, sin él

AQUI=$(cd "$(dirname "$0")" && pwd)
INQUILINOS="demo prueba victor"
DIAS=30
DESDE=""
SECCIONES=""
while [ $# -gt 0 ]; do
  case $1 in
    --dias) DIAS=$2; shift 2 ;;
    --desde) DESDE=$2; shift 2 ;;
    [1-4]) SECCIONES="$SECCIONES $1"; shift ;;
    *) echo "✗ no entiendo \`$1\`"; exit 64 ;;
  esac
done
SECCIONES=${SECCIONES:- 1 2 3 4}
toca() { case " $SECCIONES " in *" $1 "*) return 0 ;; esac; return 1; }
PROYECTO=$(gcloud config get-value project 2>/dev/null | tr -d '\r')
IMG="europe-west1-docker.pkg.dev/$PROYECTO/ore/ore-drivers:main"

if toca 1; then
  echo "== 1 · las escrituras: commits de la forja en $DIAS días (sólo cuentas)"
  # Autor `<sub>@sujeto.invalid` con committer `@ore.dev`: lo escribió ore-serve por alguien.
  # Es agente si su nombre o su correo lo dicen; sistema, lo que no pasó por ore-serve (la
  # semilla, el aprovisionador); persona, el resto.
  for t in $INQUILINOS; do
    echo "-- t-$t"
    kubectl exec -n "t-$t" forja-0 -c forgejo -- sh -c '
      cd /data/git/repositories/t-'"$t"' || exit 0
      for r in ontologia trabajo; do
        [ -d $r.git ] || continue
        git -c safe.directory="*" -C $r.git log --all --since="'"$DIAS"' days ago" --format="%ad|%ae|%an" --date=format:"%Y-%m-%dT%H" \
        | awk -F"|" -v r=$r "
            { c = (\$2 !~ /@sujeto\.invalid\$/) ? \"sistema\" : (\$2 ~ /agente/ || \$3 ~ /agente|^ore-/) ? \"agente\" : \"persona\";
              n[c]++; dia[substr(\$1,1,10)]++; hora[\$1]++ }
            END { for (d in dia) { nd++; if (dia[d] > md) md = dia[d] }
                  for (h in hora) if (hora[h] > mh) mh = hora[h]
                  printf \"   %-9s persona %4d · agente %4d · sistema %4d · días con commits %2d · máx/día %3d · máx/hora %3d\\n\", r, n[\"persona\"], n[\"agente\"], n[\"sistema\"], nd, md, mh }"
      done
      printf "   %-9s %d PR (refs/pull)\n" "PR" "$(git -c safe.directory="*" -C ontologia.git for-each-ref refs/pull --format=x 2>/dev/null | grep -c . )"
    ' 2>/dev/null
  done
  echo
fi

if toca 2; then
  echo "== 2 · el viaje: 500 peticiones a ore-iam:8090/salud desde t-demo (etiquetas del informador)"
  cat > "${TMPDIR:-/tmp}/m2-viaje.yaml" <<EOF
apiVersion: v1
kind: Pod
metadata:
  name: m2-viaje
  labels: {ore.dev/rol: informador, ore.dev/tenant: demo}
spec:
  restartPolicy: Never
  automountServiceAccountToken: false
  containers:
  - name: m2
    image: $IMG
    command: ["sh", "-c", "for i in \$(seq 1 500); do curl -s -o /dev/null -w '%{time_namelookup} %{time_connect} %{time_total}\\\\n' http://ore-iam.identidad.svc.cluster.local:8090/salud; done"]
    resources: {requests: {cpu: 50m, memory: 64Mi}, limits: {cpu: 200m, memory: 128Mi}}
    securityContext: {allowPrivilegeEscalation: false, runAsNonRoot: true, runAsUser: 1000, capabilities: {drop: [ALL]}, seccompProfile: {type: RuntimeDefault}}
EOF
  kubectl delete pod -n t-demo m2-viaje --ignore-not-found --wait=true >/dev/null 2>&1
  kubectl apply -n t-demo -f - < "${TMPDIR:-/tmp}/m2-viaje.yaml" >/dev/null
  for _ in $(seq 1 60); do
    case $(kubectl get pod -n t-demo m2-viaje -o jsonpath='{.status.phase}' 2>/dev/null) in
      Succeeded|Failed) break ;;
    esac
    sleep 5
  done
  kubectl logs -n t-demo m2-viaje 2>&1 | python -c '
import sys
sys.stdout.reconfigure(encoding="utf-8")
f = [[float(x) * 1000 for x in l.split()] for l in sys.stdin if len(l.split()) == 3]
if not f:
    print("   sin datos"); sys.exit()
p = lambda v, q: sorted(v)[min(len(v) - 1, int(len(v) * q))]
for k, v in [("dns", [a for a, b, c in f]), ("conexión", [b - a for a, b, c in f]),
             ("respuesta", [c - b for a, b, c in f]), ("total", [c for a, b, c in f])]:
    print("   %-9s n %d · p50 %5.1f ms · p95 %5.1f ms · p99 %5.1f ms · máx %5.1f ms" % (k, len(v), p(v, .5), p(v, .95), p(v, .99), max(v)))'
  kubectl delete pod -n t-demo m2-viaje --wait=false >/dev/null 2>&1
  echo "   (cada petición resuelve el nombre y abre conexión: el peor caso. Un cliente que reutiliza"
  echo "    la conexión sólo paga «respuesta»)"
  echo
fi

if toca 3; then
  echo "== 3 · la consulta: potestad::exige y una fila de la huella, 1.000 veces (en la base)"
  kubectl exec -i -n identidad idp-db-0 -- sh -c 'psql -U "$POSTGRES_USER" -d iam -q -v ON_ERROR_STOP=1' <<'SQL' 2>&1 | sed -n 's/^.*NOTICE:  /   /p'
begin;
-- Una pertenencia de verdad, sin imprimirla.
create temp table m2_quien on commit drop as
  select p.emisor, p.sub, pe.organizacion from iam.pertenencia pe join iam.persona p on p.id = pe.persona limit 1;
-- La huella, con sus índices y sin sus valores por defecto: la secuencia de la de verdad no se toca.
create temp table m2_huella (like iam.huella including indexes) on commit drop;
do $$
declare t0 timestamptz; i int; n int; e text; s text; o text;
begin
  select emisor, sub, organizacion into e, s, o from m2_quien;
  if e is null then raise notice 'no hay ninguna pertenencia: sin medida'; return; end if;
  t0 := clock_timestamp();
  for i in 1..1000 loop
    select count(*) into n from iam.potestades_de_persona pp join iam.persona p on p.id = pp.persona
     where p.emisor = e and p.sub = s and pp.organizacion = o;
  end loop;
  raise notice 'potestades   % ms por consulta (% potestades)', round((extract(epoch from clock_timestamp() - t0) * 1000 / 1000)::numeric, 3), n;
  t0 := clock_timestamp();
  for i in 1..1000 loop
    insert into m2_huella (id, cuando, quien, operacion, sobre, detalle)
    values (i, now(), 'm2', 'acceso:medida', 'organizacion/-', '{"m2": true}');
  end loop;
  raise notice 'huella       % ms por fila', round((extract(epoch from clock_timestamp() - t0) * 1000 / 1000)::numeric, 3);
end $$;
rollback;
SQL
  echo "   (dentro de la base: sin red ni transacción propia; M2.1 da la ruta entera, 41 ms p50)"
  echo
fi

if toca 4; then
  # ⛔ De `kubectl logs`, no de Cloud Logging: el cluster sólo le manda los componentes del
  #   sistema (`loggingConfig`: SYSTEM_COMPONENTS), no las cargas. Medido el 2026-09-29. Lo que
  #   se pierde: un pod reemplazado se lleva sus líneas, así que la ventana es la del pod vivo.
  #   Para los cuatro eventos de M2 basta; para una serie larga habría que encender WORKLOADS.
  if [ -n "$DESDE" ]; then DESDE_K="--since-time=$DESDE"; else DESDE_K="--since=$((DIAS * 24))h"; fi
  echo "== 4 · las peticiones: líneas \`acceso · …\` de los ore-serve${DESDE:+ desde $DESDE}"
  # ⚠️ Con `cygpath` si lo hay: en Git Bash, Python es de Windows y no ve `/tmp`.
  RUTAS="${TMPDIR:-/tmp}/m2-rutas.json"
  M1="$AQUI/medida-el-acceso.py"
  command -v cygpath >/dev/null && { RUTAS=$(cygpath -m "$RUTAS"); M1=$(cygpath -m "$M1"); }
  python "$M1" --json > "$RUTAS" || { echo "✗ medida-el-acceso.py --json falló"; exit 1; }
  for t in $INQUILINOS; do
    kubectl logs -n "t-$t" deploy/ore-serve --timestamps "$DESDE_K" 2>/dev/null \
      | awk -v ns="t-$t" '/ acceso · / { f = $1; $1 = ""; sub(/^ /, ""); print f "\t" ns "\t" $0 }'
  done \
  | python -c '
import collections, json, re, sys
sys.stdout.reconfigure(encoding="utf-8")
sys.stdin.reconfigure(encoding="utf-8")   # en Windows leería cp1252 y el `·` no casaría
rutas = json.load(open(sys.argv[1], encoding="utf-8"))
exactas = {(r["metodo"], r["camino"]): r for r in rutas}
restos = [(r["metodo"], r["camino"][:-len("{..}")], r) for r in rutas if r["camino"].endswith("{..}")]
LINEA = re.compile(r"acceso · (\w+) (\S+) · (\d+) · (\d+) ms · (\w+)")
def clase(m, c):
    if c.startswith("/v1/"):
        return "iceberg (datos)" if m in ("GET", "HEAD") and "/tables/" in c else "iceberg"
    r = exactas.get((m, c)) or next((r for mm, pre, r in restos if mm == m and c.startswith(pre)), None)
    if r is None:
        return "sin clasificar"
    datos = r["clase"] == "puesto" or re.search(r"/(ejecutar|invocar|resultados)$", c)
    return r["clase"] + (" (datos)" if datos else "")
por = collections.defaultdict(list); minuto = collections.Counter(); sujetos = collections.Counter()
for l in sys.stdin:
    partes = l.rstrip("\n").split("\t")
    if len(partes) < 3: continue
    cuando, ns, texto = partes[0], partes[1], partes[2]
    m = LINEA.search(texto)
    if not m: continue
    metodo, camino, codigo, ms, sujeto = m.groups()
    por[(ns, clase(metodo, camino))].append(int(ms))
    minuto[(ns, cuando[:16])] += 1
    sujetos[(ns, sujeto)] += 1
if not por:
    print("   ninguna línea todavía: ¿corre ya el ore-serve con M2.3?"); sys.exit()
for ns in sorted({k[0] for k in por}):
    mins = [n for (n2, _), n in minuto.items() if n2 == ns]
    total = sum(mins)
    print(f"-- {ns}: {total} peticiones en {len(mins)} minutos con actividad · media {total/len(mins):.1f}/min · pico {max(mins)}/min")
    print("   sujetos: " + " · ".join(f"{s} {n}" for (n2, s), n in sorted(sujetos.items()) if n2 == ns))
    for (n2, k), v in sorted(por.items(), key=lambda x: -len(x[1])):
        if n2 != ns: continue
        v.sort()
        print(f"   {k:24} {len(v):6}   p50 {v[len(v)//2]:5} ms   p95 {v[min(len(v)-1, int(len(v)*.95))]:6} ms")
' "$RUTAS"
fi
