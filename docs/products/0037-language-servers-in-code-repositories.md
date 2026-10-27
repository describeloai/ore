# 0037 · Language servers in code repositories

**Estado:** **aceptado · en vivo** (2026-09-23) · **Decide:** cómo sabe el editor de un code
repository **de qué hablas** mientras escribes —qué no compila, qué se puede completar, qué es lo
que tienes debajo del cursor— en cada lenguaje del producto, y cómo llega eso del puesto al
navegador. Se apoya en [`0031`](0031-el-puesto.md) (el puesto: un pod por persona y por
repositorio) y en [`0036`](0036-code-repositories.md) (las clases de repositorio y sus
semillas). El de SQL lo detalla [`0039`](0039-sql-paradigms-in-code-repositories.md).

## Qué es

El editor de un code repository es Monaco en la consola, y **el servidor de lenguaje corre donde
corre el código**: en el puesto, con el mismo intérprete, las mismas bibliotecas y el mismo árbol
que la ejecución. Lo que el editor marca como error es lo que fallaría al darle a Run, y lo que
completa es lo que hay. **`ore-serve` no entiende LSP**: lleva los mensajes de un lado a otro sin
leerlos.

| lenguaje | quién contesta | dónde |
|---|---|---|
| **Python** | pyright | en el puesto (`puesto-python`), lanzado por el agente |
| **Java** | **el propio agente**: `javac` y su árbol de sintaxis, en memoria | en el puesto (`puesto-jvm`) |
| **SQL** | `lsp_sql`, con los nombres del catálogo y DuckDB | dentro del agente de Python |
| **TypeScript · JavaScript** | el servicio de TypeScript de Monaco | en el navegador |
| **JSON** | el servicio de JSON de Monaco (la forma) | en el navegador |
| **YAML** | coloreado | en el navegador |

## ② La tubería

Un servidor de lenguaje manda decenas de mensajes por segundo mientras se teclea —medido: 296 en
3,5 s, 84 por segundo— y espera respuesta en decenas de milisegundos. La consola sólo mandaba
celdas; hacía falta **un canal en las dos direcciones**:

- **De la consola al agente:** `POST /puestos/{id}/lsp` (un lote cada 25 ms) y el agente los recoge
  de un flujo, `GET /puestos/{id}/lsp/agente`.
- **Del agente a la consola:** `POST /puestos/{id}/lsp/salida` (un lote cada 20 ms) y la consola
  los lee de otro flujo, `GET /puestos/{id}/lsp/consola`, **reanudable** con `last-event-id`: cada
  respuesta lleva su número, y se guardan las últimas 500.
- **Flujos SSE, no websocket.** Medido antes de cavar: un websocket pedía SHA-1 para el saludo,
  rompía la regla de una petición por conexión, y en el navegador habría necesitado el token en
  JavaScript, cuando la sesión vive en una cookie `httpOnly`. Un flujo es una respuesta HTTP que no
  termina (`ore-entrada`: `Flujo`, con su propio techo de conexiones, aparte del de las peticiones).
- **Cada flujo vive 240 s** y se despide con el número del último evento para que el cliente vuelva;
  late cada 10 s. La entrada deja vivir una conexión 300 s, y una comprobación de la malla vigila que
  siga siendo más que la vida del flujo.
- **Sólo el dueño del puesto** manda y lee (403 si no); un puesto cerrado es 410.

## ③a Python

`puesto-python` lleva **pyright** (con su Node, unos 156 MB en la imagen; 210 MB vivo; arranca en
medio segundo y completa en una décima). El agente lo arranca **con el primer mensaje del editor**,
no con la sesión, y nunca en un trabajo. Diagnostica, completa y hace hover sobre el código y sobre
`ore`. La imagen no se construye si pyright encuentra un error en una semilla: **lo que la semilla
usa, la semilla lo importa** (`from ore import …`).

## ③b Java

**El agente de la JVM es el servidor de lenguaje**: compila el fichero en memoria con
`javax.tools.JavaCompiler` y completa y explica con el árbol de sintaxis (`Trees`). Diagnósticos,
completar después de `.` y hover, con la descripción de las funciones del SDK. Una pasada entera
tarda 75–107 ms en caliente.

jdtls se midió y se descartó: una segunda JVM de 493–851 MB que tarda 6 s en estar lista, para lo
que la del agente ya sabe hacer.

## ③c Las dependencias de cada repositorio

Un repositorio declara lo que usa en su fichero de siempre —**`pyproject.toml`** en Python,
**`pom.xml`** en Java— y eso es **una capa** por repositorio:

- **La resuelve un trabajo**, no la sesión: `pip download` contra lo que la imagen ya trae, o Maven
  con las bibliotecas de la imagen marcadas `provided`. La capa se nombra por el digest de lo
  declarado y de su lenguaje: lo mismo se resuelve una vez.
- **El puesto la monta al arrancar** (`/capa`), comprobando cada fichero.
- **La imagen manda.** La capa va **la última** en el camino de Python y en el classpath de la JVM,
  y una comprobación de la malla vigila ese orden. Si lo declarado choca con lo que la sesión trae,
  se dice en una frase: *«pediste X, y esta sesión trae la Y: gana la de la sesión»*.
- **El editor ve la capa**: pyright la recibe en sus rutas, y `javac` la tiene en su classpath. Un
  `import` de una biblioteca declarada se resuelve en el editor igual que al ejecutar.

## SQL

Ningún servidor de lenguaje de SQL conoce nuestro esquema: no está en una base de datos, está en
nuestro índice. **Ahí el servidor de lenguaje somos nosotros**: `lsp_sql`, dentro del agente de
Python, completa `base.schema.nombre` y columnas desde el catálogo y diagnostica con DuckDB sobre
tablas vacías, sin leer una fila. El detalle, en 0039.

## La consola

- **Un cliente por puesto y por lenguaje**, que agrupa lo que se teclea cada 25 ms, espera una
  respuesta 8 s, y antes de pedir un completado manda el texto de ahora en el mismo lote.
- **Se rehace solo**: si el servidor no contestó al arrancar, lo intenta al abrir el siguiente
  fichero; si el agente se reinició y no conoce el fichero (`documento-desconocido`), se lo vuelve a
  abrir y pregunta otra vez.
- **El editor se engancha al puesto que ya está abierto**: nunca abre uno por su cuenta.
- **Lo que se ve**: las marcas del servidor de lenguaje, aparte de las de ORE (los diagnósticos del
  árbol), el completado y el hover.

## Aceptación

`el-puesto.sh` 3c (la tubería, con un servidor de mentira: los mensajes llegan intactos, 403 y
422), 3d (SQL), 9b (Java por `javac` y `Trees`), 9c y 4c (la capa, vista desde una celda y en su
orden) y 6b. El cliente de la consola de verdad contra el agente, en `el-editor-sql.py`, y a escala
(1 000 y 5 000 datasets), con fallos y con el canal, en `el-editor-sql-a-fondo.py`. Y lo medido
antes de decidir: `medida-el-servidor-de-lenguaje.py` (los cinco lenguajes),
`medida-la-tuberia.py`, `medida-los-servidores-de-lenguaje.py` (pyright, jedi y jdtls corriendo de
verdad sobre las semillas), `medida-el-java-del-arbol.py` y `medida-la-capa-de-la-jvm.py`.
