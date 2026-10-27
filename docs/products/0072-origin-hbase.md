# 0072 · Origen Apache HBase

**Leer Apache HBase directamente** —tablas de celdas versionadas, sin esquema— como origen de ORE, por la
ruta que decida este documento. El origen Phoenix ([`0071`](0071-origin-phoenix.md)) ya cubre HBase **con
Phoenix encima** (tipos, catálogo y SQL); este es el que falta: las tablas HBase que **no** son de Phoenix.

**Estado:** construido y cerrado (2026-10-21), salvo la hoja de la consola. X0 y X0b midieron las tres rutas de acceso
y Kerberos desde un cliente que no es la JVM; la ruta **nativa** (§7) se construyo en ocho hitos, N1–N8, cada uno con
su prueba y, los de comportamiento, con sabotajes. **El origen tal como es esta en §0**; de §1 a §7 queda el razonamiento
y la medicion que llevaron a el, y de N1 a N7 el registro de cada hito (con lo que *entonces* no hacia).

Sigue a [`0008`](0008-el-protocolo-del-driver.md), [`0016`](0016-el-testigo-y-el-rango.md),
[`0069`](0069-origin-mongodb.md), [`0070`](0070-origin-cassandra.md) y [`0071`](0071-origin-phoenix.md)
(el patrón: medir primero, construir después).

## 0. El origen, tal como es

`ore-read-hbase` lee tablas de Apache HBase **por su protocolo nativo** (ZooKeeper para encontrar `hbase:meta`, el RPC
protobuf contra los RegionServers), sin JVM, sin Phoenix y sin pasarelas REST o Thrift desplegadas. Es un binario del
esqueleto de los drivers (`ore_driver::origen::Origen`) y habla los siete codigos de error de siempre.

**La direccion**

```
hbase://[usuario@]zk1[:2181][,zk2[:2181]…][/espacio]?simple=acepto            ← SIMPLE: afirma un usuario, sin secreto y sin cifrar
hbase://zk1[:2181]…[/espacio]?auth=kerberos[&principal=lector@REINO&keytab=/ruta/l.keytab&servicio=hbase]
        [&znode=/hbase&timeoutMs=&paralelo=4&rezago=5000&familia=f&clave=id&traduccion=anunciado:16020=real:16020]
```

Una de las dos autenticaciones, siempre: sin `simple=acepto` ni `auth=kerberos` no se conecta. `traduccion=` lleva el
nombre que un RegionServer anuncia al sitio donde el Job si llega (§7.3). **`clave=` se puede repetir**
(`clave=id&clave=pedido_id`, sin repetidos y en orden): una conexion con tablas cuya clave de fila se llama distinto las nombra
todas (2026-10-22; antes era un solo nombre y el ultimo ganaba). `familia=` sigue siendo una sola: con varias, un nombre sin `:` seria ambiguo.

**Lo que se lee.** Una fila de HBase es una fila de OOS. Columnas: `_clave` (la clave de fila), `_clave#1`, `_clave#2`…
(sus partes, si es compuesta), `familia:cualificador` (la ultima version de cada celda) y `_ts` (la marca de tiempo de la
fila). Nada se infiere en silencio: el tipo de cada columna lo declara la tabla y su **codificacion** va en `physicalType`
(14: `utf8`, `bytes`, `texto`, `i8/i16/i32/i64be`, `u32/u64be`, `f32/f64be`, `bool`, `epoch-ms`, `decimal` —lo que
`Bytes.toBytes` de Java escribe—). El catalogo muestrea la tabla y propone `String` u `Opaque`.

**Lo que se empuja y lo que no.** Al servidor: el rango de la clave (exacto solo si la codificacion conserva el orden;
los enteros con signo se parten en dos intervalos, porque en complemento a dos los negativos ordenan *despues*), el prefijo
de una clave compuesta, el rango de tiempo y `limit`. Lo demas (`eq`/`neq`/`in`/`lt`/`le`/`gt`/`ge`/`isNull`/`isNotNull`
sobre celdas, y **todos** los filtros sobre las partes de la clave, otra vez) se evalua aqui sobre el valor decodificado,
con la semantica de SQL. No hay `like`, `orderBy` ni `estimar`.

**Como lee.** Una parte por grupo de regiones (`paralelo=`), cada una con su conexion y su hilo; ante una region que se
mueve o se parte, un RegionServer que cae, un scanner caducado o un servidor saturado, reabre **desde la ultima clave
entregada** con la espera del cliente de HBase; cancela en 100 ms; el plazo de la peticion se cumple. Los errores se
clasifican por la **clase** de la excepcion y no por `do_not_retry`.

**El incremental.** `start`/`end` sobre `_ts`, en dos pasos (las claves que cambiaron, y de ellas las filas completas). El
testigo es `reloj − rezago`, un limite. Ve las actualizaciones; **no ve los borrados**. Se declara por tabla
(`changes: { mode: upsert, key: [_clave], witness: field, field: _ts }`): el catalogo no lo declara solo.

**La seguridad.** SIMPLE solo reconocido; Kerberos (SASL/GSSAPI) con las tres protecciones del servidor, detras del
*feature* `kerberos`. La credencial nunca aparece en un error ni en una traza.

**En la plataforma.** `ore-read-hbase` es el unico binario dinamico de la imagen de los drivers (`krb5-libs`, `libgcc`); la
pasarela lo descubre por su carpeta; `ore-serve` tiene su formulario de conexion; el CI corre el kit del conector y de la
pasarela, y las tres protecciones de Kerberos.

**Lo que NO hace** (la lista vigente; las de cada hito, mas abajo, son historia)
- Escribir; leer el historial de versiones; filtros de valor en el servidor; leer *snapshots* de HFiles.
- Ver los borrados en el incremental, ni las filas escritas con una marca anterior al testigo cuando este ya paso.
- Tablas de Phoenix en crudo (la codificacion de Phoenix la lee 0071), `Time`, tipos compuestos.
- Active Directory como KDC, ZooKeeper con SASL, las ACL, TLS en el RPC (no existe en HBase 2.4), Windows como cliente de Kerberos.
- **Kerberos desde la consola**: el keytab es un fichero y el Job no tiene ficheros (el mismo hueco que Astra y la CA de Phoenix).
- **La hoja de la consola** (`rubix-platform`), por el estado de ese arbol.

**Medido** (HBase 2.4.18, una maquina): 10⁶ filas de 100 bytes con 1, 2, 4 y 8 hilos: 14,6 · 6,2 · 3,8 · 3,2 s; el kit
del conector 11 pasan / 0 fallan / 3 no aplican, con la pasarela delante 8 / 0 / 2; 2 500 filas por el incremental en
245 ms; Kerberos en las tres protecciones, en Debian y en Alpine/musl.

**Como se prueba**: [`pruebas-de-fuego/hbase/README.md`](../../pruebas-de-fuego/hbase/README.md).

## 1. Por qué hace falta y qué cambia respecto a Phoenix

Phoenix te da columnas con tipo; HBase te da **bytes**: una clave de fila, familias de columnas, y por cada
celda un cualificador, un *timestamp* y un valor sin tipo. Tres consecuencias que no existían en 0071:

1. **No hay esquema que descubrir.** Habrá que declarar un *mapeo* (columna, tipo, codificación) o inferirlo
   de una muestra. Es la pieza propia de este origen.
2. **El *timestamp* de celda existe en todas las tablas** y es un cursor incremental natural (§3.4), sin
   `ROW_TIMESTAMP` que declarar.
3. **No hay SQL**: lo único que HBase empuja con eficacia es el rango de clave de fila y el rango de tiempo.

## 2. Las rutas de acceso

| Ruta | Qué es | Cliente en Rust |
|---|---|---|
| **REST** (Stargate) | HTTP con JSON o protobuf | el HTTP y el TLS que ya tenemos |
| **Thrift2** | RPC binario | un cliente Thrift |
| **Nativa** | ZooKeeper + RPC protobuf contra los RegionServers | uno propio, sobre `prost` |
| Phoenix | ya hecho (0071) | `ore-avatica` |

Los mensajes de la ruta nativa **no se copian a mano**: los `.proto` vienen dentro de
`hbase-protocol-2.4.18.jar` (y el IDL de Thrift2 dentro de `hbase-thrift-2.4.18.jar`), igual que en Avatica
cada número de campo salió de un descriptor real.

## 3. Lo medido (laboratorio `pruebas-de-fuego/hbase/`)

Un solo nodo, loopback, `PerformanceEvaluation` con 10⁶ filas de 100 bytes y 4 regiones. **Los clientes de
la sonda son de Python**, así que las filas por segundo son una cota inferior comparable entre rutas solo
con cuidado: el decodificador de cada una pesa distinto (JSON y protobuf son nativos, Thrift es Python puro).

### 3.1 Rendimiento

| Ruta | 10⁶ filas | Notas |
|---|---|---|
| REST JSON, `batch=1000` | 9,7 s (≈104 000 filas/s) | 251 MB por la red |
| REST JSON, `batch=10000` | 6,4 s (≈155 000 filas/s) | |
| REST protobuf, `batch=10000` | 3,1 s sin decodificar | 151 MB: un 40 % menos que JSON |
| Thrift2, `caching=1000` | 15,5 s (≈64 000 filas/s) | decodificador de Thrift en Python puro |
| **Nativa, una conexión** | **3,7 s (≈269 000 filas/s)** | |
| **Nativa, 4 regiones en paralelo** | **1,7 s (≈590 000 filas/s)** | hilos de Python |

Para situarlo: Phoenix por PQS leía el mismo orden de filas en 11 s con 4 conexiones (0071 M5).

### 3.2 La ruta nativa es más barata de lo que se temía

Un cliente mínimo, en ~150 líneas de Python y sin Kerberos, hace de punta a punta: ZooKeeper
(`/hbase/meta-region-server`) → conexión RPC SIMPLE al RegionServer de `hbase:meta` → `Scan` de `hbase:meta`
para listar las regiones → `Scan` de cada región. Hallazgos que el cliente de Rust tiene que respetar:

- **`hbase:meta` no se ordena por bytes.** Un `Scan` de `hbase:meta` que empieza en `tabla,` **se salta en
  silencio la región de clave vacía** (devolvió 3 regiones de 4; la lectura perdía el 25 % de los datos sin
  error). Hay que empezar en `tabla,,`. Medido: `tabla,`, `tabla,\x00` → 3; `tabla,,` y `tabla` → 4.
- **`next_call_seq`** empieza en 0 en la primera petición de continuación y sube de uno en uno; la de cierre
  lleva el valor que toca. Si no, `OutOfOrderScannerNextException`.
- **El servidor cierra solo el scanner al acabar la región**: lo dice `more_results_in_region`. Pedir otra
  vez o cerrarlo da `OutOfOrderScannerNextException`.
- Sin `cell_block_codec` las celdas viajan dentro del protobuf, que es lo más sencillo.
- Con la sesión de ZooKeeper de 10 min del laboratorio no hubo caducidades; los puertos de HBase **deben
  escuchar en todas las interfaces** (`hbase.*.ipc.address=0.0.0.0`), porque anunciándose como `127.0.0.1`
  HBase se enlaza solo al loopback del contenedor y Docker acepta la conexión y la corta.

### 3.3 La semántica que el origen necesita (por REST; no medida aún por la ruta nativa)

`sonda_rest_semantica.py`, tabla de 10 filas con *timestamps* puestos a mano:

| Pregunta | Resultado |
|---|---|
| Rango de clave `[fila3, fila6)` | `fila3, fila4, fila5`: el fin es exclusivo. `startRow`/`endRow` van en base64. |
| Rango de tiempo `[t3, t6)` | `fila3, fila4, fila5`: se empuja al servidor. |
| Una **actualización** | crea una versión nueva con *timestamp* nuevo y **se ve** por el rango de tiempo. |
| Versiones | el scanner REST devolvió **las dos versiones** sin pedirlo; hay que fijar `maxVersions=1`. |
| Un **borrado** | la fila desaparece del scan, y el rango de tiempo **no lo señala**: el borrado no se ve. |
| Una fila con *timestamp* en el futuro | el máximo de la tabla pasa a ser ese futuro (+10 días medidos): **envenena el cursor**. |
| Scanner REST sin tocar 70 s | siguió respondiendo 200: **no se observó caducidad** a ese tiempo. |

### 3.4 Consecuencias para el incremental

El *timestamp* de celda como cursor, con rezago como en 0071 M7, **sí ve las actualizaciones** (a diferencia
de `ROW_TIMESTAMP` en Phoenix), pero **no ve los borrados** y un reloj de cliente en el futuro lo
envenena. Hará falta tope por el reloj del servidor y aviso, igual que en Phoenix.

### 3.5 Kerberos (X0b)

Un KDC de MIT en Docker (`ORE.TEST`), un HBase con `hbase.security.authentication=kerberos`, y clientes que
no son la JVM en la misma red (`pruebas-de-fuego/hbase/kerberos/`). Con la documentación de Cloudera como
contexto: el REST **no** viene con Kerberos de serie y el Thrift no cifra el tramo del cliente (§2), así que
la ruta nativa es la que trae la autenticación y el cifrado **dentro del propio protocolo**, sin que nadie
tenga que haber configurado un gateway (con TLS delante, REST y Thrift también se pueden proteger).

| Medido | Resultado |
|---|---|
| El servidor sin ticket | rechaza (`Found no valid authentication method`); con el ticket de `lector`, acepta |
| **SASL/GSSAPI desde Python** (`gssapi`, la GSSAPI del sistema) | completo en ~0,1 s; lee 200 000 filas en las **tres** protecciones |
| **El mismo apretón en Rust** (`libgssapi 0.8`) | **funciona** con `integrity` y `privacy`, con una llamada real a `hbase:meta`, en Debian |
| **En Alpine/musl**, que es lo que compila ORE | **funciona** (`privacy`), con `-C target-feature=-crt-static` |
| Sin ticket | falla **en el cliente**, antes de tocar la red: `MissingCredentialsError … No Kerberos credentials available` |
| Rendimiento (Python, 200 000 filas, 4 regiones en secuencia) | `authentication` ≈144 000 filas/s · `integrity` ≈132 000 · `privacy` ≈101 000 |

Las cifras son de un cliente de Python y no se comparan con §3.1 (otro tamaño de tabla, y cada región paga su
propio apretón); dicen que el envoltorio criptográfico cuesta hasta un tercio y que no es prohibitivo.

**Trampas del protocolo, todas medidas, que el cliente de Rust tiene que respetar:**

1. **El apretón va antes del `ConnectionHeader`**, con el código de autenticación `81` en el preámbulo. Cada
   mensaje del cliente es `[longitud][token]`; el servidor contesta `[estado][longitud][token]` con estado `0`
   si va bien, y si no, dos cadenas (clase y mensaje) con longitud de 2 bytes.
2. **Hay un mensaje vacío.** Cuando el contexto de Kerberos se completa con el primer reto, el cliente no
   tiene nada que enviar pero **debe enviar igualmente un mensaje de longitud 0**; la respuesta del servidor a
   ese es la oferta de capas de seguridad (RFC 4752), envuelta: 4 bytes (qué capas, y el tamaño máximo).
3. **El servidor ofrece una sola capa**, la que dice `hbase.rpc.protection` (`1` nada, `2` integridad, `4`
   confidencialidad). El cliente contesta con la elegida, un tamaño de búfer y **un `authzid` vacío**: con
   el nombre del usuario el servidor lo rechaza.
4. **El servidor no contesta a ese último mensaje.** Esperarlo cuelga hasta el tiempo límite; lo siguiente
   que se envía es el `ConnectionHeader`.
5. **Con integridad o confidencialidad, lo que se envuelve es el mensaje con su propia longitud**
   (`wrap([longitud][mensaje])`) y la trama lleva la longitud del envoltorio. Envolver solo el cuerpo cuelga
   la conexión sin error. Vale igual para el `ConnectionHeader` y para las respuestas.
6. **HBase independiente no hace el *login* con el keytab por sí solo**, y `UserGroupInformation` lee
   `core-site.xml`, no `hbase-site.xml`: sin un `kinit` previo y `hadoop.security.authentication=kerberos` en
   `core-site.xml`, el propio servidor no puede hablar consigo mismo. Es una trampa **del laboratorio**, no del
   cliente. Y el REST y el Thrift de HBase piden su propio principal con Kerberos; sin él se caen y tiran el
   contenedor.
7. **El nombre del servicio sale del nombre que el RegionServer anuncia** (`hbase/<nombre anunciado>`), así
   que el DNS inverso importa: `rdns = false` en `krb5.conf` en el laboratorio. Con el valor por defecto de MIT
   (resolver al revés) un nombre que no resuelva igual que el directo rompe la autenticación; **no medido**.

**El coste para ORE**, que es lo que decide el alcance: la autenticación con Kerberos **no se puede hacer sin
las librerías del sistema** (`libgssapi_krb5`, `libkrb5`, `libkrb5support`). La imagen de los drivers es
Alpine, donde son el paquete `krb5-libs`; el binario que las use se enlaza dinámicamente
(`-C target-feature=-crt-static`) y se compila con `krb5-dev` y `clang-dev`. No cambia el resto de la imagen si
se hace **un solo binario con un *feature* de Cargo** (§7.7).

## 4. Lo que NO está medido

- **Kerberos, lo que queda tras §3.5:** *Active Directory* como KDC (el más común en una empresa; MIT y AD
  difieren en tipos de cifrado y en el PAC); ZooKeeper **con** SASL (el del laboratorio está abierto; el
  znode de `meta` es legible, pero un ZooKeeper que exija autenticación al cliente obligaría a hablar SASL
  también con él); cómo llega el **keytab o el ticket** a un Job de ORE sin ficheros del usuario (la misma
  limitación que dejó `kerberos` fuera en 0071); la autorización (con `hbase.security.authorization=false` del
  laboratorio un principal autenticado cualquiera lee todo: **las ACL no se midieron**); y Windows (SSPI, que
  `cross-krb5` cubriría pero no se probó).
- La semántica de §3.3 **por la ruta nativa**: el rango de tiempo en `Scan`, `FirstKeyOnlyFilter` y `MultiRowRangeFilter` se midieron
  en N5 y un borrado no se ve; los *tombstones* con scan crudo, no.
- Lectura **de una tabla de Phoenix en crudo** (las claves y los enteros llevan la codificación de Phoenix).
- Compresión, `cell_block`, varias versiones, filtros de valor, TTL y compactaciones.
- Una región que se mueve o cae a mitad de lectura; permisos (ACL); varios nodos; Cloudera, EMR, HDInsight.
- **Qué rutas tienen los clientes reales.** No se puede medir en el laboratorio.

## 5. Lo que ya se puede decir

1. La ruta **nativa** es viable en Rust sin JVM, es la que más rinde, **y con Kerberos también** (§3.5): las
   tres protecciones del RPC, en Debian y en Alpine.
2. REST y Thrift2 sirven como atajo, pero **dependen de que alguien los haya desplegado** y, con Kerberos,
   se complican igual.
3. **El único coste de Kerberos es una dependencia del sistema** (`krb5-libs`) y un binario enlazado
   dinámicamente, que se aísla en un *feature* y un binario. No bloquea nada.
4. Lo que queda abierto de la seguridad es de entorno, no de protocolo: Active Directory, ZooKeeper con SASL,
   cómo llega el keytab a un Job, y las ACL (§4).
5. El *mapeo de tipos* es el diseño propio de este origen (§7.5).

## 6. Laboratorio

`pruebas-de-fuego/hbase/`: `Dockerfile`, `hbase-site.xml`, `arranque.sh` (HBase vigilado + REST 8080 +
Thrift2 9090), y las sondas `sonda_rest.py`, `sonda_rest_semantica.py`, `sonda_nativa.py`,
`sonda_thrift2.py`. Los `.proto` y el IDL se sacan de los jars (`hbase-protocol-2.4.18.jar`,
`hbase-thrift-2.4.18.jar`) y se compilan con `grpcio-tools` y `thriftpy2`.

