#!/usr/bin/env bash
# El eslabon vivo de la fase 1, de punta a punta y contra un PostgreSQL sucio.
#
# `discover --from` estaba cubierto por once pruebas. `discover --source` —el que
# resuelve `ore-read-postgres` en el PATH, lo ejecuta, le pasa la URL por stdin y
# analiza lo que devuelve— **no tenia ninguna**, ni unitaria ni de integracion, y
# los tres pasos se podian romper en silencio.
#
# Lo que se comprueba aqui no es que el comando no reviente: son **las decisiones
# que salen**. Un descubrimiento que termina en verde sin ver la colision de
# nombres tiene exactamente el mismo aspecto que uno correcto.
#
# El esquema lleva dentro las patologias que el inductor sabe nombrar. Falta una
# a proposito: **cero filas** no puede salir de aqui, porque este lector no emite
# el recuento —`pg_class.reltuples` es una estimacion que vale `-1` hasta que
# alguien ejecuta `ANALYZE`, y proponer borrar tablas vivas seria peor que no
# proponer nada—. Decirlo es mejor que fingir que se cubre.
#
# Necesita: un PostgreSQL en `PG_URL` y `ore` y `ore-read-postgres` en el PATH.
set -euo pipefail
cd "$(dirname "$0")/.."

PG_URL="${PG_URL:-postgres://postgres:x@localhost:5432/descubrimiento}"
T=$(mktemp -d)
# Los ficheros de respuestas viven FUERA del repositorio ontologico, y no es
# manía: `ore validate` carga todo `.yaml` del arbol y le exige `apiVersion`.
# Un fichero de respuestas dentro rompe el paquete que acaba de arreglar.
R=$(mktemp -d)
trap 'rm -rf "$T" "$R"' EXIT

falla() { echo "FALLA · $1"; exit 1; }
dice() { grep -q -- "$2" "$1" || falla "$3"; }
# Con `set -e`, `grep -q X && falla` sale del script cuando grep NO encuentra —
# que es el caso bueno—. El `|| true` es lo que convierte eso en una asercion.
no_dice() { grep -q -- "$2" "$1" && falla "$3" || true; }

# ── El esquema sucio ────────────────────────────────────────────────────────
#
# Cada objeto de aqui existe por una pregunta que tiene que salir al otro lado.
psql "$PG_URL" -q <<'SQL'
DROP SCHEMA IF EXISTS ventas CASCADE;
DROP VIEW  IF EXISTS public.v_clientes_activos;
DROP TABLE IF EXISTS public.clientes, public.pedidos, public.pedidos_2024,
                     public.log_eventos CASCADE;
DROP TYPE  IF EXISTS public.direccion CASCADE;

-- Un tipo compuesto. El lector NO lo traduce y no lo disfraza de `Opaque`:
-- `Opaque` dice «no hay estructura dentro» y el catalogo acaba de enumerarla.
CREATE TYPE public.direccion AS (calle text, cp text);

CREATE TABLE public.clientes (
  id        bigint PRIMARY KEY,
  email     text NOT NULL,
  telefono  text,
  nif       text UNIQUE,              -- clave alternativa: `uniqueKeys`
  domicilio public.direccion          -- sin tipo de OOS
);

-- Colision de identificador: las dos dan `Clientes`.
-- `email` y `telefono` se repiten entre las dos: son las candidatas a concepto,
-- y estan a proposito una a cada lado — para `email` hay concepto publicado al
-- que apuntar, para `telefono` no lo hay y hay que acunarlo.
CREATE SCHEMA ventas;
CREATE TABLE ventas.clientes (
  id       bigint PRIMARY KEY,
  email    text,
  telefono text
);

-- Familia fragmentada por fecha, con la hermana viva SIN digitos: es el caso
-- mas comun de un almacen real y el que se escapaba.
CREATE TABLE public.pedidos (
  id_pedido  bigint PRIMARY KEY,
  id_cliente bigint NOT NULL,         -- FK NO declarada: solo un parecido
  fecha      date   NOT NULL,
  email      text
);
CREATE TABLE public.pedidos_2024 (
  id_pedido  bigint PRIMARY KEY,
  id_cliente bigint NOT NULL,
  fecha      date   NOT NULL,
  email      text
);

-- Sin clave primaria y sin una sola columna tipable: no hay entidad que escribir.
CREATE TABLE public.log_eventos (
  origen  public.direccion,
  destino public.direccion
);

-- Una vista es una proyeccion: puede ser la entidad o un informe sobre ella.
CREATE VIEW public.v_clientes_activos AS SELECT id, email FROM public.clientes;
SQL

cd "$T"
ore init --name demo . >/dev/null
ore source add --name crm_prod "$PG_URL" >/dev/null

