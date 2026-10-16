# 0065 · Postgres como origen, de verdad

**Estado:** propuesto (2026-10-16). **PG0 medido** y **PG1 cerrado en producción** (2026-10-16); Supabase
pendiente de un proyecto. Cuatro decisiones corregidas por la medida (D-PG1, D-PG4, D-PG5, D-PG6). Siguiente:
PG2, esperando el go. Investigación:
[`p-postgres-origen-estado-del-arte.md`](../investigacion/p-postgres-origen-estado-del-arte.md).
**Decide** cómo lee ORE el Postgres de un cliente cuando es una base gestionada, privada o con una CA
propia, y cómo entra el CDC. Toca [`0008`](0008-el-protocolo-del-driver.md) (el contrato del driver),
[`0016`](0016-el-testigo-y-el-rango.md) (testigo y rango), [`0053`](0053-ore-federation-engine.md) (la
pasarela), [`0061`](0061-origenes-de-objetos.md) (de donde vienen la clave SSH y la federación de la
celda), [`0062`](0062-estadisticas-del-catalogo.md) (filas estimadas) y
[`0058`](0058-ore-serverless-postgres.md) Q2 (la marca de agua del CDC).

## Contexto

Postgres fue el primer origen de verdad y funciona de punta a punta: catálogo, Arrow en flujo,
*pushdown*, testigo, `cambio` y cancelación. Pero sólo se ha probado contra un `postgres:16-alpine` (CI)
y un Neon (victor). Un cliente con una base gestionada típica hoy **no conecta**, o conecta mal:

- **TLS.** `tokio-postgres` 0.7.18 sólo conoce `disable`, `prefer` y `require`. Nuestro conector
  (`native-tls`) verifica contra el almacén del sistema, y no hay manera de dar una CA. Por eso RDS,
  Cloud SQL, Supabase y Aiven, que usan CA propias, no conectan. Y **sin `sslmode` en la URL vale
  `prefer`**: si alguien quita TLS por el camino, la contraseña viaja en claro.
- **Identidad.** Sólo usuario y contraseña dentro de la URL (`credenciales.rs:813-817`), cuando la
  celda ya tiene cuentas que AWS, GCP y Azure aceptan (0046 E9b, D-O2, D-O3).
- **Red.** Sólo IPs públicas y sólo el 5432 (`malla/20-driver.yaml:60-101`). Ni un bastión, ni el 6543
  de Supabase, ni los puertos de Aiven.
- **Tipos.** 17 tipos. Los arrays, `interval`, `inet`, `timetz`, pgvector y PostGIS niegan la columna
  (0053:215).
- **Filas.** Ninguna estimación (0062 E4).
- **CDC.** No hay; sólo una medida con `test_decoding` (0063).

## Decisión

1. **D-PG1 · TLS: verificar siempre, y la CA del cliente cuando no sea pública.**
   - El driver arma su propio conector TLS. No depende del `sslmode` de `tokio-postgres`.
   - **Por defecto, `verify-full`**: la cadena y el nombre del host, contra el almacén del sistema más
     la CA que dé la fuente.
   - **La CA va con la credencial**, en el cofre junto a la URL (`?ca=<nombre>`, el PEM en la
     credencial), no en la imagen. Para RDS, el *bundle* global de AWS va en la imagen con nombre
     propio (`ca=aws-rds`), porque es el mismo para todos.
   - **`verify-ca`** (la cadena sin el nombre) sólo con un parámetro explícito. Existe para Cloud SQL
     por IP, cuyo certificado no lleva la IP.
   - **No se admiten** `disable`, `allow` ni `prefer` (texto plano por internet), ni «confiar en
     cualquier certificado». Es la misma regla que «cualquier huella» en SFTP (D-O4). Un sabor que sólo
     se puede leer así (Heroku) se queda fuera y se dice en su perfil. El laboratorio conserva
     `ORE_PG_LABORATORIO=1` para hablar en claro con un contenedor local.
   - `check` dice la versión de TLS, el emisor y el sujeto del certificado. Si falla, el error nombra
     la CA que falta.
   - **El certificado de cliente (mTLS), fuera** hasta que se pida.

