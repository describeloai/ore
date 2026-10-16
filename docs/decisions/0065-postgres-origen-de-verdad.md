# 0065 · Postgres como origen, de verdad

**Estado:** propuesto (2026-10-16). **PG0 medido**, **PG1 cerrado en producción** y **PG2 hecho en el laboratorio** **PG3 escrito** , **PG4** y **PG5 hechos en el laboratorio** (2026-10-16); Supabase
pendiente de un proyecto. Cuatro decisiones corregidas por la medida (D-PG1, D-PG4, D-PG5, D-PG6). Siguiente:
PG7 (sabores y `check`) o el CDC; se empuja todo junto al final. Investigación:
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

### PG2 · Tipos, filas y pooler: hecho en el laboratorio (2026-10-16)

**Una decisión de diseño cubre a la vez D-PG4 y D-PG6: las lecturas van por el protocolo simple.**
- `leer` en Arrow abre una transacción de sólo lectura y hace `DECLARE ore_lectura NO SCROLL CURSOR FOR …`
  y `FETCH 8192` lote a lote. Antes iba por un portal con una sentencia con nombre.
- `leer` v1 y `estimar` hacen una sola consulta simple.
- Las consultas del catálogo, `testigo`, `cambio` y `explorar` usan `query_typed`, la sentencia sin
  nombre.

Con eso:
- **no queda ninguna sentencia con nombre** (D-PG6), así que un pooler en modo transacción sin
  *prepared statements* deja de chocar;
- **cada valor llega como el texto que escribe el propio Postgres** (D-PG4), así que no queda ningún
  tipo que no se sepa leer.

| paso | qué |
|---|---|
| **PG2·1** | **Los parámetros, como literales** (`lectura::en_linea`). `'7'` sin tipo: el servidor lo coacciona igual que el parámetro en texto que se mandaba, así que un `$1` sobre un `int4` sigue siendo `int4`. Las comillas se doblan. Un `$n` dentro de un identificador o de un literal no se toca. **La sesión se fija en cada transacción** con `SET LOCAL` (detrás de un pooler un `SET` de sesión se pierde): `standard_conforming_strings`, UTC, ISO, `IntervalStyle`, `extra_float_digits = 1` y `bytea_output = hex`. Es lo que hace canónico el texto |
| **PG2·2** | **Tipos (D-PG4).** Lo exacto se analiza con `ore_core::tipos`, como antes. Normalización: `t`/`f` pasan a `true`/`false`. `infinity`/`-infinity` en fecha, `timestamp` y `timestamptz` van a 9999-12-31 / 0001-01-01; `NaN` o infinito en un `numeric(p,s)` van a nulo. Las dos cosas se avisan por stderr, contadas por columna. **Un array de un escalar exacto es una lista de Arrow** (`List<Int64>`, `List<Utf8>`, `List<Bool>`, `List<Timestamp tz>`…), sacada de su texto (`elementos`: comillas, `NULL` y límites `[2:3]=`). Uno de dos dimensiones se niega nombrando la columna. Todo lo demás sale como su texto. `texto.rs` pierde el decodificador binario (`Texto`, `Parametro`) y se queda con `causa` |
| **PG2·3** | **El catálogo dice lo que la lectura da.** `exacto` es una sola función para las dos. Un array es `list<X>` si X es exacto y `Opaque` si no; `timetz` y `money` pasan a `Opaque` (antes `Time` y `Decimal`, que no se leían). Un test lo coteja por tipo, y la prueba de fuego columna a columna contra el esquema Arrow. Los errores nombran la columna de la tabla y no el alias |
| **PG2·4** | **Filas estimadas (D-PG5)**, en la misma consulta del catálogo: `reltuples` si es ≥ 0; si no, `n_live_tup` si es > 0; si no, nada. Una particionada suma sus hojas (`pg_partition_tree`). Sale como `rowsEstimate`; **`rows` nunca**: el inductor sólo lee `rows` para proponer «cero filas», y una estimación no puede llevarle a borrar una tabla viva |
| **PG2·5** | **[`pruebas-de-fuego/pg2-tipos-filas-pooler.sh`](../../pruebas-de-fuego/pg2-tipos-filas-pooler.sh): todo verde.** (a) Las **45 columnas** (las 38 de PG0 más arrays de `bool`, `timestamptz`, `numeric(6,2)` y `bool`) se leen por v1 y por Arrow, y **el tipo Arrow de cada una es el que dice el catálogo**: 0 mal, frente a 27 negadas en PG0. PostGIS sale como su EWKB en hexadecimal. (b) `a_2d` con dos dimensiones se niega nombrando la columna. (c) v1 escribe como antes: `true`, `\xdeadbeef`, `0.1`, `12.50`, `2026-10-16 08:00:00+00`. (d) Filas: antes de `ANALYZE`, `t1=210` y `parte=4000` por `n_live_tup` y `vacia` sin estimación; después, por `reltuples`, con `vacia=0`; nunca `rows`. (e) PgBouncer en modo transacción **sin *prepared statements*** y `default_pool_size=1`: 4 `servir` + 4 v1 a la vez dan **20/20 y 12/12**, frente a 14/20 y 7/12 en PG0. `estimar` también va por el pooler. `pg1-tls.sh` repetido: 17/17, cancelar incluido |

