# 0008 · El protocolo del driver

**Estado:** aceptado · **Fecha:** 2026-08-31 · **Decide:** la petición es un fragmento del plan, y traducir es del driver

---

## El problema

Hasta M1 un driver sabía un verbo: **leer un catálogo**. Recibe una URL por stdin y escribe
un documento JSON por stdout. Con la fase ③ del plan aparece el segundo —**devolver filas**—
y con él la pregunta que decide si esto sirve para una fuente o para cientos.

[`05-ejecutor`](../../vendor/oos/spec/v1alpha1/05-ejecutor.md) §8 dice que **cómo se ejecuta
es del motor**, así que esta decisión es de ORE y no del formato.

## Decisión

> **La petición es la misma para todos los drivers. Es un fragmento del plan, no SQL.
> Traducir es del driver.**

Lo que viaja es exactamente lo que la fase ③ ya produce — `(datasource, objeto, proyección,
claves, filtros)` — serializado con `Json::jcs()`, que es la forma canónica del bundle:

```json
{"claves":[["emp-7"]],"claveColumnas":["employee_id"],
 "filtros":[{"columna":"cost_center","operador":"eq","valor":"finanzas"}],
 "objeto":"public.employees",
 "proyeccion":{"baseSalary":"base_pay"},
 "url":"postgres://…"}
```

Añadir una familia de fuentes es **escribir un traductor**, no tocar el ejecutor. Si en su
lugar viajara SQL, el ejecutor tendría que conocer el dialecto de cada origen, y entonces
«soportar DynamoDB» dejaría de ser un binario nuevo para ser un cambio en el planificador.

### La proyección es una lista cerrada, y ese es el punto

El driver **NO DEBE** pedir una columna que no esté en `proyeccion`, ni `SELECT *`. No es
higiene: es dónde se hace efectiva la máscara.

> **La forma más fuerte de aplicar una máscara es no pedir la columna.**

Una propiedad `redact` ya no está en el plan, así que no está en la petición, así que no
puede estar en el SQL. La salvaguarda es **estructural** — no hay ningún punto donde alguien
pueda olvidarse de aplicarla, porque no hay nada que aplicar.

Y por eso el SQL se construye en una **función pura y comprobable sin base de datos**: la
afirmación *«el SQL emitido contiene solo las columnas proyectadas»* tiene que ser un aserto,
no una promesa.

## Las tres decisiones de forma

### 1 · El transporte sigue siendo stdin/stdout

El mismo que el catálogo, y por las mismas razones: el motor no abre sockets, la costura ya
existe y **la URL viaja por stdin y no por argv** — que no es preferencia sino
[CVE-2024-24576](../../crates/ore-cli/src/lector.rs), y además evita que el secreto aparezca
en la tabla de procesos.

El **verbo** pasa a ser explícito: `ore-read-<tipo> catalogo <fuente>` y
`ore-read-<tipo> leer <fuente>`. Antes el único verbo estaba implícito en que solo hubiera
uno; con dos, deducirlo del contenido de stdin sería adivinar.

#### Enmienda · el tercer verbo, `testigo`

Decidido en [ADR 0016](0016-el-testigo-y-el-rango.md) A, y añadido aquí porque es **este**
protocolo el que crece:

```
ore-read-<tipo> testigo <fuente>
  stdin  ← {"objeto":"public.employees","url":"postgres://…"}
  stdout → {"modo":"log","valor":"0/1A2B3C4"}
```

**Un verbo y no un campo de los otros dos**, porque las tres cosas caducan a ritmos distintos: el
catálogo cuando alguien altera la tabla, las filas en cada consulta, y el testigo **en cada
confirmación**. Y meterlo en `leer` sería peor que en `catalogo`: llegaría **con** las filas, y
quien pregunta lo hace justamente para decidir **si hace falta leerlas**.

**Su petición no es un fragmento del plan: es una coordenada.** `url` y `objeto`, y nada más. Se
lee con `leer_coordenada` y no reusando `Peticion`, y el motivo salió al construirlo — aquella
rechaza una proyección vacía, con razón, y preguntar dónde está un origen no proyecta nada.
Reusarla habría obligado a mandar una proyección de mentira para pasar una comprobación que ahí
no aplica.

**El vocabulario es el de `changes.witness`** y no se inventa otro: `none`, `snapshot`, `log`,
`field`. Los cuatro son **ordinales** — quien los recibe los compara, no los interpreta.