**Kerberos** (`pruebas-de-fuego/hbase/kerberos/`): `kerberos.sh [authentication|integrity|privacy]` levanta el
KDC (`Dockerfile.kdc`, `kdc.sh`) y el HBase con Kerberos (`hbase-site.xml`, `core-site.xml`,
`arranque-krb.sh`) en la red `krb`; `sonda_kerberos.py` es el cliente de Python (`Dockerfile.cliente`) y
`rust-sonda/` el de Rust, que se compila y corre dentro de `rust:1.90-alpine` (`krb5-dev`, `clang-dev`).
Usuarios del laboratorio: `lector` (clave `lector`) e `intruso`.

```
docker build -t ore-hbase-lab pruebas-de-fuego/hbase
docker run -d --name hb -p 2181:2181 -p 16000:16000 -p 16020:16020 -p 16030:16030 -p 8080:8080 -p 9090:9090 ore-hbase-lab
docker exec -d hb bash -c '$HBASE_HOME/bin/hbase pe --nomapred --rows=1000000 --presplit=4 --valueSize=100 sequentialWrite 1'
```

### Escenarios de cliente · de punta a punta en la celda (2026-10-22)

**Qué es.** Lo que hace un cliente, en la consola y en la celda (`t-victor`) contra el HBase de la VM `ore-labs`
(`23.251.138.69:2181`, con `traduccion=127.0.0.1:16020=23.251.138.69:16020`). Semillas en `pruebas-de-fuego/escenarios/hbase/`.

| Origen | Namespace | Contenido | Bases |
|---|---|---|---|
| `hbase` | `ventas` | 4 tablas (`pedidos` con clave `int` tipada, `clientes` de texto, `sesiones` con clave compuesta, `lecturas`) | `hbase_f`, `hbase_s` |
| `hbase_saas` | `saas` | `cuentas` 150, `facturas` 6 000, `eventos` 4 500; **tres `clave=`** (`cuenta_id`, `factura_id`, `evento`) | `hbase_saas_f`, `hbase_saas_s` |
| `hbase_bench` | `bench` | `filas`: 1 000 000, clave `int`, 5 columnas tipadas, 8 regiones (`millon.rb`) | `hbase_bench_s` |

**Lo medido.**

- **Descubrimiento, vista en vivo y copia salen con las filas exactas** (150 / 6 000 / 4 500 en `saas`; 1 000 000 en `bench`).
- **Bug real de la vista en vivo:** `Source schema changed … columna _ts … declárala Integer/i64be`.
  `ore-serve/federado.rs::leer_federado` copiaba del plan solo `fichero` y `listado`, no `tipos` ni `codificaciones`; HBase guarda
  bytes sin tipo y los necesita. Arreglado y desplegado (PR #9). Perdía igual los tipos de los orígenes que los declaran, p. ej. Mongo.
- **`clave=` repetible:** un SaaS multi-tenant llama distinto a la clave de cada tabla, así que la conexión admite varias
  (`clave=cuenta_id&clave=factura_id&clave=evento`; tope 32). La hoja las pone con «+».
- **La copia es lo que más tardaba**, y no por copiar: dos segundos de trabajo útil tardaban ~4 minutos en verse. Causas medidas
  (job `copiar-48e7ca57`): el alta encolaba la copia de **todo el árbol** (~100 datasets), tres init containers (~40 s), los
  datasets en orden alfabético con fuentes rotas de otras bases (sftp: 4 × 20 s) y **todos los punteros en un solo commit al
  final**. Corregido en PR #11 (la pasada acotada a la base nueva, un puntero por dataset) y PR #12 (`publicar()` con
  identidad: el rebase tras un push rechazado fallaba y dejaba la copia sin publicar). El copiador residente, que quita el
  arranque del pod, está en PR #13 (iteración 1; sin medir en la celda todavía).
- **Un dataset standard sin copia no existe aún:** la consola lo muestra como *Ingesting* hasta que el puntero está en la
  forja, en vez del «Not found» / «need attention» de antes (`lib/ejecucion/copia.ts`).
- **La primera pasada de `hbase_saas` falló una vez con `VICTOR_HBASE_SAAS_URL no está definida`** y la siguiente (14:17) copió
  todo bien. La causa no está probada (los Jobs viejos se borran por TTL): una carrera entre guardar la credencial en el
  cofre y la primera pasada es la hipótesis.

**Rendimiento de la lectura (1 M de filas, `bench:filas`, `ore-read-hbase leer`).**

| `paralelo` | desde un portátil, por internet | desde un pod de la celda |
|---|---|---|
| 1 | 106 s | 41 s |
| 4 | 38 s | 39 s |
| 8 | 33 s | 54 s |
| 16 | — | 52 s |

El techo es el **servidor**, no el cliente: con 4 lectores el portátil y la celda dan lo mismo (~38 s) y el contenedor `hb` de
la VM va al 80 % de **un** core con la máquina al 25 %. Es un HBase autónomo de laboratorio; con varios RegionServers las
regiones se reparten. Copiar el millón de filas entero (job `copiar-prog-7a563a67`) tardó **150 s**: la lectura es ≥ 38 s y los
~110 s restantes (Parquet, subida) **no están explicados**; falta perfilarlos.

**Abierto.** El incremental sobre `_ts` (`lecturas_cambios.rb`) no se ha recorrido en la celda.

## 7. Diseño de la ruta nativa

**Se decide la nativa como ruta principal**: es la única que no depende de que alguien haya desplegado un
gateway (§2), la que más rinde (§3.1) y la que cuesta poco sin autenticación (§3.2). REST y Thrift2 quedan
fuera de v1; si un cliente solo los tiene, se abre otra ficha. Lo que sigue es **diseño, no medición**: cada
punto dice qué se supone y en qué hito se comprueba.

### 7.1 Piezas

Como en Phoenix: un cliente de protocolo que no sabe nada del producto, un conector con la lógica, un binario.

| Crate | Qué es |
|---|---|
| `ore-hbase-rpc` | El cliente nativo: ZooKeeper mínimo, RPC, `hbase:meta`, `Scan` y `Get`. Sin dependencias nuevas en `Cargo.lock`: `prost` ya está y todo es `std::net`, síncrono y con hilos, como `ore-avatica`. |
| `ore-hbase` | El conector: URL, catálogo, mapeo y tipos, traducción de la petición a un `Scan`, particionado, incremental. |
| `ore-read-hbase` | El binario sobre el esqueleto `Origen` (`ore_driver::origen`). |

Los mensajes **no se escriben a mano**: se generan de los descriptores compilados de los `.proto` del jar de
HBase (`generar_proto.py`, como en `ore-avatica`), solo los que se usan: `ConnectionHeader`, `RequestHeader`,
`ResponseHeader`, `ExceptionResponse`, `ScanRequest`/`ScanResponse`, `GetRequest`/`GetResponse`, `Result`,
`Cell`, `RegionInfo`, `ServerName`, `TimeRange`, `MetaRegionServer`.

**Dirección:** `hbase://zk1:2181,zk2:2181,zk3:2181/espacio?znode=/hbase&timeoutMs=…&paralelo=…&rezago=…`.
El quórum de ZooKeeper admite una lista (en Phoenix solo había un servidor). El camino es el *namespace*
(`default` si falta).

### 7.2 El cliente (`ore-hbase-rpc`)

1. **ZooKeeper, solo lo que hace falta:** apretón de manos de sesión, `getData` de
   `/hbase/meta-region-server` (formato medido en §3.2: `0xFF`, longitud del id, id, `PBUF`, protobuf), `ping`
   y cierre. Sin *watches*: la ubicación se vuelve a leer cuando falla, no se escucha. Unas 200 líneas.
2. **Conexión RPC:** preámbulo `HBas` + versión + código de autenticación, `ConnectionHeader` (usuario,
   servicio `ClientService`), y luego un mensaje por llamada con su `call_id`. **Una llamada en vuelo por
   conexión** (petición y respuesta en orden): no hace falta demultiplexar. Una conexión por RegionServer y por
   hilo lector.
3. **Ubicación de regiones** (`meta`): tabla → regiones, con caché. Empezar **siempre en `tabla,,`** (§3.2).
   Se invalida y se vuelve a leer ante `NotServingRegionException`, `RegionMovedException` y
   `RegionOpeningException`, con espera creciente (100 ms, 200, 300, 500, 1 s…) acotada por `timeoutMs`.
4. **`Scan`:** apertura, continuación con `next_call_seq` (§3.2), cierre. Proyección por familias y
   cualificadores (solo se piden las columnas que la petición lleva: en una tabla ancha es la mayor ganancia),
   `max_versions = 1`, `TimeRange`, tamaño de lote y `max_result_size` por llamada.
   - `client_handles_partials = false`: el servidor entrega la fila entera aunque sea grande.
   - `client_handles_heartbeats = true`: una respuesta vacía con `more_results` no es el final. **Se supone
     así en 2.4; se comprueba en N2** con una tabla de filas muy filtradas.
5. **Reanudar por la última clave.** Las filas llegan ordenadas, así que si un scanner muere
   (`UnknownScannerException`, `OutOfOrderScannerNextException`, caducidad del *lease*, región movida o
   partida) el cliente abre otro **desde la última clave entregada, exclusiva**, y no entrega nada dos veces.
   Es más sencillo y más exacto que en Phoenix, donde reabrir era reiniciar.
6. **`Get` en lote** para el incremental (§7.6).
7. **Errores:** las excepciones del servidor llegan en `ResponseHeader.exception` con su clase. Se mapean a:
   reintentable con refresco de `meta` (las de región), reanudable (las de scanner), de credencial
   (`AccessDeniedException`, fallos de SASL), de operador (`TableNotFoundException`,
   `TableNotEnabledException`), de red (cierre, `CallTimeoutException`) y de contención
   (`RegionTooBusyException`, `CallQueueTooBigException`: espera y reintento). **Corregido en N2:** la
   clasificación manda por la **clase**, no por `do_not_retry` (§N2); los nombres grabados están en
   `pruebas-de-fuego/hbase/trazas/excepciones.json`.