2. **D-PG2 · Identidad: las cuentas de la celda; la contraseña, de recurso.** Es lo de D-O2 y D-O3
   llevado a Postgres. El token de IAM va como contraseña, **lo firma el driver en cada conexión** y no
   se guarda: el de RDS sólo vale 15 min para conectar. `ore-serve` no canjea nada.
   - **GCP (Cloud SQL, AlloyDB).** El *access token* de `ore-driver-<celda>` por Workload Identity; el
     usuario es `ore-driver-<celda>@<proyecto>.iam`. La alternativa, como en GCS, es suplantar una
     cuenta del cliente.
     URL: `postgres://<usuario-iam>@<host>/<db>?iam=gcp[&suplanta=<sa>]`.
   - **AWS (RDS, Aurora).** El driver asume el rol del cliente, el que ya confía en las dos cuentas de
     la celda (0046 E9b), y prefirma con `ore-sigv4` el token `rds-db:connect`, sin llamar a AWS.
     URL: `postgres://<usuario>@<host>/<db>?iam=aws&rol=<arn>&region=<r>`.
   - **Azure (Flexible).** Las cuentas de la celda federadas en la app del cliente (D-O3), con el ámbito
     `https://ossrdbms-aad.database.windows.net/.default`; el rol se llama como la app.
     URL: `postgres://<rol>@<host>/<db>?iam=azure&tenant=<id>&cliente=<id>`.
   - **El asistente de credenciales** (`GET /fuentes/credenciales/postgres`) enseña los modos y, para
     cada uno, los comandos (`gcloud`, `aws`, `az`, y el `CREATE ROLE` / `GRANT` dentro de la base).
     Los mismos comandos sirven para un usuario de sólo lectura con contraseña.
   - **IAM exige TLS** (RDS lo impone; D-PG1 lo da siempre).

3. **D-PG3 · Red: directo, o por un túnel SSH con la clave de la celda.**
   - **Directo**, con la IP de salida fija de la celda en la lista de permitidos del cliente. Es lo que
     hay hoy.
   - **Túnel SSH por un bastión del cliente**, que es el estándar del sector (Fivetran, Airbyte). Usa
     la **misma clave Ed25519 de la celda y la misma huella fijada que SFTP** (D-O4), y la misma
     `ssh2`. El reenvío vive **dentro del proceso del driver**, sin un puerto que oiga fuera, y TLS va
     de punta a punta por dentro del túnel.
     URL: `…?ssh=<usuario>@<bastión>[:<puerto>]&huella=SHA256:<h>`.
   - **El puerto, por fuente.** Se cumple lo que `20-driver.yaml:86-91` dejó escrito: la fuente declara
     su puerto (y el del bastión) y el renderizador emite la `NetworkPolicy` del Job con lo que esa
     fuente necesita. El 443, el 5432 y el 22 globales se quedan hasta que todas las fuentes lo
     declaren; después se estrechan. Es un cambio de malla, con su go, y siempre **el binario antes que
     la malla**.
   - **Fuera, hasta que se pida:** PrivateLink, Private Service Connect y Azure Private Link (un montaje
     por nube y por cliente), y el túnel inverso o un agente (Cloudflare Tunnel, Tailscale).

4. **D-PG4 · Tipos: lo exacto en Arrow; lo demás, como su texto canónico, y se dice.**
   - **Tipos nativos.** Arrays → `List`; `interval` → `Interval(MonthDayNano)`; compuestos → `Struct`.
   - **Extensiones, por `typname`** (su OID cambia de una base a otra):
     - `vector` → `FixedSizeList<Float32>`;
     - `geometry` / `geography` → Binary WKB con los metadatos de GeoArrow;
     - `hstore` → `Map<Utf8, Utf8>`;
     - `citext` → Utf8.
   - **Todo lo demás viaja como `::text`**, la salida canónica de Postgres: `timetz`, `inet`, `cidr`,
     `macaddr`, `money`, `bit`, rangos, `ltree`, `tsvector` y los tipos que nadie conoce. Va como Utf8 y
     cita su `physicalType`, como el contrato de 0032. La promesa de W1 cambia de «se niega» a «viaja
     como texto, y se dice»: no se pierde nada, porque es el texto que el propio Postgres escribe.
   - **No cambia el contrato de nulos (0051).** Una columna que no se pueda castear a texto sigue
     negándose, nombrando la columna.

