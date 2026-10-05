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

## El kind `Transform` (borrador v1alpha25)

```yaml
apiVersion: oos.dev/v1alpha25
kind: Transform
metadata: { name: resumen, namespace: ventas }
spec:
  runtime: python                              # python | sql | java
  source: transforms/resumen.py
  entrypoint: resumen                          # la función; en SQL, la sentencia
  inputs: [ventas.pedidos, ventas.clientes]    # tablas, vistas, datasets, colecciones
  output: ventas.resumen                       # un Dataset escrito (o una MediaCollection)
  changes: { mode: upsert, key: [pais] }
```

**No lleva**: el commit (lo da el árbol en que vive), los builds ni su estado (son del servidor),
la programación (D3), ni efectos, endosos o `input`/`output` de valores (eso es `Function`).

**Lo que se comprueba al compilar** (códigos por asignar en v1alpha25):

| regla | por qué |
|---|---|
| `output` resuelve a un `Dataset` **escrito** o a una `MediaCollection` | uno mantenido lo produce el sistema: dos dueños de los bytes |
| un solo `Transform` por `output` | hoy dos códigos se pisan en silencio |
| cada `inputs` resuelve; la salida no es su propia entrada salvo en incremental | |
| el grafo —`Transform` + `from` de los mantenidos— es acíclico | |
| `changes` coincide con el del `Dataset` de la salida | |
| la salida lleva el join de lo que llevan sus entradas, y su conducto lo admite (`04-flow`) | la etiqueta baja **antes** de la primera ejecución; hoy sólo baja por lo observado (`derivedFrom`) |

**Lo que queda fuera**: `CREATE VIEW` produce una `View` (no hay bytes); una vista materializada o
un dataset con `from` los mantiene el sistema. Un `.sql` con varias sentencias que escriben da un
`Transform` por sentencia.

## Build y Preview

| | Preview | Build |
|---|---|---|
| código | el del editor, borradores incluidos | **el del commit** |
| qué corre | un transform concreto | un transform (y, más adelante, lo de arriba) |
| dónde | la sesión de la persona | un Job propio, con la capa **del repositorio** |
| entradas | reales, de la rama, con tope o muestra | enteras |
| `write()` | interceptado: esquema, primeras filas, recuento al panel | una transacción en la salida |
| alcance | exigido (lo que el transform declara) | exigido (lo que dice el **documento**) |
| rastro | ninguno | snapshot con procedencia `Transform@commit` + build #n; historial |
| cuánto | segundos | ~68 s medidos en `victor` para un Job (42 esperando nodo) |

**Build es un verbo, no un botón.** Lo disparan cuatro sitios y todos llaman a lo mismo: el botón del
repositorio (el transform del fichero abierto, en su rama), *Build* en la página del dataset, la
programación, y el build de lo de abajo que necesita lo de arriba.

## Lo que sigue

1. **El `Transform`**: el kind en OOS v1alpha25 y su descubrimiento al commitear, en los tres
   lenguajes. Se cierra antes de tocar Build.
2. **Build**, sobre el `Transform` asentado.