8. **Cancelación y tiempos:** el `Token` del esqueleto se mira entre llamada y llamada; cada llamada tiene su
   tiempo en el *socket*; al cancelar se cierra el scanner (`close_scanner`) si se puede y la conexión si no.
   **Corregido en N2:** cerrar el *socket* desde otro hilo despierta una lectura bloqueada en Linux y **no** en
   Windows; la cancelación lee en rodajas de 100 ms y mira una bandera entre una y otra (§N2).

### 7.3 El requisito de red que no se ve en el laboratorio

El cliente se conecta **al nombre que el RegionServer anuncia en ZooKeeper**. En el laboratorio fue
`127.0.0.1`, y ya mordió (§3.2). En un clúster real son nombres internos (`nodo17.hadoop.local`) que el Job de
ORE puede no resolver ni alcanzar (el puerto 16020 de **todos** los RegionServers, no solo el del maestro). Es
el primer requisito de red de este origen —el problema de las *advertised listeners* de Kafka— y la hoja y la
documentación lo dicen. Se ofrece una **traducción de nombres** en la dirección
(`traduccion=nodo17.hadoop.local:16020=10.0.3.17:16020`) por si el clúster solo es alcanzable por otra ruta.

### 7.4 El conector: filas, claves y tiempo

Una fila de HBase es una fila de OOS. Sus columnas:

- `_clave`: la clave de fila entera (bytes), y sus partes si el mapeo la declara compuesta (§7.5).
- `_ts`: la marca de tiempo más reciente entre las celdas leídas de esa fila (milisegundos desde la época).
- Una columna por cada `familia:cualificador` declarado, con el nombre físico `familia:cualificador`.

**Solo la última versión** en v1. El historial de versiones (una fila por celda) queda fuera.

**Qué se empuja al servidor:** el rango de clave (`eq`, `gt`, `ge`, `lt`, `le`, prefijo, y `in` como varios
rangos) y el de `_ts` (`TimeRange`). Todo lo demás lo evalúa el motor tras la lectura, **declarado por tabla en
`reads`** como en Phoenix, sin filtros de valor en el servidor (lentos y poco fiables en HBase). Un rango solo
se empuja si la codificación de la clave **conserva el orden** (§7.5).

**Particionado:** una parte por región, o por grupo de regiones hasta `paralelo=` (4 por defecto), cada una con
su conexión y su hilo. Se recorta con el rango de clave de la petición; un rango sin regiones no abre nada.

### 7.5 Mapeo y tipos: el diseño propio de este origen

HBase no tiene tipos, así que **nada se infiere en silencio**:

1. **`catalogo`** lista *namespaces* y tablas (`GetTableNames`/`GetTableDescriptors` al maestro), las
   familias de cada una, y **los cualificadores por muestreo acotado** (como el `$sample` de Mongo: tamaño y
   tiempo máximos, repartido por regiones). Cada columna se propone como `String` si **todos** los valores de
   la muestra son UTF-8 imprimible y como `Binary` si no. Se dice que es inferido y de cuántas filas.
   **Nunca propone un número**: 8 bytes pueden ser un `long`, un `double` o un texto, y adivinar es corromper.
2. **La tabla declara el tipo de OOS** (`Peticion.tipos`, como en Mongo M3) **y la codificación de los
   bytes.** Una columna con tipo numérico, fecha o decimal **sin codificación es un error que nombra la
   columna**, nunca una conversión a escondidas.
3. **Codificaciones** (cerradas, cada una con su propiedad de orden):

| Codificación | Bytes | ¿Conserva el orden? |
|---|---|---|
| `utf8` | texto | sí |
| `bytes` / `hex` | lo que haya | sí (lexicográfico) |
| `texto` | número escrito en ASCII (`"42"`) | no (`"9" > "10"`): solo igualdad |
| `i8be`, `i16be`, `i32be`, `i64be` | `Bytes.toBytes(long)` de Java: complemento a dos, big-endian | **solo entre no negativos**: un rango con negativos se parte en dos tramos |
| `u32be`, `u64be` | sin signo | sí |
| `f32be`, `f64be` | IEEE 754, `Bytes.toBytes(double)` | no: solo igualdad |
| `bool` | un byte | n/a |
| `epoch-ms` | `i64be` de milisegundos | como `i64be` |
| `decimal` | `Bytes.toBytes(BigDecimal)`: escala (4 bytes) + entero sin escala | no |
| `json` | texto JSON | no |

   Las de Phoenix (signo invertido, claves compuestas con separador `\0`) se dejan para después: ya las lee
   0071.
4. **Clave compuesta:** una lista de partes, cada una con su codificación y su ancho fijo o su separador.
   El rango solo se empuja por el **prefijo** de partes de ancho fijo y ordenables, igual que la regla de la
   primera columna de la clave en Phoenix.
5. **Decisión pendiente D1 — dónde viaja la codificación.** `Peticion.tipos` lleva `[columna, tipo de OOS]` y
   no tiene dónde poner la codificación. Hace falta **un campo nuevo de la petición** (p. ej.
   `codificaciones: [[columna, nombre]]`) y el atributo correspondiente en la `Table` de OOS. Es el único
   cambio de protocolo de este origen, igual que lo fue `tipos` en Mongo M3; se hace en el hito de tipos (N4)
   con su apartado en 0008. Mientras tanto N3 entrega todo como `String`/`Binary`, que es lo cierto.

### 7.6 Incremental

La marca de tiempo de celda existe en toda tabla (§1). El diseño aprovecha tres hechos medidos (§3.3) y evita
dos trampas:

- **El testigo es un límite, no un máximo.** Se lee con `TimeRange [start+1, end+1)` y `end = T`, donde
  `T = ahora_del_servidor − rezago`. Como una lectura con `end = T` entrega **todo** lo que tenía marca ≤ T, el
  testigo después de leerla es `T`, sin buscar el máximo (HBase no tiene `ORDER BY ts` ni índice por marca).
  Una fila con **marca en el futuro no envenena nada**: no entra hasta que `T` la alcanza.
  - Supuestos (resueltos en N5 y N8): el motor acepta un testigo `field` que es un límite —lee `materializar.rs`: lo guarda
    y lo devuelve como `start`, sin mirar si esta en los datos— y la hora es la del **cliente** (N5; el candidato del
    maestro, `report_end_time_stamp`, no se hizo).
- **Un `Scan` con `TimeRange` devuelve solo las celdas dentro del rango**, no la fila entera: una fila con una
  celda actualizada llegaría con una sola columna. Por eso el incremental son **dos pasos**: (1) un scan por
  rango de tiempo que solo pide las **claves** de las filas con alguna celda en el rango; (2) un `Scan` con
  `MultiRowRangeFilter` por tandas de esas claves para traer la fila **completa** en su última versión. El `_ts` de la fila sale de ahí. El paso 2
  puede traer una marca mayor que `T` (la fila cambió entre medias); es idempotente porque el cambio es
  `upsert` por `_clave`. Medido en N5.
- **Lo que no ve:** los borrados (la fila desaparece y ningún rango de tiempo lo señala, §3.3) y las filas
  escritas con marca **anterior** a `T` después de haber pasado `T` (cargas masivas con la fecha original).
  Los avisos de 0071 §3.17 se repiten. Un refresco completo sí refleja los borrados.
- `cambio`: `respalda: false` (HBase no tiene contadores por tabla), como Phoenix.

### 7.7 Seguridad

| Nivel | Qué es | Decisión |
|---|---|---|
| **SIMPLE** | el cliente *afirma* un nombre de usuario; no hay secreto y los datos viajan en claro | se **rechaza** salvo reconocimiento expreso en la dirección (`simple=acepto`); la hoja lo dice. Es lo que usa el laboratorio. |
| **Kerberos (SASL GSSAPI)** | el habitual en un clúster de empresa; con `hbase.rpc.protection=privacy` cifra el canal | **viable y medido (§3.5), en Rust y en Alpine; hecho en N6** con `libgssapi` detrás de un *feature* de Cargo (`kerberos`) en `ore-hbase-rpc`: la imagen necesita `krb5-libs` y el binario se enlaza dinámicamente. Las tres protecciones (`authentication`, `integrity`, `privacy`) se admiten; la que manda es la del servidor. |
| TLS en el RPC | no existe en HBase 2.4 | **a verificar** que llega con 2.6; no entra en v1. |
| DIGEST (*delegation tokens*) | pensado para trabajos de MapReduce y Spark | fuera. |

**Nada de lo hecho para Phoenix se pierde**: la credencial nunca aparece en errores ni en trazas, el asistente de
credenciales de `ore-serve` tiene su entrada `hbase` (N7) y dice lo que no admite —Kerberos desde la consola, hasta que
haya como llevar un fichero secreto a un Job—, y la hoja de la consola queda para cuando se decida como partir ese arbol.

### 7.8 Hitos