5. **D-PG5 · Filas estimadas, gratis.** El catálogo lee `pg_class.reltuples` en la misma consulta:
   - `-1` (nunca analizada, PG14+) es «sin estimación»;
   - el padre de una particionada suma las hijas;
   - el tamaño sale de `relpages × block_size`, nunca de `pg_total_relation_size` (4,8 s para 301
     tablas, medido en 0058 P8·3).
   - Es una `rowsEstimate` de 0062, nunca `rows`.

6. **D-PG6 · Sabores: un perfil por proveedor, y lo que `check` detecta.**
   - [`docs/origenes-de-bases.md`](../origenes-de-bases.md) lleva, por proveedor, su CA, su IAM, su
     pooler y puerto, su réplica y cómo se activa su replicación lógica. Cada perfil dice «probado» o
     «de su documentación», como `origenes-de-objetos.md`.
   - **`check` detecta y dice**:
     - si está detrás de un pooler en modo transacción (el modo de sesión o el puerto directo es lo
       admitido; PG0 mide qué rompe exactamente);
     - si es una réplica, y en ese caso los conflictos con la recuperación salen con un error tipado
       que nombra `hot_standby_feedback`;
     - el `wal_level` y si el usuario puede replicar (para el CDC);
     - y, como en BigQuery, **qué permiso falta**: `USAGE` en el esquema y `SELECT` en las tablas.
   - **Fuera:** Redshift, CockroachDB, YugabyteDB, Materialize y Spanner PG, aunque hablen el
     protocolo.

7. **D-PG7 · El CDC es una etapa propia, por lotes y por SQL.** Se decide del todo en su propia medida
   (CDC0). El esbozo:
   - `pgoutput`, una publicación y un slot por fuente;
   - en cada sincronización se consume del slot por SQL (`pg_logical_slot_peek_binary_changes`) desde
     el último LSN confirmado, se escribe en el lago y **sólo después** se avanza
     (`pg_replication_slot_advance`). No hay proceso largo ni protocolo de replicación;
   - la carga inicial, en el *snapshot* exportado al crear el slot;
   - la marca de agua y la publicación en Iceberg **son la misma pieza que 0058 Q2**, y se escriben
     una vez;
   - **proteger el disco del cliente**: recomendar `max_slot_wal_keep_size`, medir el retraso de cada
     slot, avisar por Data Health (0063) y borrar el slot al borrar la fuente;
   - un slot perdido (un *failover* sin los slots de PG17) es «recopiar», y se dice.

## El plan

Cada paso se construye y se prueba **en el laboratorio**: Postgres 13 a 17 en Docker, un pgbouncer, una
CA propia y un bastión SSH en una red privada de Docker. Después se prueba **en producción** contra lo
que haya cuenta. Lo que no tenga cuenta queda como deuda temporal, igual que en 0061.

| hito | qué | laboratorio | de verdad |
|---|---|---|---|
| **PG0 · medir** | sin código: la matriz de hoy contra cada sabor y modo | PG 13–17, pgbouncer en modo transacción, CA propia | Neon (victor), Supabase gratis, Cloud SQL (con go por el coste) |
| **PG1 · TLS** | D-PG1 | CA propia, nombre que no casa, servidor sin TLS | Supabase, Cloud SQL |
| **PG2 · tipos y filas** | D-PG4 y D-PG5 | una tabla con todos los tipos; pgvector y PostGIS en sus imágenes | victor (`brain_embeddings`) |
| **PG3 · el puerto por fuente** | la `NetworkPolicy` por Job (D-PG3); el binario antes que la malla | `kind` o la celda de pruebas | el pooler de Supabase en el 6543 |
| **PG4 · túnel SSH** | D-PG3 con la clave de la celda | bastión y Postgres en una red privada de Docker | un bastión en una VM nuestra |
| **PG5 · IAM de GCP** | D-PG2, Cloud SQL y AlloyDB | — (no hay emulador) | una instancia Cloud SQL mínima nuestra (con go) |
| **PG6 · IAM de AWS y Azure** | D-PG2: prefirma RDS y Entra | vectores de prueba de la firma; Entra contra un IdP de mentira | **deuda** hasta que haya cuenta (como F9·5) |
| **PG7 · sabores** | D-PG6: perfiles, `check` y el asistente | todo lo anterior | lo que hayan probado PG1–PG6 |
| **CDC · etapa propia** | D-PG7: CDC0 medir, CDC1 publicación y slot desde `check` y el asistente, CDC2 carga en el *snapshot*, CDC3 consumir por lotes y avanzar tras el *commit*, CDC4 proteger el disco (retraso, avisos, borrado), CDC5 borrados, TOAST y TRUNCATE | PG 16–17 con `wal_level=logical` | Neon y Supabase; Cloud SQL con su flag |

