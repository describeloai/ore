# 0006 · El artefacto de topología

**Estado:** aceptado · **Fecha:** 2026-08-31 · **Decide:** ORE no opera ninguna base de datos

---

## El problema

`03-binding` §3.1 declara **qué** se materializa —`topology` y `payload`— y calla dónde se
guarda, a propósito: eso es del motor. Este documento contesta esa pregunta para ORE.

`DESIGN` §7.3 la tenía abierta en tres filas que se contradecían entre sí: *«el almacén del
modo cache… probablemente dos almacenes»*, *«RocksDB frente a redb / fjall»* y *«distribución
del índice: construir una vez y distribuir como artefacto (checkpoint), no que cada nodo
reconstruya y diverja»*.

La tercera fila ya contenía la respuesta y nadie la había leído junto a las otras dos.

## Decisión

**ORE no opera ninguna base de datos.** Su estado son dos cosas, y ninguna es un motor de
almacenamiento:

| Plano | Qué es | Tamaño | Forma | Refresco |
|---|---|---|---|---|
| **Contexto** | el bundle compilado | MB | artefacto inmutable, mapeado en memoria | por commit |
| **Topología** | claves de join y aristas | GB | **artefacto inmutable, mapeado en memoria** | por ventana |
| **Carga útil** | las propiedades de `payload` | TB | **tabla Iceberg en el lago del cliente** | por ventana |

> **El índice de topología es la misma clase de cosa que el plano de contexto.** Cambia el
> tamaño y la cadencia, no la naturaleza: se construye una vez, se firma, se distribuye y se
> mapea.

### 1 · Por qué no una base de datos de grafos

Cinco razones, y la última es una medida ajena:

1. **El grafo es derivado e inmutable por versión.** La maquinaria entera de un motor de
   grafos es la mutación transaccional. La pagaríamos —en operación, en licencia, en
   latencia— y no usaríamos ni una de sus garantías.
2. **La travesía es acotada y por clave.** Desde una raíz, N saltos, produce un conjunto de
   claves (`05-ejecutor` §3 ②). No es analítica de grafos: no hay PageRank, ni detección de
   comunidades, ni caminos mínimos sobre el grafo entero.
3. **Sería un segundo origen de verdad.** El bundle es la verdad; el índice es una proyección
   suya más las aristas vigentes. Un almacén mutable puede divergir de su fuente, y no habría
   forma de notarlo.
4. **Contradice el argumento de producto.** *«ORE es stateless por defecto y stateful por
   declaración»* no sobrevive a un servicio que hay que operar para arrancar.
5. **La literatura lo dice al revés de como suele leerse.** La estructura canónica de
   adyacencia —CSR, *compressed sparse row*— «no puede acomodar actualizaciones dinámicas sin
   reconstruir el array de aristas entero». Eso es un **defecto** si estás construyendo una
   base de datos, y un **no-problema** si tu índice se reconstruye por ventana. Toda la
   investigación reciente —CSR++, LSMGraph, los índices de adyacencia persistentes— existe
   para resolver un problema que nosotros no tenemos.

> **Lo que la literatura llama la limitación de CSR es exactamente nuestro modo de operación.**

### 2 · Por qué la carga útil sí es una tabla, y no nuestra

Aquí la industria ya convergió y no hay nada que inventar:

- **Dremio** materializa sus *reflections* como **tablas Iceberg respaldadas por Parquet en
  object storage**.
- **Trino** le da a cada vista materializada una **«storage table»**: una tabla Iceberg real,
  en un esquema que el operador configura.

Y Trino hace algo más que nos ahorra un diseño entero: **guarda los `snapshot-id` de las
tablas de origen en los metadatos de la vista y los compara al consultarla** para saber si
está al día. Eso **es** la marca de agua de `05-ejecutor` §7, que derivamos por P2 sin saber
que era el mecanismo del propio ecosistema.

La consecuencia operativa es la que importa: **la caché no es un subsistema nuestro.** Es una
tabla en el lago del cliente, que el cliente puede abrir con sus herramientas, y cuya frescura
se demuestra con el mecanismo nativo del formato.

### 3 · Por qué esto NO son «dos almacenes en el mismo motor»

Era la objeción correcta a la formulación anterior. No lo son porque **solo uno de los dos es
un almacén**:

| | Contexto y topología | Carga útil |
|---|---|---|
| Qué es | **un fichero que producimos** | **una tabla que no es nuestra** |
| Quién lo lee | `mmap`, nosotros | el mismo driver que cualquier otra fuente |
| Quién lo opera | nadie | el cliente, con su catálogo |

No hay que elegir entre RocksDB, redb y fjall porque **no hace falta ninguno**. Y el escaneo
columnar que reclamaba Arrow/Parquet lo hace el mismo camino que ya lee cualquier tabla del
cliente: la caché entra por la puerta que ya existe.

### 4 · La consecuencia de seguridad, que no estaba escrita

`DESIGN` §4.1 dice que la topología es *«la más sensible en muchos sectores»* —*saber que el
paciente X está enlazado con la clínica oncológica Y es el diagnóstico*—. Y una clave primaria
es un valor.

> **El artefacto de topología contiene datos. El plano de contexto no.**

De ahí una separación que hay que respetar aunque las dos cosas se mapeen igual: **el bundle
viaja en la imagen; el artefacto de topología no.** Se construye contra las fuentes del
cliente, vive en su almacenamiento y se cifra y se controla como un dato, no como una
configuración. Meterlo en la imagen OCI sería publicar las aristas de un cliente en un
registro.

## Lo que se acepta a cambio