| Hito | Contenido | Prueba |
|---|---|---|
| **N1 · el cliente** (**hecho**, abajo) | `generar_proto.py`; ZooKeeper mínimo; RPC SIMPLE; `meta`; `Scan` y `Get`; **trazas grabadas** del laboratorio como fixtures (`pruebas-de-fuego/hbase/trazas/`) y un RegionServer de mentira en proceso, como `tests/comun` de Avatica | trazas + laboratorio: las 10⁶ filas de §3.1 |
| **N2 · robustez** (**hecho**, abajo) | caché de regiones y reintentos con espera; reanudar por la última clave; *heartbeats*; *lease*; cancelación; tiempos; clases de excepción **grabadas** | laboratorio que parte una región, deshabilita una tabla, para y arranca el RegionServer, y deja caducar un scanner |
| **N3 · el conector** (**hecho**, abajo; **el banco del kit pasa a N4**) | URL, `catalogo` (con muestreo), `leer` a Arrow como `String`/`Binary`, proyección, rango de clave, particionado por regiones; `ore-read-hbase`; `capacidades`; banco del kit | kit contra el laboratorio |
| **N4 · mapeo y tipos** (**hecho**, abajo) | las codificaciones de §7.5, clave compuesta, **D1** en el protocolo, **y el banco del kit** (§N3: el kit mide valores tipados) | cada codificación con bytes que escribe Java (`Bytes.toBytes`) en el laboratorio |
| **N5 · incremental** (**hecho**, abajo) | `_ts`, rango de tiempo, los dos pasos, testigo-límite, `cambio`, rezago | punta a punta con actualizaciones, borrados y una fila fechada en el futuro |
| **N6 · seguridad** (**hecho**, abajo) | SIMPLE con reconocimiento; Kerberos con `libgssapi` (las 7 trampas de §3.5; un solo binario con el *feature*; `krb5-libs` en la imagen; cómo llega el keytab a un Job: decisión propia); avisos | el KDC y el HBase de `pruebas-de-fuego/hbase/kerberos/` con las tres protecciones, más un test de que sin ticket falla en el cliente antes de tocar la red |
| **N7 · cableado** (**hecho salvo la consola**, abajo) | `ore-cli` (`hbase://`), pasarela, `ore-serve` (credenciales), imagen, `ci/compilar-binarios.sh`, el job de CI con el laboratorio, la hoja de la consola (`rubix-platform`) | las guardas de huella y dependencias |
| **N8 · cierre** (**hecho**, abajo) | el README del laboratorio; la ficha reescrita como imagen de lo que es el origen | revisión adversaria de la ficha, como en 0071 |

### 7.9 Fuera de v1

Escritura; historial de versiones; filtros de valor en el servidor; coprocesadores; leer *snapshots* de HFiles
directamente (la vía de volumen, que recorre el almacenamiento sin pasar por los RegionServers y merece su
ficha); tablas con la codificación de Phoenix; réplicas de lectura; `cell_block` y compresión (una mejora de
rendimiento que se mide en N2 y no se promete).

> **De aqui a N7 es el registro de cada hito**: lo que se hizo, lo que se midio y lo que *entonces* no se hacia. Lo que
> el origen es hoy esta en §0; una lista de «lo que no hace» de un hito puede haberse cumplido en el siguiente.

### N1 · hecho (2026-10-21)

El crate `ore-hbase-rpc`: ZooKeeper mínimo (`getData`), RPC con autenticación SIMPLE, `hbase:meta`, `Scan` y
`Get`. Sin dependencias nuevas: `Cargo.lock` solo gana la entrada del propio crate (`prost`, `serde_json` y
`base64` ya estaban). Los 29 mensajes y 5 enumerados de `proto.rs` los genera `pruebas-de-fuego/hbase/generar_proto.py` de
los `.proto` del jar de HBase; ni un número de campo está escrito a mano.

**Qué se prueba, y contra qué**

| Prueba | Qué es | Resultado |
|---|---|---|
| unitarias (16) | tramas de ZooKeeper y de RPC, el nodo de `meta`, la traducción de nombres, las peticiones de `Scan`, una excepción de HBase y una respuesta truncada | pasan |
| **trazas** (`tests/trazas.rs`) | 5 conexiones (2 de ZooKeeper, 3 de RPC, 19 tramas en la del RegionServer) **grabadas de un HBase 2.4.18 de verdad** con un proxy en Python (`grabar_trazas.py`); un servidor falso las repite y exige que el cliente diga **exactamente los mismos bytes** | pasan; 5/5 repeticiones |
| laboratorio (9) | las 4 regiones contiguas, la tabla inexistente, un barrido de 10⁶ filas ordenado, un rango de claves que cruza dos regiones (10 000 exactas), soltar el escáner a medias, `Get` (existente, ausente, fuera de la región), el rango de tiempo | pasan contra HBase vivo |
| sabotajes | empezar el scan de `meta` en `tabla,` (el defecto de §3.2); quitar `include_start_row` | **los dos hacen fallar las trazas** |

**Rendimiento** (compilado en `release`, una conexión, una región tras otra, 10⁶ filas de 100 bytes, el
cliente materializa cada fila): 4,7 a 6,3 s, de ≈160 000 a ≈210 000 filas/s. El X0 con Python midió 3,7 s sin
materializar nada; **el paralelismo por regiones y las asignaciones de cada fila se miden en N3**, cuando haya un
lector que las use.

**Lo que N1 aprendió y no estaba en §3**
- El `Scan` de `hbase:meta` de una tabla de 4 regiones cabe en **una sola llamada** (la traza lo muestra):
  el escáner de `meta` no llega a pedir continuación.
- La clase de la excepción por pedir una clave fuera de la región es
  `org.apache.hadoop.hbase.regionserver.WrongRegionException` (grabada). Las demás clases de §7.2.7 se graban en N2.
- **La traducción de nombres (§7.3) ya existe** (`Opciones::con_traduccion`) y es lo que permite reproducir las
  trazas y correr el laboratorio en puertos desplazados.
- Un fallo mío de método, anotado para el siguiente: restaurar un fichero con `Copy-Item` **conserva su fecha
  antigua**, y `cargo` no recompila; la prueba siguió probando el binario saboteado. Se toca el fichero.

**Lo que N1 NO hace** (es N2): reintentos y refresco de la caché de regiones, reanudar por la última clave,
caducidad del escáner, cancelación, tiempos por llamada más finos que el del *socket*, ZooKeeper con más de un
miembro caído a mitad de sesión, y los latidos (`heartbeat_message`: el cliente los pide pero ninguna traza los
ha producido todavía).

### N2 · hecho (2026-10-21)

El **lector** (`Cliente::leer`): las filas de una tabla en un rango de claves, de una región a la siguiente,
que ante cualquier fallo clasifica, espera y **reanuda desde la última clave entregada, exclusiva**. Como las
filas llegan ordenadas y una fila es atómica, no hay nada más que recordar y no se pierde ni se repite nada.
Añade: `Politica` (la espera de HBase: 100 ms × 1, 2, 3, 5, 10, 20, 40, 100…300; 15 intentos, unos dos minutos y
medio), `Cancelacion`, caché de regiones con `invalidar`, `estado_de_tabla`, y contadores (`llamadas`, `latidos`).

**Qué se prueba, y contra qué**

| Prueba | Qué es | Resultado |
|---|---|---|
| unitarias (25) | la espera, la clasificación con las **siete excepciones grabadas**, la cancelación, el plazo | pasan |
| **cluster simulado** (16, `tests/simulado.rs`) | ZooKeeper, `hbase:meta` y un RegionServer en el proceso, con los mensajes de verdad y fallos inyectados: región partida, arriendo caducado, conexión cortada, servidor caído y que vuelve, que no vuelve, error fatal, servidor saturado, latidos, tabla deshabilitada, servidor que repite filas, cancelar con la llamada dormida, soltar a medias | pasan, 0,45 s; corren en el CI |
| trazas (1) | regrabada con la cabecera nueva y con una lectura del lector que cruza dos regiones (6 conexiones) | pasa, byte a byte |
| **caos contra HBase vivo** (5, `tests/caos.rs`, con `docker`) | 10⁶ filas leídas despacio mientras se **parten las 4 regiones en 8**, mientras se **reinicia el contenedor entero**, mientras se **deshabilita** la tabla, con un **arriendo que caduca de verdad** (10 s) y con un filtro que descarta todo | pasan: 10⁶ exactas, en orden y sin repetir tras partir (40 s) y tras reiniciar (60 s); `TablaDeshabilitada` a los 10,7 s |
| **sabotajes** | reanudar en la clave inclusive y sin la defensa de duplicados · no tirar la caché ante un fallo de región · tratar el scanner perdido como fatal · no avanzar a la región siguiente · no mirar el estado de la tabla | **los cinco hacen fallar las pruebas** (7, 1, 3 y 1 fallos; el cuarto cuelga la prueba) |

**Lo que el laboratorio enseñó y no estaba en el diseño**
- **`do_not_retry` no sirve para decidir.** `UnknownScannerException` y `OutOfOrderScannerNextException` llegan
  con `do_not_retry = true` y, para un lector que reanuda, significan «abre otro». `NotServingRegionException` y
  `WrongRegionException` llegan con `false`. Manda la clase; `do_not_retry` solo para las clases desconocidas.
- **El valor de `table:state` en `hbase:meta` no lleva la marca `PBUF`** (el de `info:regioninfo` sí). El simulador
  lo ponía con ella y sus pruebas pasaban; contra un HBase de verdad, el lector **no detectaba la tabla
  deshabilitada y agotaba los 15 reintentos (174 s)**. Se arregló el cliente y se hizo fiel el simulador.
  Lección: un simulador solo vale lo que valga su fidelidad a lo medido.
- **Cancelar por `shutdown` del *socket* no es portable** (Windows no despierta la lectura): se lee en rodajas.
- Una región que se parte da primero `UnknownScannerException` (el scanner de la madre desaparece) y luego
  `NotServingRegionException` al reabrirla por su nombre viejo: el lector pasa por las dos clases sin intervención.
- Un fallo mío de método, la segunda vez: matar un script de sabotaje a medias (el sistema lo paró por falta de
  memoria) deja el fichero saboteado en disco. Se comprobó y se restauró; los sabotajes ahora restauran en
  `finally`.

**Lo que N2 NO pudo comprobar**
- **Los latidos contra un servidor de verdad.** El cliente los pide y el simulador prueba que sabe seguirlos,
  pero no se consiguió que HBase mandara uno: descarta una región entera en ~100 ms, y con un plazo de llamada
  de 250 y de 60 ms el cliente agotó el tiempo antes de recibir ninguno. Los decide el arriendo del scanner del
  servidor, no el plazo de la llamada. **Sigue sin observarse.**
- `RegionMovedException`, `RegionOpeningException`, `RegionTooBusyException` y `CallQueueTooBigException`: se
  clasifican por su nombre, pero solo el simulador las produce; el laboratorio de un nodo no las genera.
- Varios RegionServers: todo ocurrió en uno. La caché y la traducción por servidor están pensadas para varios,
  y el simulador solo tiene uno.
- La cancelación **no cierra el scanner en el servidor**; caduca solo (10 s aquí, 60 s por defecto).

