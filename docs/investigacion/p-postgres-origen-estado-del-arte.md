# Postgres como origen, de verdad — estado del arte

**Fecha:** 2026-10-16. **Para:** [`0065`](../products/0065-postgres-origen-de-verdad.md). **Alcance:** leer un
Postgres del cliente como origen de datos, igual que BigQuery (0042) o un almacén de objetos (0061).
No trata ORE Serverless Postgres (0058), salvo donde reutiliza piezas. El código citado es el de
`origin/main` en `6df6c9b9`.

## 1. Lo que hay hoy

Postgres fue el primer origen de verdad (septiembre de 2026) y funciona de punta a punta. Lo que tiene:

- **Verbos.** `ore-read-postgres` sabe `catalogo`, `explorar`, `check`, `leer` (v1 NDJSON y v2 Arrow),
  `testigo`, `cambio`, `capacidades`, `estimar` y `servir` (`crates/ore-read-postgres/src/main.rs:175-303`).
  Son los mismos que BigQuery. Le faltan dos cosas que BigQuery sí tiene:
  - un `check` que diga qué permiso falta (hoy sólo conecta);
  - el número de filas en el catálogo.
- **Lectura.**
  - Arrow en flujo por un portal, en una transacción de sólo lectura, de 8192 filas por lote
    (`lectura.rs:364-460`).
  - `statement_timeout` y cancelación en el origen.
  - Los SQLSTATE traducidos a errores tipados.
  - El *pushdown* de la pasarela: `eq/neq/in/range/isNull`.
- **Catálogo.**
  - Lee `pg_catalog`: tablas, particionadas, vistas, materializadas y foráneas.
  - Saca claves primarias, únicas y foráneas, dominios, enums y comentarios.
  - `filas: None` a propósito (`main.rs:50-57`).
- **Incremental.**
  - Rangos por una columna cursor.
  - El testigo es el LSN, y sólo con `wal_level=logical`.
  - `cambio` da una huella por tabla (`pg_stat_user_tables`, `relfilenode`, columnas).
  - **No hay CDC.** 0063 midió la decodificación lógica con `test_decoding` y no la construyó.

Y lo que le falta para ser «de verdad»:

| tema | hoy | dónde |
|---|---|---|
| TLS | `native-tls` con el almacén del sistema; `sslmode` el de `tokio-postgres` (`disable`, `prefer` o `require`); sin `sslrootcert` | `lectura.rs:72`, `main.rs:335-369` |
| identidad | sólo usuario y contraseña dentro de la URL (`modo: cadena`) | `credenciales.rs:813-817` |
| red | sólo al 5432, sólo a IPs públicas (la malla excluye RFC1918); sin túnel | `malla/20-driver.yaml:60-101` |
| tipos | 17 tipos; el resto se niega nombrando la columna (`vector`, arrays, `interval`, `inet`, `timetz`, PostGIS…) | `texto.rs:89-167`, 0053:215 |
| filas | ninguna estimación | 0062:76 (E4) |
| sabores | probado sólo contra `postgres:16-alpine` (CI) y un Neon (victor) | `ci.yml:140-168`, 0063:354 |

## 2. TLS y la autoridad de certificación

**libpq** define seis `sslmode`: `disable`, `allow`, `prefer` (el defecto), `require`, `verify-ca` y
`verify-full`. Sólo `verify-full` comprueba a la vez la cadena y el nombre del host. `require` cifra
pero no verifica, salvo que exista `root.crt`, una rareza histórica de libpq. Con `prefer`, si alguien
en medio quita TLS, la sesión sigue en claro.