**Lo que cambia para quien ya tiene fuentes:**
- una columna `timetz` o `money` pasa de `Time`/`Decimal` a `Opaque` en el catálogo, y su huella de
  `cambio` (que lleva el tipo de cada columna) se mueve una vez;
- las tablas ganan `rowsEstimate`.

Las Neon de producción no tienen arrays (PG0); sus tipos `Time` habrá que comprobarlos al desplegar.

**Lo que no entra en PG2 (D-PG4, esperan a que se pidan):** `vector` como `FixedSizeList<Float32>`,
PostGIS como WKB binario con GeoArrow, `hstore` como `Map`, `interval` como `Interval` y los
compuestos como `Struct`. Hoy los cinco salen como su texto, que es exacto.

### PG3 · El puerto: un rango, no una política por fuente (2026-10-16)

**D-PG3 se corrige.** Lo que el plan decía —«la fuente declara su puerto y el renderizador emite la
`NetworkPolicy` del Job»— no casa con la malla, por dos razones:
- la cola sólo puede crear `batch/jobs`: que ore-serve escriba políticas sería el gobernado escribiendo
  su gobierno (`13-el-inquilino-reconciliado.yaml`, `cola.rs`);
- la pasarela es **un** pod que lee todas las fuentes, así que una política por Job no la cubriría.

**Lo que hace el sector:**
- Foundry exige *network egress policies* (DNS para HTTP(S); CIDR y puerto para lo demás) que aprueba
  el *Information Security Officer*.
- Snowflake usa `NETWORK RULE … MODE = EGRESS` con `host:puerto`.
- Databricks serverless tiene *network policies* opcionales («todos los destinos» o «restringido»),
  sólo en sus planes altos.
- Fivetran y Airbyte no restringen su salida.

**Decidido:** `salida-del-driver` abre **TCP 1024–65535** (`endPort`), además del 443 y el 22, siempre
a IPs públicas y sólo para `ore.dev/rol: driver`. El 5432 entra en el rango.
- Lo que se añade es poco: el 443 ya alcanza a cualquier host público, y en un driver sólo corre lo
  que manda su plantilla.
- Restringir por destino (`host:puerto` aprobado por un rol, con `FQDNNetworkPolicy` de GKE) es otra
  pieza, para cuando un cliente regulado la pida.

| paso | qué |
|---|---|
| **PG3·1** | `malla/20-driver.yaml`: los puertos son `443`, `22` y `1024–65535`. `gen-inquilino.py --comprobar`: «la plantilla y el renderizador dicen lo mismo». Victor renderizado lleva el rango. `kubectl apply --dry-run=server` acepta el de demo y el de victor |
| **PG3·2** | **Sin aplicar**: entra con el despliegue final (Flux, al converger), después del binario de PG1/PG2. Quedará comprobar desde un Job de victor que un Postgres en el 6543 llega (el pooler de Supabase, cuando haya proyecto) |

### PG4 · Túnel SSH: hecho en el laboratorio (2026-10-16)