**Orden:** PG0 → PG1 → PG2 → PG3 → PG4 → PG5 → PG7 → (PG6 cuando haya cuenta) → CDC. PG1 y PG2 sólo
tocan el driver y se pueden hacer en paralelo. PG3 es malla y lleva su propio go.

### PG0 · Medir, sin código

| paso | qué |
|---|---|
| **PG0·1** | En el laboratorio: el driver de hoy contra PG 13, 14, 15, 16 y 17, con y sin TLS y con una CA propia. Se apunta qué pasa con `sslmode` ausente, `prefer` y `require`, y se confirma la bajada a texto plano con `prefer` ante un servidor sin TLS |
| **PG0·2** | Pgbouncer en modo transacción (el del laboratorio de 0058): `catalogo`, `leer` en Arrow y `servir` reutilizando la sesión. Qué rompe exactamente (*prepared statements* con nombre, portales) |
| **PG0·3** | El inventario de tipos: una tabla con cada tipo de §5 de la investigación, más pgvector y PostGIS, por `leer` v1 y v2. Qué se niega hoy y qué devuelve `::text` |
| **PG0·4** | `reltuples` y `relpages` en una base con 300 tablas y particiones, con el tiempo del catálogo antes y después |
| **PG0·5** | De verdad, con lo que haya: Neon (ya conecta), Supabase (CA propia y los dos puertos del pooler) y, con go, una Cloud SQL mínima. Se apunta cuál conecta hoy y con qué error falla cada uno |
| **PG0·6** | Una réplica en el laboratorio: una lectura larga contra un `VACUUM` en la primaria. Qué SQLSTATE sale y cómo lo traduce el driver |

PG0 cierra con una tabla en este ADR, «lo que conecta hoy y lo que no», que confirma o corrige D-PG1 a
D-PG6 antes de escribir código.

### Lo que entra en verde en cada paso

- los tests de Rust de los crates tocados;
- `ore-kit --banco postgres --exige todos`, con y sin pasarela;
- `descubrimiento.sh`, `la-copia-desde-el-origen.sh`, `fuentes-reales.sh` y `los-verbos.sh`;
- `dependencias.rs` (`ore-serve` no lee orígenes);
- la nueva prueba de fuego de cada paso (`pruebas-de-fuego/pg-*.sh`);
- el servicio Postgres del CI gana una CA propia y TLS obligatorio desde PG1.

### PG0 · Medido (2026-10-16): lo que conecta hoy

Se midió el `ore-read-postgres` de `main` (`6df6c9b9`) tal cual, sin cambiar una línea. Hay dos guiones:

- **Laboratorio:** [`pg0-lo-que-conecta.sh`](../../pruebas-de-fuego/pg0-lo-que-conecta.sh), con las fases
  `tls`, `pooler`, `tipos`, `filas` y `replica`.
  - Monta, en Docker y sin cuentas:
    - PG 13–17 con una CA propia;
    - un nombre que el certificado no lleva (`pg-mal`);
    - un Postgres sin TLS;
    - tres PgBouncer 1.22 (modo transacción, modo transacción con `max_prepared_statements=0` y modo
      sesión, todos con `default_pool_size=1`);
    - pgvector y PostGIS;
    - una primaria con su réplica (`max_standby_streaming_delay=1s`).
  - El driver se compila dentro de `rust:1-bookworm`, con OpenSSL como en producción.
- **De verdad:** [`salud/pg0-de-verdad.sh`](../../pruebas-de-fuego/salud/pg0-de-verdad.sh), que se lanza
  con `pg0-en-la-celda.py <fuente>`.
  - Corre en un Job de `t-victor` con el binario de `ore-drivers:main`.
  - La URL sale del cofre y no se imprime.
  - Se midió contra `pg_quality`: Neon, PG 17.11, en `us-east-1`. Su URL es la del pooler
    (`-pooler`) con `sslmode=require&channel_binding=require`.

**La tabla:**

