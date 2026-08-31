# Handoff · `discover` de punta a punta

> **Este documento es desechable.** Se borra el día que la fila `1` de la tabla de fases se
> ponga en verde. Un plan que sobrevive a su ejecución deja de ser un plan y pasa a ser
> documentación de un pasado que ya nadie comprueba.
>
> Fecha: 2026-08-31 · Escrito **después** de medir, no antes.

---

## 1. Dónde estamos

La cadena de punta a punta de la fase 1 es esta, y **cuatro de los cinco eslabones existen**:

```
ore init  →  ore source add  →  ore discover --source  →  ore review  →  ore validate
   ✅              ✅                    ✅                    ❌             ✅
```

Lo que hay hoy no es un esqueleto. `discover` son **dos actos separados a propósito**, y la
costura entre ellos es la decisión de diseño que sostiene todo lo demás:

| | Qué sabe | Dónde vive | Cuánto |
|---|---|---|---|
| **el lector** | el sistema de tipos **de su fuente** | `lector.rs` + un `ore-read-<tipo>` en el `PATH` | 671 líneas · 7 pruebas |
| **el inductor** | el sistema de tipos **de OOS** | `inductor.rs`, puro: sin red, sin credenciales | 980 líneas · 11 pruebas |

El catálogo llega con tipos de OOS **ya traducidos**. Si llegara con `NUMERIC` o `int8`, el
inductor tendría que saber de Postgres y de BigQuery, y la costura no serviría de nada.

Y la regla que gobierna lo que emite: **se emite lo que es un hecho, se reporta lo que es
una conjetura.** Una tabla es una entidad —hecho—; una columna `id_cliente` que *parece*
apuntar a `clientes` es una conjetura, y `01-package` §5 dice qué hacer con ella: marcarla,
nunca inventarla.

### Medido hoy, no recordado

Contra un catálogo sucio de seis tablas —colisión de nombres, tabla sin PK, tipos sin
traducir, vista, tabla vacía, columna repetida, FK no declarada— `discover` produjo:

```
✓ 3 entidades y sus bindings · todas en DRAFT: nada de esto es verdad todavía
8 decisiones te esperan. Ninguna se ha tomado por ti.
```

Las ocho, y son **la taxonomía completa de preguntas que `review` va a tener que saber
hacer**:

| # | Clase | Ejemplo medido |
|---|---|---|
| 1 | colisión de identificador | `public.clientes` · `ventas.clientes` → ambas dan `Clientes` |
| 2 | sin clave primaria | `log_eventos`, `v_clientes_activos` |
| 3 | columna sin tipo de OOS | `payload` (`jsonb`), `importe` (`numeric`) |
| 4 | ninguna columna tipable | `log_eventos` — no hay entidad que escribir |
| 5 | no es una tabla | `v_clientes_activos` es una vista: ¿entidad o informe? |
| 6 | cero filas | ¿viva y vacía, o un resto? |
| 7 | ¿el mismo concepto? | `email: string` en tres tablas |
| 8 | ¿una relación? | `pedidos.id_cliente`, sin FK declarada |

Y hay una novena que no disparó porque hacen falta dos hermanas numeradas: **tablas
fragmentadas por fecha**.

---

## 2. El landscape, y dónde está el hueco

| Eslabón | Estado | Evidencia | Hueco |
|---|---|---|---|
| `ore init` | ✅ | 4 pruebas · ejecutado hoy | — |
| `ore source add` | ✅ | 8 pruebas | — |
| catálogo desde fichero (`--from`) | ✅ | 11 pruebas · ejecutado hoy | — |
| catálogo desde fuente viva (`--source`) | ⚠️ | **ninguna** | nunca se ha ejecutado contra una base de datos |
| `ore-read-postgres catalogo` | ✅ | existe el verbo | no se ejercita desde `ore` en ninguna prueba |
| `ore review` | ❌ | — | no implementado |
| `ore validate` sobre lo inducido | ✅ | ejecutado hoy | — |

**El eslabón que nadie ha tirado nunca es el vivo.** `discover --from` está cubierto por once
pruebas; `discover --source` —el que resuelve `ore-read-postgres` en el `PATH`, lo ejecuta,
le pasa la URL por stdin y analiza su salida— **no tiene una sola prueba, ni unitaria ni de
integración, ni corre en CI**. `fuentes-reales.sh` ejercita L2 y no toca el descubrimiento.

Es la ley de esta semana en el sitio de siempre: *lo que no se ejecuta tiene exactamente el
mismo aspecto que lo que pasa.*

### El bloqueo declarado, que sigue siendo cierto

El README dice —y se comprobó— que **no existe un vocabulario de conceptos publicado**:
`kind: Property` aparece en 55 ficheros de conformidad y en **cero** de `examples/` o
`docs/`. Sin conceptos a los que mapear, la pregunta 7 de la tabla de arriba —*«¿el mismo
concepto?»*— no tiene respuestas que ofrecer, solo un hueco de texto libre. Y las *«cinco
preguntas»* del criterio de la fase 1 vuelven a ser cinco ensayos.

