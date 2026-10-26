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

## 11. Arrow en ORE: el mapa completo, y qué hacer con `ore-arrow`

Esta sección nace de una pregunta —*«¿ese Arrow no es `ore-arrow`?»*— y de una comprobación que **no hice antes de poner el
nombre**: miré que no existiera un crate llamado así, no cómo se usa Arrow en el resto de la plataforma. Es lo que sigue.

### 11.1 Qué es Arrow en ORE: tres papeles

| papel | qué es | dónde está dicho |
|---|---|---|
| **El formato del camino origen → lago** | el driver emite un flujo **Arrow IPC**; `ore` lo encauza sin leerlo; `ore-store sellar-flujo` lo ajusta al contrato y lo escribe lote a lote. Nada tiene la tabla entera en memoria | ADR [`0043`](../decisions/0043-ore-arrow-stream.md) *ORE Arrow Stream* (2 M de filas: 180 s → 17,9 s; `ore-store` 2 017 MB → 90 MB) |
| **La verdad común en el puesto** | `over()`, `sql()` y `arrow()` devuelven tablas Arrow en Python (`pyarrow`, pandas con `ArrowDtype`), Node (`@duckdb/node-api`) y JVM (`ArrowReader`); `write()` convierte lo que le den a Arrow y lo manda por IPC al agente, que lo escribe con `ore-store escribir` | ADR [`0031`](../decisions/0031-el-puesto.md) §4 y §10, [`0032`](../decisions/0032-el-contrato-de-tipos.md), `docs/sdk.md` |
| **El físico del lago** | cada tipo de OOS tiene un tipo de Arrow y de Parquet/Iceberg; la copia **es** una tabla Iceberg cuyo esquema sale del contrato | ADR `0032` §1, `ore-store/src/carga.rs` |

Los tres comparten **un contrato**: el de tipos del ADR 0032 (un valor tiene «un tipo en cuatro sitios» —el origen, el árbol, la copia y el lenguaje de la celda— y el contrato es «la tabla que los une»). En código, ese contrato son tres tipos de
`ore-base` que **no dependen de Arrow**: `Fisico` (los 9 físicos escalares), `Forma` (un escalar, o un struct, una lista, un
vector o una geometría) y `Valor` (un valor ya analizado, en la unidad en que Arrow lo guarda).

### 11.2 El río, de punta a punta

```
 origen (Oracle, Postgres, Cassandra…)
   │  celda nativa ──[cada origen]──▶ texto canónico ──[Fisico::analizar]──▶ Valor
   ▼
 ┌─ ore-arrow ────────────────────────────────┐
 │ RecordBatchBuilder: Valor → builders       │   ← lo que escribí yo (los orígenes, hoy)
 └────────────────────────────────────────────┘
   │  lote Arrow ──StreamWriter──▶ flujo IPC por stdout            (ADR 0043)
   ▼
 ore  (lo encauza, no lo lee)
   ▼
 ore-store sellar-flujo ── carga::al_contrato ──▶ Parquet lote a lote ──▶ tabla Iceberg   ← el lago
   ▼
 el lago ──iceberg_scan (DuckDB)──▶ over() / sql() / arrow()  ──▶ Python · Node · JVM       ← el puesto
   ▲
 write(): el SDK convierte a Arrow ──IPC──▶ agente ──▶ ore-store escribir ──▶ commit Iceberg
```

### 11.3 Dónde se materializa el contrato en Arrow, hoy

| pieza | qué hace con Arrow | notas |
|---|---|---|
| `ore-base::tipos` | define `Fisico`, `Forma`, `Valor` y `Fisico::analizar` (texto canónico → `Valor`) | **sin Arrow**: pura, está en el cierre de `ore` |
| **`ore-arrow`** (mía) | `Fisico` → tipo de Arrow y *builders*; `Valor` → `RecordBatch`; **una fila entra entera o no entra** | lo usan Phoenix, Cassandra, HBase, Mongo y Oracle. **Solo `Fisico`**: no conoce `Forma` |
| **`ore-store::carga`** | **el contrato del lago** (≈1 500 líneas): `arrow_del_oos`, `arrow_de_forma`, `campo`, `oos_de_arrow` (OOS ↔ Arrow, **con** struct, lista, vector y geometría); `lote` (filas en texto → lote tipado, con `sin_estrechar`); `al_contrato`, `conformar`, `al_esquema`, `ensanchar`, `normalizar` (un lote Arrow que llega → el contrato); `huella` (el *digest*, determinista); `leer`/`lotes_de_parquet`; la geometría como `Binary` con la extensión `geoarrow.wkb` | es el **otro** sitio donde `Fisico` se vuelve Arrow, y es más grande que el mío |
| `ore-read-postgres` | su propio `Col` con `Geo` (`BinaryBuilder`) y `Lista`; **el mismo diseño que Oracle**: el valor llega en texto canónico y `ore_core::tipos` lo analiza, *«el mismo analizador con el que el almacén estrecha»* | emite Arrow (el ADR 0043, revisado el 2026-10-02, aún dice que Postgres sigue en texto: **está desfasado**) |
| `ore-read-bigquery` | `v2.rs`: la **Storage Read API** (Arrow nativo, 8 streams) cuando puede, y REST con su propio constructor cuando no; **declina** el camino Arrow para `BYTES`, `JSON`, `STRUCT`, `ARRAY`, `BIGNUMERIC` y `GEOGRAPHY` | los tipos que faltan en OOS son justo los que cierran este camino rápido |
| `ore-read-objetos` | `filas.rs`: CSV/JSONL/Parquet de un bucket a Arrow, con `_rescued_data` | otro constructor |
| `ore-conector-kit` | los bancos (S3…) cargan sus semillas con builders propios | de pruebas |
| los SDK, el agente, `ore-store escribir` | consumen y producen Arrow; `ore-teselas`, `ore-medios` y `ore-federation` también lo tocan | no se han mirado en detalle |