# El vocabulario publicado, la escala con la que se clasifica y el techo por el
# que se sirve. Los tres existen ANTES de descubrir, que es el orden real: sin
# ellos la septima pregunta no tiene nada que ofrecer y la clasificacion no
# tiene niveles entre los que elegir.
cat > lattices.yaml <<'YAML'
apiVersion: oos.dev/v1alpha3
kind: Lattice
metadata: { name: sensitivity, namespace: gdpr }
spec:
  levels: [none, low, medium, high, critical]
YAML
cat > conduits.yaml <<'YAML'
apiVersion: oos.dev/v1alpha1
kind: ConduitPolicy
metadata: { name: default }
spec:
  owner: team:security
  conduits:
    contextSurface:
      gdpr.sensitivity: medium
      oos.maturity: DRAFT
YAML
mkdir -p packages/gdpr/concepts
cat > packages/gdpr/package.yaml <<'YAML'
apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: gdpr, version: 1.0.0, status: active, domain: compliance }
spec: { owner: "team:compliance" }
YAML
cat > packages/gdpr/concepts/personalEmail.yaml <<'YAML'
apiVersion: oos.dev/v1alpha4
kind: Property
metadata: { name: personalEmail, namespace: gdpr }
spec:
  type: String
  labels: { gdpr.sensitivity: high }
  description: La direccion de correo de una persona fisica.
  aiContext:
    synonyms: [email, correo, e_mail, mail]
YAML

# ── 1 · El eslabon vivo ─────────────────────────────────────────────────────
#
# Resolver el driver en el PATH, ejecutarlo, pasarle la URL por STDIN y analizar
# su salida. Los tres pasos, sin doble ninguno.
ore discover --source crm_prod --out packages/ventas > informe.txt 2> aviso.txt \
  || falla "discover --source fallo: $(cat aviso.txt)"

CAT=packages/ventas/discover.catalog.json
[ -s "$CAT" ] || falla "no dejo el catalogo que leyo"
dice "$CAT" '"source": "crm_prod"' "el catalogo no dice de que fuente vino"
# Solo el lector vivo produce esto: el tipo compuesto CITADO, no interpretado.
dice "$CAT" '"sourceType": "direccion"' "el tipo compuesto no llego entero"
dice "$CAT" '"uniqueKeys"' "perdio la clave alternativa de nif"
dice "$CAT" '"kind": "view"' "no distinguio la vista de una tabla"

# ── 2 · Y las decisiones que salen ──────────────────────────────────────────
COLA=packages/ventas/discover.pending.json
for d in \
  'colision/Clientes' \
  'clave/public.v_clientes_activos' \
  'tipo/public.log_eventos.origen' \
  'vacio/public.log_eventos' \
  'vista/public.v_clientes_activos' \
  'concepto/email.String' \
  'familia/public.pedidos' \
  'dueno/ventas'
do
  dice "$COLA" "\"$d\"" "no vio la decision \`$d\`"
done

# Y la septima pregunta ofrece lo que el repositorio ya publica, con la
# clasificacion que se hereda al elegirlo. Sin vocabulario la unica respuesta
# posible seria acunar, que es la cara: cuatro mil columnas dan cuatro mil
# conceptos, que es igual que no tener vocabulario.
dice "$COLA" 'gdpr.personalEmail' "no ofrecio el concepto publicado como candidato"

# Lo que NO se emite es la mitad del contrato. Una tabla sin ninguna columna
# tipable no tiene entidad, y dos que colisionan no tienen ninguna: emitir una
# decidiria cual de las dos existe.
[ ! -e packages/ventas/entities/Log_eventos.yaml ] || falla "emitio una entidad sin nada que escribir"
[ ! -e packages/ventas/entities/Clientes.yaml ]    || falla "resolvio la colision por su cuenta"

# ── 3 · Contestar, en dos sentadas ──────────────────────────────────────────
#
# En dos y no en una a proposito: la segunda pasada tiene que respetar lo que
# decidio la primera. Volviendo a inducir con solo lo ultimo, la desharia — en
# silencio y sobre ficheros que ya parecian buenos.
cat > "$R/a1.yaml" <<'YAML'
answers:
  colision/Clientes:
    public.clientes: Clientes
    ventas.clientes: ClientesVentas
  dueno/ventas: team:datos
YAML
ore review packages/ventas --answers "$R/a1.yaml" > r1.txt || falla "la primera pasada fallo"
dice packages/ventas/entities/Clientes.yaml 'name: Clientes' "no emitio la colision resuelta"
dice packages/ventas/entities/ClientesVentas.yaml 'name: ClientesVentas' "solo emitio una de las dos"

