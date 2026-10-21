# 0072 · Origen Apache HBase

**Leer Apache HBase directamente** —tablas de celdas versionadas, sin esquema— como origen de ORE, por la
ruta que decida este documento. El origen Phoenix ([`0071`](0071-origin-phoenix.md)) ya cubre HBase **con
Phoenix encima** (tipos, catálogo y SQL); este es el que falta: las tablas HBase que **no** son de Phoenix.

**Estado:** X0 a medio hacer (2026-10-21), contra HBase 2.4.18 en Docker. Medidas las rutas REST, Thrift2 y
nativa **sin autenticación**. **Kerberos no está medido** (§4): es la incógnita que decide el alcance.
Nada está construido todavía.

Sigue a [`0008`](0008-el-protocolo-del-driver.md), [`0016`](0016-el-testigo-y-el-rango.md),
[`0069`](0069-origin-mongodb.md), [`0070`](0070-origin-cassandra.md) y [`0071`](0071-origin-phoenix.md)
(el patrón: medir primero, construir después).

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

## 4. Lo que NO está medido

- **Kerberos / SASL.** Es la incógnita que más pesa: un HBase de empresa casi siempre lo lleva. Con la
  documentación de Cloudera: el REST **no** viene con Kerberos de serie (autenticación simple hasta que se
  configure a mano, con SPNEGO), el Thrift lo deja Cloudera Manager preparado con suplantación (`doAs`) pero
  sin cifrar el tramo del cliente, y HBase exige Kerberos también en ZooKeeper. Candidatos en Rust:
  `cross-krb5` (SSPI en Windows, GSSAPI en Linux) y `libgssapi`: ambos dependen de librerías del sistema, lo
  que choca con la imagen sin dependencias nativas. **Pendiente: un KDC en Docker y un HBase con Kerberos.**
- La semántica de §3.3 **por la ruta nativa** (rango de tiempo en `Scan`, tombstones con scan crudo).
- Lectura **de una tabla de Phoenix en crudo** (las claves y los enteros llevan la codificación de Phoenix).
- Compresión, `cell_block`, varias versiones, filtros de valor, TTL y compactaciones.
- Una región que se mueve o cae a mitad de lectura; permisos (ACL); varios nodos; Cloudera, EMR, HDInsight.
- **Qué rutas tienen los clientes reales.** No se puede medir en el laboratorio.

## 5. Lo que ya se puede decir

1. La ruta **nativa** es viable en Rust sin JVM y es la que más rinde; sin autenticación cuesta poco.
2. REST y Thrift2 sirven como atajo, pero **dependen de que alguien los haya desplegado** y, con Kerberos,
   se complican igual.
3. **Lo que decide el alcance es Kerberos**, no el protocolo.
4. El *mapeo de tipos* es el diseño propio de este origen y todavía no está diseñado.

## 6. Laboratorio

`pruebas-de-fuego/hbase/`: `Dockerfile`, `hbase-site.xml`, `arranque.sh` (HBase vigilado + REST 8080 +
Thrift2 9090), y las sondas `sonda_rest.py`, `sonda_rest_semantica.py`, `sonda_nativa.py`,
`sonda_thrift2.py`. Los `.proto` y el IDL se sacan de los jars (`hbase-protocol-2.4.18.jar`,
`hbase-thrift-2.4.18.jar`) y se compilan con `grpcio-tools` y `thriftpy2`.

```
docker build -t ore-hbase-lab pruebas-de-fuego/hbase
docker run -d --name hb -p 2181:2181 -p 16000:16000 -p 16020:16020 -p 16030:16030 -p 8080:8080 -p 9090:9090 ore-hbase-lab
docker exec -d hb bash -c '$HBASE_HOME/bin/hbase pe --nomapred --rows=1000000 --presplit=4 --valueSize=100 sequentialWrite 1'
```