- **Refrescar es reconstruir la ventana, no mutar.** Con `strategy: table_version` sobre
  Iceberg o Delta eso es barato porque solo se toca lo que cambió de versión; con `poll` no lo
  es. Es el coste declarado de no tener un almacén mutable, y `03-binding` ya obliga a
  declarar la estrategia.
- **Hay una latencia entre el commit y el índice.** La marca de agua existe justamente para
  que esa latencia sea *observable* en lugar de invisible (`05-ejecutor` §7).
- **La distribución del artefacto es trabajo real.** Construir una vez y repartir es más
  complejo que dejar que cada nodo reconstruya — y es lo correcto: reconstruir en cada nodo
  produce nodos que responden distinto a la misma pregunta, que es la peor forma de fallar.
- **No se mide todavía.** Este documento decide una forma, no un rendimiento. La afirmación
  *«la travesía es sub-milisegundo»* sigue sin medición y no debe repetirse como si la
  tuviera.
- **No se mapea en memoria: se lee entero.** El formato está preparado —anchuras fijas, sin
  analizar nada— pero `mmap` es una dependencia y se paga cuando el artefacto sea lo bastante
  grande para que se note. Afirmar que se mapea sin mapearlo sería la clase de promesa que
  este proyecto no hace.

## Construido, y medido contra un PostgreSQL de verdad

```
3 arista(s), 1 relacion(es) -> /tmp/topo.bin
--- travesia emp-42, 3 saltos ---
ceo
jefa
--- determinismo ---
byte a byte identicos
```

Tres aristas de cuatro filas: `ceo` no reporta a nadie y su `NULL` no produce arista. La
travesía de tres saltos devuelve la cadena entera **sin abrir ninguna conexión** — el índice
ya estaba construido— y dos construcciones sobre la misma instantánea dan el mismo fichero.

Y hay una pieza del protocolo que se validó sola: **el driver no se entera de que esto es un
índice.** Las aristas se leen con una petición de la fase ③ cuya proyección se llama `desde`
y `hasta`, y como las filas salen con nombres de propiedad, lo que devuelve **ya es una
arista**. Que el mismo verbo sirva para la carga útil y para el índice es la prueba de que
[el protocolo](0008-el-protocolo-del-driver.md) estaba bien cortado.

---

## Y es también el modelo de consistencia, que no se vio al decidirlo

Esta decisión se tomó por almacenamiento —dónde se guarda lo materializado— y contesta una
pregunta distinta que apareció cuando se empezó a diseñar el motor de funciones:

> **¿Cómo se consigue consistencia sobre dato que no se posee?**

Foundry la consigue **poseyendo el dato**: materializa en su object backend y entonces
transacciona. Es una solución buena, y es la razón por la que hay que meterlo todo dentro. La
industria no puede: su dato está en BigQuery, en SAP, en un mainframe.

La respuesta que estos tres planos ya daban, sin nombrarla:

> **Lo difícil de una consulta federada no son los valores: es la correspondencia.**

Saber qué filas de un sistema son las mismas cosas que qué filas de otro es la parte cara, y
es exactamente lo que el índice de topología contiene. Y como es **un artefacto inmutable y
versionado**, de ahí sale reproducibilidad sin instantánea global:

| | Se posee | Qué garantiza |
|---|---|---|
| **contexto** | sí | mismo bundle → mismas propiedades autorizadas, mismas podadas y por qué |
| **topología** | sí | misma versión → **mismo conjunto de claves** en una travesía |
| **carga útil** | **no** | nada. Y por eso la frescura se declara en vez de prometerse |

### Lo que da y lo que no

Poseer los dos primeros da **reproducibilidad**, no **aislamiento**. Si el dato cambia en el
origen entre dos lecturas, la topología no lo evita — y decir lo contrario sería la clase de
promesa que este proyecto no hace.

Lo que cierra ese hueco no es una instantánea: es **acotar la mentira**. `05-ejecutor` §7 lo
tenía escrito antes de que esta pregunta existiera —*«el digest del bundle y la marca de agua…
son los dos ejes: **qué significaba** y **hasta cuándo era cierto**»*— y `Respuesta` ya lleva
los dos, más el instante de autorización y el motivo de degradación.

> **La consistencia no viene de poseer el dato. Viene de poseer el significado y la
> correspondencia, y de decir la verdad sobre la frescura de lo demás.**

Es más débil que Foundry en el eje del dato y más fuerte en el de la auditoría. Para un
regulado, el segundo es el que se compra.

### Y de aquí sale lo que un efecto tiene que llevar dentro

Si una función se computa sobre este régimen, su resultado no puede ser solo *qué escribir*:
tiene que llevar **bajo qué se decidió**. Las cuatro identidades —digest del bundle, versión de
topología, marcas de agua y el plan de lectura— son lo que hace que un efecto sea reproducible,
y lo que convierte *«se computó sobre dato rancio»* en una pregunta contestable en vez de una
sospecha. Está desarrollado en [`docs/functions.md`](../functions.md).

Y el peligro que §7.1 ya nombra es de este motor antes que de ningún otro: **refrescar responde
a que el dato cambió; reconstruir, a que la REGLA cambió.** Un efecto computado bajo una regla
nueva sobre datos enmascarados con la vieja es *«la clase de fallo que no tiene aspecto de
fallo»*.

Ese peligro dejó de ser una advertencia el 2026-09-01: es el veredicto `ReglaDistinta` de
`ore_core::cache`, y se comprueba con `ore cache check`. Lo que hacía falta para poder
comprobarlo era metadato —bajo qué bundle se escribieron las filas—, que es exactamente lo que
esta decisión dice que se posee. El alcance del motor está en [`docs/engine.md`](../engine.md).
