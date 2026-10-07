# 0055 · Transforms in code repositories — el paradigma Build

**Estado:** propuesto (2026-10-05). Decididos el kind, su salida, la programación aparte, la
versión de OOS y la frontera entre **Build** y **Preview** (§ Decisiones). Primero se asienta el
`Transform` —el kind y su descubrimiento al commitear—; Build va después, sobre él.

Construye sobre lo que ORE ya define —el `Dataset` escrito de OOS (v1alpha12), la `View`
(v1alpha8), el `Ruleset` (v1alpha3), el flujo (`04-flow`), el puesto ([`0031`](0031-el-puesto.md)),
el SQL del árbol ([`0039`](0039-sql-paradigms-in-code-repositories.md), [`0040`](0040-sql-views.md)),
las ramas protegidas ([`0044`](0044-ramas-globales.md)), el acceso
([`0047`](0047-ore-access-control.md)) y la media incremental
([`0049`](0049-media-paradigms-in-code-repositories.md))— y toma de
[`0050`](0050-functions-in-code-repositories.md) su regla primera: **el código manda; el documento
se deriva**.

## Qué es

Un repositorio de functions responde a **qué se invoca**. Uno de transforms responde a **qué se
construye**: lee datasets de entrada, los transforma y escribe datasets de salida —y vistas y
colecciones—. Su eje es el linaje y el build, no la invocación.

> **Un `Transform` es el productor declarado de un dataset escrito: código, en un commit, que lee
> unas entradas y escribe una salida. Lo que lee y lo que escribe se leen en su documento sin
> ejecutarlo.**
>
> **Build es el verbo que lo cumple**: corre el código **del commit** y escribe una transacción en
> la salida, con procedencia `Transform@commit`. **Preview** lo ensaya: corre el código **del
> editor** y enseña el resultado, sin registrar, escribir ni versionar nada.

El spec ya le guardaba el hueco. El `Dataset` dice de sí mismo (v1alpha12 §2): *«No es el trabajo
que lo escribió. El código vive en un commit; el puntero lo nombra.»* Ese trabajo es el `Transform`.

| kind | qué es | quién pone los bytes |
|---|---|---|
| `Table` | lo que es de otro, espejado | nadie de aquí |
| `View` | la pregunta, sin bytes | nadie |
| `Dataset` mantenido (`from`) | la pregunta guardada | **el sistema** cumple el plan |
| `Dataset` escrito (`columns`) | lo que se tiene, con historia | **código** |
| `Function` | lógica que se invoca sobre la copia; propone efectos | no produce datasets |
| **`Transform`** | **el código que produce un dataset escrito** | es ese código |

## Lo que hay hoy (2026-10-05)

Los tres lenguajes **ejecutan y escriben**; ninguno es **conocido** por la plataforma como pipeline.

| | Python | SQL | Java |
|---|---|---|---|
| plantilla | `transforms-python` v6: `pyproject.toml` + `transforms/example.py` | `transforms-sql` v6: `transforms/example.sql` | `transforms-java` v6: `pom.xml` + `transforms/Example.java` |
| declarar | `ore.transform(inputs, output)` envolviendo una función; `over()`/`sql()` leen, `write()` escribe | `CREATE OR REPLACE DATASET … AS` (overwrite), `INSERT INTO` (append), `INSERT OR REPLACE` (upsert); también `VIEW`, `MATERIALIZED VIEW`, `MEDIA COLLECTION` | `transform(name, inputs, output, () -> …)`, métodos estáticos |
| motor | DuckDB en el puesto (50 % de la RAM, derrama a disco) | DuckDB; `cotejar` lo comprueba sin ejecutar | DuckDB + Arrow por JDBC |
| Run | la celda entera en la sesión; **escribe** en la rama | igual, sentencia a sentencia | igual, en JShell |
| como trabajo | `POST /trabajos {codigo}`: un Job de una celda desde el commit, con `ORE_CODIGO=ruta@commit`. **Sin repositorio**, con la capa del árbol, sin exigir el alcance, y **la consola no lo llama** | igual (la sentencia que escribe se vuelve un `@transform` generado) | igual |
| entorno | el de functions-python (P0–P5): capa por repositorio, Libraries, sesión automática, pyright | — | capa-jvm (Maven) |
| pruebas | el agente sabe pytest; la consola sólo lo enseña en functions | — | — |

Lo que ya es común y se conserva:

- **El alcance se exige en vivo** (`declarar_transform`, `puestos.rs`): fuera de lo declarado no se
  lee ni se escribe.
- **Cada escritura deja su procedencia** (`_procedencia` → `derivedFrom` del `Dataset`,
  `escritas.rs`).
- **Tres modos de escritura**: overwrite, append, upsert con clave.
- **La media incremental** por ítem (`collection().apply()`, 0049 B5).

