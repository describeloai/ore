# 0075 · Origen Oracle Database

**Leer Oracle Database** —tablas y vistas, por SQL— como origen de ORE, con **`oracledb`**, el driver *thin* oficial de
Oracle: el protocolo nativo (TNS/TTC) hablado desde Rust, **sin Instant Client** ni ninguna librería de Oracle. Sirve
**Oracle 12.1 o posterior**; lo anterior (11g) y lo que el *thin* no habla (el cifrado nativo, Kerberos, las wallets) queda
fuera, y se dice.

**Estado:** X0 medido (2026-10-25) contra Oracle **26ai Free** en Docker, y el **11g XE** para el techo del protocolo (§3).
**La capa 1 está hecha** (el conector, su catálogo, el kit, el cableado y la hoja de la consola); el **CDC**, el **TLS contra un
listener TCPS real**, el e2e en la consola y el **segundo driver** (OCI, para 11g) **no** (§6 y §7). Este documento es la
**imagen de lo que es hoy el origen Oracle en ORE**; la decisión que lo respalda, corta, está en
[`0075`](../decisions/0075-origen-oracle.md). Rama `0075-oracle`, PR #30.

Sigue a [`0008`](../decisions/0008-el-protocolo-del-driver.md), [`0016`](../decisions/0016-el-testigo-y-el-rango.md),
[`0032`](../decisions/0032-el-contrato-de-tipos.md), [`0043`](../decisions/0043-ore-arrow-stream.md) y a los orígenes
[`0069`](0069-origin-mongodb.md), [`0070`](0070-origin-cassandra.md), [`0071`](0071-origin-phoenix.md) y
[`0072`](0072-origin-hbase.md): el patrón es el mismo —medir primero, construir después; el esqueleto `ore_driver::origen`;
el vocabulario cerrado de operadores; el kit como puerta—.

## 1. Qué se decide

- **v1: Oracle por su protocolo nativo, solo lectura.** Un binario **`ore-read-oracle`** sobre el esqueleto `Origen`, y un crate
  **`ore-oracle`** con todo lo que sabe de Oracle. Dirección `oracle://usuario:clave@host[:puerto]/SERVICIO` o, sin servicios,
  `oracle://usuario:clave@host[:puerto]/?sid=ORCL`.
- **El driver *thin* oficial (`oracledb`), no el cliente grueso.** Sin Instant Client: la imagen de los drivers es Alpine (musl)
  y el cliente de Oracle es una librería propietaria de glibc (§3.2, §3.9). El precio —11g, NNE, Kerberos— está en §7.
- **Catálogo real**, de `ALL_TAB_COLUMNS`, `ALL_CONSTRAINTS` y `ALL_COL_COMMENTS`; no se infiere nada (§4.4).
- **Los tipos son los de OOS**, con la regla de PostgreSQL: lo que OOS no modela es `Opaque`, y lo que no se sabe leer se
  cataloga **sin tipo** y no se lee (§4.3).
- **Los parámetros se atan tipados**, no como el texto que la sesión sepa leer (§4.5).
- **Una fila que no cabe en su tipo es un error con su columna y su fila**: ni un nulo ni un redondeo (0032 §4).
- **Cancelar con lo que el driver deja**: la bandera, la sesión matada desde otra, y `timeoutMs` (§4.7).
- **No hay incremental ni CDC todavía**, y **no se declaran** (`cambia: none`): Oracle tiene redo log y SCN, pero este conector
  no los lee (§5).

## 2. Lo que hacen los demás

Leído el 2026-10-25.

| | qué publica |
|---|---|
| **Fivetran** | Dos conectores de base de datos: **`Oracle`** (Binary Log Reader, *Teleport Sync* —consulta, para tablas de menos de 100 GB, sin LOBs— o XStream Out) y **`HVA Oracle`** (Direct Capture o Archive Log Only). Oracle 11g R2 a 21c. RAC y multitenant son tipos de instancia, no conectores; RDS para Oracle, solo el primario. Límites: 5 TB por conector, 200 GB de redo por hora, sin *standby* físico. **E-Business Suite** es un conector de aplicación aparte. Conexión directa, red privada, túnel SSH o agente proxy |
| **Airbyte** | **Uno**: `Oracle DB`. Lectura completa e incremental **por cursor**; **sin CDC** (ni LogMiner ni Flashback); no replica borrados. Oracle 11g a 21c. *Service name* o SID; cifrado nativo (AES256, RC4_56, 3DES168) o TLS; túnel SSH. Tipos: casi todo lo que no es numérico sale como cadena |
| **Foundry** | Un formulario estructurado: *Connection settings* (host —nombre o IPv4—, puerto 2484, *SID* o *Service name*, conmutador de TLS) y *Authentication* (usuario, contraseña). Sin TLS, «se intenta el cifrado nativo (NNE) si la base lo tiene» (§4.9) |

Lo que sale de ahí: **un solo conector «Oracle»**, que es el de la **base relacional**; el CDC por el redo log es un segundo
nivel que solo Fivetran tiene, y las aplicaciones sobre Oracle (E-Business Suite, Fusion, NetSuite), NoSQL o Essbase son
orígenes distintos. Este documento es el primer nivel.

## 3. Lo medido

Laboratorio: contenedores `gvenzl/oracle-free:23-slim-faststart` (26ai Free, 6,46 GB descomprimida; acepta conexiones a los
~35 s), `gvenzl/oracle-xe:11-slim` (11g XE, 892 MB) y, para el kit, el mismo 26ai (`pruebas-de-fuego/oracle/`). Sonda: un
crate suelto con los dos drivers y la tabla `ORE.TIPOS` (24 columnas, 3 filas).