**Todos los lectores emiten ya Arrow IPC** (Postgres, BigQuery, objetos, Cassandra, HBase, Mongo, Phoenix y Oracle): `StreamWriter::try_new`
aparece en cada uno, y en `ore-federation` y `ore-store`.

### 11.4 Lo que se repite y lo que no

**Se repite (el mismo trabajo en sitios distintos):**
1. **`Fisico` → `DataType` de Arrow** y su *builder*: `ore-arrow`, `ore-store::carga` (`arrow_del_oos`, `lote`), `ore-read-postgres`,
   `ore-read-bigquery` (v2), `ore-read-objetos` y los bancos del kit.
2. **El escritor del flujo IPC** (`StreamWriter::try_new` + escribir lotes + `finish`): ocho lectores más `ore-federation`. **Es el
   siguiente duplicado**, y todavía nadie lo ha extraído.

**No se repite (es distinto, y no se puede unir sin cambiar el comportamiento):**
- `carga::lote` parte de **texto** y, si un valor no analiza, **la columna entera queda como `string`** y el informe lo dice
  (`sin_estrechar`); el constructor de los orígenes, en cambio, da **un error con la columna y la fila**. Son dos reglas
  deliberadas (el lago acepta una copia «honesta» con texto; un lector que declaró un tipo y recibe otro, no).
- `carga::al_contrato` y `conformar` **validan y convierten un lote que ya es Arrow** (un decimal que estrecha se convierte y se
  vuelve a convertir: *«si no sale idéntico, se niega»*). Eso no es construir: es cumplir el contrato.
- `Forma` (struct, lista, vector, geometría) solo la entiende el lago; `ore-arrow` solo conoce los escalares.

### 11.5 Por qué el nombre `ore-arrow` hoy es engañoso

Un lector que vea `ore-arrow` pensará que es *el* Arrow de la plataforma: el del puesto y el del lago incluidos. **No lo es**: cubre
la **parte de los orígenes** de un río que tiene tres papeles. Lo escribí como si el nombre estuviera libre porque no había
ningún crate con él; la pregunta correcta era si **otra cosa ya se llamaba así en el lenguaje de la plataforma**, y sí:
*ORE Arrow Stream* (ADR 0043) y «Arrow como verdad común» (ADR 0031/0032).

### 11.6 Las opciones

| | qué es | a favor | en contra |
|---|---|---|---|
| **A · ampliar el alcance** | `ore-arrow` pasa a ser **el único sitio que sabe cómo se materializa un tipo de OOS en Arrow**: acepta `Forma` (no solo `Fisico`), y lo usan los orígenes, `ore-store::carga` y el lector de Postgres | el nombre es exacto; desaparece el duplicado más grande; es lo que ya dice el ADR 0031 §10 del escritor (*«0032 se implementa una vez»*); y es el sitio natural de los tipos nuevos (§12) | toca la **escritura al lago**, que es el camino más crítico (el *digest* debe seguir siendo idéntico); es un PR grande |
| **B · estrechar el nombre** | `ore-driver-arrow` (o similar): «el lado del origen» | no toca nada ajeno; honesto con lo que hace hoy | consagra la duplicación con `carga.rs` y el lector de Postgres; un nombre más largo para algo que no cambia |
| **C · dejarlo y documentar** | `ore-arrow`, con el alcance dicho en la cabecera del crate | cero riesgo | el nombre sigue prometiendo más de lo que da; la duplicación sigue |

### 11.7 Recomendación: A, por fases, y no antes de cerrar el PR #30

**Mantener el nombre `ore-arrow` y ampliar el alcance**, no renombrarlo otra vez. Un segundo cambio de nombre en dos días, sobre
un crate ya publicado en un PR, cuesta más que cumplirlo. Y la duplicación (`Fisico` → Arrow en tres sitios) es una deuda real
que el nombre, si se amplía, deja de ocultar.