### N3 · hecho (2026-10-21)

Los crates `ore-hbase` (el conector) y `ore-read-hbase` (el binario): la URL, el catálogo por muestreo, los filtros,
la lectura a Arrow y los cinco verbos de HBase. `Cargo.lock` solo gana esas dos entradas: nada externo.

**Qué hace**
- **URL** `hbase://[usuario@]zk1:2181,zk2/espacio?simple=acepto&…`. `simple=acepto` es **obligatorio**: SIMPLE afirma un
  usuario, sin secreto y sin cifrar, y no se conecta sin que la URL lo reconozca. Una contraseña en la URL se rechaza.
- **Catálogo** (`hbase:meta` lista las tablas, sin pasar por el maestro): por cada tabla habilitada, unas filas de
  cada región (1 000 en total, repartidas) y lo que tienen. Columnas `_clave` y `familia:cualificador`; `String` si
  todo lo visto es texto, `Opaque` (base64) si no. Una tabla deshabilitada se salta con aviso.
- **`leer`**: la clave de fila se empuja como **intervalos** (`eq`, `in`, `lt`, `le`, `gt`, `ge`) y solo se piden las
  regiones y las celdas que hacen falta; todo lo demás (`neq` y los filtros sobre celdas) se evalúa en el conector con
  la semántica de SQL (un nulo no cumple nada, la cadena vacía no es nula). Reparto por regiones en `paralelo`
  hilos (4 por defecto); con `limit`, un hilo. `orderBy` y `like` **no se declaran**.
- **Verbos**: `check`, `explorar`, `catalogo`, `testigo` (`none`) y `cambio` (`sin-contadores`).
- **El plazo de la lectura corta también los reintentos**: un vigilante cancela la lectura al vencer `timeoutMs` y el
  error sale como `tiempo`, no como `cancelada`. Sin eso, un servidor colgado alargaba la lectura hasta dos minutos y
  medio de reintentos.

**Qué se prueba, y contra qué**

| Prueba | Qué es | Resultado |
|---|---|---|
| unitarias (41) | la URL, el esquema, los filtros (intervalos, intersección, residuales), el lote Arrow y el base64 (vectores del RFC 4648), la preparación de la petición, el mapa de errores | pasan |
| **cluster simulado** (16) | el conector entero (`Hbase::leer` y los verbos) contra el simulador de N2: toda la tabla, rango, `in`, filtros sobre celdas, `limit`, `Opaque`, errores sin flujo, tabla vacía, región partida a mitad, plazo, cancelar, rearmar | pasan; corren en el CI |
| **laboratorio** (10, con `docker`) | HBase 2.4.18 de verdad: el catálogo, 10⁶ filas en paralelo, rango de 10 000 filas que cruza dos regiones, filtro por valor, `limit`, tabla inexistente, tipo no admitido, y una tabla `mixta` | pasan |
| el binario | `capacidades`, `check`, `explorar`, `testigo`, `cambio`, `catalogo` y `leer` por stdin/stdout; un error con usuario en la URL **no** lo devuelve | correcto |
| **sabotajes** | no empujar la clave como rango · no evaluar los residuales · `limit` con varios hilos · no rearmar el token · sin vigilante del plazo | **los cinco hacen fallar justo la prueba que les corresponde** (2, 1, 1, 1 y 1) |

**Medido contra HBase** (release, 10⁶ filas de 100 bytes, 4 regiones, un nodo; incluye armar y escribir Arrow):

| `paralelo` | 1 | 2 | 4 | 8 |
|---|---|---|---|---|
| 10⁶ filas (clave + celda) | 9,5 s | 4,7 s | 3,1 s | 3,2 s |

Cuatro es el punto, que es el número de regiones: más hilos que regiones no ayudan. (Phoenix, mismo orden de filas y
cuatro conexiones: 6,2 s; el PQS de 1 GB era el límite.) Solo la clave: 1,9 s. El Arrow de las 10⁶ filas pesa 134 MB.

**Lo que el laboratorio enseñó y no estaba en el diseño**
- **El nombre de la familia lo decide HBase, no uno.** `TestTable` tiene la familia `info0`, no `info`: el catálogo la
  dijo bien (`info0:0`) y mi prueba suponía otra. Pedir una familia que no existe da `NoSuchColumnFamilyException` y
  sale como `esquema` con el mensaje del servidor.
- **`FirstKeyOnlyFilter` funciona** y es lo que se pide cuando solo hace falta la clave (1,9 s las 10⁶).
- **El shell de HBase no escribe bytes:** `put … "\xff\xfe"` guarda los caracteres `ÿþ` en UTF-8 (válido), no los
  bytes. Para escribir bytes de verdad hay que usar la pasarela REST del laboratorio; con el shell, mi prueba de
  «bytes que no son UTF-8» pasaba sin probar nada.
- **Los nulos y la cadena vacía se distinguen**: una celda con valor vacío llega como `""` y una que falta, como nulo.
  A diferencia de Phoenix, donde `''` es `NULL`.
- Un script de `hbase shell -n` aborta en el primer fallo: borrar una tabla que quizá no existe va en una invocación aparte.

**El kit de conformidad pasa a N4.** El kit mide los 14 casos con una semilla *tipada* (`id` entero, `Decimal<12, 2>`,
fechas…) y casi todos leen `id` como entero. Un conector que en N3 solo entrega `String` y `Opaque` no puede
pasarlo, y un modo «solo texto» en el kit lo habría falseado en vez de medirlo. Con las codificaciones de N4 el banco
de HBase puede sembrar valores tipados escribiendo los bytes de Java por REST, y entonces el kit sí mide. Hasta
entonces, la conformidad de N3 es la de sus propias pruebas.

**Lo que N3 NO hace**
- Tipos: todo es `String` u `Opaque` (N4). Una columna declarada `Integer` es `esquema` con la columna nombrada.
- `start`/`end` solo sobre `_ts` (N5); sobre cualquier otra columna, `operador`. `orderBy`: no se declara.
- No cataloga una familia o un cualificador que no salga en las 1 000 filas de la muestra; lo dice en un aviso.
- No lista las tablas por el maestro (usa `hbase:meta`): una tabla sin regiones asignadas no aparece.
- Sin estimación de filas (`estimar: false`) y sin `like`.
- **Sin integrar en la plataforma**: ni `ore-cli`, ni la pasarela, ni la imagen, ni `ci/compilar-binarios.sh`, ni
  la hoja de la consola (N7).

### N4 · hecho (2026-10-21)

Los tipos de HBase: de unos bytes sin tipo a un `Integer`, un `Decimal<12, 2>` o un `DateTimeTz`, sin adivinar.

**D1, resuelta con una desviacion que conviene decir.** Lo acordado era un campo nuevo de la peticion
(`codificaciones: [[columna, nombre]]`) **y un atributo nuevo en la `Table` de OOS**. El campo esta hecho tal cual
(`Peticion.codificaciones`, junto a `tipos`). El atributo **no**: la fuente de cada codificacion es el
**`physicalType`** de la columna, que ya es una clave admitida en toda `Table` («el tipo del origen») y que el
inductor ya escribe desde el catalogo. En HBase el tipo fisico de una celda *es* como estan escritos sus bytes.
Asi no hay `apiVersion` nuevo ni cambio de gramatica (que vive en su propio repositorio, `vendor/oos`). Si se prefiere
un atributo propio (`encoding:`), eso si es una version de OOS nueva: el transporte no cambia.

**Las 14 codificaciones** (cerradas; las de `org.apache.hadoop.hbase.util.Bytes`): `utf8`, `bytes` (`Opaque`),
`texto` (el valor escrito: `"42"`), `i8be` `i16be` `i32be` `i64be`, `u32be` `u64be`, `f32be` `f64be`, `bool`
(`0xFF`/`0x00`), `epoch-ms` (`Date`, `DateTime`, `DateTimeTz`) y `decimal` (`BigDecimal`: escala y valor sin escala).
Un tipo que no es `String` ni `Opaque` **sin codificacion es un error de `esquema` que nombra la columna** y lista las que hay.
Una pareja imposible (un `Integer` leido como `f64be`) tambien. Un valor que no cabe (4 bytes donde van 8, un `u64` que
pasa de `int64`, un decimal con mas decimales que la escala) es un error que dice **columna y clave**: ni nulo ni redondeo.

**El catalogo** propone `String` u `Opaque` y escribe en el `physicalType` `utf8` o `bytes`; **nunca un numero**. El usuario
cambia `type: Integer` y `physicalType: i32be` en la `Table` que el inductor le dejo.

**Claves enteras y el orden de los bytes.** En complemento a dos los negativos (`0x80…`) ordenan **despues** de los positivos
(`0x00…`): `id < 5` son dos tramos de bytes. El conector los parte, y tambien convierte microsegundos a milisegundos con el
redondeo que toca. Solo se empujan las claves cuyo orden de bytes es el de los valores (`utf8`, `bytes`, `u*`, `i*`,
`epoch-ms` de un instante); en `f*`, `decimal`, `texto`, `bool` el filtro se evalua sobre la clave decodificada.
Un rango que cruza el cero abre un scanner **vacio** en la region cuyo hueco de bytes lo contiene: es lo correcto sin
conocer los datos.

**El defecto que encontro el kit.** Un `Scan` con columnas concretas **no devuelve las filas que no tienen ninguna de ellas**.
`SELECT id, nombre` perdia en silencio las filas sin `nombre`, y `nombre IS NULL` no encontraba nada. Se lee, por intervalo,
**un flujo de claves (`FirstKeyOnlyFilter`) y otro de celdas, en paso**, y la fila sale con nulos si el segundo no la trae
(`completa.rs`). Coste medido: 10⁶ filas con 4 hilos, de 3,1 a 3,8 s (+20 %); con 1 hilo, de 9,5 a 14,6 s. Solo la clave no paga nada.

**Opciones de la URL nuevas:** `familia=f` (un nombre sin `:` es el cualificador de esa familia) y `clave=id` (otro nombre para
`_clave`): el kit pide `id`, `texto`… y una tabla de una sola familia no necesita `familia:cualificador`.