# Y con la colision resuelta aparecen las preguntas que estaban DETRAS de ella.
# Mientras no se emitia ninguna de las dos tablas no habia documento donde poner
# el tipo de una columna suya, ni entidad a la que pudiera apuntar una relacion.
dice "$COLA" 'tipo/public.clientes.domicilio' "no pregunto por el tipo que tapaba la colision"
dice "$COLA" 'relacion/public.pedidos.id_cliente' "no propuso la relacion tras resolver la colision"

cat > "$R/a2.yaml" <<'YAML'
answers:
  tipo/public.clientes.domicilio: omitir
  vacio/public.log_eventos: omitir
  vista/public.v_clientes_activos: omitir
  familia/public.pedidos: fecha
  concepto/email.String: gdpr.personalEmail
  concepto/telefono.String: telefonoPersonal
  clasificacion/ventas.telefonoPersonal: "gdpr.sensitivity: critical"
  concepto/fecha.Date: no
  relacion/public.pedidos.id_cliente: si
YAML
ore review packages/ventas --answers "$R/a2.yaml" > r2.txt || falla "la segunda pasada fallo"

# Lo que decidio la PRIMERA sentada sigue en pie.
dice packages/ventas/entities/ClientesVentas.yaml 'name: ClientesVentas' "la segunda pasada deshizo la primera"
dice packages/ventas/package.yaml 'team:datos' "la segunda pasada perdio el dueno"

# Unir una familia es UNA entidad servida desde N tablas, que es N bindings:
# lo que el ejecutor ya sabe federar.
dice packages/ventas/entities/Pedidos.yaml 'name: Pedidos' "no unio la familia"
[ -e packages/ventas/bindings/public_pedidos.yaml ]      || falla "falta el binding de la hermana viva"
[ -e packages/ventas/bindings/public_pedidos_2024.yaml ] || falla "falta el binding de la hermana fechada"

# Apuntar a un concepto publicado NO escribe una copia: acunar lo que ya existe
# es la inflacion por la otra puerta. Se apunta, y se hereda su clasificacion.
[ ! -e packages/ventas/concepts/personalEmail.yaml ] || falla "acuno una copia de un concepto publicado"
dice packages/ventas/entities/Clientes.yaml 'is: gdpr.personalEmail' "no hablo el concepto publicado"

# Y acunar uno nuevo SI lo escribe —`is` exige que exista, y una referencia
# colgando seria peor que no preguntar— con la clasificacion que alguien dijo.
# Un concepto sin etiquetas no gobierna nada, y eso no lo puede decidir el
# silencio.
dice packages/ventas/concepts/telefonoPersonal.yaml 'kind: Property' "no acuno el concepto nuevo"
dice packages/ventas/concepts/telefonoPersonal.yaml 'labels: { gdpr.sensitivity: critical }' \
  "acuno un concepto que no clasifica nada"

# Y lo omitido se va del paquete, no se queda de resto.
[ ! -e packages/ventas/entities/V_clientes_activos.yaml ] || falla "dejo la vista que alguien omitio"

# ── 4 · El criterio: contestar deja un paquete que el compilador acepta ─────
dice "$COLA" '"pending": \[\]' "quedaron decisiones sin cerrar: $(cat r2.txt)"
ore validate . > validado.txt 2>&1 || falla "lo revisado no valida: $(cat validado.txt)"
dice validado.txt 'ok · sin errores' "valido, pero no en verde: $(cat validado.txt)"

# ── 5 · Y lo que decidir un concepto SIGNIFICA ─────────────────────────────
#
# Aqui se cierra el circulo, y es lo unico que demuestra que contestar la
# septima pregunta sirve para algo. La etiqueta de un concepto es la tercera
# fuente de la clasificacion efectiva; la clasificacion efectiva es lo que poda
# la superficie emitida. `email` hereda `high` del concepto publicado y
# `telefono` lleva `critical` del recien acunado: el techo admite hasta
# `medium`, asi que **ninguno de los dos esta en el contrato que un agente puede
# pedir**. Nadie escribio una etiqueta en una entidad.
#
# Y `nif` SI sale, que es la otra mitad de «exactamente»: nadie dijo que fuera
# sensible, asi que el techo no lo toca. Podar de mas es una fuga de
# disponibilidad igual que podar de menos es una fuga de datos.
ore export . --format graphql > superficie.graphql 2>&1 \
  || falla "no se pudo emitir la superficie: $(cat superficie.graphql)"
dice superficie.graphql 'type Clientes' "no emitio la entidad"
no_dice superficie.graphql 'email' "sirvio lo que el concepto publicado clasifico por encima del techo"
no_dice superficie.graphql 'telefono' "sirvio lo que el concepto acunado clasifico critical"
dice superficie.graphql 'nif' "podo una columna que nadie clasifico"

echo "el eslabon vivo, diez de las once preguntas, un paquete en verde y una superficie podada"