### 3.1 El laboratorio, y lo que costó

- **Docker Desktop se cayó dos veces** durante descargas de imágenes (`unexpected EOF` en el `pull`; la tubería del motor
  desaparece). A la tercera fue estable. No es de Oracle, pero afecta a quien reproduzca esto en Windows.
- **El usuario de pruebas** (`APP_USER` de `gvenzl`) crea tablas, vistas y funciones, pero no mata sesiones ni lee `V$SESSION`:
  el kit necesita `GRANT ALTER SYSTEM` y `GRANT SELECT ON v_$session` para los casos 8 y 10 (§4.7); sin ellos salen «no aplica».
- **El caso 10 del kit (`SIGTERM`) no se puede medir en Windows**: `kill` no señaliza un proceso nativo. Se mide en un
  contenedor (`rust:1-bookworm`, `--add-host=host.docker.internal:host-gateway`).

### 3.2 La elección del driver

| | `oracle-rs` 0.1.7 (de un tercero) | `oracledb` 26.0.0-beta.4 (`oracle/rust-oracledb`) | `rust-oracle` (ODPI-C) |
|---|---|---|---|
| naturaleza | *thin*, asíncrono (Tokio) | *thin*, **síncrono** | grueso: envuelve el cliente de Oracle |
| los 24 tipos de la sonda | **corrompe** `BINARY_FLOAT/DOUBLE`, `NVARCHAR2`, los dos `INTERVAL` y `BOOLEAN` (bytes crudos leídos como texto); `XMLTYPE` falla | **los lee bien**, salvo `XMLTYPE` | no medido |
| versiones | 12.1+ (protocolo ≥ 315) | 19c, 21c y 26ai declarados | 11.2+ |
| dependencias de sistema | ninguna | ninguna | Instant Client (glibc, licencia de Oracle) |
| encaja con el esqueleto | no (asíncrono) | **sí** (el trait `Origen` es síncrono) | sí, con hilos |

**Se elige `oracledb`.** Es el repositorio oficial de Oracle (`oracle/rust-oracledb`, de los mantenedores de `python-oracledb`),
con licencia UPL/Apache. Es una **beta**: se fija exacta (`=26.0.0-beta.4`). `rust-oracle` no se descarta (es la salida para
11g, NNE y Kerberos), pero no entra aquí: el cliente es una librería propietaria de glibc y la imagen es Alpine. **`oracle-rs`
se descarta por correcto, no por lento.**

### 3.3 Las versiones, y el techo del *thin*

- **Oracle 11g XE** (protocolo 314): **los dos drivers lo rechazan**. `oracledb`: *«connections to this database server are not
  supported»*; `oracle-rs`: *«server protocol version 314 not supported (minimum: 315)»*. Con el mismo binario, la 26ai
  conecta. Es un techo del protocolo, no un fallo que se pueda parchear: ningún driver *thin* actual lo habla.
- **Lo que el apéndice oficial de `oracledb` dice que NO soporta**: el cifrado nativo de red (NNE: «usa TLS»), autenticación
  externa, Kerberos, RADIUS, por token y LDAP; las wallets SEPS; TAF, Application Continuity, Transaction Guard y FAN; SODA,
  *direct path*, scrollables, SOCKS. **Sí**: RAC, multitenant, CMAN, `tnsnames.ora`, un grupo homogéneo de conexiones, TLS y mTLS,
  conexión por proxy, DRCP.
- No se midieron 12c, 19c ni 21c. La 12.1 entra justo en el mínimo del protocolo (315); 19c, 21c y 26ai son las versiones que
  `oracledb` declara.

### 3.4 Tipos (`ORE.TIPOS`, 24 columnas, 3 filas: completa, casi toda nula, extremos)

| Oracle | lo que se mide con `oracledb` |
|---|---|
| `NUMBER` sin precisión | **exacto**, como texto (36 dígitos de `3,14159…` y `-1E-39` llegan enteros) |
| `NUMBER(18,0)`, `NUMBER(38,0)`, `NUMBER(12,2)`, `NUMBER(10,-2)`, `FLOAT` | exactos; `FLOAT(53)` con `1E100` llega como los 101 dígitos |
| `BINARY_FLOAT`, `BINARY_DOUBLE` | bien, incluido `Infinity` |
| `CHAR(5)` | **con su relleno** (`ab   `): es el valor |
| `NVARCHAR2`, `VARCHAR2` con `ñ €` | bien (UTF-8) |
| `DATE` | **lleva hora** (segundos); `0001-01-01` llega |
| `TIMESTAMP(6)`, `TIMESTAMP(9)` | los nanosegundos llegan al driver; Arrow los guarda en microsegundos |
| `TIMESTAMP WITH TIME ZONE` | los campos llegan **ya en UTC** con el desfase aparte: `13:14:15 +05:30` llega como `07:44:15` y `+5:30`. El `Display` del driver los mezcla y **engaña**; el instante es correcto |
| `INTERVAL YEAR TO MONTH`, `INTERVAL DAY TO SECOND` | `P12Y3M`, `P5DT4H3M2.000001000S` |
| `RAW`, `BLOB` | bytes; un `CLOB` pequeño llega **como texto** (el driver lo trae como `LONG`) |
| `JSON` | un árbol (`JsonValue`) cuyos objetos están en un `HashMap`: el orden de las claves **no se conserva** |
| `BOOLEAN` (23ai) | bien |
| `XMLTYPE` | **falla** (`unsupported database type DB_TYPE_OBJECT`) salvo envolviendo la columna en SQL (`XMLSERIALIZE`) |