Y lo que falta, **igual en los tres**: nada se descubre al commitear; no hay Build en la consola ni
historial; no hay grafo (la página Lineage es un hueco); Run escribe con código que puede no estar
en git; no hay incremental general, ni expectativas de datos, ni programación (el botón Schedule es
un stub), ni cadena de ramas para los datasets.

## Primeros principios

1. **El código manda; el `Transform` se deriva.** Nadie escribe el `Transform` en YAML: se lee del
   código al commitear, sin ejecutarlo, y se escribe en el mismo commit. Si difieren, es un error.
2. **Sólo Build escribe.** Toda transacción de un dataset producido por un repositorio de
   transforms apunta a un commit. Preview nunca deja rastro.
3. **El `Dataset` es lo que se tiene; el `Transform`, quien lo produce.** La versión de los datos
   vive en el dataset (cada build es un snapshot); la del productor, en git. El historial de builds
   es estado, como el puntero.
4. **Un productor por salida.** Dos códigos que escriben lo mismo es un error de compilación, no
   una carrera.
5. **Lo declarado es el alcance**, al compilar y al correr: el Build exige lo que dice el documento
   del commit, no lo que el código declare al vuelo.
6. **Lo que se puede saber sin ejecutar, se sabe en el commit**: el grafo, los ciclos, las
   entradas que no existen y la etiqueta que baja por el grafo.
7. **Cuándo no es qué.** El `Transform` dice qué se lee y qué se escribe; cuándo se construye vive
   aparte.
8. **Un modelo, tres lenguajes.** Python, SQL y Java derivan el mismo `Transform` y se construyen
   con el mismo Build; lo que cambia es cómo se lee la declaración.

## Decisiones

| # | decisión | fecha |
|---|---|---|
| D1 | **Kind propio**, `Transform`, y no una clave `producedBy` en el `Dataset`: lo que se construye, se programa y tiene historial es el productor, no los datos | 2026-10-05 |
| D2 | **Una salida por `Transform`**, por ahora (lo que el SDK ya impone). En el grafo, los nodos son datasets y las aristas transforms. Varias salidas, si llegan, como `outputs` | 2026-10-05 |
| D3 | **La programación fuera del `Transform`**: un objeto aparte que nombra transforms o salidas y un disparador. Cambiar la hora no toca el código; la misma pieza se programa distinto por rama; una programación cubre varias | 2026-10-05 |
| D4 | **Se publica en OOS v1alpha25** (spec en `C:\oos`, y luego el submódulo) | 2026-10-05 |
| D5 | **Build registra la salida, corre el transform del commit y escribe una transacción** con procedencia `Transform@commit` y el id del build. Es el único que escribe | 2026-10-05 |
| D6 | **Preview sustituye a Run en los repositorios de transforms**, en la barra inferior: el código del editor, un transform concreto, el resultado en memoria; no registra, no escribe, no versiona. En functions sigue Dry Run | 2026-10-05 |
| D7 | **Sin `changes` en el `Transform`**: lo que una salida admite lo declara su `Dataset` escrito; el modo va en cada escritura | 2026-10-05 |
| D8 | **Salida por nacer**: un `output` que aún no resuelve es válido y lo registra el primer Build; si resuelve, es un `Dataset` escrito o una `MediaCollection` | 2026-10-05 |
| D9 | **Python declara con literales**, constantes del módulo asignadas una vez o `ore.collection("…")`; lo calculado no se deriva y el commit se rechaza con su línea. La plantilla v7 no se llama a sí misma | 2026-10-05 |
| D10 | **La identidad es la salida**: no es un activo de Assets. Vive en el repositorio, `<repo>/pipeline/<salida>.yaml`, derivado y marcado; se llama por lo que produce (`ore.build("db.schema.t")`, `POST /builds`), y la ficha del dataset dice *Produced by*. Carpeta visible: el compilador ignora las ocultas | 2026-10-05 |
| D11 | **Java al final**: falta un lector de Java en Rust y, seguramente, una forma de declarar que se lea sin ejecutar. Primero Python y SQL | 2026-10-05 |
| D12 | **Build sustituye a Run** en los repositorios `transforms-*`: Build primero, Preview cuando Build cierre | 2026-10-06 |
| D13 | **Con cambios sin commitear, Build está desactivado** («Commit first» en el tooltip): construye el commit, no el editor | 2026-10-06 |
| D14 | **Build construye todos los transforms del fichero**, con un resultado por transform en el panel, como el SQL de varias sentencias | 2026-10-06 |
| D15 | **Un `@transform` llamado al cargar el módulo hace fallar el build**, con su línea en el panel: el build lo llama él, y dos llamadas serían dos escrituras | 2026-10-06 |
| D16 | **`main` es una rama más**: se construye en la rama en la que se está, salvo que esté protegida (`.arbol/ramas.yaml`) | 2026-10-06 |
| D17 | **`ore.build()` fuera del plan B**: encadenar lo cubre construir lo de arriba en orden, y repetir, la programación; las dos son de la plataforma y no exigen que una sesión (un agente) lance trabajos en nombre de su persona. Vuelve, si hace falta, como otra forma de pedir lo mismo | 2026-10-07 |
| D18 | **Preview a la derecha de Build**: al pasar el cursor despliega los `@transform` del fichero abierto, por su nombre, y la persona elige uno. Su vista vive en la barra inferior, con entrada propia. Corre el código del editor, sin commitear | 2026-10-07 |
| D19 | **Preview sin tope**: entradas enteras; se enseña el esquema, las primeras filas y el recuento exacto | 2026-10-07 |
| D20 | **Preview avisa del cambio de esquema** respecto a la salida actual (o dice *New dataset*), en la cabecera de su resultado | 2026-10-07 |
| D21 | **Preview exige lo mismo que Build**: una entrada no declarada o una llamada al cargar fallan con su línea y el mismo mensaje | 2026-10-07 |

