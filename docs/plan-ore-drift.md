# `ore drift` — el plan

> **Estado:** medido, sin construir · **Fecha:** 2026-09-07
>
> Este documento es **desechable**: se borra el día que su última pieza esté en verde, que es
> su condición. Lo que sobreviva de él tendrá que estar en las cabeceras del código o en la
> especificación, no aquí.
>
> Sale de tres medidas —`pruebas-de-fuego/medida-el-espectro-de-la-deriva.py`,
> `medida-deriva-tramos-2-y-3.py` y `medida-la-forma-del-catalogo.py`— y de seis referencias
> publicadas. Nada de lo que sigue es una preferencia.

---

## 1. Qué se declara hoy, y qué es de verdad

`--help` dice: *«Compara la declaración con el esquema físico real **y abre un pull request»*.

Son **tres verbos**, no uno, y fallan por motivos distintos:

| | qué es | estado |
|---|---|---|
| **conseguir** | el esquema físico del origen | **hecho** — `ore source catalog` |
| **comparar** | la declaración contra él | el grueso · §3 |
| **proponer** | escribir la corrección | §6 · y **no abre nada** |

Es la misma partición que `discover` ya tiene por dentro: `--source` y `--from` existen porque
*«son dos actos, y se piden por separado porque fallan por separado»*.

## 2. Las seis referencias, y qué transfiere cada una

| | regla |
|---|---|
| **Terraform** `plan -refresh-only` | *«propone actualizar el estado **registrado**; **no** propone cambiar los objetos remotos»* |
| **Terraform** `-detailed-exitcode` | `0` sin deriva · `1` error · `2` **hay deriva** |
| **Atlas** pre-apply check | corre al empezar `migrate apply` y con `on_error = FAIL` aborta antes de ejecutar nada |
| **AWS Glue** `SchemaChangePolicy` | dos ejes, y el defecto recomendado es **asimétrico**: actualizar en los cambios, solo **registrar** en los borrados |
| **Confluent** BACKWARD/FORWARD | la compatibilidad tiene **dirección**, y cuál importa depende de quién se mueva primero |
| **Iceberg** promoción de tipos | solo en la dirección que **ensancha** y preserva información |
| **Observabilidad** (MC · Datafold) | el modo de fallo que **todos** reportan es la fatiga de alertas |

Y **tres de ellas ya están aquí**, medidas: la dirección, en los pares espejo `OOS5028`/`OOS5029`
y sus dos hermanos —*«cada dirección le duele a otro»*—; el *blast radius*, **exacto y no
aprendido**, porque el linaje por columna incluye la arista `INDIRECT` del `where`; y la
dirección única de la corrección, que aquí no es un modo sino un hecho —`ore` no puede escribir
en BigQuery—.

> **Y una que hay que leer con cuidado.** La promoción de Iceberg es sobre **un lector de
> ficheros que él controla**. Un contrato publicado es otra pregunta: ensanchar el tipo de una
> propiedad sirve valores que un consumidor viejo no sabe leer, así que **también rompe**, en la
> dirección contraria. Las dos direcciones duelen; lo que cambia es a quién.

## 3. Tramo 2 · contra qué se compara

**No contra las `kind: Table`.** Las 18 claves del catálogo **no aterrizan en un documento**:

| kind | qué recibe del catálogo |
|---|---|
| `Table` | `columns` · `reads` · `changes` — más `datasource` y `object` |
| `Entity` | `primaryKey` · `uniqueKeys` · `properties` **(los tipos)** · `relations` |
| `View` | `fields` · `where` |

Comparar solo contra `Table` se dejaría tres clases enteras del espectro —justo donde vive
`OOS5019`— y el tipo de una columna estaría fuera.

**La comparación es catálogo contra paquete**, y la única forma de hacerla sin escribir un
segundo repartidor es usar el que ya hay: `inducir_con` es una **función pura de (catálogo,
decisiones)**, y *«contestar dos veces lo mismo produce el mismo paquete byte a byte»*.

```
ore drift --source X   →   catálogo(hoy) ─inducir_con(decisiones)→ árbol(hoy)
                                                                      ↕  comparar
                                                            el paquete que hay
```

## 4. La frontera que decide si esto sirve

El problema del tramo 2 no es técnico. Un paquete gobernado tiene cosas que **el catálogo no
puede saber**, y un árbol reinducido las trae en blanco:

| del ORIGEN · se compara | de GOBIERNO · se ignora |
|---|---|
| `columns` y sus tipos | `owner` |
| `reads` · `changes` | `oos.maturity` |
| `primaryKey` · `uniqueKeys` | `freshness` |
| `relations` | `materialized` |
| el objeto físico | etiquetas y descripciones escritas por una persona |

Sin esa frontera **todos los paquetes gobernados salen derivados, siempre**, y el detector es
inservible el primer día. Es la fatiga de alertas del §2, pero por construcción.

> **El aserto que la fija, y va primero:** el catálogo capturado
> —`tests/catalogos/bigquery-rubix-demo-ventas.json`— contra el paquete que sale de él tiene que
> dar **cero deriva**. Lo que sobre es exactamente lo que hay que meter en la columna derecha.
> Un detector que encuentra deriva donde no la hay no se puede usar dos veces.