Dos cosas de Oracle que no son del driver y mandan: **`''` es `NULL`** (una cadena vacía no existe; el kit lo mide como
`vacio_es_nulo`), y **los nombres son mayúsculas** salvo que se hayan creado entre comillas. La conversión `Arrow` propia de
`oracledb` (`query_arrow`) **no vale** para ORE: un `NUMBER` sin precisión lo degrada a `Float64` (pierde exactitud), los
`INTERVAL` no los mapea y los instantes con zona los deja sin ella.

### 3.5 Lectura masiva (`ORE.GRANDE`: 10⁷ filas, 6 columnas, 1,3 GB; creada dentro de Oracle en 45 s)

Medido con el driver solo, en release, contra el 26ai en Docker en la misma máquina (**cotas inferiores**: el cliente usaba el
61 % de un núcleo y el servidor el 46 %, así que el límite es la latencia de ida y vuelta de una lectura síncrona más la red de
Docker Desktop):

| `fetch_array_size` | solo leer | convirtiendo las 6 columnas a texto | memoria máx. |
|---|---|---|---|
| 1 000 | 37,6 s · 266 k filas/s | 55,8 s · 179 k | 8 MB |
| **5 000** | **33,1 s · 302 k** | **50,0 s · 200 k** | 14 MB |
| 20 000 | 36,3 s · 276 k | 54,2 s · 185 k | 29 MB |

Con 100 filas por viaje, 10⁶ filas dan 116 k filas/s: **5 000 es el óptimo**, y es el valor por defecto. La memoria no pasa de
29 MB para 10⁷ filas: **es streaming**. Con una tabla de 10⁵ filas (ocho columnas comunes, con `CLOB` y `BLOB` o sin ellos, y nulos), `fetch=1` tarda de 61 a 70 s y
`fetch=5000`, de 5 a 6,4 s (compilación de depuración): de 10 a 14 veces más.

**El conector completo** (catálogo, SQL, conversión, lote Arrow, IPC) da **139 k filas/s** en release (10⁷ filas a `/dev/null`
en 72 s). Con la red de Docker Desktop de por medio, la misma lectura da 76 k filas/s; con el conector dentro del espacio de red
del servidor, 127 k (§3.9). El kit mide 10⁶ filas en 16 s con 15 MiB de pico (glibc, depuración).

### 3.6 Cortes y tiempos (`ore-read-oracle`, medido con `fiabilidad`)

`oracledb` **no tiene una llamada para interrumpir una sentencia**: busqué `cancel`, `break` e `interrupt` en su código y solo
hay `set_call_timeout`.

| prueba | resultado |
|---|---|
| `call_timeout` de 2 s sobre `dbms_session.sleep(8)` | error a los **4,0 s** (el doble), **conexión cerrada**: `unable to recover from error: connection has been closed`; después, ni `ping` ni consultas |
| `ALTER SYSTEM KILL SESSION` desde otra conexión, a los 3 s de una lectura | la lectura falla tras 1,3 M filas con *«the database or network closed the connection»*; el *kill* tarda **0,08 s** |
| `docker pause` del servidor, con `call_timeout` de 5 s | error a los **10 s** de la última fila buena (el doble otra vez), conexión cerrada |

Dos consecuencias: **un timeout cierra la conexión** (la sesión de `servir` se tira y se abre otra), y **un timeout no
interrumpe la consulta en el servidor**: el driver cierra *su* lado y Oracle sigue ejecutando hasta el final. Sin un permiso
de matar la sesión, una consulta abandonada gasta la base de otro. De ahí §4.7.

### 3.7 Un fallo del driver, abierto

`oracledb` 26.0.0-beta.4 falla con *«unknown TTC message type N»* en un caso concreto: **3 filas, una casi toda `NULL`, leídas con
`fetch_array_size ≥ 3` y una proyección que mezcla un `CLOB`/`BLOB`/`JSON` con otras columnas.** El mínimo hallado por
reducción: `T_TSLTZ`, `I_YM`, `I_DS`, `R_RAW`, `L_CLOB`, `L_BLOB`, `J_JSON`, `B_BOOL` y el `ID`. Con `fetch=1` o `2` funciona.
**No se reproduce** con 15 columnas de tipos comunes, ni con 10⁵ filas con `CLOB`, `BLOB` y nulos en cualquier tamaño de lote:
hace falta esa forma (pocas filas, una casi nula). Repro en `pruebas-de-fuego/oracle/` (`tipos.sql` y `LEEME.md`);
**hay que reportarlo a `oracle/rust-oracledb`**. No se mitiga en el conector: una tabla realista no lo dispara y esconderlo con
`fetch=1` costaría de 10 a 14× en las que sí tienen LOB.

### 3.8 Cómo se nombra la base, y TLS

- **Servicio y SID.** El *thin* habla las dos: un descriptor con `(CONNECT_DATA=(SID=FREE))` conecta a la instancia (`FREE`,
  contenedor raíz); uno con `(SERVICE_NAME=FREEPDB1)` a la PDB; un SID que no existe da *«SID "NOEXISTE" is not registered with
  the listener … (Similar to ORA-12505)»*. Una PDB **no tiene SID**: el servicio es lo normal.
- **TLS.** `tcps://host:puerto/servicio` se analiza y se intenta el *handshake*: contra un listener sin TLS da *«unexpected end
  of file»*. **No se ha probado contra un listener TCPS real** (el laboratorio no lo tiene).