**Y `none` es una respuesta, no un fallo.** Un origen que no sabe fecharse lo dice, y con eso ya
afirma algo cierto. Devolver «ahora» inventaría una marca que no respalda ningún refresco.

Lo que la implementación de referencia contesta:

| driver | qué devuelve | por qué |
|---|---|---|
| `ore-read-postgres` | `{log, <LSN>}` de `pg_current_wal_lsn()` | un LSN es una **posición de confirmación**: orden total sin empates, y replayable. Un `now()` no lo es |
| `ore-read-jsonl` | `{snapshot, <sha256 del fichero>}` | el digest **nombra esa versión del fichero**, que es lo que `snapshot` significa. La `mtime` habría sido la respuesta cómoda y es peor: dos escrituras en el mismo segundo empatan |

La segunda fila mide algo que ningún argumento mide, otra vez: **un directorio de ficheros sabe
fecharse**, y sin reloj. La primera versión de ese driver se negaba, y era demasiado modesta.

### 2 · Las filas salen en NDJSON

Una fila por línea, un objeto JSON por fila, con las **propiedades** como claves — no las
columnas físicas. El nombre físico es del binding y no tiene por qué salir del driver.

Se elige NDJSON porque **la costura ya habla JSON** para el catálogo, y dos codificaciones en
la misma costura son una de más. Y porque es la única forma que se puede leer *a medida que
llega* sin que el driver tenga que saber cuántas filas va a devolver.

**Su sucesor es Arrow IPC**, y conviene decir qué lo dispara para que el cambio no se haga por
gusto: NDJSON reserializa cada valor a texto y lo vuelve a analizar en el motor, lo que cuesta
por **fila**. Mientras las lecturas sean por clave —que es el caso principal por §5.1— las
filas son pocas y el coste no se nota. **El disparador es la caché de carga útil**: en cuanto
haya escaneos columnares sobre lo materializado, el coste por fila deja de ser despreciable y
Arrow deja de ser una optimización para ser la forma correcta.

### 3 · La conexión es de solo lectura

**`05-ejecutor` §6.2: L2 es solo lectura.** El driver **DEBE** abrir la conexión en modo
solo lectura por el mecanismo del origen —en PostgreSQL, `SET SESSION CHARACTERISTICS AS
TRANSACTION READ ONLY`— y no confiar en que nadie escriba.

La diferencia importa y es la de siempre: **un motor que no escribe porque no tiene código
para escribir tiene una propiedad; uno que promete no hacerlo tiene una política.** Aquí el
código de escritura existe —lo trae el driver de PostgreSQL— así que la propiedad hay que
comprarla, y se compra pidiéndosela al servidor.

## Lo que se acepta a cambio

- **Tres verbos que compartir.** Cada driver nuevo implementa los tres o declara cuál no sabe —
  y en el caso de `testigo`, «no sé» es literalmente una de las respuestas válidas.
  Es la superficie mínima: menos de dos y no hay federación.
- **La petición lleva la URL.** Es lo que ya hacía el catálogo, y mantiene al driver sin
  estado — pero significa que **quien invoca elige la identidad**, que es justo lo que §6.2
  pide al separar el proceso que refresca del que responde.
- **El coste por fila de NDJSON**, con su disparador escrito arriba para que se cambie cuando
  toque y no cuando apetezca.
- **Nada de esto se prueba contra una base de datos en la suite por defecto.** La construcción
  del SQL sí —es pura—; que PostgreSQL conteste lo que se espera exige un servidor, y eso vive
  donde ya vive el resto de lo que necesita red.

## Medido contra un PostgreSQL de verdad

No se deja como afirmación. Sobre `postgres:16` con dos filas y la petición del ejemplo de
arriba:

```
{"baseSalary":"52000","employeeId":"emp-7"}
```

Una sola fila para dos claves pedidas, porque el **filtro del ámbito** —`cost_center =
"finanzas"`— dejó fuera a `emp-9`. El recorte por filas que se declaró en un `Ruleset` esta
mañana viajó hasta el `WHERE` de un servidor, y **la fila salió con nombres de propiedad**,
no de columna. `national_id` no aparece en el SQL ni en la salida: nunca estuvo en el plan.

Y el modo solo lectura, en el mismo servidor:

```
SET
ERROR:  cannot execute INSERT in a read-only transaction
```

Sin esa línea, el mismo `INSERT` dice `INSERT 0 1`. **La propiedad se compra pidiéndosela al
servidor**, y aquí está el recibo.