| fase | qué | cómo se sabe que está bien |
|---|---|---|
| **0 · decir el alcance** | la cabecera de `ore-arrow` dice qué es y qué **no** es (ni el Arrow del puesto, ni el escritor IPC, ni `al_contrato`). Un cambio de un par de líneas, **antes** de más usos | revisión |
| **1 · `Forma`** | `RecordBatchBuilder` acepta `Forma` (struct, lista, vector, geometría), no solo `Fisico`. Es el prerrequisito de todo lo demás, y es donde entrarán `Binary` y `Map` (§12) | pruebas de `ore-arrow` con los compuestos; los orígenes no cambian |
| **2 · el mapa de tipos** | `ore-store::carga::{arrow_de_forma, campo, arrow_del_oos}` pasan a `ore-arrow` (es una **función pura**, sin riesgo de datos) | el mapa da **los mismos** `DataType` para todo el conjunto cerrado; las pruebas de `carga` sin tocar |
| **3 · el lote tipado de `carga::lote`** | el camino *«todo analiza»* se construye con `ore-arrow`; el camino *«un valor no analiza → la columna es `string`»* se conserva **fuera**, como política del lago | **el *digest* (`huella`) es idéntico byte a byte** sobre las pruebas de `carga`; medir antes y después (`sellar` de 10⁶ filas) |
| **4 · Postgres** | el lector deja su `Col` y usa `ore-arrow` (conserva su geometría y sus listas como `Forma`) | el kit de Postgres; es **el mismo diseño** que Oracle, así que debería ser la migración más limpia |
| **5 · el escritor IPC** | una función `emitir(lotes) -> flujo IPC con su marca de fin` para los ocho lectores | el kit de cada uno; el caso 7 (flujo) |
| **6 · BigQuery y objetos** | se miran **después**: BigQuery tiene su propio camino (Storage Read) y objetos, el rescate de `_rescued_data` | decidir con los datos de las fases anteriores |

**Salvaguardas** (lo que no se puede romper):
- **Determinismo.** `huella` produce *«mismos bytes para la misma entrada»*, y es lo que el digest del snapshot exige. La fase 3
  no se fusiona sin una prueba que lo demuestre sobre el conjunto cerrado de tipos.
- **Las dos reglas de error** (§11.4) se quedan **separadas**: unirlas cambiaría lo que ve un cliente.
- **Un PR por fase**, cada uno con las suites de los crates que toca con **los mismos recuentos** que antes (como en la
  extracción que ya se hizo: 72/76/94/51/24).
- **Rendimiento.** El camino de escritura al lago está medido (*10 M filas en 2,1 s* en `ore-store escribir`); una abstracción
  que lo empeore se descarta.

**Lo que `ore-arrow` NO es, y se debe decir en su cabecera:** el Arrow de los SDK ni del agente del puesto; el flujo IPC (ADR
0043) ni su protocolo; el contrato `al_contrato`/`conformar` (validar un lote que llega); ni una dependencia de `ore` (Arrow no
puede entrar en el cierre del compilador: `dependencias.rs`).

**Estado (2026-10-26):** la **fase 0 está hecha** (la cabecera de `ore-arrow` dice qué es y qué no es) y la visión está en el ADR 0075
(«`ore-arrow`: de tipo OOS a tipo Arrow, en un solo sitio»). Las fases 1–6, después de fusionar el PR #30 y de ver el CI. **`Binary` va antes
de la fase 1** y `Map` con ella (§12.11).

**Quién sirve qué** (aclarado, porque el §11.1 se leía mal): `ore-store` es **el lago** —escribe la tabla Iceberg y fija el contrato del
físico en `carga.rs`—, no quien sirve `over()`/`sql()`/`arrow()`: eso lo hacen los SDK del puesto (Python, Node, JVM), que leen el lago con
DuckDB (`iceberg_scan`) y cumplen el contrato por su cuenta; `write()` manda Arrow por IPC al agente, que llama a `ore-store escribir`.
`ore-arrow` unifica la conversión **dentro del Rust**; no puede serlo de los SDK. Renombrar `ore-store` a `ore-storage` se aplaza (193 ficheros,
727 menciones, y los binarios `ore-store-r2`/`ore-store-gcs` los llaman por nombre el `Dockerfile`, el CI y el SDK Python).

## 12. Los tipos que faltan: `Binary` y `Map` (y lo que va detrás)

### 12.1 El contexto: cómo salió