## El kind `Transform` (borrador v1alpha25)

```yaml
apiVersion: oos.dev/v1alpha25
kind: Transform
metadata: { name: resumen, namespace: ventas }  # en etl/pipeline/ventas.curado.resumen.yaml
spec:
  runtime: python                              # python | sql | java
  source: transforms/resumen.py
  entrypoint: resumen                          # la función; en SQL, la sentencia
  inputs: [ventas.pedidos, ventas.clientes]    # tablas, vistas, datasets, colecciones
  output: ventas.resumen                       # un Dataset escrito (o una MediaCollection)
```

**No lleva**: el commit (lo da el árbol en que vive), los builds ni su estado (son del servidor),
la programación (D3), ni efectos, endosos o `input`/`output` de valores (eso es `Function`).

**Lo que se comprueba al compilar** (códigos por asignar en v1alpha25):

| regla | por qué |
|---|---|
| `output`, si resuelve, es un `Dataset` **escrito** o una `MediaCollection`; si no, está por nacer (D8) | uno mantenido lo produce el sistema: dos dueños de los bytes |
| un solo `Transform` por `output` | hoy dos códigos se pisan en silencio |
| cada `inputs` resuelve; la salida no es su propia entrada salvo en incremental | |
| el grafo —`Transform` + `from` de los mantenidos— es acíclico | |
| la salida lleva el join de lo que llevan sus entradas, y su conducto lo admite (`04-flow`) | la etiqueta baja **antes** de la primera ejecución; hoy sólo baja por lo observado (`derivedFrom`) |

**Lo que queda fuera**: `CREATE VIEW` produce una `View` (no hay bytes); una vista materializada o
un dataset con `from` los mantiene el sistema. Un `.sql` con varias sentencias que escriben da un
`Transform` por sentencia.

## Build y Preview

| | Preview | Build |
|---|---|---|
| código | el del editor, borradores incluidos | **el del commit** |
| qué corre | el transform que se elige (D18) | todos los del fichero (D14; más adelante, lo de arriba) |
| dónde | la sesión de la persona | un Job propio, con la capa **del repositorio** |
| entradas | reales, de la rama, enteras (D19) | enteras |
| `write()` | interceptado: esquema, primeras filas, recuento y cambio de esquema al panel | una transacción en la salida |
| alcance | exigido (lo que el transform declara) | exigido (lo que dice el **documento**) |
| rastro | ninguno | snapshot con procedencia `Transform@commit` + build #n; historial |
| cuánto | segundos | ~68 s medidos en `victor` para un Job (42 esperando nodo) |

**Build es un verbo, no un botón.** Lo disparan cuatro sitios y todos llaman a lo mismo: el botón del
repositorio (el transform del fichero abierto, en su rama), *Build* en la página del dataset, la
programación, y el build de lo de abajo que necesita lo de arriba.

## Lo que sigue

**T1 · el `Transform` y su descubrimiento** (Build después, sobre él):

| paso | qué |
|---|---|
| T1·0 | medir: v1alpha25 libre, cómo resuelven entradas y salidas, coste de derivar en el commit |
| T1·1 | spec v1alpha25 (`C:\oos`): kind, reglas y códigos, esquema, conformidad; luego el submódulo |
| T1·2 | `ore-core`: leer y validar (por nacer, productor único, entradas, sin ciclos, etiqueta, sólo derivado) |
| T1·3 | `ore-code`: derivar de Python (decorador) y SQL (sentencias que escriben); emitir determinista |
| T1·4 | el commit escribe `pipeline/` en el mismo commit; la puerta rechaza lo que no casa; índice salida→`Transform` |
| T1·5 | plantillas v7 de transforms-python y transforms-sql |
| T1·6 | consola: aviso del commit, `pipeline/` en el repositorio, *Produced by* en la ficha |
| T1·7 | Java |