**El kit de conformidad, por fin.**

| | pasa | falla | no aplica |
|---|---|---|---|
| el conector, los 14 casos | **11** | **0** | 3 (9: no hay SQL que escriba · 10: nada lento que cancelar · 14: no estima) |
| con `ore-federation` delante | **8** | **0** | 2 (la url no lleva clave · las colecciones son de los almacenes de objetos) |

Antes de arreglar la lectura doble eran **7 pasan y 4 fallan**, las 4 por la misma causa. El banco siembra por la pasarela REST con
los bytes de `ore-hbase` (`HBASE_KIT_URL`, `HBASE_KIT_REST`). Dos limites de HBase que el banco tiene que respetar:
**una fila sin celdas no existe** (la fila 7 de la semilla es toda nula: se le pone una celda marcadora `f:id`) y **un
instante son milisegundos** (`Banco::milisegundos`). `grande` (10⁶ filas) se carga en una sola region.

**Que se prueba, y contra que**

| Prueba | Resultado |
|---|---|
| unitarias (82) | las 14 codificaciones con los vectores de Java, la matriz tipo/codificacion, los intervalos de claves enteras, la comparacion de SQL (NaN, -0.0), la mezcla de flujos |
| simulador (19 + 12) | todos los tipos de punta a punta con una clave `int` de -200 a 199, rangos que cruzan el cero, `in`, filtros numericos (no de texto), errores con columna y clave, filas con huecos |
| **bytes escritos por Java** (4, `tipada.rb`) | `int`, `short`, `long`, `float`, `double`, `boolean`, `BigDecimal`, milisegundos, UTF-8 y texto leidos con su valor; rangos exactos sobre claves `int` incluidos `MIN_VALUE` y `MAX_VALUE`, con regiones partidas por `00000000` y `80000000` |
| laboratorio completo (14) | pasan; 10⁶ filas con 1, 2, 4 y 8 hilos: 14,6 · 6,2 · 3,8 · 3,2 s |
| **sabotajes** (7 + 1) | un rango sin partir en signo · enteros comparados como texto · `i32be` sin signo · decimal sin escala · tipo sin codificacion · clave decimal empujada · redondeo de ms al reves · **sin mezclar flujos**: los ocho hacen fallar justo lo que deben |

#### La clave compuesta

La clave de muchas tablas reales es `usuario#dia`. Sus **partes se declaran como columnas** `_clave#1`, `_clave#2`…, cada una
con su `type` y su `physicalType` = `<codificacion>[:<ancho>|:fin=<hex>]`: `utf8:fin=23` (acaba en `#`), `i32be` (4 bytes).
Sin huecos en la numeracion; una parte no declarada o una clave que no tiene la forma dicha es `esquema` y nombra la clave.

- **Lectura.** La clave se trocea una vez por fila; cada parte se decodifica con su codificacion y sale como columna tipada.
- **Empuje.** Un `eq` en las primeras partes es un recorrido por prefijo; si la siguiente parte es un entero de ancho fijo, su
  rango se suma al prefijo (partido en dos tramos si cruza el cero, como en N4). Lo empujado es **un superconjunto**:
  **todos** los filtros sobre partes se vuelven a evaluar sobre los valores decodificados, asi que el empuje nunca decide.
- **Un recorrido por prefijo puede abrir un scanner vacio** en el hueco de bytes que cierra una region (`beto#` empieza en el
  hueco entre `ana…` y la region siguiente): correcto, y no se tocan las regiones ajenas.

| Capa | Prueba |
|---|---|
| unidad | troceo, intervalos de prefijo, sucesor del prefijo |
| simulador (`compuesta.rs`, 10) | partes como columnas, `eq` + rango que cruza el cero, texto no empujado, solo la 2.ª parte, imposible, mal formada, declaracion incompleta, paralelo |
| **Java** (`compuesta`, `tipada.rb`) | claves `usuario#` + `int` escritas con `Bytes.add`, regiones partidas por usuario, rango `-1..0` de un usuario |
| **sabotajes** (3) | troceo sin consumir el separador · prefijo sin incrementar · partes sin reevaluar: los tres hacen fallar justo lo que deben |

**Lo que el laboratorio enseno y no estaba en el diseno**
- Un script de JRuby dentro de `hbase shell -n < tipada.rb` es el oraculo de las codificaciones: escribe con `Bytes.toBytes` de Java
  y se lee con las nuestras. Reproduce `Boolean` como `0xFF`/`0x00`, el `BigDecimal` como escala + `toByteArray`, etc.
- Los cuatro fallos del kit eran uno solo, y ninguna prueba mia lo habia visto: el kit existe por esto.
- El simulador ignoraba la restriccion de columnas, igual que ignoraba la ausencia de `PBUF` en N2: **un simulador vale lo que
  valga su fidelidad**. Ahora la respeta y tiene `FirstKeyOnlyFilter`.

**Lo que N4 NO hace**
- Las codificaciones de Phoenix (signo invertido, separadores `\0`): ya las lee 0071.
- `Time` (no tiene codificacion binaria acordada) y los tipos compuestos.
- `like`, `orderBy`, `estimar`.
- La integracion en la plataforma (N7).

### N5 · hecho (2026-10-21)

**El incremental sin pedirle nada al usuario.** Toda celda de HBase tiene marca de tiempo, asi que la columna sintetica
`_ts` (la mayor marca de las celdas de la fila, en milisegundos) es el cursor de cualquier tabla: `witness: field`,
`field: _ts`. El catalogo la lista (`DateTime`, `epoch-ms`) pero **no declara el incremental**: lo declara el dueño de la
tabla, porque no ve los borrados y eso se elige, no se descubre.

| Pieza | Que hace |
|---|---|
| `incremental.rs` | `start`/`end` → rango de tiempo de celda `[start + 1, end + 1)`: `start` exclusivo, `end` inclusivo (0016). Solo sobre `_ts`; sobre otra columna, `operador` y por que. Un `start` que no es un entero, o un filtro sobre `_ts`, tambien |
| `Cambiadas` | los **dos pasos**: (1) scan con el rango de tiempo y `FirstKeyOnlyFilter` → claves de las filas con alguna celda en el rango; (2) por tandas de 1000 claves, un scan con `MultiRowRangeFilter` (un rango de una fila por clave) → las filas **completas**, de una pasada. Una fila que se borro entre los dos pasos no sale. Con solo `_clave` se queda en el paso 1 |
| `ore-hbase-rpc::filas` | el filtro: cuatro campos protobuf escritos a mano (no se genera `Filter.proto` para ellos); un `Scan` con sus reintentos y su cancelacion, **no un `Get` por clave** |
| `testigo` | con `cursor: _ts`, `field` y el valor `reloj − rezago` (`rezago=` en la URL, 5000 ms por defecto, como Phoenix). **Es un limite, no un maximo**: una lectura con `end = T` entrega todo lo de marca ≤ T, asi que el testigo es `T` sin buscar el maximo. Una fila con marca del año 2255 no envenena nada: no entra hasta que el reloj la alcanza |
| `cambio` | sigue `respalda: false`: HBase no tiene contadores por tabla |

**Lo que se midio contra HBase 2.4.18 y no estaba en el diseño**
- **El tope del rango es un `long` con signo.** `u64::MAX` llega como −1 y el servidor contesta `IOException: Timestamp
  cannot be negative`. El cliente, como cualquiera, **reintenta un `IOException`**: 15 intentos, 200 s para un error de
  programacion. Ahora el tope se acota a `Long.MAX_VALUE`, y el simulador rechaza lo mismo que el servidor.
- `TimeRange` con `FirstKeyOnlyFilter`, y `MultiRowRangeFilter` con una clave por rango, **hacen lo que se dedujo**; lo
  primero no estaba medido y era la mitad del diseño. Una celda actualizada hace cambiar a su fila, que llega con la celda
  que **no** cambio (la parte que un `Scan` con rango de tiempo devolveria a medias), y su `_ts` es la del update.
- 2 500 filas por el incremental (tres tandas): **245 ms**.

| Capa | Prueba |
|---|---|
| unidad | exclusivo/inclusivo, sin `start`/`end`, rango vacio, otra columna, extremos, el testigo y su rezago |
| simulador (`incremental.rs`, 13) | los bordes, filas completas aunque una celda cambiara fuera del rango, `_ts` = mayor marca, actualizaciones, solo `start`/solo `end`, sin cambios, solo la clave (un paso), paralelo, `_ts` sin rango, **fila borrada entre los dos pasos**, **mas de una tanda**, errores |
| **servidor de verdad** (3) | marcas explicitas escritas con `put … , ts`: rango, actualizacion, borrado, **fila fechada en 2255 con el testigo como tope**; solo la clave; tandas sobre `TestTable` |
| **sabotajes** (9) | `start` inclusivo · `end` exclusivo · sin el paso 2 · marca = la menor · una tanda llena corta la lectura · el paso 2 sin filtro de filas · tope sin acotar · rango vacio que se pregunta · testigo sin rezago |

**Lo que N5 NO hace, y los supuestos que quedan**
- **El reloj es el del cliente, no el del servidor.** El candidato del diseño (`report_end_time_stamp` del estado del
  clúster) pide hablar con el maestro, y el rezago ya cubre unos segundos de desfase. Si el reloj del trabajo va
  adelantado mas que el rezago, se pierden filas escritas justo despues del testigo; **se arregla con `rezago=`**, y la
  hoja de la consola lo dira junto al aviso de los borrados.
- **Que el motor acepte un testigo `field` que es un limite** y no un valor presente en los datos (0016) sigue siendo un
  supuesto: se comprueba cuando el motor lea de HBase (N7).
- Lo que el incremental no ve (§3.4): **los borrados**, y las filas escritas con una marca **anterior** al testigo cuando
  este ya paso (cargas masivas con la fecha original, o un cliente con el reloj atrasado). Un refresco completo si los
  refleja.
- Una fila cuya celda cambia entre los dos pasos puede traer una marca mayor que `end`: es idempotente (`upsert` por
  clave), pero el testigo siguiente la volvera a traer una vez.