**`tokio-postgres` 0.7.18**, nuestra versión, sólo tiene `Disable`, `Prefer` y `Require`
(`config.rs:46-53`). `verify-ca` y `verify-full` no entraron (rust-postgres PR #988). Lo que verifica lo
decide el conector TLS que se le pase. Con el nuestro (`native_tls::TlsConnector::new()`, que valida
cadena y nombre contra el almacén del sistema) pasa esto:

- **`require` se comporta ya como `verify-full`** con las CA públicas. Eso está bien.
- Un servidor con **CA privada** (RDS, Cloud SQL, Supabase, Aiven) **no conecta**: no hay manera de dar
  su CA.
- **Sin `sslmode` en la URL vale `prefer`**: si el servidor no ofrece TLS, o alguien lo quita por el
  camino, la contraseña viaja en claro por internet.

**Qué hace el sector:**
- Databricks (Lakeflow y Federation) pide la CA en PEM (`ssl_ca`). Tiene un `trustServerCertificate`
  que la propia Databricks dice «no usar en producción».
- Fivetran pide TLS activado en RDS y Aurora antes de conectar directo.
- Ninguno de los dos acepta texto plano por internet como opción recomendada.

**Las CA por sabor** (de su documentación; se verifica en PG0):

| sabor | CA del servidor |
|---|---|
| RDS / Aurora | CA propia de AWS, un *bundle* global descargable (`global-bundle.pem`) |
| Cloud SQL | CA **por instancia** (o la compartida por Google, según la configuración); el nombre DNS de la instancia en el certificado |
| AlloyDB | CA gestionada por Google; Google empuja el conector o el proxy |
| Azure Flexible | CA pública (DigiCert / Microsoft RSA Root), en el almacén del sistema |
| Neon | Let's Encrypt, en el almacén del sistema (medido: victor conecta hoy) |
| Supabase | CA propia, descargable desde el panel |
| Aiven | CA del proyecto, descargable |
| Heroku | certificados que no se pueden verificar contra una CA pública; Heroku documenta `require` |

**Certificado de cliente (mTLS):** Cloud SQL tiene «sólo certificados de cliente de confianza». Los demás
lo admiten pero no lo piden. Es raro en el sector como modo de entrada de un ETL.

## 3. Identidad: las IAM de las nubes

Las tres nubes hacen lo mismo: **un token corto de su IAM va como contraseña** y el servidor lo valida.
Psql y cualquier cliente funcionan sin saber nada de IAM.

| | token | quién | qué pide en la base | dura |
|---|---|---|---|---|
| **RDS / Aurora** | `generate-db-auth-token`: una URL prefirmada con SigV4 (servicio `rds-db`), firmada en local, sin llamada a AWS | un rol con `rds-db:connect` sobre `dbuser:<id>/<usuario>` | `GRANT rds_iam TO <usuario>`; TLS obligatorio | 15 min para **conectar**; la sesión sigue después |
| **Cloud SQL / AlloyDB** | el *access token* OAuth de la cuenta (`iam.serviceAccounts.getAccessToken`) | una cuenta de servicio, como usuario `<sa>@<proyecto>.iam` (sin `.gserviceaccount.com`) | flag `cloudsql.iam_authentication=on`; usuario IAM creado; `GRANT` dentro | 1 h |
| **Azure Flexible** | un token de Entra para `https://ossrdbms-aad.database.windows.net/.default` | una app o identidad gestionada, con rol en el servidor de su mismo nombre | autenticación Entra activada; el rol creado por un admin de Entra | ~1 h |

**Lo que ORE ya tiene para cada una** (y por eso este paso es barato):
- **GCP.** Las cuentas de la celda por Workload Identity (`ore-driver-<celda>`). BigQuery ya se lee así,
  y GCS también (D-O2).
- **AWS.** El rol del cliente que confía en las dos cuentas de Google de la celda (0046 E9b,
  `role_arn`). La firma SigV4 está en `ore-sigv4`. El token de RDS es una prefirma más: no hace falta
  un SDK.
- **Azure.** Las cuentas de la celda federadas en una app del cliente (D-O3), que ya canjean su ID token
  de Google por uno de Entra. Sólo cambia el ámbito.

**Lo que hace el sector:**
- Fivetran y Airbyte piden usuario y contraseña en casi todos los casos. Fivetran tiene «IAM auth» para
  RDS como opción.
- Databricks Federation pide usuario y contraseña.
- Las cuentas que ya tiene la celda permiten a ORE hacerlo sin ningún secreto guardado, como ya hace con
  los objetos. Es una ventaja frente al sector.

**Una trampa:** el token de RDS sólo vale 15 min para **abrir** la conexión. Si `servir` mantiene la
sesión abierta 60 s, no pasa nada. Lo que no se puede hacer es guardar el token: hay que firmarlo en
cada conexión.

## 4. Red: llegar a una base que no está en internet

Los modos que ofrece el sector, de más a menos común:

1. **Directo, con lista de IPs permitidas.** Es lo de Fivetran, Airbyte Cloud y Databricks Serverless.
   ORE ya tiene **IP de salida fija por celda** (Cloud NAT, `ORE_IP_SALIDA`, la de SFTP).
2. **Túnel SSH por un bastión del cliente.** Lo ofrecen Fivetran, Airbyte, Hightouch y Census. El
   cliente pega la clave pública del proveedor en `authorized_keys`. Fivetran: «debes conectar por SSH
   si tu base está en una subred inaccesible». **ORE ya tiene la pieza entera**: la clave Ed25519 de la
   celda, la huella del host fijada y `ssh2` (D-O4, `ore-read-sftp`).
3. **Túnel SSH inverso o agente.** Lo tiene Fivetran: el cliente abre la conexión hacia fuera y no hay
   que abrir nada de entrada. Su equivalente moderno es un conector saliente, como Cloudflare Tunnel
   (que ORE ya usa en su propia entrada, 0060 C2) o Tailscale.
4. **PrivateLink / Private Service Connect / Azure Private Link.** En Fivetran sólo existe en el plan
   *Business Critical*. Cada nube pide su propio montaje, y en GCP un *endpoint* PSC en nuestra VPC por
   cliente.

**El puerto.** La malla sólo deja salir al 443, al 5432 y al 22, y el propio fichero dice cuál es la
salida cuando la lista pese: «que la fuente declare su puerto y que el renderizador emita la política
del Job» (`20-driver.yaml:86-91`). Postgres ya la necesita:
- el *pooler* de Supabase en modo transacción va por el 6543;
- Aiven usa un puerto aleatorio por servicio;
- Crunchy y Timescale también tienen puertos propios.

## 5. Tipos

Los tipos que **no** se leen hoy y aparecen en bases reales, con su mapeo natural a Arrow:

| tipo | Arrow | nota |
|---|---|---|
| arrays (`int4[]`, `text[]`…) | `List<…>` | multidimensionales: aplanar o texto |
| `interval` | `Interval(MonthDayNano)` | es exacto: meses, días y microsegundos |
| `timetz` | Utf8 | Arrow no tiene hora con zona |
| `inet`, `cidr`, `macaddr`, `macaddr8` | Utf8 | su salida canónica |
| `money` | Utf8 (o Decimal con `lc_monetary`) | el binario es int64 con decimales según la *locale* del servidor |
| `bit`, `varbit` | Utf8 | |
| rangos y multirangos | Utf8 (o Struct) | |
| compuestos | Struct (o JSON) | |
| **extensiones** (OID variable; se reconocen por `typname`) | | |
| `vector` (pgvector) | `FixedSizeList<Float32>` | lo pidió F2·5 (0053:215) |
| `geometry` / `geography` (PostGIS) | Binary WKB con metadatos GeoArrow | o WKT; lo que hacen DuckDB y BigQuery |
| `hstore` | `Map<Utf8, Utf8>` | |
| `citext`, `ltree`, `tsvector` | Utf8 | |

El patrón del sector:
- Fivetran convierte lo que no conoce a texto.
- Airbyte convierte arrays a arrays y el resto a texto.
- Databricks Federation da error con los tipos no soportados.

**Postgres sabe escribir cada valor como texto canónico** (`::text`), así que una regla general
—«lo que no se tipa con exactitud viaja como su texto, citando `physicalType`»— no pierde nada. Sólo
cambia la promesa de W1: «se niega» pasa a «viaja como texto, y se dice».

## 6. Filas estimadas

- `pg_class.reltuples` es gratis: está en la misma consulta del catálogo.
- Desde PG14 vale **-1 si la tabla nunca pasó por VACUUM ni ANALYZE**: «desconocido», no cero.
- El padre de una tabla particionada da 0 o -1. Hay que sumar sus hijas.
- `pg_stat_user_tables.n_live_tup` es otra fuente, pero se reinicia con un *crash* y no existe en una
  réplica para la base primaria.
- Lo que **no** se usa es `pg_total_relation_size` por tabla: medido en 0058 P8·3, sale a 4,8 s para
  301 tablas. `relpages × block_size` da el tamaño sin coste.
- Encaja con 0062: es una `rowsEstimate`, nunca `rows`.

## 7. Sabores gestionados

| sabor | TLS | IAM | pooler | réplica de lectura | replicación lógica |
|---|---|---|---|---|---|
| RDS | CA de AWS | sí (token SigV4) | RDS Proxy (opcional) | sí | `rds.logical_replication=1` (reinicio) |
| Aurora | CA de AWS | sí | RDS Proxy | sí (lectoras) | igual, en el clúster |
| Cloud SQL | CA de instancia | sí (OAuth) | Managed Connection Pooling (reciente) | sí | flag `cloudsql.logical_decoding=on` |
| AlloyDB | gestionada | sí | integrado | *read pool* | flag |
| Azure Flexible | pública | sí (Entra) | PgBouncer integrado (6432) | sí | `wal_level=logical` |
| Supabase | propia | no | Supavisor: sesión en 5432, transacción en 6543 | sí (planes de pago) | sí |
| Neon | Let's Encrypt | no | PgBouncer en el host `-pooler` | ramas y réplicas | interruptor en el panel |
| Aiven, Crunchy, Timescale, DigitalOcean | CA propia | no | según el caso | sí | sí, normalmente |
| Heroku | sin verificación | no | PgBouncer | *followers* | limitada |

La tabla sale de la documentación de cada proveedor y del conocimiento del sector; PG0 la verifica
contra los que se puedan probar, y el perfil de cada uno dirá «probado» o «de su documentación».

Lo que importa para el driver:

- **Poolers en modo transacción.** Supabase lo dice claro: el modo transacción no admite *prepared
  statements* con nombre. `tokio-postgres` los usa siempre, y nuestro `servir` reutiliza la sesión entre
  transacciones. Si eso rompe hay que medirlo (el pgbouncer del laboratorio de 0058 sirve). Fivetran
  exige a pgbouncer el modo sesión. Lo prudente es pedir el modo sesión o el puerto directo, y que
  `check` lo detecte y lo diga.
- **Réplicas de lectura.** Son el sitio recomendado para copias grandes. Una consulta larga puede
  morir con «*canceling statement due to conflict with recovery*», que se arregla con
  `hot_standby_feedback` o `max_standby_streaming_delay`. Ese SQLSTATE (`40001`) tiene que salir con un
  error tipado que lo diga (está por comprobar en PG0). El testigo ya usa el LSN de replay en una réplica.
- **Fuera de este ADR, aunque «hablan Postgres»:** Redshift, CockroachDB, YugabyteDB, Materialize y
  Spanner PG. Cada uno se queda corto de `pg_catalog`, del protocolo o de las dos cosas. Necesitan un
  perfil propio si se piden.

## 8. CDC

**Cómo se hace en el sector** (Debezium, Airbyte, Fivetran, PeerDB, Estuary):

- **Decodificación lógica con `pgoutput`.** Viene con Postgres desde la 10, sin extensiones. Hace falta
  una **publicación** con las tablas y un **slot** por consumidor. El usuario necesita el atributo
  `REPLICATION`, o `rds_replication` en RDS.
- **La carga inicial coherente.** El slot se crea con `EXPORT_SNAPSHOT`, o con `USE_SNAPSHOT` por el
  protocolo de replicación, y la copia se lee en ese mismo *snapshot*. Así la carga y el flujo se cosen
  sin huecos ni duplicados.
- **El riesgo es el disco del cliente.** Un slot retiene WAL hasta que el consumidor confirma. Si el
  consumidor se para, el WAL crece sin límite. En el foro de Airbyte se cuenta un caso de 22 GB en
  3 días. La protección es `max_slot_wal_keep_size` (PG13+), además de vigilar el retraso y borrar el
  slot al borrar la fuente.
- **Bases tranquilas.** En una base sin escrituras en las tablas publicadas, el slot no avanza mientras
  el resto del servidor sí escribe. Debezium usa un latido (una tabla en la que escribe).
- **Lo que no viaja:**
  - los valores TOAST sin cambiar, en un UPDATE sin `REPLICA IDENTITY FULL`;
  - el DDL;
  - los TRUNCATE, salvo que se publiquen;
  - las secuencias.
- **Alta disponibilidad.** Desde PG16 se puede decodificar en una réplica. Desde PG17 hay slots de
  *failover* (`failover`, `sync_replication_slots`): sin ellos, un *failover* pierde el slot y obliga a
  copiar de nuevo.
- **La alternativa sin WAL** es `xmin` (Airbyte): incremental por la columna de sistema `xmin`. Tiene
  dos límites: no ve los borrados y la vuelta del contador (*wraparound*) obliga a recopiar.

**Lo que encaja con ORE:**
- ORE no tiene procesos largos por fuente: los Jobs son por lotes. Un **CDC por lotes** es lo natural:
  en cada sincronización se consume del slot desde el último LSN confirmado hasta el actual, se escribe
  en el lago y **sólo entonces** se avanza el slot.
- Se puede hacer **sin el protocolo de replicación** (que `tokio-postgres` no trae):
  `pg_logical_slot_peek_binary_changes(slot, upto, NULL, 'proto_version','1','publication_names','…')`
  y, tras el *commit* en el lago, `pg_replication_slot_advance(slot, lsn)`. Es lo que hacen los
  conectores por lotes, y 0063 ya midió `test_decoding` así.
- La marca de agua y la publicación en Iceberg son la misma pieza que **0058 Q2** (publicar las tablas
  de nuestro Postgres por CDC). Se escribe una vez.
- El testigo LSN de hoy y la huella de `cambio` ya dicen «nada cambió» sin abrir el slot.

## 9. Recomendación

1. **TLS primero:** `verify-full` por defecto, con la CA del cliente cuando no sea pública, y nunca
   texto plano por internet. Es lo que hoy impide conectar RDS, Cloud SQL y Supabase.
2. **Tipos y filas estimadas:** sólo tocan el driver y salen baratos. La regla general del texto
   canónico cierra la lista.
3. **El puerto por fuente** en la malla, y después **el túnel SSH** con la clave de la celda: con eso
   se llega a las bases privadas sin montar nada nuevo.
4. **IAM**, en el orden en que hay cuenta para probar: GCP (la nuestra), y después AWS y Azure,
   construidos y probados en el laboratorio, con la prueba real como deuda (como en 0061).
5. **Los sabores:** un perfil por proveedor, probado o «de su documentación», con lo que `check`
   detecta (pooler, réplica, `wal_level`).
6. **CDC como etapa propia**, por lotes y por SQL, compartiendo la marca de agua con 0058 Q2.

**Fuera, hasta que se pida:** PrivateLink/PSC, túnel inverso o agente, certificado de cliente, `xmin`,
otros motores (MySQL, SQL Server…) y los «compatibles con Postgres».

## Fuentes

- AWS, IAM con RDS y Aurora PostgreSQL: <https://aws.amazon.com/blogs/database/securing-amazon-rds-and-aurora-postgresql-database-access-with-iam-authentication/>;
  RDS Proxy entre cuentas: <https://aws.amazon.com/blogs/database/use-amazon-rds-proxy-with-iam-authentication-for-cross-account-access/>
- Google, IAM en Cloud SQL: <https://docs.cloud.google.com/sql/docs/postgres/iam-logins>;
  <https://cloud.google.com/blog/topics/developers-practitioners/authenticating-cloud-sql-postgresql-iam-service-accounts>
- Microsoft, Entra en Azure Flexible: <https://learn.microsoft.com/da-dk/azure/postgresql/flexible-server/how-to-configure-sign-in-azure-ad-authentication>;
  identidad gestionada: <https://learn.microsoft.com/da-dk/azure/postgresql/security/security-connect-with-managed-identity>
- Fivetran, modos de conexión: <https://fivetran.com/docs/connector-sdk/connection-options>;
  Postgres genérico: <https://fivetran.com/docs/connectors/databases/postgresql/setup-guide>
- Databricks, validación TLS: <https://docs.databricks.com/gcp/en/ingestion/lakeflow-connect/tls-server-certificate-validation>
- Supabase, Supavisor y el modo transacción: <https://supabase.com/docs/guides/troubleshooting/supavisor-faq-YyP5tI>;
  <https://supabase.com/docs/guides/self-hosting/accessing-postgres>
- Airbyte, Postgres (CDC y xmin): <https://docs.airbyte.com/integrations/sources/postgres>;
  <https://airbyte.com/blog/replicate-postgres-datasets-of-any-size-in-airbyte>;
  el slot que crece: <https://discuss.airbyte.io/t/postgres-to-snowflake-replication-slot-growing-in-size/7405>
- PostgreSQL, `reltuples = -1`: <https://postgresql.org/message-id/E1kCQ5k-0006BR-56%40gemulon.postgresql.org>;
  `pg_class`: <https://www.postgresql.org/docs/19/catalog-pg-class.html>
- PostgreSQL 17, slots de *failover*: <https://pgedge.com/blog/setting-up-failover-slots-in-postgresql-17>
- `sslmode` explicado: <https://ankane.org/postgres-sslmode-explained>; rust-postgres PR #988:
  <https://github.com/sfackler/rust-postgres/pull/988>
