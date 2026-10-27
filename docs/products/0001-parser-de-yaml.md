# 0001 · Parser de YAML

**Estado:** aceptado · **Fecha:** 2026-08-29 · **Decide:** `saphyr-parser`, capa de eventos

---

## Contexto

Es la primera dependencia de `ore-core` y no es una cualquiera. La forma canónica
de OOS exige que dos entradas equivalentes produzcan **bytes idénticos**
(`90-canonical-form`), y eso depende de cómo el parser resuelva tipos implícitos,
anclas y alias. `serde_yaml` está archivado, hay varios sucesores, y elegir mal
compromete la garantía **G1** desde el primer commit.

## Contra qué se midió

No «cuál es mejor» en abstracto, sino qué exige **nuestra** especificación. Dos
criterios eliminatorios y tres deseables.

| | Criterio | Por qué |
|---|---|---|
| **T1** | YAML 1.2: `no` es la cadena `"no"` | un enum de negocio puede contener `no`, `yes`, `on`, `off`. El *problema de Noruega* corrompería datos en silencio |
| **T4** | distinguir escalar **plano** de **entrecomillado**, sin perder precisión | `OOS6003` rechaza `68400.50` como número y lo admite como cadena. Sin esa distinción el código es inaplicable, y con `f64` la precisión ya se perdió |
| T2 | posiciones por nodo | *el error es el producto*: `Employee.yaml:22` |
| T3 | claves duplicadas | dos verdades en un documento |
| T6 | anclas y alias | la spec exige que no sobrevivan a la normalización |

## Resultados

| | T1 | T4 estilo | T4 precisión | T3 | T2 | T6 |
|---|:---:|:---:|:---:|:---:|:---:|:---:|
| `serde_yaml_ng` 0.10 | ✓ | ✓ | ✗ `68400.5` | ✓ | ✗ | ✓ |
| `saphyr` 0.0.12 (árbol) | ✓ | ✓ | ✗ `68400.5` | ✗ acepta | ✗ | ✓ |
| `marked-yaml` 0.8 | ✓ | ✗ no distingue | ✓ | ✗ acepta | ✓✓ | ✗ no soporta |
| `yaml-rust2` 0.12 | ✓ | ✓ `Real("68400.50")` | ✓ | ✓ | ✗ | ✓ |
| **`saphyr-parser` 0.0.12** | **✓** | **✓ estilo explícito** | **✓ texto crudo** | **n/a** | **✓ por evento** | **✓ eventos** |

Salida real de la capa de eventos:

```
escalar "68400.50"   estilo Plain          linea 2
escalar "68400.50"   estilo DoubleQuoted   linea 3
escalar "texto\n"    estilo Literal        linea 5
```

## Decisión

**`saphyr-parser`, a nivel de eventos.** No la API de árbol de `saphyr`, ni ningún
deserializador de serde.

El razonamiento que lo ordena todo:

> **No queremos un deserializador. Queremos un front-end de compilador.**

El trabajo de serde es *tirar la sintaxis* y quedarse con los datos. Nosotros
necesitamos la sintaxis: el estilo del escalar decide si `OOS6003` se dispara, y
la posición decide si el error sirve para algo. Todos los candidatos basados en
serde perdieron por hacer bien aquello para lo que existen.

Ventajas concretas:

1. **Texto crudo del escalar.** `68400.50` llega tal cual. La precisión no se
   pierde porque nunca se convierte.
2. **Estilo explícito.** `Plain` frente a `DoubleQuoted` resuelve `OOS6003` sin
   heurísticas.
3. **Posición por evento.** `Employee.yaml:22` deja de ser aspiracional.
4. **Rust puro**, sin `unsafe-libyaml` ni FFI de plataforma — coherente con
   mantener la compilación cruzada trivial.
5. **Linaje mantenido:** `saphyr` sucede a `yaml-rust2`, que sucede a `yaml-rust`.

## Consecuencias

**Claves duplicadas y anclas pasan a ser decisión nuestra.** La capa de eventos
no construye el mapa: lo construimos nosotros, así que detectamos duplicados
**con las dos posiciones** y podemos decir *«`name` declarado en la línea 1 y en
la 2»* en lugar del genérico del parser. Es más trabajo y mejor error.

Lo mismo con anclas: vemos los eventos de ancla y alias y aplicamos la política
de la spec —que no sobrevivan a la normalización— en lugar de heredar la de una
librería.

**Coste asumido:** construir el árbol es trabajo nuestro. Es el precio de un
front-end de compilador, y es exactamente el trabajo que la fase 0 tenía que
hacer de todos modos.

**Revisable si:** `saphyr-parser` deja de mantenerse, o si aparece un parser YAML
1.2 en Rust puro que exponga estilo y posiciones con una API de árbol. La
evaluación es reproducible; el criterio, no la conclusión, es lo que hay que
conservar.