Al mapear Oracle a OOS (§4.3) una y otra vez el destino era **`Opaque`**: `RAW`, `LONG RAW`, `BLOB`, los dos `INTERVAL`, `JSON`. No era una
decisión de Oracle: es lo que **ya hace PostgreSQL** (`bytea`, `json`, `jsonb`, `interval`, `xml`…) y lo que hacen los demás
orígenes con sus binarios. La pregunta fue si el sistema de tipos tiene lo que necesita; la respuesta corta es que **le faltan
`Binary` y `Map`**, y detrás, `Interval` y un tipo semiestructurado.

Es una decisión **de la especificación OOS** (`vendor/oos`, repositorio `describeloai/oos`), no de un origen: Oracle, Mongo,
Cassandra, HBase, Phoenix, PostgreSQL y BigQuery lo piden a la vez. Por eso **no se resuelve en este PR**.

### 12.2 Dónde estamos

**Lo que OOS tiene hoy** (leído en `spec/v1alpha1/02-entity.md` §3 y `spec/v1alpha17/01-los-tipos.md`):

| familia | tipos |
|---|---|
| escalares (10, un conjunto **cerrado**) | `String`, `Integer`, `Decimal`, `Float`, `Boolean`, `Date`, `Time`, `DateTime`, `DateTimeTz`, `Opaque` |
| parametrizados | `Decimal<p, s>` (1 ≤ p ≤ 38), `Money<…>`, `Quantity<…>`, un tipo importado (`iso.CountryAlpha2`) |
| compuestos (desde v1alpha17) | `list<T>`, `Struct<a: T, b: U>`, `Vector<n>`, `Media<c>`, `Anchor` |
| geográficos (v1alpha32) | `Geometry<…>`, `Geography<…>` |

`Opaque` es, en la propia especificación, *«"no lo modelamos"»*, y ir a él *«retira el gobierno»* (§3.4, la tabla de lo que **no** es un ensanche).

**Lo que cada origen hace hoy con ellos** (comprobado en el código):

| origen | binario | mapa / colección | semiestructurado | intervalo |
|---|---|---|---|---|
| **Oracle** | `RAW`, `LONG RAW`, `BLOB` → `Opaque` (base64) | — (`VARRAY`/tabla anidada: sin leer) | `JSON` → `Opaque` (texto); `XMLTYPE`: sin leer | los dos `INTERVAL` → `Opaque` (ISO 8601) |
| **PostgreSQL** | `bytea` → `Opaque` | extensiones como `hstore`: no están en la tabla de `escalar`, sin tipo; arrays → texto | `json`, `jsonb`, `xml` → `Opaque` | `interval` → `Opaque` |
| **Cassandra** | `blob` → `Opaque` (base64) | `map<k,v>`, `set`, `list`, `tuple`, UDT, `vector` → **`String` con JSON** | — | `duration` → `String` |
| **MongoDB** | `binData` → `Opaque` (base64) | documentos anidados → `String` JSON | `Json`/`Mixto` → `String` | — |
| **Phoenix** | `BINARY`/`VARBINARY` → `Opaque` (base64) | `T ARRAY` → `String` con JSON | — | — |
| **HBase** | bytes → `Opaque` (base64) | — | — | — |
| **BigQuery** | `BYTES` **cierra el camino Arrow rápido** (cae a REST en texto) | `STRUCT`/`ARRAY` igual | `JSON` igual | — |

Dos hechos que importan: **el binario ya existe en el lago** (una geometría es `Binary` con la extensión `geoarrow.wkb`: `carga.rs` ya
construye, valida y escribe columnas `Binary`), y **BigQuery pierde su camino rápido** por tipos que OOS no tiene (ADR 0043: 17,9 s frente a 180 s).

### 12.3 Qué tienen los demás

Cotejado por el ADR 0032 («Lo mirado», 2026-09-20):

| | binario | mapa | estructura | intervalo | semiestructurado |
|---|---|---|---|---|---|
| **Foundry** (`SchemaFieldType`, 15 tipos) | `BINARY` | `MAP` | `STRUCT`, `ARRAY` | — | — |
| **Spark / Databricks** | `BINARY` | `MAP` | `STRUCT`, `ARRAY` | `INTERVAL` | `VARIANT` |
| **Iceberg** (14 primitivos) | `binary`, `fixed(L)` | `map` | `struct`, `list` | — | — (a verificar para v3) |
| **Arrow** | `Binary`, `LargeBinary`, `FixedSizeBinary` | `Map` | `Struct`, `List` | `Interval(…)` | — |
| **OOS hoy** | **`Opaque`** | **—** | `Struct<…>`, `list<T>` | **`Opaque`** | **`Opaque`** |

El ADR 0032 ya lo dejó dicho: *«Anidados: struct/map/array son de primera clase en los tres; OOS los aplana… Es una brecha conocida frente a
orígenes JSON»*. Desde v1alpha17 se cerró para `Struct` y `list`; **`Map` y `Binary` siguen abiertos**.

### 12.4 Por qué `Opaque` no basta