| paso | qué |
|---|---|
| **PG4·1** | **`ore-sftp` presta su sesión.** `ore_sftp::sesion` hace el saludo, compara la huella **antes** de autenticar y entra con la clave de la celda (`ORE_SFTP_CLAVE`): la identidad de D-O4, sin copiarla. `Fuente::bastion` construye la coordenada de un bastión. `conectar` de SFTP pasa a usarla, sin cambiar su comportamiento |
| **PG4·2** | **`crates/ore-read-postgres/src/tunel.rs`.** La URL lleva `ssh=<usuario>@<bastión>[:<puerto>]&ssh_huella=SHA256:<…>[&ssh_legado=1]`; `separar` lo quita antes que nada. `abrir` comprueba ya el bastión y el camino hasta la base: el primer canal `direct-tcpip` se abre al empezar, así que el error sale con su causa (la huella, la clave, «¿llega a la base y permite el reenvío, `AllowTcpForwarding`?»). Después deja un reenvío en `127.0.0.1:<efímero>`, dentro del proceso, con una sesión SSH y su canal por cada conexión aceptada (cancelar abre la suya). El bombeo va sin bloquear: un canal de `ssh2` no se lee y escribe desde dos hilos |
| **PG4·3** | **TLS de punta a punta.** `tls::abrir` escribe el puerto del reenvío en la URL y pone `hostaddr = 127.0.0.1`: el **nombre** del host sigue siendo el de la base, y el certificado se verifica contra él, no contra el bastión. De paso, `sslrootcert` deja de convertir `+` en espacio (un PEM en base64 lo lleva; libpq tampoco lo hace) |
| **PG4·4** | **El asistente** tiene un segundo modo, `tunel-ssh`: el formato, el comando con la clave pública de la celda para el `authorized_keys` del bastión, y la IP de salida |
| **PG4·5** | **La clave, en el Job de catálogo** (`malla/44-el-catalogo.yaml`). Su init ya traía secretos a un tmpfs: ahora trae también `t-<celda>-sftp-clave` a `/puesto/sftp`. Es **opcional**, como en la pasarela, y va con `ORE_SFTP_CLAVE`. La copia y la comprobación, cuando hay pasarela, pasan por ella, que ya la tenía. `gen-inquilino.py --comprobar` en verde. **Sin aplicar**: entra con el despliegue final |
| **PG4·6** | **[`pruebas-de-fuego/pg4-tunel-ssh.sh`](../../pruebas-de-fuego/pg4-tunel-ssh.sh): 13/13.** La base (TLS, CA propia) en una red Docker **interna**; el bastión (OpenSSH) en las dos; el driver sólo en la pública. (1) Sin túnel no se alcanza. (2) Sin huella: no autentica y enseña la vista. (3) Con huella y CA: `check`, `catalogo`, Arrow, v1 y `servir` ×3, con TLS verificando `pgdb`. (4) Huella ajena: se niega sin autenticar. (5) Clave que el bastión no tiene: se niega y dice `authorized_keys`. (6) Host que el bastión no alcanza y (7) bastión sin reenvío: lo dicen. (8) SIGTERM a mitad de un flujo: la sesión desaparece del origen (1 → 0). (9) 4 lectores a la vez, cada uno con su túnel: 20/20 |

Lo medido de paso: el `sshd_config` de Alpine trae `AllowTcpForwarding no`, y sshd se queda con la
**primera** aparición. Es lo primero que fallará con un bastión real, y el error ya lo nombra.

### PG4c · La consola, al día con el driver (2026-10-16)

El paso 3 del alta («Connection details»), para Postgres, era la hoja genérica de una base con host:
- los cinco modos SSL de libpq, **con `disable` por defecto**;
- un certificado de cliente y una CA como **rutas** de fichero;
- «Allow Unauthorized Certs».

**Y nada de eso llegaba a la URL**: `direccionDe` componía sólo `postgresql://usuario:clave@host:puerto/base`.

`rubix-platform` d54d27a cambia eso con una hoja propia, `PostgresConnection.tsx`:
- **TLS:** «verificar el certificado y el nombre» (por defecto, sin `sslmode`) o «sólo el certificado»
  (`verify-ca`).