- La pila TLS de `oracledb` es **`rustls` con `aws-lc-rs`** y las **raíces de Mozilla incrustadas** (`webpki-roots`), no el
  almacén del sistema: una CA privada no se verifica (§4.9).

### 3.9 En la imagen de verdad (Alpine, musl)

Medido con `rust:1.90-alpine`, los `apk` del `Dockerfile` **sin añadir nada** y el perfil `imagen`:

- **Compila** en 5 min en frío (313 s), `aws-lc-sys` (C) incluido: el `cc`/`gcc`/`clang` que la imagen ya trae basta; no hace
  falta `cmake`, `perl` ni `go`.
- El binario pesa **6,5 MB**, es estático (solo aparece el cargador de musl en `ldd`) y arranca.
- **El kit pasa 12 de 12** con él, incluidos `servir` con hilos, cancelar y `timeoutMs`; el caso 7 da 10⁶ filas en 4,5 s con 6 MiB.
- **127 k filas/s** en musl, frente a 139 k en nativo (~9 % menos). El `malloc` de musl, que penaliza a HBase y a las teselas
  por sus hilos, aquí no pesa: el driver es síncrono y lee con uno solo. **No lleva `mimalloc`.**

## 4. Cómo está hecho

Tres capas, como los demás orígenes, y dos piezas compartidas que salieron de esto:

```
 oracle://u:p@host:2484/SERVICIO?tls=true            ┌─ ore-oracle ───────────────────────────────────────────┐
        │                                            │ url.rs        la dirección y lo que no admite          │
        ▼                                            │ transporte.rs conexión, sesión, catálogo, celdas, ORA→ │
 ore-read-oracle  (main de una línea)  ────────────▶ │ tipos.rs      ALL_TAB_COLUMNS → tipo de OOS            │
   ore_driver::origen::principal::<Oracle>           │ atado.rs      parámetros tipados                       │
   capacidades · leer · estimar · servir             │ lectura.rs    `Origen`: leer a Arrow, cancelar         │
   catalogo · check · explorar · testigo · cambio    │ lote.rs       texto canónico → Valor → lote            │
                                                     │ verbos.rs     catálogo, check, explorar, testigo…      │
 ore_sql  (dialecto ORACLE)  ◀─ la forma de SELECT ──┴────────────────────────────────────────────────────────┘
 ore_arrow::RecordBatchBuilder  ◀─ el lote de Arrow, el mismo que arman Phoenix, Cassandra, HBase y Mongo
 ore_driver::{base64, url}      ◀─ lo que los orígenes escribían cada uno por su cuenta
```

`ore-oracle` son 2 512 líneas con 24 pruebas; `ore-read-oracle`, 8; el banco del kit, 298; `ore-arrow`, 387 con 5.

### 4.1 La dirección

`oracle://usuario:clave@host[:puerto]/SERVICIO?opciones`, o `oracle://usuario:clave@host[:puerto]/?sid=ORCL&opciones`. La
credencial va **dentro** (al custodio), con la contraseña codificada. Nombre de servicio **o** SID, nunca los dos.

| opción | qué hace |
|---|---|
| `sid=` | el SID en vez de la ruta; compone el descriptor `(DESCRIPTION=(ADDRESS=(PROTOCOL=TCP\|TCPS)(HOST=…)(PORT=…))(CONNECT_DATA=(SID=…)))`. Solo letras, cifras y `_ $ # . -`: **con un paréntesis se podría cerrar el nodo y añadir otro** |
| `tls=true` | `tcps://`. El certificado se verifica siempre. Por defecto, no |
| `esquema=` | el dueño de las tablas al que se limita el catálogo; sin él, todos los que el usuario ve menos los que mantiene Oracle |
| `timeoutMs=` | el tiempo de cada llamada (`call_timeout`) |
| `fetch=` | filas por viaje (1 a 100 000; **5 000** por defecto) |

**Se rechazan** (con su porqué): `wallet`, `tlsCAFile` y `sslRootCert` (son ficheros y el conector corre en un Job sin tus
ficheros), `tlsInsecure`, `tlsAllowInvalid…` y `verify=false` (apagarían la verificación). La contraseña no sale en un
`{:?}` ni en `publica()`.

### 4.2 La sesión

Al abrir se fijan **la zona y los formatos**, porque Oracle convierte el texto de un parámetro según la sesión (`NLS_*`) y un
cursor por fecha que dependiera del `NLS` del usuario daría otras filas sin fallar:

`TIME_ZONE = 'UTC'` (un `TIMESTAMP WITH LOCAL TIME ZONE` llega en la zona de la sesión, sin decir cuál),
`NLS_NUMERIC_CHARACTERS = '.,'`, `NLS_DATE_FORMAT`, `NLS_TIMESTAMP_FORMAT` y `NLS_TIMESTAMP_TZ_FORMAT` en la forma canónica.
Después, el `call_timeout` si la URL lo pide. Oracle cifra el *inicio de sesión* (O5LOGON) aunque no haya TLS; los datos, no.

### 4.3 Los tipos

El tipo sale del catálogo (`DATA_TYPE`, `DATA_PRECISION`, `DATA_SCALE`), no de los metadatos de un resultado, y de él, el
tipo de OOS y su físico en Arrow (0032 §1):