`Opaque` mezcla **dos significados** que no se parecen:
1. **«Son bytes, y lo sé»** (`RAW`, `bytea`, `blob`): se pueden guardar, comparar por igualdad, medir, transportar y devolver intactos.
2. **«No sé qué es»** (un tipo exótico): no se puede tratar.

Tratarlos igual tiene un coste: **un cliente que escribe desde el SDK un `bytes` de Python recibe una columna de texto en base64**
(el lago guarda `string`), y lo que lee de vuelta no es lo que escribió; **BigQuery no puede usar su camino rápido** con una columna `BYTES`; y
**no hay forma de declarar** «esta columna son bytes» en el contrato. Separar los dos significados es lo que hace falta.

### 12.5 `Binary` (la primera, y la más clara)

**Por qué primero:** lo piden **todos** los orígenes; el lago ya sabe construir, validar y escribir `Binary` (la geometría); y en el
camino de datos **no cambia nada en el cable**: hoy ya viaja en base64 como texto canónico.

| capa | qué cambia |
|---|---|
| **OOS** (spec) | un escalar nuevo, `Binary`: una secuencia de bytes de cualquier longitud (incl. vacía), **nulo ≠ vacío**; comparable **por igualdad** y por orden de bytes; **sin** `like`, sin aritmética. Una versión nueva (`v1alphaNN`) con su `00-scope`, su `01-los-tipos` (con la tabla *Arrow/Parquet/Iceberg/DuckDB*, como las de v1alpha17), sus casos de conformidad y sus errores `OOSxxxx` |
| **`ore-base`** | `Fisico::Binario`, `Valor::Bytes(Vec<u8>)`, el nombre `Binary` en `escalares()` (que `ore review` ofrece), `Fisico::analizar` de un texto **base64** (la forma canónica **que ya se usa**) |
| **Arrow / Parquet / Iceberg / DuckDB** | `Binary` · `BYTE_ARRAY` · `binary` · `BLOB` |
| **`ore-arrow`** | un `BinaryBuilder` más (el `RecordBatchBuilder` es donde entra; por eso la fase 1 de §11.7) |
| **`ore-store`** | `arrow_de_forma`, `lote`, `normalizar`, `al_contrato`, **`huella`** (el digest tiene que hashear bytes, no el texto del tipo: a revisar) |
| **orígenes** | `RAW`/`LONG RAW`/`BLOB`, `bytea`, `blob`, `binData`, `VARBINARY`, bytes de HBase y `BYTES` de BigQuery pasan de `Opaque` a `Binary`. **BigQuery recupera el camino rápido** para `BYTES` |
| **SDK** | `bytes` (Python, y pandas con `ArrowDtype(binary)`), `Uint8Array`/`DuckDBBlobValue` (Node), `byte[]` (JVM); `write()` los escribe como `binary`, no como texto |
| **consola** | enseñar un binario como hexadecimal o base64 recortado, con su tamaño |

**Qué no cubre:** un binario enorme (decenas de MB por celda) pide otro tratamiento —por referencia, como `Media<c>`—; `Binary` es para valores
que caben en una fila. Una `FixedSizeBinary` (un hash, un UUID de 16 bytes) es una restricción más, no un tipo aparte.

### 12.6 `Map<K, V>` (la segunda)

**Por qué:** la piden Cassandra (`map<k,v>`, hoy un `String` con JSON), Foundry, Spark e Iceberg, y un cliente que escribe desde el
SDK una columna con un diccionario de Python, un `Map` de Java o un `MAP` de DuckDB. Hoy, según el ADR 0032, pandas clásico entrega un `map` como lista de pares y Java lo pierde (`{}`), y de los anidados dice que *«la copia los guarda si vienen (Arrow los tiene), el contrato no los promete»*.

| capa | qué cambia |
|---|---|
| **OOS** (spec) | `Map<K, V>`, escrito como `Struct<…>` y `list<…>` (un espacio tras la coma: forma canónica). **Clave**: un escalar **no nulo y comparable** (`String`, `Integer`, `Date`…; ni `Float`, ni `Opaque`, ni compuestos); **valor**: cualquier tipo del documento, anidable. **Las claves son únicas dentro de un valor**; el **orden de las entradas no es parte del valor** (o se declara que sí: decisión abierta, §12.10). `Map` no se ordena ni se compara por `<`; solo `isNull` y acceso por clave |
| **`ore-base`** | `Forma::Mapa(Box<Forma>, Box<Forma>)` (es un **compuesto**, como `Struct` y `Lista`: solo llega entero desde un SDK, en Arrow; el protocolo del driver entrega escalares como texto) |
| **Arrow / Parquet / Iceberg / DuckDB** | `Map` (una lista de `struct<key, value>`, con `keys_sorted`) · el tipo lógico `MAP` (`key_value`) · `map` (con *ids* de clave y de valor) · `MAP(K, V)` |
| **`ore-arrow` / `ore-store`** | `MapBuilder`; en `carga`, los *ids* de campo de las entradas (como ya hace con los hijos de `Struct`: `esquema_deseado`) |
| **orígenes** | **Cassandra** `map<k,v>` (hoy `String` JSON) y lo que Spark/Iceberg ya escriban en el lago; **el origen que entregue Arrow directo** puede mandar un `MapArray`; **por el camino de texto no hay forma canónica** (como un `Struct`): se declina o se queda `String` |
| **SDK** | `dict` (Python, con `ArrowDtype(map_)`: las operaciones son limitadas), `Map` (JS: `DuckDBMapValue`), `Map<K,V>` (Java) |
| **consola** | enseñar el mapa como pares `clave → valor` |