| caso | qué pasa hoy | decisión |
|---|---|---|
| **CA privada** (RDS, Cloud SQL, Supabase, Aiven), PG 13–17 | **no conecta** con ningún `sslmode` salvo `disable`. `check` dice sólo «error performing TLS handshake»; la causa (`unable to get local issuer certificate`) sólo sale en `catalogo` | D-PG1 |
| la misma CA, puesta en el almacén del sistema | conecta sin `sslmode`, con `prefer` y con `require`. **El nombre se verifica**: `pg-mal` falla con los tres. Nuestro `require` ya es `verify-full` | D-PG1, confirmada |
| `sslmode=verify-ca` / `verify-full` | **«invalid connection string»** en todos los servidores: la URL que trae la guía de RDS no se acepta | D-PG1 |
| `sslmode=disable` en la URL | conecta **en claro** contra cualquier servidor que lo admita (el log de pg17 tiene 4 sesiones sin TLS). Neon lo niega, y el driver lo dice como «db error», sin causa | D-PG1 |
| **servidor sin TLS** | sin `sslmode` y con `prefer`, **conecta en claro** (8 de 8 sesiones sin TLS en el log); `require` falla. Es la bajada que D-PG1 prohíbe | D-PG1 |
| **Neon**, por el pooler y directo | conecta sin `sslmode`, con `prefer` y con `require` (CA pública). El proxy termina TLS, así que `pg_stat_ssl` dice `false` aunque la sesión vaya cifrada. Catálogo: 49 tablas en 1,6 s, ninguna con filas; 8 columnas `Opaque`. Arrow ×2, `servir` ×3 y **4 lectores a la vez: 20/20**, por el pooler y directo | — |
| PgBouncer en **modo transacción**, `max_prepared_statements=200` (el defecto desde 1.21) | todo bien, también con 4 lectores a la vez (20/20) | — |
| PgBouncer en modo transacción **sin *prepared statements*** (como Supavisor en el 6543 o un pgbouncer < 1.21) | con un solo lector, todo bien. Con 4 a la vez: **14/20** lecturas Arrow y **7/12** de v1. El error es `42P05: prepared statement "s0" already exists`, y en v1 sale como «la consulta falló: db error» | D-PG6, corregida |
| PgBouncer en **modo sesión** | 20/20 | — |
| **réplica**, una lectura larga contra un `VACUUM` en la primaria | el driver da `{"codigo":"origen","mensaje":"40001: terminating connection due to conflict with recovery","reintentable":true}`: está tipado y es reintentable, pero no dice qué hacer (`hot_standby_feedback`) | D-PG6 |
| testigo en la primaria y en la réplica con `wal_level=replica` | `{"modo":"none"}` con su aviso, como se espera | — |
| **tipos**: 38 columnas | **leen 11**: `xml`, `"char"`, `numeric` (también `NaN`), `timestamp`, `timestamptz`, `date`, enum, `jsonb`, `bytea`, `uuid` y `float8`. **Se niegan 27**: arrays (también 2D y `numeric[]`), `interval`, `timetz`, `inet`, `cidr`, `macaddr`, `macaddr8`, `money`, `bit`, `varbit`, rangos y multirangos, compuestos, `hstore`, `citext`, `ltree`, `tsvector`, `vector`, `point`, `box`, `circle`, `pg_lsn`, `regclass`, y `geometry` / `geography` (PostGIS) | D-PG4 |
| `infinity` / `-infinity` en `date`, `timestamp` y `timestamptz` | **v1 los lee; Arrow los niega** («`infinity` no es un timestamp[us]»): la misma fila sale por un camino y no por el otro | D-PG4, corregida |
| el catálogo frente a la lectura | **el catálogo promete lo que la lectura no da**: `int4[]` → `list<Integer>`, `timetz` → `Time`, `money` → `Decimal`, `citext` → `String`, y las cuatro columnas se niegan al leerlas. Los rangos, compuestos, `hstore`, `ltree`, `vector`, `regclass`, `"char"` y PostGIS salen sin `type` («no supo»). En Arrow, el error nombra el alias (`c`) y no la columna | D-PG4, corregida |
| `reltuples` | **-1 en tablas recién cargadas** (t300, 3200 filas, sin pasar por autovacuum) y en la particionada. Tras `ANALYZE`, exacto en las pequeñas (t1 210, t300 3200); la padre da 40 000 en PG17 (`relpages` -1), y la suma de sus hijas también 40 000 | D-PG5, corregida |
| el coste del tamaño | sobre 306 relaciones: `relpages × block_size`, **10 ms**; `pg_total_relation_size`, **278 ms**. El catálogo de 302 relaciones tarda unos 0,5 s y no trae ni `rows` ni `rowsEstimate` | D-PG5, confirmada |

