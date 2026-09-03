# Decisiones

Una decisión llega aquí cuando **había más de una respuesta razonable** y elegir una cierra
puertas: una dependencia que entra o no entra, un formato que otros van a tener que leer, una
frontera que se pone en un sitio y no en otro. Lo que solo tiene una forma sensata de hacerse
no es una decisión — es código, y se explica donde está escrito.

Cada documento dice **qué se aceptó a cambio**. Un registro que solo guarda los motivos a
favor no es un registro: es una colección de justificaciones.

| | Decide | Estado |
|---|---|---|
| [0001](0001-parser-de-yaml.md) · parser de YAML | leer YAML con un analizador propio, y por qué el ecosistema no servía | aceptado |
| [0002](0002-sin-validador-de-json-schema.md) · sin validador de JSON Schema | los esquemas se publican, no se ejecutan; manda el diagnóstico semántico | aceptado |
| [0003](0003-lectura-estructural-de-cedar.md) · lectura estructural de Cedar | leer la **forma** de una política, no evaluar su semántica | aceptado |
| [0004](0004-distribucion-del-binario.md) · distribución del binario | determinismo comprobado, procedencia atestada, binarios crudos | aceptado |
| [0005](0005-la-superficie-de-contexto.md) · la superficie de contexto | qué sirve `ore dev` por MCP, y qué no toca | aceptado |
| [0006](0006-el-artefacto-de-topologia.md) · el artefacto de topología | CSR sellado contra el digest del bundle | aceptado |
| [0007](0007-enlazar-el-evaluador-de-cedar.md) · enlazar el evaluador de Cedar | el ejecutor enlaza `cedar-policy`; el compilador **no** | aceptado |
| [0008](0008-el-protocolo-del-driver.md) · el protocolo del driver | la petición es un fragmento del plan, y traducir es del driver | aceptado |
| [0009](0009-que-se-distribuye.md) · qué se distribuye | se publica el compilador; el ejecutor se construye desde la fuente | aceptado |
| [0010](0010-el-refresco-sustituye.md) · el refresco sustituye | una fila es el conjunto de aristas de su clave, y la marca avanza | aceptado |
| [0011](0011-el-informe-no-lista-incumplimientos.md) · el informe no lista incumplimientos | `ore report` atribuye; el compilador es quien rechaza | aceptado |
| [0012](0012-el-estado-es-parcial-y-vive-en-el-cliente.md) · el estado es parcial y vive en el cliente | el mantenimiento incremental recuerda por clave, en el almacenamiento del cliente; un *miss* es un plan | aceptado |
| [0013](0013-el-protocolo-del-mantenedor.md) · el protocolo del mantenedor | mantener es una **sesión** por stdin, y el dictamen de coste no se obedece a sí mismo | aceptado |
| [0014](0014-no-se-mide-el-tiempo-se-cuenta-el-trabajo.md) · no se mide el tiempo, se cuenta el trabajo | la unidad de coste es **una fila mirada por un operador**; y medir destapó que la incrementalización estaba escrita y no ocurría | aceptado |
| [0015](0015-el-protocolo-del-almacen.md) · el protocolo del almacén | una copia es un **artefacto nombrado por su digest** —sobre nuestro, carga Parquet— y subirlo es de un programa delegado | aceptado |
| [0016](0016-el-testigo-y-el-rango.md) · el testigo y el rango | preguntarle al origen **hasta dónde está**, y poder pedirle las filas **de un rango** | **A aceptada · B propuesta** |
| [0017](0017-la-escritura-sobre-el-sustrato.md) · la escritura sobre el sustrato | una copia nueva **reserva a su base**, y se reescribe entera hasta que una medida diga lo contrario | **propuesto** |

**0016 es la única propuesta, y el estado es deliberado**: las quince anteriores se escribieron
después de construir lo que decidían, y esta se escribe antes porque toca un protocolo con tres
implementaciones. Es también la única que decide **mirando fuera**: sus cinco preguntas abiertas
se contestaron leyendo lo que Debezium, Iceberg, Delta, Snowflake, BigQuery y Airbyte tienen
escrito, y los seis coinciden. Una de esas lecturas —que el modo `field` es *at-least-once* por
construcción— cambió la propuesta en vez de confirmarla.

**0015, 0016 y 0017 son las tres preguntas de una copia**: qué es y cómo se nombra; hasta cuándo
fue cierta; y qué pasa cuando dos quieren cambiarla a la vez. La tercera salió de que la primera
celebrase que *«la carrera es inofensiva»* — cierto para leer, **falso en cuanto los escritores
producen cosas distintas**.

**0008, 0015 y 0016 son la misma costura creciendo**: la petición del driver era un fragmento del
plan; el almacén añadió el artefacto; y esto añade lo único que ninguno de los dos podía contestar
— **hasta cuándo era cierto lo que se copió**. Sin ella el ciclo puebla una vez y no mantiene.

**0013 y 0014 salieron del mismo trabajo**: construir el ejecutor delegado dejó por primera vez
un sitio donde el circuito corre, y eso hizo posible medirlo — que fue lo que destapó que dos
integradores no estaban indexados.

**0006, 0008, 0013 y 0015 son la misma frontera puesta cuatro veces**: `ore` no abre sockets, así
que todo lo que toca el mundo —leer un origen, correr el circuito Δ, subir una copia— es un
programa delegado, y lo que viaja entre ellos es **un fragmento del plan o el artefacto**, nunca
una llamada a un sistema concreto. Si alguna vez se relaja una, hay que abrir las cuatro.

**0006 y 0015 son el mismo artefacto con dos cargas**: aristas en CSR y filas en Parquet, con el
mismo sobre sellado contra el digest del bundle. La segunda salió de mirar la primera y ver que
la topología ya era una vista materializada escrita a mano.

**0008 y 0013 son el mismo protocolo con y sin memoria**: el driver es una función —entra una
petición, sale una respuesta, el proceso muere— y el mantenedor es una sesión, porque el estado
de una junta no cabe en una petición. El transporte es el mismo a propósito.

**0006 y 0012 se leen juntas**: la primera dice que ORE no opera ninguna base de datos; la
segunda, dónde vive entonces lo que el mantenimiento incremental tiene que recordar, y por qué
eso no contradice a la primera.

**0007 y 0009 son la misma decisión mirada en dos momentos**: la primera saca el evaluador del
compilador, la segunda decide qué pasa con lo que quedó fuera el día de publicar. Si alguna
vez se revisa una, hay que abrir la otra.
