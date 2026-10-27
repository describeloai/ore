# El laboratorio de HBase (ficha 0072)

Un HBase 2.4.18 independiente (ZooKeeper embebido) en Docker, con las pasarelas REST y Thrift2, y todo lo que el origen
`ore-read-hbase` se prueba contra el: las sondas con que se midio (X0), las trazas grabadas, las tablas de las pruebas de
tipos, de clave compuesta y de incremental, y el laboratorio con Kerberos. La ficha es
[`docs/products/0072-origin-hbase.md`](../../docs/products/0072-origin-hbase.md); esto es como se usa.

## Levantarlo

```
docker build -t ore-hbase-lab pruebas-de-fuego/hbase          # ESPEJO=https://archive.apache.org/dist en el CI
docker run -d --name hb -p 2181:2181 -p 16000:16000 -p 16020:16020 -p 16030:16030 -p 8080:8080 -p 9090:9090 ore-hbase-lab
until [ "$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:8080/version)" = 200 ]; do sleep 3; done
```

Los puertos se publican con el mismo numero dentro y fuera y HBase se anuncia como `127.0.0.1`: el cliente nativo descubre
los RegionServers por ZooKeeper y se conecta **al nombre que ellos anuncian** (ficha §7.3). Si publicas otros puertos
(`-p 22181:2181 -p 26020:16020`), dilo con `traduccion=127.0.0.1:16020%3D127.0.0.1:26020` en la URL.

`arranque.sh` vigila al maestro y lo levanta si cae (en una maquina cargada el HBase independiente a veces sale antes de
tiempo: `docker start` otra vez).

## Que hay aqui

| Fichero | Para que |
|---|---|
| `Dockerfile`, `hbase-site.xml`, `arranque.sh` | el laboratorio (sesion de ZooKeeper de 600 s, *lease* de 10 s: lo justo para que las pruebas de robustez no duren una hora) |
| `generar_proto.py` | los mensajes de la ruta nativa, sacados de los `.proto` del jar de HBase (`ore-hbase-rpc/src/proto.rs`) |
| `grabar_trazas.py`, `trazas/` | los bytes reales de un servidor, grabados: `nativo.json` y `excepciones.json` (las clases de excepcion con su `do_not_retry`) |
| `sonda_*.py` | las mediciones de X0: REST, REST con semantica, Thrift2, nativa, excepciones |
| `tipada.rb` | las tablas `tipada` (clave `int`, partida por los bytes del signo) y `compuesta` (`usuario#` + `int`), escritas por **Java** con `Bytes.toBytes`: el oraculo de las codificaciones |
| `kerberos/` | el laboratorio con Kerberos (abajo) |

## Las pruebas contra el laboratorio

```
export HBASE_LAB_ZK=127.0.0.1:2181          # 127.0.0.1:22181 si publicaste otro puerto
export HBASE_LAB_TRADUCCION=127.0.0.1:16020=127.0.0.1:26020     # solo con otros puertos
export HBASE_LAB_CAOS=hb                    # el contenedor: las pruebas crean sus tablas con `docker exec … hbase shell`
cargo test -p ore-hbase --test laboratorio -- --test-threads=1
```

Sin `HBASE_LAB_ZK` esas pruebas no hacen nada (pasan). Las tablas:

| Tabla | La crea | Para |
|---|---|---|
| `TestTable` | `hbase pe … sequentialWrite` (10⁶ filas, 4 regiones; ver la ficha §6) | el barrido, el paralelismo, el rango de clave |
| `mixta` | la prueba | texto, bytes que no son UTF-8 y celdas que faltan |
| `tipada`, `compuesta` | la prueba (`tipada.rb`) | las 14 codificaciones; la clave compuesta |
| `cambios` | la prueba | el incremental con marcas explicitas, una actualizacion, un borrado y una fila fechada en 2255 |

El kit de conformidad (`crates/ore-conector-kit`) siembra su propia tabla por REST:

```
HBASE_KIT_URL='hbase://127.0.0.1:2181?simple=acepto' HBASE_KIT_REST=http://127.0.0.1:8080 \
  cargo run -p ore-conector-kit --features hbase -- --conector <ruta a ore-read-hbase> --banco hbase [--pasarela <ruta a ore-federation>] --exige todos
```

Medido: el conector 11 pasan, 0 fallan, 3 no aplican; con la pasarela delante, 8 / 0 / 2. (`--conector` lleva la ruta
**absoluta**: se resuelve desde otro directorio.)

## Kerberos

`kerberos/`: un KDC de MIT (`ORE.TEST`) y un HBase con Kerberos en la red Docker `krb`, con `lector` (clave `lector`,
y `lector.keytab`) e `intruso`.

```
bash pruebas-de-fuego/hbase/kerberos/kerberos.sh privacy          # o integrity, o authentication
docker cp pruebas-de-fuego/hbase/kerberos/kt.rb hbk:/tmp/kt.rb && docker exec hbk /opt/hbase/bin/hbase shell -n /tmp/kt.rb
```

Las pruebas corren **dentro** de la red `krb` (un contenedor con el codigo montado, `krb5-user`, `libkrb5-dev` y `clang`):
`kerberos/probar_capas.sh [integrity authentication privacy]` hace todo, para cada proteccion, y sale con error si no pasan los
cuatro tests de Kerberos. Compila con `--features kerberos`; en Windows no se compila fuera de Docker.

## Trampas que ya mordieron

- `hbase shell -n` **aborta en el primer fallo**: borrar una tabla que quiza no existe va en su propia invocacion.
- El shell de HBase **no escribe bytes que no sean UTF-8** (`"\xff"` se convierte en U+00FF): para eso, la pasarela REST.
- `hbase:meta` no esta ordenada por bytes: leer las regiones de una tabla empieza en `tabla,,` y no en `tabla,`.
- Cerrar un *socket* desde otro hilo no despierta una lectura bloqueada en Windows: la lectura va por rodajas de 100 ms.
- Un tope de tiempo por encima de `Long.MAX_VALUE` llega como −1 y HBase contesta `Timestamp cannot be negative`.
- Un binario dinamico en Alpine (Kerberos) necesita `krb5-libs` **y `libgcc`**.
- Con `docker cp` y `docker run -v` en Git Bash: `MSYS_NO_PATHCONV=1` y rutas de Windows (`C:/…`), no las de MSYS.
- Un `RUSTFLAGS` con `-crt-static` apagado es de **toda** la compilacion: por eso `ore-read-hbase` se compila aparte.
