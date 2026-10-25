# Oracle: el laboratorio y el repro del fallo del driver (ADR 0074)

## El laboratorio

Una imagen de la comunidad (`gvenzl`), Oracle 23ai Free; acepta conexiones a los ~35 s.

```
docker run -d --name ora23 -p 1521:1521 -e ORACLE_PASSWORD=sonda -e APP_USER=ore -e APP_USER_PASSWORD=ore \
  gvenzl/oracle-free:23-slim-faststart
```

Los casos 8 y 10 del kit (la consulta que sigue viva, cancelar) necesitan que el usuario pueda matar su sesión y
verla; un cliente no lo daría, el laboratorio sí:

```
printf 'ALTER SESSION SET CONTAINER = FREEPDB1;\nGRANT ALTER SYSTEM TO ore;\nGRANT SELECT ON v_$session TO ore;\nEXIT\n' \
  | docker exec -i ora23 sqlplus -s / as sysdba
```

El kit de conformidad (el CI lo hace en el grupo `oracle` del job `conectores`):

```
cargo build -p ore-conector-kit -p ore-read-oracle -p ore-federation --features ore-conector-kit/oracle
ORACLE_KIT_URL=oracle://ore:ore@localhost:1521/FREEPDB1 \
  target/debug/ore-kit --conector target/debug/ore-read-oracle --banco oracle
```

En Windows el caso 10 (`SIGTERM`) no se puede medir: `kill` no señaliza un proceso nativo. En Linux o en un contenedor
(`rust:1-bookworm`, con `--add-host=host.docker.internal:host-gateway`) sí.

## El fallo de `oracledb` 26.0.0-beta.4

`tipos.sql` crea `ORE.TIPOS`: una columna de cada tipo y tres filas (una completa, una casi toda `NULL`, una con
valores extremos). Cargada en el laboratorio:

```
docker cp tipos.sql ora23:/tmp/tipos.sql
docker exec ora23 sqlplus -s ore/ore@//localhost/FREEPDB1 @/tmp/tipos.sql
```

Leer la tabla entera con el conector falla; con un tamaño de lote de 1 o 2, no:

```
echo '{"url":"oracle://ore:ore@127.0.0.1:1521/FREEPDB1","objeto":"ORE.TIPOS","formato":"arrow",
 "proyeccion":{"id":"ID","a":"T_TSLTZ","b":"I_YM","c":"I_DS","d":"R_RAW","e":"L_CLOB","f":"L_BLOB","g":"J_JSON","h":"B_BOOL"}}' \
  | target/debug/ore-read-oracle leer >/dev/null
# ore-read-oracle: {"codigo":"origen","mensaje":"internal error: unknown TTC message type N at packet 1, offset M", …}

# …con la URL "oracle://ore:ore@127.0.0.1:1521/FREEPDB1?fetch=2" (o 1) lee las tres filas.
```

Hay que reportarlo a <https://github.com/oracle/rust-oracledb>. Una tabla de 10⁵ filas con `CLOB`, `BLOB` y nulos no
lo reproduce con ningún tamaño de lote: hace falta esta forma (pocas filas, una de ellas casi toda nula).