### N6 · hecho (2026-10-21)

**Kerberos (SASL/GSSAPI) en el cliente nativo, con las tres protecciones, medido contra un HBase 2.4.18 con Kerberos.**

| Pieza | Que hace |
|---|---|
| `sasl.rs` | el `Contexto` (paso, completo, envolver, desenvolver) y `Kerberos { principal, keytab, servicio }`; la `Capa` que se acuerda: del byte de capas que ofrece el servidor se elige la mas fuerte |
| `rpc.rs` | el protocolo, **sin saber de Kerberos**: preambulo con el codigo `81`; el apreton antes de la `ConnectionHeader` con las trampas 1–5 de §3.5 (el mensaje vacio, la oferta envuelta, la eleccion con `authzid` vacio, el servidor que no contesta, el mensaje envuelto *con su propia longitud*); un rechazo llega con su clase y su texto |
| `gss.rs` | el contexto de verdad: `libgssapi` sobre `libgssapi_krb5`, detras del *feature* `kerberos`. **Sin el feature el crate no pide ninguna biblioteca del sistema** y pedir Kerberos es un error que lo dice. El servicio es `hbase@<nombre que el RegionServer anuncia>` (trampa 7), no el de la traduccion |
| URL | `auth=kerberos`, `principal=`, `keytab=` (la **ruta** de un fichero: la URL no lleva secretos), `servicio=`. `simple=acepto` y `auth=kerberos` son lo contrario, y `usuario@` con Kerberos se rechaza (el usuario es el principal) |
| errores | `Error::Autenticacion` → `credencial`; no se reintenta. Sin ticket falla en el cliente, **antes de tocar la red** |

**Dos decisiones**
- **Un keytab por proceso.** La GSSAPI lo lee de `KRB5_CLIENT_KTNAME`, que es del proceso entero; se fija una vez y otro distinto despues es un error y no un cambio silencioso. Un driver lee con una identidad; si hicieran falta varias, serian procesos distintos.
- **Como llega el keytab a un Job** es de N7 (un secreto montado como fichero, con su ruta en `keytab=`). Lo que no se hace es meterlo en la URL.

| Capa | Prueba |
|---|---|
| unidad | las capas, el usuario del principal, la URL (`auth` con y sin principal; lo que no se mezcla) |
| protocolo (3) | un servidor falso que habla como el de HBase y un contexto de juguete: **los bytes de cada mensaje** en las tres capas (1, 2 y 4), y un rechazo con su clase |
| **servidor de verdad** (4 × 3 capas) | `authentication`, `integrity` y `privacy`: tres regiones en paralelo (**cada conexion hace su apreton**), empuje por rango, incremental y testigo, catalogo, y un principal sin credenciales → `credencial` en menos de 30 s |
| compilacion | Debian (glibc) y **Alpine/musl** con `-C target-feature=-crt-static`, `krb5-dev` y `clang-dev`; el binario de Alpine hace el catalogo contra el servidor con Kerberos |

Los laboratorios: `pruebas-de-fuego/hbase/kerberos/` (`kerberos.sh`, `kt.rb`, `probar_capas.sh`). La prueba corre **dentro** de la red `krb` (contenedor con `krb5-user`, el codigo montado y `HBASE_LAB_KERBEROS_URL`).

**Lo que N6 NO hace**
- *Active Directory* como KDC (MIT y AD difieren en tipos de cifrado y en el PAC) y un ZooKeeper que exija SASL: siguen sin medir (§4).
- Renovar el ticket de una lectura de muchas horas: la GSSAPI lo hace con el keytab, pero **no se ha medido** una lectura mas larga que la vida del ticket (10 h en el laboratorio).
- El binario de la imagen con el *feature* y `krb5-libs` (N7). Una compilacion local sin krb5 sigue siendo SIMPLE.
- Que el DNS inverso importe con un `krb5.conf` sin `rdns = false`: sigue sin medir (trampa 7).

### N7 · hecho salvo la hoja de la consola (2026-10-21)

**HBase entra en la plataforma como los otros tres.** La pasarela descubre los conectores por su carpeta
(`/opt/ore/conectores/ore-read-*`): el tipo `hbase://` aparece al copiar el binario, sin tocar ninguna lista.

| Pieza | Que hace |
|---|---|
| `ci/compilar-binarios.sh` | `ore-read-hbase` es el **unico binario dinamico**. Kerberos pide `libgssapi_krb5` del sistema y eso, `-C target-feature=-crt-static`, es de toda la compilacion: puesto en la de los demas, los de las imagenes `scratch` (`ore-iam`, `ore-postgres`…) dejarian de ser estaticos. Se compila aparte (su `RUSTFLAGS`, su carpeta `dinamico/`, `--features kerberos`) y el resto sigue igual; pedir solo el dinamico no compila los demas |
| `Dockerfile` | `krb5-dev` y `clang-dev` en `herramientas`; `krb5-libs` y **`libgcc`** en `drivers`; `COPY ore-read-hbase` en la carpeta de los conectores. El `krb5.conf` lo pone quien opera la celda |
| `ci.yml` | en `conectores`: el laboratorio de HBase, el kit del conector y el de la pasarela; el job `hbase-kerberos` (las tres protecciones, `probar_capas.sh`, que ahora falla si no pasan los cuatro) y `imagen` lo espera |
| `ore-serve` | el formulario de conexion `hbase`: **SIMPLE reconocido en la URL** (no hay credencial que guardar) con los pasos (quorum de ZooKeeper, red hasta los RegionServers y `traduccion=`, los tipos) y lo que no se admite, con su porque. Un `usuario@` en la autoridad y un `keytab=` se tratan como credencial; el `principal=`, no |
| `ore-cli` | `hbase://` es su propio tipo, comprobado |

**Lo que el cableado enseno**
- **Un binario dinamico necesita mas que `krb5-libs`: tambien `libgcc`.** En un `alpine:3.22` con solo `krb5-libs` ni arranca
  (`Error loading shared library libgcc_s.so.1`). Medido, antes de que la imagen lo descubriera.
- Los guardianes de huella (`prueba-huella-de-los-binarios` 15/15, `prueba-huella-de-la-imagen`) pasan con el binario nuevo:
  28 binarios de 27 paquetes. `Cargo.lock` sube en `libgssapi` y compania sin tocar el cierre de `ore` (`dependencias`).
- El binario hecho **con el guion de la imagen** (perfil `imagen`, 2,2 MB) arranca en un `alpine:3.22` con `krb5-libs` y
  `libgcc` y lee el catalogo contra el HBase con Kerberos. Con SIMPLE: kit del conector 11/0/3, y **la pasarela 8/0/2**
  (la copia completa, el catalogo con `_ts`, la cancelacion y la cola).

**Lo que N7 NO hace**
- **La hoja de la consola** (`rubix-platform`): ese arbol tiene trabajo ajeno sin confirmar, y la hoja de Phoenix tampoco
  esta confirmada. Cuando se decida como partirlo, la hoja de HBase copia la de Phoenix (`ConnectionStep.tsx`,
  `registry.ts`) con los campos de la URL de arriba.
- **Kerberos desde la consola.** El keytab es un fichero y el conector corre en un Job sin tus ficheros: es el mismo hueco que el
  *secure connect bundle* de Astra y la CA privada de Phoenix, y se cierra una sola vez, para los tres, cuando haya como llevar
  un fichero secreto hasta un Job. Mientras, en un despliegue propio se montan el keytab y `krb5.conf` en la carpeta de los drivers.
- **La NetworkPolicy** de los drivers tiene que dejar salir a ZooKeeper (2181) y a los RegionServers (16020): es de quien opera la celda.
- Ejecutar el job `hbase-kerberos` y los pasos nuevos de `conectores` en el CI de verdad: solo se han corrido sus piezas en local.

### N8 · hecho (2026-10-21)

**El README del laboratorio** (`pruebas-de-fuego/hbase/README.md`): como se levanta, que tablas hay y quien las crea, las
variables de las pruebas, el kit, Kerberos y las trampas que ya mordieron. **La ficha**, con §0 como imagen de lo que es
el origen y los apartados que el trabajo dejo viejos corregidos.

**La revision adversaria** (se intento romper lo que la ficha afirma, contra el codigo y contra el motor):

| Se afirmaba | Se comprobo | Resultado |
|---|---|---|
| «el motor acepta un testigo que es un limite» (N5, abierto) | se leyo `materializar.rs`: el testigo se guarda, se compara y se devuelve como `start` | **cierto, con dos precisiones.** Se compara **como texto** (`v.as_str() < actual`): vale porque una marca en milisegundos tiene 13 cifras hasta el año 2286. Y la fusion por clave (`fundir`) pide `cursor` **y** clave: la tabla tiene que declarar las dos, que es lo que §0 dice |
| «el incremental ahorra trabajo» | el testigo cambia con el reloj, asi que la cabecera de la copia **nunca** es la misma | **matiz**: el atajo de «nada ha cambiado» no corta nunca; cada refresco hace el paso 1 (un scan de claves por rango de tiempo, que sin cambios no devuelve nada). Barato, pero no cero. HBase no tiene como saber si una tabla cambio (`cambio` no respalda) |
| el incremental respeta los filtros | se probo con filtros de clave, de valor, `limit` y clave compuesta | **cierto** (tres pruebas nuevas); un `limit` corta pronto y el lector cierra lo que abrio |
| «la credencial no sale» | el caso 12 del kit, 42 salidas; un keytab es una **ruta** | **cierto**; el principal puede salir (no es secreto) |
| «`ore-read-hbase` arranca en la imagen» | se compilo con el guion de la imagen y se arranco en un `alpine:3.22` con solo `krb5-libs` | **fallaba**: faltaba `libgcc`. Corregido en el `Dockerfile` |

**Lo que sigue sin comprobarse, dicho**: el recorrido completo por `ore` (`materializar` con un origen HBase) —el arbol de
`vendor/oos` no esta en esta maquina—; el CI de verdad (sus pasos nuevos se han corrido por piezas); la hoja de la consola;
una lectura mas larga que la vida del ticket de Kerberos; Active Directory.