| Oracle | OOS | físico (Arrow) |
|---|---|---|
| `NUMBER(p, 0)`, `p ≤ 18` | `Integer` | `int64` |
| `NUMBER(p, s)`, `NUMBER(p, 0)` con `p > 18`, escala negativa, `NUMBER(*, 0)` | `Decimal<p, s>` (≤ 38 dígitos) | `decimal128(p, s)` |
| `NUMBER` sin precisión, o de más de 38 dígitos | `String` (texto **exacto**) | `utf8` |
| `FLOAT`, `BINARY_FLOAT`, `BINARY_DOUBLE` | `Float` | `float64` |
| `CHAR`, `VARCHAR2`, `NVARCHAR2`, `CLOB`, `NCLOB`, `LONG`, `ROWID` | `String` | `utf8` |
| `DATE` (lleva hora), `TIMESTAMP` | `DateTime` | `timestamp[us]` |
| `TIMESTAMP WITH [LOCAL] TIME ZONE` | `DateTimeTz` | `timestamp[us, UTC]` |
| `BOOLEAN` | `Boolean` | `boolean` |
| `RAW`, `LONG RAW`, `BLOB` | `Opaque` (base64) | `utf8` |
| `INTERVAL YEAR TO MONTH`, `INTERVAL DAY TO SECOND` | `Opaque` (ISO 8601) | `utf8` |
| `JSON` | `Opaque` (su texto, claves ordenadas) | `utf8` |
| `XMLTYPE`, `VECTOR`, `BFILE`, objetos | **sin tipo**: se catalogan y no se leen | — |

Ajustes de `NUMBER` que salen del catálogo: la escala **mayor que la precisión** (`NUMBER(3,5)`: `0,00ddd`) pide una precisión de
Arrow al menos igual a la escala (`Decimal<5, 5>`); la escala **negativa** (`NUMBER(10,-2)`, redondea a centenas) es un entero de
`p - s` dígitos (`Decimal<12, 0>`).

**Dos declaraciones** que la tabla puede hacer y que el dato cumple **valor a valor** (un valor que no cabe es un error con su
columna y fila): `Date` sobre un `DATE` o `TIMESTAMP` (falla si trae hora: no se tira a escondidas) e `Integer` sobre un
`NUMBER(19, 0)`, `NUMBER(38, 0)` o `NUMBER` libre (falla si tiene decimales o desborda un `i64`). Y `Decimal<p, s>` sobre un
`NUMBER` libre.

**Los tipos que faltan** —`Binary`, `Map`, `Interval`, un semiestructurado (`Variant`)— se vieron aquí y **no se resuelven aquí**:
son de la especificación OOS, no de un origen, y Oracle, Mongo y Cassandra los piden a la vez. Hasta entonces `Opaque` con el
`physicalType` guardado, que es lo que PostgreSQL hace con `bytea`, `interval` y `json`.

### 4.4 El catálogo

Tres consultas, todas contra las vistas `ALL_*` (lo que el usuario ve, no lo que existe):

- **Objetos**: `ALL_TABLES` (sin temporales, anidadas, papelera `BIN$…` ni la parte de desbordamiento de una IOT) y `ALL_VIEWS`,
  menos los esquemas con `ORACLE_MAINTAINED = 'Y'` (12.1+, que es el mínimo del *thin*), o solo `esquema=`. `NUM_ROWS` va como
  `filas_estimadas` (una estimación de las estadísticas, que puede no haberse calculado).
- **Columnas**: `ALL_TAB_COLUMNS` con `ALL_COL_COMMENTS`, en `COLUMN_ID`. (`ALL_TAB_COLUMNS` ya excluye las ocultas.)
- **Claves**: `ALL_CONSTRAINTS` + `ALL_CONS_COLUMNS` habilitadas, `P` (primaria) y `U` (únicas), en el orden de la restricción.

Cada columna lleva su **tipo de Oracle tal cual** (`NUMBER(12,2)`, `TIMESTAMP(6) WITH TIME ZONE`) como `sourceType`, y los
**comentarios** del origen como descripción. Una tabla que no se deja catalogar **no tumba el catálogo**: se salta con su aviso
y su motivo; solo falla si no se pudo con ninguna. Los **avisos** dicen lo que el catálogo no puede llevar: un `NUMBER` sin
precisión, un `DATE` con hora, los nanosegundos que Arrow pierde, los `Opaque` y lo que no se lee.

La cara de lectura (`reads`) declara `fullScan: cheap` y lo empujable (`eq`, `neq`, `in`, `range`, `isNull`); la de cambios,
`none` (§5). **No hay `estimar`**: no se declara.

### 4.5 La traducción, y el atado

La forma de la consulta es la de `ore-sql`, con un **dialecto `ORACLE`** que se escribió como dato (cita por partes con `"`, marca
`:1`, recorte por tupla). Es el **cuarto** dialecto, y el que **sí trajo ejes nuevos** a una forma que se había extraído de
tres con la apuesta *«si aparece un quinto eje, la forma estaba extraída de dos casos y no de la clase»*: Oracle **no tiene
`LIMIT`** (es `FETCH FIRST n ROWS ONLY`, desde 12c) **ni `FALSE`** (el booleano llega en 23ai; `1 = 0` vale en todas). Entran
como datos (`Limite`, `falso`) y la forma aguanta. Además, `Consulta` devuelve ahora **la columna de cada parámetro**.

Los **diez operadores** se empujan al servidor (`like` incluido) y el orden lleva `NULLS LAST` en los dos sentidos.