## 5. Tramo 3 · la rejilla

Cada deriva se responde **tres veces**, y las tres preguntas son de sitios distintos:

- **¿ensancha o estrecha?** — Iceberg · es del cambio en sí
- **¿a quién le duele?** — Confluent · tiene dirección
- **¿a cuántos?** — observabilidad · y aquí se sabe, por linaje

| clase | dirección | a quién | salida |
|---|---|---|---|
| objeto nuevo | ensancha | a nadie | informe |
| objeto que desaparece | estrecha | a quien lo nombre | `OOS5007` |
| columna nueva | ensancha | a nadie | informe |
| columna que desaparece | estrecha | a quien la proyecte | `OOS5007` |
| tipo que ensancha | ensancha | al consumidor viejo | `OOS5033` — §7 |
| tipo que estrecha | estrecha | a quien la lea | `OOS5002` |
| `reads` admite menos | estrecha | al planificador | `OOS5031` |
| `reads` admite más | ensancha | a nadie | informe |
| `changes` degrada | estrecha | a la copia | `OOS5032` |
| `primaryKey` cambia | incomparable | al índice | `OOS5019` |

**Seis de diez son informe y no código.** No es que falten seis códigos: una columna nueva no
rompe un contrato, y darle un código de compatibilidad sería inventar errores para avisos — que
es literalmente cómo se llega a la fatiga que el sector reporta.

## 6. Lo que se decide, y por qué

**El código de salida es `0` / `2` / `1`**, y no un `sysexit`. Es la convención de Terraform y
existe para que un detector entre en un pipeline sin que nadie parsee su salida.

**No se borra nada solo, nunca.** `DeleteBehavior: LOG` es el defecto que recomienda quien lleva
años con esto. De los dos errores posibles se comete el reversible.

**El tercer verbo no abre ningún PR.** Escribe los documentos corregidos y para; quien abre el PR
es el CI que lo llama, que ya tiene el permiso. El argumento decide solo: **un mando que abre un
PR no se puede ejercer en la suite; uno que escribe ficheros, sí.** Y si algún día hiciera falta
hablar con la forja, el patrón de la casa ya está probado dos veces —`bq` y `psql`—: delegar en
un programa que el usuario ya autenticó, nunca meter un token de escritura en el mismo binario
que lee los datos del cliente.

**Lee de las dos, y `--from` es la que decide.** Un aserto que exigiera un servidor no se
ejecutaría nunca en la suite, así que `--from` es lo que hace esto **probable**; `--source` es un
acto en vez de dos. Y separarlas evita lo que importa: *«no pude preguntarle al origen»* y *«el
origen cambió»* son dos respuestas, y mezclarlas convierte una credencial caducada en una alerta
de deriva.

**No es precondición de `ore materialize`, y se descartó con su motivo.** El argumento a favor es
estrecho pero real —una copia viaja **sellada** con la clasificación de los campos de la vista, y
si el origen dejó de empujar la proyección el sello miente (`OOS2029`)—. Contra él, dos cosas que
pesan más: un chequeo completo antes de cada poblado es una consulta facturada en el camino
caliente, y **la mitad barata ya se hace** — `materializar.rs` compara el testigo que el origen
contesta con el que la tabla declara, sin pedir nada de más, porque viene en la respuesta que ya
se pide.

> **La regla que sale de ahí:** lo que el driver ya contesta se comprueba gratis; lo que hay que
> ir a preguntar es **otro acto**. Si algún día hay puerta, será declarada y opcional —la forma
> de Atlas, `on_error = FAIL | CONTINUE`— y sobre la operación deliberada, no sobre la programada.

## 7. Lo que hay que tener antes de empezar

**`OOS5002` se dispara hoy sobre cualquier cambio de tipo**, incluidos los que ensanchan, y su
texto dice *«tipo estrechado»*. El veredicto es correcto —las dos direcciones rompen— pero **la
atribución es falsa**, y un código que dice algo que no pasó es lo que este árbol persigue.

Hace falta la relación de ensanche, y **es normativa**: distingue dos códigos, y un código decide
un salto de versión, que es una promesa a los consumidores. Una promesa no puede vivir en una
herramienta. Por eso va en OOS y no en el detector — y por eso es lo primero.

## 8. El orden

1. **`OOS5002` y su espejo** — la relación de ensanche en `ore-core::types`, escrita en la
   especificación, y `diff` atribuyendo la dirección correcta. *(§7)*
2. **El aserto de cero deriva** sobre el catálogo capturado. *(§4)*
3. **La frontera origen/gobierno**, que el aserto anterior define por diferencia.
4. **`ore drift --from`**, con la rejilla del §5 y el código de salida del §6.
5. **`--source`**, que es el mismo mando con el catálogo recién pedido.
6. **La corrección escrita**, que es el emisor del inductor sobre documentos que ya existen. Lo
   difícil no es técnico: es decidir qué se corrige solo y qué se pregunta. Va aparte.
