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

**0006 y 0012 se leen juntas**: la primera dice que ORE no opera ninguna base de datos; la
segunda, dónde vive entonces lo que el mantenimiento incremental tiene que recordar, y por qué
eso no contradice a la primera.

**0007 y 0009 son la misma decisión mirada en dos momentos**: la primera saca el evaluador del
compilador, la segunda decide qué pasa con lo que quedó fuera el día de publicar. Si alguna
vez se revisa una, hay que abrir la otra.