**Los parámetros se atan tipados.** `ore-sql` entrega los valores como texto canónico de OOS (`2026-10-03T23:59:59`, `…Z`) y
Oracle convierte un parámetro de texto según el `NLS` de la sesión, y ese formato no es el suyo (`ORA-01861`); fijar un `NLS`
que lo admita no basta (la `Z` no existe en ninguna máscara de zona). Con la columna de cada parámetro y su tipo del
catálogo, **una fecha se ata como `OracleTimestamp`, un instante como `OracleTimestamp` con zona y un lógico como `bool`**; el
resto, como texto, que Oracle coacciona sin ambigüedad. Antes de esto el kit fallaba 4 de las 30 pruebas de operadores
(`eq`, `ge` y `lt` sobre fechas).

### 4.6 `leer`

Un `leer` es: la tabla del catálogo de la sesión (se guarda; si la petición pide una columna que no tiene, se vuelve a leer una
vez) → los tipos declarados por la petición mandan si el dato los lleva → la consulta de `ore-sql` → la cabecera Arrow **después**
de preparar la sentencia (lo que falla ahí sale como un error limpio y no como un flujo a medias) → un lote cada **8 192 filas**
con `fetch_array_size = prefetch_rows =` el de la URL → cada celda a **texto canónico** → `Fisico::analizar` → `Valor` →
`RecordBatchBuilder`. Una fila entra entera o no entra.

Dos propiedades de la conversión: pasar la celda a texto y de ahí al valor es **parte** de la distancia entre los 200 k filas/s
del driver (ya convirtiendo a texto) y los 139 k del conector; el resto es construir el lote y escribir el IPC. **No se ha
perfilado**: una conversión directa de `OracleNumber` a `i64`/`i128` es la mejora evidente, y no se ha hecho. Y **es el mismo
`analizar` que usa el lago**, así que el conector no puede estar en desacuerdo con él sobre qué cabe.

### 4.7 Cancelar y el tiempo

En tres escalones, porque el driver no deja otra cosa (§3.6):

1. **La bandera**, que se mira en cada fila: mientras las filas fluyen, corta en milisegundos.
2. **`ALTER SYSTEM KILL SESSION '<sid>,<serial#>' IMMEDIATE`** desde otra conexión, **si el usuario puede** (la identidad sale de
   `V$SESSION` por el `AUDSID`, que también hay que poder ver). Muere en 0,08 s y corta también una sentencia que aún no ha
   devuelto la primera fila.
3. **`timeoutMs`** de la petición, que se clasifica como `tiempo` (el driver dice «no puedo seguir» y ya pasó el 90 % del límite)
   y, al vencer, **mata la sesión propia** si puede. Es lo que evita que una consulta abandonada siga gastando la base.

**Sin el permiso `ALTER SYSTEM`** (lo normal en un cliente) la lectura se corta igual, pero la sentencia termina sola en el
servidor. Está dicho en la consola de credenciales. El kit lo mide: el caso 8 da `tiempo` a los 2,2–2,7 s con *nada vivo en el
origen*, y el 10, que la consulta deja el origen en 1–2 ms.

### 4.8 Los verbos y los errores

| verbo | |
|---|---|
| `capacidades` | los diez operadores, `limit` sí, `orderBy` sí, `estimar` no, `servir` sí |
| `leer`, `servir` | del esqueleto (`servir` guarda una sesión por credencial y la tira a los 60 s ociosa o si se pierde la conexión) |
| `catalogo`, `explorar` | §4.4; `explorar` lista los esquemas con su número de objetos |
| `check` | abre, pregunta la versión (`PRODUCT_COMPONENT_VERSION`), lista lo visible y **lee una fila de una tabla** (el permiso de lectura) |
| `testigo` | `none`: es una respuesta cierta (0016), no «ahora» |
| `cambio` | `sin-contadores`, `respalda: false`: la copia no se fía |

Los errores de Oracle salen **literales** (`ORA-xxxxx: …`) y se clasifican: `1017`, `28000`, `28001` y `28009` →
`credencial`; `942` y `4043` → `objeto`; `1013` y `3156` → `tiempo` (reintentable); `3113`, `3114`, `3135`, `12170`, `12537`,
`12541`, `12543` y `12545` → `conexion` (reintentable); un *stream*/TLS/listener/SID/servicio mal → `conexion`; lo demás, `origen`.
El *thin* contra una base anterior a 12.1 da `conexion` con *«este servidor es anterior a Oracle 12.1: el driver thin no lo habla
(haría falta el cliente OCI)»*. Un valor que no cabe es `esquema`.

### 4.9 Seguridad

- **La credencial va al custodio**, dentro de la dirección, cifrada con la llave de la organización; el manifiesto solo declara
  de qué variable sale. No se enseña, y no sale en ninguna salida (el caso 12 del kit: 44 salidas sin una URL ni una credencial).
- **TLS se verifica siempre**, contra las CA públicas y contra el nombre del host. Una CA **privada** (por fichero) no se
  admite todavía: un certificado interno probablemente no conecte.
- **Sin TLS los datos viajan en claro. No hay NNE de reserva**: a diferencia de Foundry («se intenta el cifrado nativo si la
  base lo tiene»), el *thin* no lo habla, y si la base lo exige la conexión no se abre. La hoja de la consola lo dice así.
- **Una pila TLS aparte**: `rustls` + `aws-lc` + raíces de Mozilla, distinta del OpenSSL del sistema de los demás drivers. No entra
  en el cierre de `ore` (`dependencias.rs`: 12 de 12 sin tocar un tope).