**Oracle no lo necesita** (no tiene un tipo mapa; sus colecciones son `VARRAY` y tablas anidadas, que serían `list`, y los objetos, `Struct`): lo
pide el resto de la plataforma. Por eso va **segundo**.

### 12.7 Lo que va detrás (sin proponer cierre)

- **`Interval`**: Oracle tiene dos familias (`YEAR TO MONTH` y `DAY TO SECOND`), PostgreSQL uno (`interval`), Spark `INTERVAL`, Arrow tres
  (`YearMonth`, `DayTime`, `MonthDayNano`) e **Iceberg ninguno**: el lago no podría guardarlo nativo. Mientras tanto, `Opaque` en ISO 8601
  (lo que hace hoy Oracle). **No urge.**
- **Un semiestructurado** (`Variant`): `JSON` de Oracle, `jsonb` de PostgreSQL, `Json`/`Mixto` de Mongo. No es `Struct` (un struct tiene
  esquema fijo; un JSON, no). Spark tiene `VARIANT`; Iceberg v3 lo incorpora (**a verificar en la especificación de Iceberg antes de decidir**).
  Es lo que más cambia el sistema de tipos, y lo que más pide una decisión de producto.
- **XML**: no necesita tipo propio; es texto con una convención (`String` con una etiqueta), o un `Variant`.

### 12.8 La compatibilidad: lo que hay que decidir antes de tocar nada

Cambiar `Opaque` por `Binary` en un origen **no es gratis**:

1. **Las copias ya hechas.** Una columna que hoy es `string` (base64) en el lago pasaría a `binary`. El ADR 0032 («Lo mirado», punto 3) fija que la única evolución de tipo que puede hacer una copia sucesora de la misma vista es ensanchar sin
   pérdida (`int → long`, `float → double`, un decimal de más precisión), y que estrechar es otra vista. `string` → `binary` **no es una de ellas**. Hay que decidir si es una vista nueva, una columna nueva o un camino explícito de migración.
2. **La ontología.** `ore diff` calcula si un cambio rompe a un consumidor. `Opaque → Binary` no está en la tabla del ensanche (§3.4); hay que
   decir qué es (hoy, *cualquiera → `Opaque`* **retira el gobierno**; el inverso es un afinado que **añade** gobierno y no debería tratarse como un estrechamiento).
3. **`discover` repetido.** Una fuente ya catalogada con `Opaque` que se vuelve a descubrir dará `Binary`: el cambio tiene que ser **aceptable
   sin romper** (propuesta: se ofrece, no se impone, y el `physicalType` guardado dice cuál era).
4. **Los SDK en uso.** Quien hoy lee base64 de una columna `Opaque` lo seguirá leyendo hasta que la columna cambie: es un cambio de tipo visible.

### 12.9 El orden y el coste

1. **OOS** (`vendor/oos`): la versión nueva con `Binary`, sus casos de conformidad y las reglas de compatibilidad (§12.8). Es **lo primero y lo
   que decide**; todo lo demás sale de ahí. Un `Map` puede ir en la misma versión o en la siguiente.
2. **`ore-base`**: `Fisico::Binario`, `Valor::Bytes`, `analizar`, `escalares()`; `Forma::Mapa`.
3. **`ore-arrow` y `ore-store`** (fases 1–3 de §11.7): los *builders*, el mapa de tipos, `huella`.
4. **Los orígenes**, uno a uno, con su kit; BigQuery recupera su camino rápido.
5. **Los SDK y la consola.**

Cada paso es un PR con las mismas salvaguardas que §11.7. **`Binary` sin `Map`** es perfectamente coherente: es el 80 % del valor con la parte
sin compuestos.

### 12.10 Decisiones abiertas