**B · Build**, sobre el `Transform` asentado (T1 en vivo desde `c4e5725`, 2026-10-06):

| paso | qué |
|---|---|
| B0 | medido (abajo) |
| B1 | `POST /builds {output, rama?}`: el `Transform` de esa salida en la cabeza de la rama (índice de T1·4); el arnés —Python carga el módulo y llama al `def`; SQL corre la sentencia `n`—; `lanzar_trabajo` con el repositorio, **su** capa y el documento como techo; procedencia `{transform, entrypoint, commit, build}`. Y la puerta del commit exige la base y el schema de la salida (`OOS2037`) |
| B2 | `GET /builds?output=…`, `GET /builds/{id}`: estado, duración, filas, snapshot, log y el error con su línea |
| B3 | consola: Build en lugar de Run en `transforms-*` (D12–D16); pestaña **Builds** en el panel de resultados |
| B4 | la ficha del dataset: el último build y su botón Build — **pendiente** |
| ~~B5~~ | ~~`ore.build("<salida>")` en el SDK de Python~~ — fuera (D17) |
| B6 | pruebas de fuego y el ADR — **pendiente** |

**Build cerrado por ahora (2026-10-07).** En vivo en `victor`: tres transforms pequeños sobre
`bq.ventas.clientes`/`pedidos` y tres pesados sobre `bq.ventas.ore_e2e_sintetica` (2 M filas: ventanas,
`rolling` de pandas, `QUALIFY`), todos *succeeded*, **~25 s o menos** cada uno, 2 M de filas incluidas.
Los cuatro huecos de esas pruebas, cerrados (`427e9f2`): un fichero que no se lee es OOS2043 con su
línea y su documento se queda; los diagnósticos señalan el código, no el YAML derivado; «nada que
construir» dice por qué; y los mensajes, en inglés. Quedan B4 y B6.

**P · Preview** (D18–D21), sobre el arnés de B1:

| paso | qué |
|---|---|
| P0 | medir: el arnés en modo preview en una sesión caliente con `sintetica_enriquecida` entera (2 M filas): tiempo y memoria de la sesión |
| P1 | backend Python: el arnés en la sesión con el código del editor y el `@transform` elegido; `write()` devuelve esquema, primeras filas y recuento; D15 y la entrada no declarada fallan con su línea; y la lista de `@transform` del contenido del editor, para el desplegable |
| P2 | consola: Preview a la derecha de Build con su desplegable, la entrada en la barra inferior y la vista (esquema, filas, recuento, tiempo, error con su línea) |
| P3 | el aviso de cambio de esquema contra la salida actual, o *New dataset* |
| P4 | SQL: el desplegable lista las sentencias por su salida; Preview corre su `SELECT` entero |
| P5 | pruebas de fuego, en vivo, y el ADR |

**P0 · medido (2026-10-07).** En `victor`, en una sesión Python recién abierta (tope 4 GiB, 2 vCPU), el
código de los dos transforms pesados con `write()` cambiado por esquema + 100 filas + recuento, con
las entradas enteras (`bq.ventas.ore_e2e_sintetica`, 2 M filas):

| transform | filas de salida | 1ª vez | 2ª vez | pico del proceso |
|---|---|---|---|---|
| `sintetica_diaria` (DuckDB + pandas `rolling`) | 24 024 | 4,2 s | 2,1 s | 314 MiB |
| `sintetica_enriquecida` (ventanas, Arrow) | 2 000 000 (200 MB en Arrow) | 7,6 s | 5,6 s | 765 MiB |

El pico de todo el contenedor, 842 MiB. Interceptar `write()` cuesta ~1 ms (320 ms la primera vez:
importar pyarrow). **Sin tope cabe**: segundos, a un quinto de la memoria de la sesión; el build del
mismo transform son ~25 s, casi todo arrancar el Job y escribir.

**B0 · medido (2026-10-06).** En `victor`, con el nodo de sistema caliente, un Job del puesto pasa de
creado a listo en **8 s** (en cola 0 s, la capa 0 s, la imagen ya en el nodo); las copias y las capas,
de creadas a arrancadas en 1 s. Los ~68 s de 0050 eran de nodo frío. Del código: `lanzar_trabajo` ya
recibe el techo (`transform`) y el arnés (lo usa la invocación de functions, 0050 P3);
`capa_para(entorno, rama, alcance)` da la capa de un repositorio con su `alcance`; la rama es la de
la persona; y la protección es sólo la de `main` (`.arbol/ramas.yaml`). Un trabajo lanzado a mano en
`victor` no se midió: exige el token de una persona.