- **La CA, pegada como PEM** y compuesta como `sslrootcert`.
- **Los no admitidos**, que salen del asistente: en claro, cualquier certificado y la CA como fichero.
- **«Mi base no está en internet»:** bastión, puerto, usuario y huella. Enseña la clave pública de la
  celda y el comando para el `authorized_keys`. El asistente gana `clavePublica` en el modo
  `tunel-ssh`.

La dirección enseñada tapa la CA como `(PEM)`. Una cadena de conexión escrita a mano sigue mandando,
tal cual. Comprobado así:
- la composición de la URL, en 6 casos: por defecto, `verify-ca`, PEM, túnel, túnel sin huella y
  cadena a mano;
- `tsc` sin errores en el asistente.

**Sin verla en el navegador:** el banco no arranca mientras otra sesión tiene el `next dev` del mismo
directorio, y la consola real pide iniciar sesión.

### PG5 · IAM de Google: hecho en el laboratorio (2026-10-16)

| paso | qué |
|---|---|
| **PG5·1** | **`crates/ore-read-postgres/src/iam.rs`.** `iam=gcp` en la URL: el *access token* de la celda (`ore_gcp::Credencial::del_entorno`: Workload Identity en GKE, `ORE_GCP_TOKEN` en local), pedido en **cada conexión**, va como contraseña. El usuario es la cuenta sin `.gserviceaccount.com` (`ore-driver-<celda>@<proyecto>.iam`, con la `@` como `%40`). Con `suplanta=<cuenta del cliente>` el token es el de esa cuenta: `ore_gcp::suplantar` (`generateAccessToken`, alcances `sqlservice.login` y `cloud-platform`; `ORE_GCP_IAM` para el laboratorio). Con `iam=gcp`, una contraseña en la URL o la falta de usuario se niegan, y una autenticación rechazada dice qué mirar: la bandera de la instancia, el usuario IAM y `roles/cloudsql.instanceUser`. `iam=aws` o `iam=azure` dicen «PG6» |
| **PG5·2** | **El asistente**: modo `iam-gcp` con el usuario IAM de la celda (sale de `--cuenta-driver`), el formato y los cinco comandos. Cloud SQL: la bandera (avisando de que `--database-flags` sustituye las que ya hay), el usuario IAM y el rol. Dentro de la base, el `GRANT`. AlloyDB, en su variante |
| **PG5·3** | **La consola** (`rubix-platform`, en la hoja de PG4c): «Cómo entra», usuario y contraseña o «la cuenta de Google de esta celda». Con IAM no hay campo de contraseña, el usuario sale del asistente, se enseñan los pasos y se puede suplantar una cuenta propia. URL: 9/9 casos (los 6 de PG4c y 3 de IAM). `tsc` limpio |
| **PG5·4** | **[`pruebas-de-fuego/pg5-iam-gcp.sh`](../../pruebas-de-fuego/pg5-iam-gcp.sh): 11/11.** Un Postgres con usuarios «IAM» cuya contraseña es el token que les toca, `ORE_GCP_TOKEN` en el papel de Workload Identity y una API de IAM de mentira. (1) La celda: `check`, `catalogo`, Arrow y v1. (2) Con contraseña en la URL y (3) sin usuario: se niegan. (4) Token no aceptado: dice qué mirar. (5) Suplantando: entra y lee como la cuenta del cliente. (6) Suplantación no concedida: 403 y el rol. (7) `iam=aws`: PG6 |

**Lo que falta, y es deuda:** la prueba contra una Cloud SQL de verdad. Pide crear una instancia
mínima en nuestro proyecto (con go, por el coste) y medir que el token de Workload Identity de un Job
entra. No hace falta tocar la malla: los Jobs ya tienen la cuenta, y la salida al 443 para la API de
IAM y al 5432 de Cloud SQL ya está abierta.

## Lo que no se hace aquí

- Otros motores: MySQL, SQL Server, Oracle, Snowflake. Irán en su propio ADR, que reutilizará D-PG1,
  D-PG2 y D-PG3.
- Los «compatibles con Postgres» (D-PG6).
- PrivateLink y PSC, el túnel inverso o un agente, y el certificado de cliente (D-PG1 y D-PG3).
- El incremental por `xmin`: el testigo y `cambio` ya cubren el «¿cambió?», y el CDC cubre el resto.
- Escribir de vuelta en el Postgres del cliente.