1. **¿`Binary` y `Map` en una versión de OOS o en dos?** (Propuesta: dos; `Binary` primero.)
2. **El orden de un `Map`:** ¿las entradas se comparan como conjunto o como lista? (Arrow lo marca con `keys_sorted`; Iceberg no lo define como orden.)
3. **Qué pasa con `Opaque → Binary`** en una fuente ya catalogada (§12.8): ¿refinamiento ofrecido, vista nueva, o migración explícita?
4. **`Binary` frente a `FixedSizeBinary`:** ¿una restricción de longitud (`Binary<16>`, como `Vector<n>`) o no?
5. **El tamaño máximo** de un `Binary` en una celda, y qué se hace por encima (rechazar, o `Media<c>`).
6. **`Interval` y `Variant`:** ¿se abren a la vez o se esperan a que haya un cliente que los pida?
7. **La nomenclatura:** los nombres de OOS se alinean *«deliberadamente con el enum de Apache Ossie»* (§3.1); hay que comprobar cómo llama Ossie
   a un binario y a un mapa antes de fijar `Binary` y `Map`.
8. **La forma textual de `Binary`: CERRADA (2026-10-26), base64** (RFC 4648, alfabeto estándar, con relleno). Es lo que ya viaja de los orígenes y pesa un tercio más en vez del doble; la geometría sigue en hexadecimal (WKB, v1alpha32). Solo afecta al texto de tránsito: en Arrow, Parquet e Iceberg son los mismos bytes. Ver §12.12.

### 12.11 Lo medido: qué toca añadir `Binary` y `Map` (2026-10-26)

Contado en el código, tomando como molde una variante escalar existente (`Fisico::Hora`/`Valor::Hora`) y el precedente de la especificación
(v1alpha17, que añadió `Struct`, `list` y `Vector`). **No son horas**: son puntos de contacto y PRs; el tiempo depende de las decisiones de §12.10.

| capa | medida | qué implica |
|---|---|---|
| **OOS** (`vendor/oos`) | el molde, v1alpha17, son 5 documentos (`00-scope`, `01-los-tipos`, `02`–`04`) más casos de conformidad (`valid/`, `invalid/`, y las carpetas `canonical`, `diff`, `digest`, `emit`) y un esquema por versión (`schemas/`) | una versión nueva con `Binary` (y `Map` en ella o en la siguiente): spec, casos válidos e inválidos, la compatibilidad de §12.8 y los errores `OOSxxxx`. **Ojo con el vendor:** el `vendor/oos` de `C:\ORE` llega a v1alpha28 y `main` fija v1alpha34 (`ee5a7fc`, 14 commits más). La especificación nueva se escribe en el repositorio `describeloai/oos` partiendo de ese pin, no del checkout viejo |
| **`ore-base`** | `Fisico::Hora` se nombra en 5 sitios de `tipos.rs`, `Valor::Hora` en 3; `Forma::Lista` en 1 | `Binary` es una variante de `Fisico` y de `Valor` (+ `analizar` de base64); `Map` es una variante de `Forma` |
| **crates con un `match` sobre `Fisico`** | `Fisico::` aparece en **38 ficheros** de `crates/`; `Fisico::Hora`/`Valor::Hora` en **17** | el compilador señala cada `match` no exhaustivo: es mecánico, pero son ~17 sitios para `Binary` (los orígenes, `ore-store::carga`, el kit, BigQuery, objetos, Postgres) |
| **`ore-arrow`** | 2 sitios por variante (`Col` y `nuevo`) | un `BinaryBuilder`; para `Map`, el `MapBuilder` y `Forma` (fase 1) |
| **`ore-store::carga`** | `Fisico::Hora` en 4 sitios, `Forma::Lista` en 1 | `arrow_de_forma`, `lote`, `normalizar`, `al_contrato` y `huella` (que tiene que hashear los bytes) |
| **orígenes** | `Opaque` aparece en ~15 ficheros de `crates/` (Postgres 17, HBase 14, BigQuery 9, Oracle 8…) | pasar `RAW`/`bytea`/`blob`/`binData`/`BYTES` de `Opaque` a `Binary`; Cassandra `map<k,v>` a `Map`; uno por PR, con su kit |
| **SDK** | contrato/tipos en `puesto/python`, `node`, `jvm` y `spark`; `Opaque` en `docs/sdk.md`, `contract.mjs`, `__init__.py`, `sql_functions.py` | un tipo nuevo en cuatro lenguajes y sus pruebas de contrato |
| **consola** | no medida | enseñar bytes (hex/base64 recortado y tamaño) y pares clave → valor |

**Iteración, en PRs:** (1) OOS `Binary` · (2) `ore-base` + `ore-arrow` + `ore-store` · (3) un PR por origen (7: Oracle, Postgres, Cassandra,
Mongo, Phoenix, HBase, BigQuery) · (4) SDK y consola. `Map` repite el esquema con menos orígenes (Cassandra) y más SDK (el `dict` es lo
difícil). **Lo que decide el ritmo no es el código sino §12.8**: qué hacer con las copias que ya guardan base64 como `string`.