**Supabase, pendiente.** Hace falta un proyecto (gratuito) dado de alta como fuente en `t-victor`.
Después se lanza `pg0-en-la-celda.py <fuente>`. El guion ya prueba la fuente y sus dos puertos: el 5432
(sesión) y el 6543 (transacción, que la malla no deja salir; se apuntará como tal, y es PG3).

**Lo que la medida cambia en las decisiones:**

- **D-PG1 · confirmada, con dos añadidos.**
  - El error de TLS lleva su causa en todos los verbos: hoy `check` la pierde y `catalogo` la da
    entera.
  - `sslmode=verify-ca` / `verify-full` en la URL **se aceptan** como lo que dicen; hoy rompen la URL.
  - «db error» deja de salir sin causa en ningún verbo.
- **D-PG4 · corregida.**
  1. **El catálogo y la lectura dicen lo mismo**: el `type` del catálogo es lo que la lectura entrega, y
     una prueba lo coteja columna a columna sobre la tabla de PG0.
  2. **`infinity` se lee igual por v1 y por Arrow.** Propuesta: en Arrow, el valor máximo o mínimo de la
     columna con un aviso, o la columna como texto. Se decide en PG2, con la regla del contrato de nulos
     (0051).
  3. El error nombra la columna de la tabla, no el alias.
- **D-PG5 · corregida.** `reltuples = -1` es lo normal en una tabla recién cargada, así que no basta con
  él. El orden queda así:
  1. `reltuples` si es ≥ 0;
  2. si no, `pg_stat_user_tables.n_live_tup`, que sube con cada inserción sin esperar a `ANALYZE`;
  3. si no, sin estimación.

  Para la particionada, la suma de sus hijas. PG2 mide si `n_live_tup` está en un tiempo despreciable.
- **D-PG6 · corregida.** No basta con detectar el pooler y decirlo: **el driver deja de usar sentencias
  con nombre** (`query_typed` / `query_typed_raw` del crate `postgres` 0.19, que usan la sentencia sin
  nombre). Así funciona detrás de cualquier pooler en modo transacción, también Supavisor en el 6543. Se
  mide con la misma concurrencia de PG0 (con `max_prepared_statements=0` tiene que dar 20/20). El error
  `40001` de una réplica dice qué hacer: `hot_standby_feedback` o una espera más larga.

### PG1 · TLS: hecho en el laboratorio (2026-10-16)