- **El usuario solo lee.** Un usuario con escritura, o con `SELECT ANY TABLE`, hace más de lo que el conector necesita.
- **La dirección ha de ser pública**: la `NetworkPolicy` de los drivers excluye 10/8, 172.16/12, 192.168/16 y 169.254/16, y el
  puerto cae dentro del 1024–65535 que ya permite.

## 5. Incremental y CDC

**No hay incremental ni CDC todavía**, y el catálogo **no lo declara**: `cambia: { mode: none, witness: none }`. Oracle tiene redo
log y SCN, y por tanto una base para `witness: log`, pero este conector no los lee y no se declara lo que no se hace.

| camino | qué sería | estado |
|---|---|---|
| Incremental por cursor (`witness: field`) | la petición ya admite `cursor`, `start` y `end` y el traductor los pone como dos filtros más; falta que el catálogo declare **qué columna** es cursor y un `testigo` que la lea | **no declarado**: el rango funciona en `leer` (probado con un cursor `TIMESTAMP`), no está en el catálogo |
| CDC por **LogMiner** | son consultas SQL sobre `V$LOGMNR_CONTENTS`, así que el *thin* las puede hacer; pide permisos de DBA, *supplemental logging* y un diccionario | **sin construir** |
| **Flashback** / `ORA_ROWSCN` | `ORA_ROWSCN` es por bloque, no una huella de tabla; Flashback Query lee el pasado, no los cambios | descartados como testigo |

Tampoco hay `estimar` (no se declara).

## 6. La hoja de ruta

| hito | qué entrega |
|---|---|
| **X0 · medir** | **Hecho (2026-10-25):** la elección del driver (§3.2), las versiones (§3.3), los tipos (§3.4), el volumen (§3.5), los cortes (§3.6), el SID y el TLS (§3.8), musl (§3.9) |
| **C1 · el conector** | **Hecho:** `ore-oracle` y `ore-read-oracle` (§4); el kit **12 de 12** en Linux, la pasarela 8 de 8 |
| **C1b · el cableado** | **Hecho:** `compilar-binarios.sh`, la imagen de los drivers, el quinto grupo del job `conectores` (`gvenzl/oracle-free`), el formulario de credenciales de `ore-serve`, la detección de una URL con credencial, la prueba de huellas |
| **C1c · Alpine/musl** | **Hecho** (§3.9) |
| **C1d · dos extracciones** | **Hechas:** `ore_driver::{base64, url}` (−428 líneas en cinco drivers) y `ore-arrow` (el lote que armaban cinco orígenes; una fila entra entera) |
| **C1e · servicio o SID** | **Hecho** (§3.8, §4.1) |
| **C1f · la hoja de la consola** | **Hecha, sin ver renderizada** (§8) |
| **C2 · ejecutarlo en el CI** | el grupo `oracle` **nunca ha corrido** en el CI real; los kits de Phoenix, HBase, Cassandra y Mongo **no se corrieron** tras las extracciones (sin sus laboratorios) |
| **C3 · TLS** | un Oracle con listener TCPS (la imagen de la comunidad no lo trae) para medir `tls=true`, el certificado y el nombre |
| **C4 · el e2e en la consola** | alta, descubrimiento, base *foreign* y *standard* con 2–3 escenarios de cliente |
| **C5 · tipos** | `XMLTYPE` (envolver la columna en el `SELECT`), `VECTOR`, y lo que la especificación OOS decida de `Binary`, `Map`, `Interval`, `Variant` |
| **C6 · rendimiento** | la conversión directa de `OracleNumber` y `OracleTimestamp` a `Valor` (sin pasar por texto) |
| **C7 · CDC** | LogMiner (§5) |
| **C8 · el segundo driver** | uno sobre OCI (ODPI-C), para 11g, NNE y Kerberos, **si hay quien lo pida** |

## 7. Lo que se aceptó a cambio

- **Una beta como driver**, fijada exacta, con un fallo abierto (§3.7).
- **Sin 11g ni NNE ni Kerberos ni wallets.** Un techo del protocolo, no de este crate; con una parte del parque sin actualizar
  (la estimación de la que se partió pone en torno a un 20–30 % de las instancias en 11g; no está verificada), la salida es un segundo driver, no un parche.
- **Una pila TLS aparte** (`rustls` + `aws-lc`), con raíces de Mozilla incrustadas y **sin probar contra TCPS**.
- **Una conversión por celda** (texto canónico y de ahí a Arrow) en vez de una directa: 139 k frente a 200–300 k filas/s.
- **Un timeout no interrumpe la consulta en el servidor** salvo que el usuario pueda matar su sesión.
- **`cambia: none`**: lo que no se lee no se declara.
- **El orden de las claves de un `JSON`** no se conserva (el driver lo guarda en un `HashMap`); salen ordenadas.
- **Un `NUMBER` sin precisión es texto** hasta que la tabla lo declare `Decimal<p, s>`.

**Sin medir:** 12c, 19c y 21c reales; RAC; una base multitenant leída por la raíz (solo se probó el SID del contenedor raíz); LOBs
grandes (se leyeron de unas decenas de bytes); `XMLTYPE`; `VECTOR`; tablas particionadas; lectura en paralelo de una tabla
(el origen lee con una sola conexión); muchas sesiones calientes en la pasarela; Autonomous Database; una latencia de red real
(todo se midió en la misma máquina).

## 8. La hoja de conexión de la consola

`rubix-platform/components/ingestion/wizard/OracleConnection.tsx` (más `lib/ingestion/oracle.ts` con la lógica pura y
`scripts/oracle.mjs` con sus 55 casos). Sigue el modelo de **Foundry** —un formulario estructurado de dos tarjetas— en lo que
vale y lo cambia en lo que no:

| de Foundry | en ORE |
|---|---|
| puerto, usuario, contraseña (obligatoria) | igual |
| *SID* \| *Service name* | igual; el conector lo admite (§3.8) |
| el conmutador de TLS, activado | igual |
| *Hostname* \| *IPv4* (dos botones) | **un solo campo**, con el mensaje de Foundry si lo que parece una IPv4 no lo es; y **la dirección ha de ser pública** (`localhost`, 127.x, 10.x, 172.16–31.x, 192.168.x, 169.254.x, `::1`, `fd…`, `fe80…` se rechazan) |
| puerto por defecto 2484 | **sigue al TLS**: 2484 con TLS, 1521 sin él, y siempre explícito en la URL |
| «sin TLS se intenta el cifrado nativo (NNE)» | **no hay NNE**: sin TLS los datos van en claro y, si la base lo exige, no conecta |
| (no lo enseña) | `Schema`, `Query timeout` y `Fetch size`, plegados en *Advanced* |

Lo que compone es exactamente la URL de §4.1; las seis direcciones que genera (servicio, SID, con y sin TLS, todas las opciones,
IPv6; una contraseña con `@ : / #`) las acepta el analizador de Rust. El panel de abajo lo rellena
`GET /fuentes/credenciales/oracle` con los pasos (el usuario que solo lee, el nombre de servicio o el SID, la red) y lo que **no**
se admite (Oracle 11g, el cifrado nativo y Kerberos, el TLS inseguro, una wallet por fichero, un usuario con escritura).
**No se ha visto renderizada** (no había navegador) y el repositorio tiene cambios ajenos sin confirmar en los mismos ficheros
(`ConnectionStep.tsx`, `registry.ts`).

## 9. Cómo se prueba

```
# El laboratorio (una imagen de la comunidad; ~35 s) y los permisos que piden los casos 8 y 10
docker run -d --name ora23 -p 1521:1521 -e ORACLE_PASSWORD=sonda -e APP_USER=ore -e APP_USER_PASSWORD=ore \
  gvenzl/oracle-free:23-slim-faststart
printf 'ALTER SESSION SET CONTAINER = FREEPDB1;\nGRANT ALTER SYSTEM TO ore;\nGRANT SELECT ON v_$session TO ore;\nEXIT\n' \
  | docker exec -i ora23 sqlplus -s / as sysdba

# El kit: la puerta única (el CI lo hace en el grupo `oracle` del job `conectores`)
cargo build -p ore-conector-kit -p ore-read-oracle -p ore-federation --features ore-conector-kit/oracle
ORACLE_KIT_URL=oracle://ore:ore@localhost:1521/FREEPDB1 \
  target/debug/ore-kit --conector target/debug/ore-read-oracle --banco oracle                       # el conector
ORACLE_KIT_URL=… target/debug/ore-kit --conector … --banco oracle --pasarela target/debug/ore-federation --exige todos
```

El banco (`ore-conector-kit/src/bancos/oracle.rs`) crea `KIT_TIPOS`, `KIT_VACIA`, `KIT_GRANDE` (10⁶ filas, **dentro de Oracle**, unos
segundos) y la vista `KIT_LENTA` (una función `KIT_DORMIR` que duerme 10 s por fila) en el esquema del propio usuario. La semilla
guarda el `Boolean` en un `BOOLEAN` (23ai), el `Integer` en un `NUMBER(19, 0)` (el borde de un `i64` tiene 19 dígitos) y la cadena
vacía como nulo. Los casos 9 y 14 no aplican (no hay SQL que escriba; `estimar` no se declara). **En Windows el 10 no se puede
medir** (§3.1).

Resultados: en Linux, **12 de 12** casos aplicables y la pasarela, 8 de 8; con el binario musl, 12 de 12; en Windows, 11 de 12 (el 10,
por `kill`). Suites de unidad: `ore-oracle` 24, `ore-arrow` 5, `ore-driver` 35, `ore-sql` 16, y las de Phoenix (72), Cassandra (76),
HBase (94) y Mongo (51), que no han cambiado de recuento al migrar a las piezas compartidas.

## 10. Fuera de este producto

- **Las aplicaciones sobre Oracle** (E-Business Suite, Fusion, NetSuite), **Oracle NoSQL** y **Essbase**: orígenes distintos.
- **Escribir** en Oracle: una fuente lee.
- **El CDC** (LogMiner) y el segundo driver (OCI): capas posteriores (§5, §6).
- **Los tipos nuevos de OOS** (`Binary`, `Map`, `Interval`, `Variant`): un cambio de la especificación, no de un origen.

## Fuentes

- Fivetran, *Oracle* y *HVA Oracle*: <https://fivetran.com/docs/connectors/databases/oracle> (leído el 2026-10-25).
- Airbyte, *Oracle DB*: <https://docs.airbyte.com/integrations/sources/oracle>.
- `oracledb` (Oracle): <https://docs.rs/oracledb> · <https://github.com/oracle/rust-oracledb> (la guía y su apéndice A).
- `oracle-rs`: <https://docs.rs/oracle-rs>. `rust-oracle` y ODPI-C: <https://odpi-c.readthedocs.io/en/stable/user_guide/installation.html>.
- Mediciones y repro: `pruebas-de-fuego/oracle/` · código: `crates/ore-oracle`, `crates/ore-read-oracle`, `crates/ore-arrow`,
  `crates/ore-conector-kit/src/bancos/oracle.rs`.