### 12.12 Lo leído en `vendor/oos` (v1alpha34): cómo sería v1alpha35

Leído en el pin de `main` (`ee5a7fc`) el 2026-10-26. Los dos precedentes cercanos son v1alpha17 (`Struct`, `list`, `Vector`) y v1alpha32 (la geometría).

- **Forma de una versión.** v1alpha17: 5 documentos y 30 casos de conformidad (8 válidos, 22 inválidos). v1alpha32: **2 documentos**
  (`00-scope`, `01-la-geometria`) y una carpeta de casos; cada caso son 5–8 ficheros pequeños (`case.yaml`, `README.md` y un `input/` con
  paquete, conductos, retículo y ontología). No toda versión trae esquema JSON: v1alpha34 sí, v1alpha32 no. **`Binary` es del tamaño de la
  geometría**: un escalar nuevo, no una familia.
- **Dónde se anota.** Los tipos compuestos se explican en `01-los-tipos` con una **tabla de correspondencias** (Arrow · Parquet · Iceberg ·
  DuckDB) y una tabla de reglas con su código. El siguiente código libre es **`OOS3010`** (`OOS3008` es geo, `OOS3009` el enriquecimiento);
  `OOS3001` es «un tipo que no existía en esa versión» y valdría para `Binary` en v1alpha34 o antes.
- **`Opaque` ya está prohibido como campo de un `Struct`** (`OOS3007`). Con `Binary`, un struct con un campo de bytes pasaría a ser
  expresable; hoy no lo es. Es un argumento a favor, no solo un detalle.
- **⚠ Choque con lo que escribí en §12.5.** Dije que el texto canónico de un binario es **base64**, «el que ya se usa». Es cierto de los
  orígenes (Oracle, Cassandra, Mongo…), pero **la especificación fijó para la geometría el WKB en hexadecimal** (v1alpha32 01 §…, «en texto…
  el valor es su WKB en hexadecimal»). `Binary` tiene que elegir y justificar: hex (coherente con geo, el doble de largo) o base64 (lo que
  ya viaja). **Era una decisión nueva (§12.10, punto 8), ya cerrada: base64.** Condiciona `Fisico::analizar`, la huella y todos los orígenes.
- **Borrador escrito** (sin commit, en `vendor/oos`): `spec/v1alpha35/00-scope.md` y `01-el-binario.md`, 5 casos de conformidad (3 válidos, 2 inválidos de `OOS3001`/`OOS3010`, y 1 de `OOS3007` que sigue valiendo) y `OOS3010` en `99-errors.md`. Quedan abiertos: longitud fija (`Binary<n>`), tamaño máximo y qué pasa con las copias en base64 (§12.8).
- **Los casos mínimos de `Binary`**, por el molde de v1alpha17: válido (una columna `Binary`, nulo distinto de vacío, `Binary` en una
  `list`, en un `Struct`); inválido (`Binary` en v1alpha34 o antes → `OOS3001`, `Binary` con argumentos, como clave primaria si se decide
  que no comparable). Unos 6–8 casos.

## Fuentes

- Fivetran, *Oracle* y *HVA Oracle*: <https://fivetran.com/docs/connectors/databases/oracle> (leído el 2026-10-25).
- Airbyte, *Oracle DB*: <https://docs.airbyte.com/integrations/sources/oracle>.
- `oracledb` (Oracle): <https://docs.rs/oracledb> · <https://github.com/oracle/rust-oracledb> (la guía y su apéndice A).
- `oracle-rs`: <https://docs.rs/oracle-rs>. `rust-oracle` y ODPI-C: <https://odpi-c.readthedocs.io/en/stable/user_guide/installation.html>.
- Mediciones y repro: `pruebas-de-fuego/oracle/` · código: `crates/ore-oracle`, `crates/ore-read-oracle`, `crates/ore-arrow`,
  `crates/ore-conector-kit/src/bancos/oracle.rs`.
- Para §11 y §12: ADR [`0031`](../decisions/0031-el-puesto.md), [`0032`](../decisions/0032-el-contrato-de-tipos.md) y
  [`0043`](../decisions/0043-ore-arrow-stream.md); `docs/sdk.md`; `crates/ore-store/src/carga.rs`, `crates/ore-base/src/tipos.rs`,
  `crates/ore-read-postgres/src/lectura.rs`, `crates/ore-read-bigquery/src/v2.rs`, `crates/ore-read-objetos/src/filas.rs`;
  la especificación OOS (`describeloai/oos`): `spec/v1alpha1/02-entity.md` §3, `spec/v1alpha1/91-versioning.md` y
  `spec/v1alpha17/01-los-tipos.md`. **No se han leído** las especificaciones de Apache Iceberg (v3), Arrow ni Ossie: lo que §12 dice de ellas
  sale del ADR 0032 y de lo que ya hace el código, y está marcado «a verificar».