| paso | qué |
|---|---|
| **PG1·1** | **`crates/ore-read-postgres/src/tls.rs`.** Lee `sslmode` y `sslrootcert` de la URL, en forma URL o de pares, y los quita antes de que la URL llegue a `tokio-postgres`. Decide: sin `sslmode`, `require` o `verify-full` → la cadena y el nombre; `verify-ca` → sólo la cadena; `disable`, `allow` y `prefer` → **se niegan** sin abrir nada. Con `sslrootcert=<PEM codificado>` (uno o varios certificados) **sólo esa CA vale**, como en libpq; `sslrootcert=system` es el almacén del sistema. Una ruta o un nombre se niegan diciendo que el PEM va dentro de la URL. 8 tests |
| **PG1·2** | **Todos los verbos abren por `tls::abrir`**: `check`, `explorar`, `catalogo`, `leer` v1 y v2, `testigo`, `cambio`, `estimar` y `servir`. Antes había siete `Client::connect` con su propio conector. Todos ganan los 10 s para conectar, el `application_name` y la sesión de sólo lectura. **Cancelar en el origen** habla TLS con el mismo conector (lo guarda la sesión, por URL): Neon enruta por SNI y una CA propia no estaría en el de por defecto |
| **PG1·3** | **El error dice la causa y qué hacer.** `check` ya no dice sólo «error performing TLS handshake»: lleva la cadena entera del crate y una pista para tres casos. CA desconocida → «da su CA en `sslrootcert`»; nombre que no casa → «`sslmode=verify-ca`»; servidor sin TLS → «activa `ssl = on`». Las consultas que decían «db error» llevan la causa (`texto::causa`) |
| **PG1·4** | **El asistente** (`GET /fuentes/credenciales/postgres`) tiene su modo propio: `cadena` con el formato (`sslmode`, `sslrootcert`), y tres `noAdmitidos`: `en-claro`, `cualquier-certificado` y `fichero` |
| **PG1·5** | **`ORE_PG_LABORATORIO=1`** deja pasar `disable` y `prefer`, y sin `sslmode` vale `prefer`, lo de antes. Está en el `env` del flujo de la CI (sus servicios Postgres no hablan TLS) y en las pruebas de fuego de laboratorio (`pg0-…`, `medida-f0-…`). **Nunca en la malla** |
| **PG1·6** | **[`pruebas-de-fuego/pg1-tls.sh`](../../pruebas-de-fuego/pg1-tls.sh): 17/17 a la primera.** (1) CA propia sin darla: no conecta y pide `sslrootcert`. (2) Con la CA en la URL: `check`, `catalogo`, Arrow y `servir` ×3. (3) `require`, `verify-full` y `verify-ca` con la CA. (4) `pg-mal` se niega y propone `verify-ca`; con él, conecta. (5) La CA de otro: no conecta. (6) Sin TLS: no conecta y lo dice; `disable`, `allow` y `prefer` se niegan. (7) Con la bandera del laboratorio, como antes. (8) SIGTERM a mitad de un flujo por TLS con la CA propia: 1 → 0 consultas vivas en el origen |
| **PG1·7** | **El censo de producción** ([`salud/pg1-censo.sh`](../../pruebas-de-fuego/salud/pg1-censo.sh), en un Job por inquilino). Las fuentes Postgres son **9**: 4 en victor y 5 en demo. Todas son Neon por el pooler con `sslmode=require&channel_binding=require`, y todas pasan con `require` forzado, que es lo que hará el driver nuevo sin `sslmode`. **Desplegar no rompe ninguna** |

**Lo que no entra en PG1:**
- el *bundle* de RDS como nombre (`sslrootcert=aws-rds`): hoy se pega el PEM, como cualquier otra CA;
- el emisor y el sujeto del certificado en `check`: el conector no los expone sin otra dependencia, y la
  causa del error ya dice lo que falta;
- el certificado de cliente.

Las tres esperan a que alguien las pida.

#### PG1 · En producción (2026-10-16): cerrado

- **Desplegado.** `fe9d51a5` en `main`. La CI quedó en verde con `ORE_PG_LABORATORIO=1`, incluidos
  `conectores`, `descubrimiento`, `fuentes-reales` y `los-verbos`, que usan su Postgres sin TLS. La
  imagen `ore-drivers:main` se publicó.
- **Neon, con el binario nuevo** (`pg0-de-verdad.sh`, Job en `t-victor`). El lanzador ahora pide
  `imagePullPolicy: Always`, porque `:main` se mueve y la caché del nodo daría el binario de antes.

| | antes (PG0) | ahora |
|---|---|---|
| la URL tal cual (`require` + `channel_binding`) | ok | ok |
| sin `sslmode`, `require` | ok | ok |
| `verify-full`, `verify-ca` | «invalid connection string» | **ok** |
| `disable` | «db error» | **se niega**, diciendo por qué y qué poner |
| `prefer` | ok | **se niega**, diciendo por qué y qué poner |
| catálogo | 49 tablas, 1,6 s | 49 tablas, 1,5 s |
| Arrow ×2, `servir` ×3, 4 lectores a la vez (pooler y directo) | 20/20 | 20/20 |

**PG1 queda cerrado.** Siguiente: PG2 (tipos, filas estimadas y sentencias sin nombre).

## Lo que no se hace aquí

- Otros motores: MySQL, SQL Server, Oracle, Snowflake. Irán en su propio ADR, que reutilizará D-PG1,
  D-PG2 y D-PG3.
- Los «compatibles con Postgres» (D-PG6).
- PrivateLink y PSC, el túnel inverso o un agente, y el certificado de cliente (D-PG1 y D-PG3).
- El incremental por `xmin`: el testigo y `cambio` ya cubren el «¿cambió?», y el CDC cubre el resto.
- Escribir de vuelta en el Postgres del cliente.