---

## 3. Prioridades, en orden de riesgo retirado

### P0 · Tirar del eslabón vivo, y meterlo en CI

Lo primero no es escribir código: es **ejecutar el que ya hay** contra un PostgreSQL sucio.

- Un `pruebas-de-fuego/descubrimiento.sh` hermano del de fuentes reales: levanta un esquema
  con las siete patologías de la tabla de arriba, corre `init → source add → discover
  --source → validate`, y comprueba **las decisiones que salen**, no que el comando no
  reviente.
- Un trabajo de CI, o la extensión del que ya existe.

**Listo cuando:** la CI falla si alguien rompe la resolución del driver, el paso de la URL
por stdin, o el análisis del catálogo. Hoy los tres se pueden romper en silencio.

**Por qué primero:** es la única tarea que puede cambiar todo lo demás. Si el eslabón vivo
tiene un defecto de forma, las prioridades de abajo se reordenan.

### P1 · Publicar un vocabulario mínimo

Un paquete de conceptos de verdad, fuera de `conformance/`. La forma ya está definida y
probada —`v1alpha4/valid/vocabulary-package-has-no-entities`—, así que esto no es diseño: es
elegir el contenido.

**Listo cuando:** `discover` puede proponer un `is:` para una columna, y la pregunta *«¿el
mismo concepto?»* ofrece **candidatos** en vez de un hueco.

**Aviso de alcance:** el criterio es *mínimo*. Acuñar un concepto por columna repetida es la
inflación que `02-property` §6.2 nombra —cuatro mil columnas dan cuatro mil conceptos, que es
igual que no tener vocabulario—. Empezar por lo que se repite entre **fuentes distintas**.

### P2 · `ore review`

La cara interactiva de una cola que **ya existe y ya está serializada**: `discover.pending.json`.

- Un formulario por cada una de las ocho clases (nueve con las fragmentadas).
- La respuesta **escribe en los ficheros inducidos**, no en un estado aparte.
- Un modo no interactivo —`--answers <fichero>`— o esto no se puede probar en CI, y
  volvemos al problema de P0 una capa más arriba.

**Listo cuando:** contestar las decisiones de un catálogo sucio deja un paquete que
`ore validate` acepta.

### P3 · La costura del paquete inducido

Medido hoy: `discover --out <dir fuera de un repo>` escribe bindings con
`datasourceRef: crm_prod` y un manifiesto que no declara ese datasource → `OOS2004` ×2. Es
coherente —el inductor no inventa— pero significa que **el camino de verdad es dentro de un
repositorio**, y eso hoy no lo dice nadie.

**Listo cuando:** o el comando avisa, o `--out` fuera de un repo es un error con ayuda.

### P4 · Tres cosas pequeñas que salieron al medir

1. **El mensaje del tipo sin traducir es el de un tipo estructurado**, para todos. A
   `numeric` le dice *«puede ser un objeto embebido o una entidad aparte»* cuando la pregunta
   real es **precisión y moneda** —y hay un código para eso, `money-without-precision`—.
2. **Una tabla y una sola hermana fechada no se detectan como familia.** `fragmentadas()`
   exige dos nombres con sufijo numérico, y `pedidos` + `pedidos_2024` —el caso más común en
   un almacén real— pasa de largo porque `pedidos` no lleva dígitos.
3. **Una vista se marca como pendiente y aun así se emite como entidad.** `log_eventos`, sin
   PK y sin tipos, no se emite; `v_clientes_activos`, sin PK, sí. La asimetría puede estar
   bien —una sin columnas tipables no tiene nada que escribir— pero no está dicha.

---

## 4. Lo que **no** entra

- **`drift-detect`**: es la fase 1 también, pero es otro verbo y otro riesgo. No bloquea
  `discover`.
- **El LLM**. El criterio dice *«un arquitecto dice que está un 80% bien tras contestar cinco
  preguntas»*, no *«un modelo lo adivina»*. El scaffolder está fuera del camino de ejecución
  de confianza y ahí se queda: lo que produce es una propuesta en `DRAFT`, y la única vía de
  propuesta a verdad es un commit revisado.
- **Cualquier cosa que meta red en `ore`.** El binario no sabe hablar por la red, y eso es
  una propiedad comprobada por `dependencias.rs`, no una promesa. Se delega, siempre.

---

## 5. Deriva encontrada de paso

Tres sitios que dicen algo que ya no es cierto, y que conviene arreglar cuando se toque esto:

- `README.md` · *«De la fase 1 existe `source add`»* — también existe `discover`, con 980
  líneas y once pruebas.
- `README.md` · *«`02-property` y `03-interface` **todavía sin escribir**»* — están escritos
  desde el 30 de agosto.
- El mensaje de `ore review` enumera los comandos implementados **sin `report`**, y es la
  tercera copia a mano de esa lista.
