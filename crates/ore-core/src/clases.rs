//! **Las clases de repositorio** (0036): la tabla del producto.
//!
//! Un repositorio guarda en su manifiesto **una clave** (`plantilla`) y **una
//! versión** (`plantillaVersion`), y nada más. Qué significa esa clave —con qué
//! ficheros nace, qué puede hacer su sesión, qué enseña la consola— vive
//! **aquí**, en el producto, y se versiona con él. Es lo que Foundry hace con
//! los tipos de repositorio y sus PRs de actualización.
//!
//! # Por qué la tabla no está en el árbol
//!
//! Porque si estuviera, cada cliente tendría una versión distinta del producto
//! y «actualizar la plantilla» no querría decir nada. El árbol guarda **qué
//! clase soy**; el producto sabe **qué es esa clase hoy**.
//!
//! # Una plantilla es UN ÁRBOL DE FICHEROS (⑧a)
//!
//! Medido antes de escribirlo (`medida-la-plantilla.py`): las cinco clases
//! nacían con **dos ficheros y CERO líneas de código** —el manifiesto y un
//! comentario que describía lo que habría que escribir—, y ninguna sembraba
//! `pyproject.toml`, así que toda instancia heredaba la capa de la celda: el
//! motor de la capa por repositorio (③) estaba hecho y **la plantilla no lo
//! usaba**. Una plantilla es, desde aquí, lo que una instancia necesita para
//! ser útil el primer minuto: la disposición, un ejemplo que **corre** y **el
//! fichero donde se declara su entorno**.
//!
//! # El lenguaje es una CLASE, no un campo
//!
//! `transforms-python` y `transforms-java` son dos plantillas con **dos
//! versiones**: con una sola clase, el día que cambie una, o se suben las dos
//! o se miente en la columna «UPGRADE». `familia` y `lenguaje` están para
//! AGRUPARLAS en la consola; quien identifica es `id`, como siempre.
//!
//! # Lo que una clase NO puede
//!
//! **Conceder.** Una clase ajusta hacia abajo —«este repositorio no escribe
//! datasets»— y nunca hacia arriba: lo que gobierna sigue siendo la etiqueta
//! (el conducto), la declaración (`@transform`) y quién escribió. El techo se
//! aplica donde ya se aplica el gobierno, no aquí.

/// Una clase de repositorio, tal como el producto la trae hoy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clase {
    /// La clave que el manifiesto guarda. **Es la identidad**, y lleva el
    /// lenguaje dentro (`transforms-python`).
    pub id: &'static str,
    /// Qué clase de trabajo es, sin el lenguaje: para agrupar en la consola.
    /// No identifica nada — dos familias iguales son dos plantillas distintas.
    pub familia: &'static str,
    /// En qué se escribe. `""` cuando no hay código (`semantics` edita
    /// documentos del árbol). Tampoco identifica: agrupa.
    pub lenguaje: &'static str,
    /// Cómo se llama en la consola.
    pub titulo: &'static str,
    /// Qué se hace aquí, en una frase. La consola la enseña en su tarjeta: la
    /// tenía escrita a mano, y así había dos descripciones de lo mismo.
    pub descripcion: &'static str,
    /// La versión de la plantilla en **este** producto. Sube cuando la semilla
    /// o lo que la clase promete cambian.
    pub version: i64,
    /// Con qué ficheros nace, relativos a la carpeta del repositorio. El
    /// manifiesto no está aquí: lo escribe quien crea.
    pub semilla: &'static [(&'static str, &'static str)],
    /// **El techo** (0036 ⑤): ¿lo que corre aquí puede **escribir datos**?
    ///
    /// `false` no concede nada nuevo a nadie: **quita**. Un `analytics` no
    /// escribe aunque su código lo declare, y eso se aplica donde ya se aplica
    /// el gobierno —el catálogo y el `confirmar`—, no en el SDK.
    pub escribe: bool,
    /// ¿Hay sesión que abrir? `semantics` edita documentos del árbol: no
    /// ejecuta, y pedirle un puesto es un 422 que lo dice.
    pub ejecuta: bool,
    /// El perfil de máquina que pide la clase (0027). `None`: el de hoy.
    /// Todavía no lo usa nadie — lo dice la ADR, no el código.
    pub perfil: Option<&'static str>,
}

/// El ejemplo de `transforms-python`: **código, no un comentario**. Corre tal
/// cual en cuanto las dos referencias apuntan a algo (como el
/// `SOURCE_DATASET_PATH` de Foundry), y es el mismo transform que la prueba de
/// fuego ejercita contra agentes de verdad (`el-puesto.sh` 10 y 11).
const TRANSFORMS_PY: &str = "\
# Un transform DECLARA qué lee y qué escribe, y el servidor lo hace cumplir
# (ADR 0031 · W3.7): mientras corre, la sesión sólo resuelve sus `inputs` y
# sólo escribe su `output`. Pedir otra cosa es un PermissionError, no un aviso.
#
# `transform`, `over` y `write` son del SDK del puesto y ADEMAS los pone la
# sesión en el espacio de la celda: el import no cambia lo que corre — hace que
# el editor sepa de qué hablas (ADR 0037 ③a).
#
# Los nombres son de tres partes, `base.schema.nombre` (0038, como Unity): la
# base de datos, su schema y el dataset. En `default` basta `base.nombre`.
# Cambia las dos referencias por las tuyas y dale a Run.

from ore import transform, over, write

ENTRADA = \"mi_base.mi_schema.mi_dataset\"
SALIDA = \"mi_base.mi_schema.mi_resumen\"


@transform(inputs=[ENTRADA], output=SALIDA)
def resumir():
    tabla = over(ENTRADA, format=\"arrow\")
    return write(SALIDA, tabla)


escrito = resumir()
print(\"rows\", escrito[\"rows\"])
";

/// El `pyproject.toml` de una instancia de Python: **el sitio donde declarar**.
///
/// ⭐ Nace VACÍO a propósito. Lo que hacía falta no era una dependencia: era el
///   fichero — sin él, un repositorio no puede declarar nada y se come la capa
///   de la celda (0036 ③). Sembrar `polars` «por si acaso» costaría construir
///   una capa para algo que el ejemplo no usa.
const PYPROJECT_PY: &str = "\
# Las dependencias de ESTE repositorio (ADR 0036 ③). Lo que declares aquí es
# SUYO: se resuelve en su propia capa y no la cargan sus vecinos. Lo que está
# en el paquete o en la raíz del árbol lo sigue teniendo todo el mundo.
#
# Nace vacío: declarar algo que nadie usa sería construir una capa para nada.
# Descomenta la línea de abajo, guarda y abre el repositorio — la capa se
# resuelve sola y la sesión nace con ella.

[project]
name = \"repositorio\"
version = \"0.1.0\"
dependencies = []
# dependencies = [\"polars\"]
";

/// El `pom.xml` de una instancia de Java: **el sitio donde declarar** (0037 ③c).
///
/// ⭐ Nace VACÍO, por lo mismo que el de Python: lo que faltaba no era una
///   biblioteca, era el fichero. Hasta ③c un repositorio de Java no podía usar
///   ni una, porque la capa (0036 ③) leía sólo `pyproject.toml`.
///
/// ⛔ Y dice lo que NO hace: de aquí se lee `<dependencies>` y nada más. Un pom
///   trae `<build>`, `<plugins>`, `<profiles>`; no se honran, y por eso se
///   avisa en el propio fichero en vez de dejar que alguien lo descubra cuando
///   su plugin no corra.
const POM_JVM: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!--
  Las dependencias de ESTE repositorio (ADR 0036 iii, 0037 iii.c). Lo que
  declares aqui es SUYO: se resuelve en su propia capa y no la cargan sus
  vecinos. Lo que esta en el paquete o en la raiz del arbol lo sigue teniendo
  todo el mundo.

  Nace vacio: declarar algo que nadie usa seria construir una capa para nada.
  Descomenta el ejemplo de abajo, guarda y abre el repositorio - la capa se
  resuelve sola y la sesion nace con ella.

  DE ESTE FICHERO SE LEE "dependencies" Y NADA MAS. Ni "dependencyManagement",
  ni "build", ni "plugins", ni "profiles": no se honran y no se finge que si.
  Cada dependencia necesita su "version" -sin ella no hay quien la fije- y el
  ambito "test" no baja al puesto, que la capa es lo que hace falta para CORRER.

  Y LO QUE TRAE LA IMAGEN, MANDA: Arrow, Jackson, slf4j y el resto del SDK
  llegan con la sesion. Si declaras otra version de algo de eso, gana la de la
  sesion y el informe de la capa te lo dice.
-->
<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>
  <groupId>repositorio</groupId>
  <artifactId>repositorio</artifactId>
  <version>0.1.0</version>

  <dependencies>
    <!-- <dependency>
      <groupId>org.apache.commons</groupId>
      <artifactId>commons-lang3</artifactId>
      <version>3.17.0</version>
    </dependency> -->
  </dependencies>
</project>
"#;

/// El mismo transform, en Java, y **como un fichero Java de verdad** (0037 ③b):
/// un import estático y una clase con `main`.
///
/// ⭐ No es un capricho de estilo: era un SNIPPET DE JSHELL —`var` y llamadas
///   sueltas en el tope— y ningún editor entiende eso. Medido con jdtls sobre
///   esta misma semilla: cuatro errores de sintaxis sobre nuestra propia
///   plantilla. Como unidad de compilación, cero.
///
/// ⛔ Y el ejecutor no cambia: el agente ya parte la celda en snippets y, si
///   declara una clase con `static void main(`, la llama — lo dice su propio
///   comentario desde W3.4. Medido con un JShell 21 de verdad: esto corre e
///   imprime, con el `public` y el import estático dentro.
const TRANSFORMS_JAVA: &str = "\
// Un transform DECLARA qué lee y qué escribe, y el servidor lo hace cumplir
// (ADR 0031 · W3.7): mientras corre, la sesión sólo resuelve sus `inputs` y
// sólo escribe su `output`.
//
// `transform`, `over` y `write` son del SDK del puesto (`ore.Ore`) y ADEMÁS los
// pone la sesión: el import estático no cambia lo que corre — hace que el
// editor sepa de qué hablas (ADR 0037 ③b).
//
// ⛔ El nombre del fichero ES el nombre de la clase pública: si renombras uno,
//   renombra el otro.
//
// Los nombres son de tres partes, `base.schema.nombre` (0038, como Unity): la
// base de datos, su schema y el dataset. En `default` basta `base.nombre`.
// Cambia las dos referencias por las tuyas y dale a Run.
import static ore.Ore.*;

import java.util.List;
import java.util.Map;

public class Ejemplo {
    static final String ENTRADA = \"mi_base.mi_schema.mi_dataset\";
    static final String SALIDA = \"mi_base.mi_schema.mi_resumen\";

    public static void main(String[] args) throws Exception {
        Map<String, Object> escrito = transform(\"resumir\", List.of(ENTRADA), SALIDA,
                () -> write(SALIDA, over(ENTRADA)));
        System.out.println(\"rows \" + escrito.get(\"rows\"));
    }
}
";

/// El transform escrito en SQL: **un `.sql` de verdad** (0038 P7), una sentencia
/// que escribe.
///
/// ⭐ Antes era un `.py` con la consulta en una cadena, y con motivo: una celda
///   SQL a secas leía pero no escribía (0036). Desde el SQL del árbol (0037 y
///   6d451da) un `.sql` es la unidad —en la sesión y como trabajo—, y el que
///   escribe es UNA sentencia `create or replace dataset … as select …`: lo que
///   lee y lo que escribe lo dice la propia sentencia (`ore sql`), sin
///   `@transform` que lo repita. Medido en `medida-la-semilla-sql.sh`.
const TRANSFORMS_SQL: &str = "\
-- Un transform escrito en SQL: UNA sentencia que escribe. Lo que lee y lo que
-- escribe lo dice la propia sentencia, y el servidor lo hace cumplir.
--
-- Lo que se escribe es un dataset: `CREATE OR REPLACE DATASET … AS SELECT`
-- sobrescribe; `INSERT INTO … SELECT` anexa; `INSERT OR REPLACE INTO … SELECT`
-- hace upsert. Un `SELECT` suelto lee y no escribe.
--
-- Los nombres son de tres partes, `base.schema.nombre` (0038, como Unity): la
-- base de datos, su schema y el dataset. En `default` basta `base.nombre`.
-- Cambia los dos por los tuyos y dale a Run.

CREATE OR REPLACE DATASET mi_base.mi_schema.mi_resumen AS
SELECT pais, count(*) AS n
FROM mi_base.mi_schema.mi_dataset
GROUP BY pais
";

const ANALYTICS_PY: &str = "\
# Un análisis LEE, y no declara nada: leer no escribe. Y su clase lo hace
# cumplir — un `analytics` no escribe datos aunque el código lo pida (0036 ⑤),
# así que aquí se mira, se cuenta y se decide qué hacer después.
#
# `over` y `sql` son del SDK del puesto y ADEMAS los pone la sesión en el
# espacio de la celda: el import no cambia lo que corre — hace que el editor
# sepa de qué hablas (ADR 0037 ③a).

from ore import over, sql

# Los nombres son de tres partes, `base.schema.nombre` (0038, como Unity): la
# base de datos, su schema y el dataset. En `default` basta `base.nombre`.
FUENTE = \"mi_base.mi_schema.mi_dataset\"

filas = over(FUENTE)
print(FUENTE, \"→\", len(filas), \"filas\")
print(sql(f\"select count(*) as n from {FUENTE}\"))
";

const MODELS_PY: &str = "\
# El entrenamiento de un modelo. Lo que salga se declara como `TrainedModel`
# con su `trainedFrom`: es lo que hace que el linaje no se corte (ADR 0029).
#
# `over` y `declare` son del SDK del puesto y ADEMAS los pone la sesión en el
# espacio de la celda: el import no cambia lo que corre — hace que el editor
# sepa de qué hablas (ADR 0037 ③a).

from ore import over, declare

# Los nombres son de tres partes, `base.schema.nombre` (0038, como Unity): la
# base de datos, su schema y el dataset. En `default` basta `base.nombre`.
ENTRADA = \"mi_base.mi_schema.mi_dataset\"
MODELO = \"mi_base.mi_schema.mi_modelo\"
BASE, SCHEMA, NOMBRE = MODELO.split(\".\")

filas = over(ENTRADA)
# ... entrenar con lo que declares en el pyproject.toml de este repositorio ...
digest = \"sha256:0000000000000000000000000000000000000000000000000000000000000000\"

print(declare({
    \"kind\": \"TrainedModel\",
    \"metadata\": {\"name\": NOMBRE, \"namespace\": BASE, \"schema\": SCHEMA},
    \"spec\": {\"owner\": \"team:cambiame\", \"trainedFrom\": [ENTRADA], \"digest\": digest},
}))
";

/// La semilla de `functions-python`: **solo el código** (0050 G1, G2, G4).
///
/// v8: un ejemplo que enseña lo que el contrato cumple (G3) —`Decimal` para el
/// dinero, `date`, opcionales y una salida con un campo que puede faltar— y
/// cómo se usa: Run en la sesión corre su bloque `if __name__ == "__main__"`
/// (la sesión ejecuta el fichero como `__main__`; el arnés, como
/// `ore_funcion`, así que al invocarla no corre), desde código con
/// `ore.get_function(...)` y desde Pipelines. Nombres en inglés, como la consola.
///
/// `@function` y las anotaciones del `def` son el contrato, y el documento
/// `Function` se deriva de ellos (OOS v1alpha18 01 §4): no se siembra, lo
/// escribe el commit que crea el repositorio, en `functions/` del paquete
/// —Assets → Functions—, como con cualquier `@function` que se guarde. Nace
/// **sin `over`**, sobre sus parámetros, para que compile e invoque en cuanto
/// el repositorio existe. `{{paquete}}`, `{{carpeta}}` y `{{funcion}}` los pone
/// [`sembrar`]: el `def` se llama como la función, para que dos repositorios
/// del mismo paquete no choquen.
const FUNCTIONS_PY: &str = "\
# Una FUNCIÓN publicada: `{{paquete}}.{{funcion}}` (ORE 0050).
#
# Escribes Python; la plataforma hace el resto. `@function` y las anotaciones
# del `def` son su contrato —lo que recibe y lo que devuelve— y el docstring,
# su descripción. Al hacer commit se publica en Assets → Functions: su
# documento lo escribe ore, no se edita.
#
#   · Run, en tu sesión: corre el bloque `if __name__ == \"__main__\"` de abajo.
#   · Desde otro código: `ore.get_function(\"{{paquete}}.{{funcion}}\")`, y se llama.
#   · Desde Pipelines:   el operador Function, con sus parámetros.
#
# Los tipos se cumplen: \"2026-09-15\" llega como `date` y 120.50 como `Decimal`
# exacto; devolver otro tipo es un error que dice su línea.
from dataclasses import dataclass
from datetime import date
from decimal import Decimal

from ore import function


@dataclass
class InvoiceStatus:
    status: str                        # \"paid\", \"current\" u \"overdue\"
    outstanding: Decimal
    days: int                          # hasta el vencimiento; negativo si ya venció
    surcharge: Decimal | None = None   # solo si venció


@function(timeout=\"30s\")
def {{funcion}}(amount: Decimal, due: date, paid: Decimal = Decimal(\"0\"),
                today: date | None = None) -> InvoiceStatus:
    \"\"\"The status of an invoice: what is outstanding, the days until it is due and the surcharge if it is overdue.\"\"\"
    today = today or date.today()
    outstanding = max(amount - paid, Decimal(\"0\"))
    days = (due - today).days
    if outstanding == 0:
        return InvoiceStatus(\"paid\", outstanding, days)
    if days >= 0:
        return InvoiceStatus(\"current\", outstanding, days)
    surcharge = (outstanding * Decimal(\"0.0005\") * -days).quantize(Decimal(\"0.01\"))  # 0,05 % por día
    return InvoiceStatus(\"overdue\", outstanding, days, surcharge)


# Sobre un dataset (una llamada por fila), lo que lee y los modelos que llama
# se declaran en el decorador:
#     over=\"mi_base.mi_schema.invoices\"     la fila llega como primer parámetro
#     reads=[\"mi_base.mi_schema.clients\"]   y se lee con `ore.over`
#     models=[\"extractor\"]                  y se llama con `ore.model`

if __name__ == \"__main__\":
    print({{funcion}}(Decimal(\"120.50\"), \"2026-09-15\", today=\"2026-10-02\"))
    # InvoiceStatus(status='overdue', outstanding=Decimal('120.50'), days=-17, surcharge=Decimal('1.02'))
";

/// Si un fichero de la semilla se siembra en `paquete`. Un documento gobernado
/// (`functions/*.yaml`) solo vive en un paquete cuyo nombre puede ser
/// `namespace` (`OOS2030`): uno con guion —el de un proyecto de antes del
/// 2026-10-01, `test-project`, hasta `ore migrate proyectos`— no puede
/// tenerlo, y sembrarlo haría que el commit que crea el repositorio no
/// compilara. Ahí nace el código solo, como en la v4.
pub fn se_siembra(rel: &str, paquete: &str) -> bool {
    !rel.ends_with(".yaml") || crate::pertenencia::puede_ser_namespace(paquete)
}

/// El ejemplo de `functions-typescript`: el mismo `def` que el de Python, como
/// función exportada de un módulo. **Corre en la sesión** (`puesto-node`, 0031
/// W3.4: un `.ts` con `export` se importa y sus exports quedan en el contexto)
/// y no se invoca todavía como `Function`: `runtime: node` entra por la misma
/// regla que `python` cuando haya quien lo ejecute (0050 R3), y la gramática no
/// promete lo que nadie cumple. Por eso no siembra contrato. Y no siembra
/// `package.json`: la sesión de Node nace con lo que trae su imagen.
const FUNCTIONS_TS: &str = "\
// Una función de TypeScript: entra lo que declara, sale un objeto. Hoy corre en
// tu sesión (Run); publicarla como `Function` invocable con parámetros
// —`runtime: node`, como las de Python— llega con ORE 0050 R3.
//
// Desde la sesión lees los datasets por su nombre en tres partes (0038),
// `over(\"mi_base.mi_schema.mi_dataset\")`, y `sql(...)`: los pone el SDK del
// puesto en el contexto.

export function ejemplo(texto: string, veces: number = 1): { resultado: string; longitud: number } {
  const resultado = Array(veces).fill(texto).join(\" \");
  return { resultado, longitud: resultado.length };
}

console.log(ejemplo(\"hola\", 2));
";

/// Rellena los huecos de una semilla: `{{paquete}}` (el `namespace` de lo que
/// declare), `{{carpeta}}` (dónde vive el repositorio, para su `entrypoint`) y
/// `{{funcion}}` (un nombre de función **único en el paquete**, sacado de la
/// carpeta: dos repositorios en el mismo paquete no siembran el mismo). Una
/// semilla sin huecos sale igual.
pub fn sembrar(contenido: &str, paquete: &str, carpeta: &str) -> String {
    let ultimo = carpeta.rsplit('/').next().unwrap_or(carpeta);
    let mut f: String = ultimo
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if !f.starts_with(|c: char| c.is_ascii_alphabetic()) {
        f.insert_str(0, "f_");
    }
    contenido
        .replace("{{paquete}}", paquete)
        .replace("{{carpeta}}", carpeta)
        .replace("{{funcion}}", &format!("{f}_invoice_status"))
}

/// Las cinco clases de hoy. Añadir una es una fila más, y subir su `version`
/// es lo que hace que un repositorio se pueda actualizar.
pub const CLASES: &[Clase] = &[
    Clase {
        id: "transforms-python",
        familia: "transforms",
        lenguaje: "python",
        escribe: true,
        ejecuta: true,
        perfil: None,
        titulo: "Transforms",
        descripcion: "Transform and integrate datasets using Python.",
        // 2 porque la plantilla CAMBIÓ (⑧a): lo escrito con la de antes —un
        // comentario y sin `pyproject.toml`— sale `actualizable: true`, que es
        // lo que la columna «UPGRADE» existe para decir.
        // 4: la semilla nombra en tres partes (0038 P7).
        version: 5,
        semilla: &[
            ("pyproject.toml", PYPROJECT_PY),
            ("transforms/ejemplo.py", TRANSFORMS_PY),
        ],
    },
    Clase {
        id: "transforms-java",
        familia: "transforms",
        lenguaje: "java",
        escribe: true,
        ejecuta: true,
        perfil: None,
        titulo: "Transforms",
        descripcion: "Transform and integrate datasets using Java on the JVM.",
        // 3 porque la plantilla CAMBIÓ (0037 ③c): lo escrito con la de antes
        // —sin `pom.xml`— sale `actualizable: true`, que es lo que la columna
        // «UPGRADE» existe para decir.
        // 4: la semilla nombra en tres partes (0038 P7).
        version: 5,
        semilla: &[
            ("pom.xml", POM_JVM),
            ("transforms/Ejemplo.java", TRANSFORMS_JAVA),
        ],
    },
    Clase {
        id: "transforms-sql",
        familia: "transforms",
        lenguaje: "sql",
        escribe: true,
        ejecuta: true,
        perfil: None,
        titulo: "Transforms",
        descripcion: "Transform and integrate datasets writing SQL.",
        // 4: un `.sql` de verdad, en tres partes y SIN `pyproject.toml` (0038 P7):
        // la consulta corre en el entorno de Python, pero no puede usar nada de
        // lo que ese fichero declare — en un repositorio de SQL sólo confundía.
        // Actualizar deja lo de antes (`ejemplo.py`, `pyproject.toml`): es de
        // quien lo tenga.
        // 5: lo que se escribe es un DATASET (`create or replace dataset`): una
        // Table es un puntero a un objeto de un origen y no se crea desde SQL.
        version: 5,
        semilla: &[("transforms/ejemplo.sql", TRANSFORMS_SQL)],
    },
    Clase {
        id: "analytics-python",
        familia: "analytics",
        lenguaje: "python",
        escribe: false,
        ejecuta: true,
        perfil: None,
        titulo: "Analytics",
        descripcion: "Analyze your datasets using your preferred data science environment.",
        // 4: la semilla nombra en tres partes (0038 P7).
        version: 4,
        semilla: &[
            ("pyproject.toml", PYPROJECT_PY),
            ("analisis/ejemplo.py", ANALYTICS_PY),
        ],
    },
    Clase {
        id: "models-python",
        familia: "models",
        lenguaje: "python",
        escribe: true,
        ejecuta: true,
        perfil: None,
        titulo: "Models",
        descripcion: "Create, test and train models for machine learning, forecasting and more.",
        // 4: la semilla nombra en tres partes (0038 P7).
        version: 4,
        semilla: &[
            ("pyproject.toml", PYPROJECT_PY),
            ("modelos/entrenar.py", MODELS_PY),
        ],
    },
    Clase {
        id: "functions-python",
        familia: "functions",
        lenguaje: "python",
        escribe: false,
        ejecuta: true,
        perfil: None,
        titulo: "Functions",
        descripcion: "Write typed Python functions over your datasets and models, invocable with parameters.",
        // 4: la semilla nombra en tres partes (0038 P7).
        // 5: nace con una función de verdad, la pareja contrato + código (0050 P5).
        // 6: el código es la fuente, `@function` con anotaciones (0050 G1).
        // 7: sin documento: lo escribe el commit, en el paquete (0050 G2).
        // 8: un ejemplo de verdad: Decimal, date, opcionales y Run (0050 G4).
        version: 9,
        semilla: &[
            ("pyproject.toml", PYPROJECT_PY),
            ("funciones/ejemplo.py", FUNCTIONS_PY),
        ],
    },
    // 0050: la familia `functions` en dos lenguajes, como `transforms` en
    // tres. TypeScript corre en la sesión (`puesto-node`); invocarla como
    // `Function` es R3 (`runtime: node`).
    Clase {
        id: "functions-typescript",
        familia: "functions",
        lenguaje: "typescript",
        escribe: false,
        ejecuta: true,
        perfil: None,
        titulo: "Functions",
        descripcion: "Write reusable functions in TypeScript. They run in your session; publishing them as invocable Functions comes next.",
        version: 1,
        semilla: &[("funciones/ejemplo.ts", FUNCTIONS_TS)],
    },
    // `semantics` no siembra código: lo suyo son documentos del árbol, y
    // sembrar una `Entity` a medias sería sembrar algo que no compila.
    Clase {
        id: "semantics",
        familia: "semantics",
        lenguaje: "",
        descripcion: "Define object types, links and actions: the semantic layer over your data.",
        escribe: false,
        ejecuta: false,
        perfil: None,
        titulo: "Semantics",
        version: 1,
        semilla: &[],
    },
];

/// Lo que se escribió con la clave de antes de ⑧a sigue resolviendo: un árbol
/// de un cliente puede tener `plantilla: transforms` escrito ayer, y no se
/// rompe por haberle puesto el lenguaje al nombre. No es una clase más —no se
/// lista ni se ofrece—: es una clave vieja que apunta a la de hoy.
const ANTIGUAS: &[(&str, &str)] = &[
    ("transforms", "transforms-python"),
    ("analytics", "analytics-python"),
    ("models", "models-python"),
    ("functions", "functions-python"),
];

/// Una familia de plantillas: lo que agrupa a las clases que hacen lo mismo
/// en distintos lenguajes. **No es una clase**: no se crea, no tiene versión y
/// no se guarda en ningún manifiesto — es cómo se dice el grupo.
///
/// ⭐ Vive aquí, y no en la consola, por lo mismo que las clases: la consola
///   tenía las cinco descripciones escritas a mano, y el día que cambiara una
///   habría dos textos para lo mismo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Familia {
    pub id: &'static str,
    pub titulo: &'static str,
    /// Qué se hace en esta familia, **sin nombrar lenguaje**: es la frase de
    /// la tarjeta que agrupa, y detrás hay una plantilla por lenguaje.
    pub descripcion: &'static str,
}

pub const FAMILIAS: &[Familia] = &[
    Familia {
        id: "transforms",
        titulo: "Transforms",
        descripcion: "Transform and integrate datasets using Python, SQL or Java.",
    },
    Familia {
        id: "analytics",
        titulo: "Analytics",
        descripcion: "Analyze your datasets using your preferred data science environment.",
    },
    Familia {
        id: "models",
        titulo: "Models",
        descripcion: "Create, test and train models for machine learning, forecasting and more.",
    },
    Familia {
        id: "functions",
        titulo: "Functions",
        descripcion: "Write reusable code for pipelines, transforms and applications.",
    },
    Familia {
        id: "semantics",
        titulo: "Semantics",
        descripcion: "Define object types, links and actions: the semantic layer over your data.",
    },
];

pub fn familia(id: &str) -> Option<&'static Familia> {
    FAMILIAS.iter().find(|f| f.id == id)
}

pub fn de(id: &str) -> Option<&'static Clase> {
    let id = ANTIGUAS
        .iter()
        .find(|(viejo, _)| *viejo == id)
        .map_or(id, |(_, nuevo)| *nuevo);
    CLASES.iter().find(|c| c.id == id)
}

/// El entorno de puesto que pide una clase por su lenguaje: `python`, `node`
/// o `jvm`. `None` cuando no hay código (`semantics`) o el lenguaje no corre
/// en ninguno — y entonces manda lo que pida quien abre, como antes de ⑧b.
pub fn entorno_de(clase: &Clase) -> Option<&'static str> {
    match clase.lenguaje {
        "python" | "sql" => Some("python"),
        "typescript" | "javascript" | "node" => Some("node"),
        "java" | "jvm" => Some("jvm"),
        _ => None,
    }
}

/// Los nombres, para decirlos en un error.
pub fn nombres() -> String {
    CLASES.iter().map(|c| c.id).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// 0050 P5: un repositorio `functions-python` nace con una función que
    /// COMPILA, en el árbol de verdad —el commit que lo crea pasa por la puerta
    /// de «no empeorar»—. También en un paquete con guion (`test-project`, el
    /// de un proyecto de antes en `victor`), y dos repositorios en el mismo
    /// paquete no chocan.
    /// Lo que el commit escribe de la semilla, byte a byte.
    const DOCUMENTO: &str = "\
# generado por ore desde {{carpeta}}/funciones/ejemplo.py:{{funcion}} · se edita el def, no este fichero
apiVersion: oos.dev/v1alpha18
kind: Function
metadata:
  name: {{funcion}}
  namespace: {{paquete}}
  description: 'The status of an invoice: what is outstanding, the days until it is due and the surcharge if it is overdue.'
spec:
  runtime: python
  entrypoint: {{carpeta}}/funciones/ejemplo.py:{{funcion}}
  input:
    amount: { type: Decimal, required: true }
    due: { type: Date, required: true }
    paid: { type: Decimal }
    today: { type: Date }
  output:
    status: { type: String, required: true }
    outstanding: { type: Decimal, required: true }
    days: { type: Integer, required: true }
    surcharge: { type: Decimal }
  limits: { timeout: '30s' }
";

    #[test]
    fn la_semilla_es_lo_que_su_codigo_da() {
        // La semilla es solo código, y se deriva entera: el documento que el
        // commit escribe de ella es este.
        assert!(
            de("functions-python")
                .unwrap()
                .semilla
                .iter()
                .all(|(r, _)| !r.ends_with(".yaml"))
        );
        let (paquete, carpeta) = ("ventas", "funciones-de-riesgo");
        let py = sembrar(FUNCTIONS_PY, paquete, carpeta);
        let d = ore_code::python::derivar(&py, &format!("{carpeta}/funciones/ejemplo.py"));
        assert!(d.sintaxis.is_empty() && d.version.is_empty(), "{:?}", d);
        let [f] = d.funciones.as_slice() else {
            panic!("una función: {:?}", d.funciones)
        };
        let firma = f.resultado.as_ref().expect("la plantilla se deriva");
        assert_eq!(
            ore_code::emitir::documento(firma, paquete),
            sembrar(DOCUMENTO, paquete, carpeta)
        );
    }

    #[test]
    fn la_semilla_de_functions_compila_donde_nace() {
        let c = de("functions-python").unwrap();
        for paquete in ["ventas", "test-project"] {
            let raiz =
                std::env::temp_dir().join(format!("ore-semilla-{paquete}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&raiz);
            let pkg = raiz.join("packages").join(paquete);
            std::fs::create_dir_all(&pkg).unwrap();
            std::fs::write(
                raiz.join("ontology.config.yaml"),
                "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: { name: t, version: 0.1.0 }\n",
            )
            .unwrap();
            std::fs::write(
                pkg.join("package.yaml"),
                format!(
                    "apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: {{ name: {paquete}, version: 0.1.0, status: draft, domain: {paquete} }}\nspec: {{ owner: \"team:x\" }}\n"
                ),
            )
            .unwrap();
            for carpeta in ["funciones-de-riesgo", "otra"] {
                for (rel, contenido) in c.semilla.iter().filter(|(r, _)| se_siembra(r, paquete)) {
                    let f = pkg.join(carpeta).join(rel);
                    std::fs::create_dir_all(f.parent().unwrap()).unwrap();
                    std::fs::write(f, sembrar(contenido, paquete, carpeta)).unwrap();
                }
            }
            // Lo que hace el commit que lo crea (G2): generar lo sembrado.
            let (p, _) = crate::validate::cargar_paquete(&raiz);
            crate::generar::aplicar(&crate::generar::plan(&p)).unwrap();
            let contrato = pkg.join("functions/funciones_de_riesgo_invoice_status.yaml");
            if paquete == "ventas" {
                assert!(pkg.join("functions/otra_invoice_status.yaml").exists());
                assert!(!pkg.join("funciones-de-riesgo/functions").exists());
                let yaml = std::fs::read_to_string(&contrato).unwrap();
                assert!(
                    yaml.contains("name: funciones_de_riesgo_invoice_status"),
                    "{yaml}"
                );
                assert!(
                    yaml.contains(
                        "entrypoint: funciones-de-riesgo/funciones/ejemplo.py:funciones_de_riesgo_invoice_status"
                    ),
                    "{yaml}"
                );
                assert!(!yaml.contains("{{"), "{yaml}");
            } else {
                // `test-project` no puede ser `namespace`: el código sí, el contrato no.
                assert!(!contrato.exists());
                assert!(
                    pkg.join("funciones-de-riesgo/funciones/ejemplo.py")
                        .exists()
                );
            }
            let d = crate::validate::validate_package(&raiz);
            assert!(
                d.is_empty(),
                "{paquete}: {:?}",
                d.iter()
                    .map(|x| format!("{} {}", x.code.as_str(), x.message))
                    .collect::<Vec<_>>()
            );
            let _ = std::fs::remove_dir_all(&raiz);
        }
        // Una carpeta que empieza por número sigue dando un nombre válido.
        assert!(sembrar("{{funcion}}", "p", "a/2024").starts_with("f_2024"));
    }

    #[test]
    fn el_techo_quita_y_nunca_concede() {
        // Lo que escribe datos es lo que produce un Dataset o un modelo; leer y
        // publicar una `Function` no escriben.
        assert!(de("transforms-python").unwrap().escribe);
        assert!(de("transforms-java").unwrap().escribe);
        assert!(de("models-python").unwrap().escribe);
        assert!(!de("analytics-python").unwrap().escribe);
        assert!(!de("functions-python").unwrap().escribe);
        // Y lo que no ejecuta es lo que sólo edita documentos.
        assert!(!de("semantics").unwrap().ejecuta);
        assert!(!de("functions-typescript").unwrap().escribe);
        assert!(CLASES.iter().filter(|c| c.ejecuta).count() == 7);
    }

    /// ⭐ Una plantilla trae su entorno y CÓDIGO, no un comentario (⑧a).
    #[test]
    fn la_plantilla_es_un_arbol_de_ficheros() {
        let t = de("transforms-python").unwrap();
        let rutas: Vec<&str> = t.semilla.iter().map(|(r, _)| *r).collect();
        assert!(rutas.contains(&"pyproject.toml"), "{rutas:?}");
        assert!(rutas.contains(&"transforms/ejemplo.py"), "{rutas:?}");
        let (_, py) = t
            .semilla
            .iter()
            .find(|(r, _)| *r == "transforms/ejemplo.py")
            .unwrap();
        let codigo = py
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
            .count();
        assert!(
            codigo >= 5,
            "el ejemplo tiene que CORRER, no describir: {codigo}"
        );
        assert!(py.contains("@transform("), "{py}");
    }

    /// La clave de antes de ⑧a resuelve, y no se lista como una clase más.
    #[test]
    fn la_clave_vieja_sigue_resolviendo() {
        assert_eq!(de("transforms").unwrap().id, "transforms-python");
        assert_eq!(de("models").unwrap().id, "models-python");
        assert!(!nombres().split(", ").any(|n| n == "transforms"));
        assert!(nombres().split(", ").any(|n| n == "transforms-python"));
    }

    /// ⭐ LO QUE LA SEMILLA USA, LA SEMILLA LO IMPORTA (0037 ③a).
    ///
    /// La sesión pone `over`, `write`, `transform`, `sql` y `declare` en el
    /// espacio de la celda —son literalmente `ore.over`, `ore.write`… (lo hace
    /// `Kernel.__init__` del agente)—, así que durante mucho tiempo la semilla
    /// no importó nada y corría igual.
    ///
    /// ⛔ Pero un fichero que usa nombres que no declara **no lo entiende
    ///   ningún editor**. Medido con pyright sobre esta misma semilla: tres
    ///   errores —«"transform" is not defined», «"over" is not defined»,
    ///   «"write" is not defined»— sobre nuestra propia plantilla, que es el
    ///   primer regalo que recibiría quien la abre. Con el import: cero.
    ///
    /// Y el atajo de enseñarle al editor lo que la sesión inyecta NO existe:
    /// un `builtins.pyi` en el `stubPath` de pyright no añade a typeshed, lo
    /// reemplaza (medido: acto seguido `print` deja de existir).
    /// S3 · Lo que una semilla llama del SDK, por sus nombres de hoy: ninguno de
    /// antes (S5 los retira) en el código de ninguna plantilla, en ningún lenguaje.
    #[test]
    fn la_semilla_habla_el_sdk_en_ingles() {
        for c in CLASES {
            for (ruta, texto) in c.semilla {
                let antes = crate::sdk::nombres_de_antes_en(texto);
                assert!(antes.is_empty(), "{} · {ruta}: {antes:?}", c.id);
            }
        }
    }

    #[test]
    fn la_semilla_importa_lo_que_usa() {
        // Lo que la sesión pone, y que por tanto podría no importarse.
        const PUESTOS: [&str; 5] = ["over", "write", "transform", "sql", "declare"];
        // ⭐ Y en Java lo mismo, con su forma: un import estático de `ore.Ore`
        //   trae los cinco de golpe. Sin él, jdtls y `javac` no los resuelven.
        for c in CLASES {
            if c.lenguaje == "java" {
                for (ruta, texto) in c.semilla {
                    if !ruta.ends_with(".java") {
                        continue;
                    }
                    let usa = PUESTOS.iter().any(|n| texto.contains(&format!("{n}(")));
                    if usa {
                        assert!(
                            texto.contains("import static ore.Ore.*;"),
                            "{} usa el SDK y no lo importa: un editor lo subrayaría en rojo",
                            c.id
                        );
                    }
                    assert!(
                        texto.contains("public class ") && texto.contains("static void main("),
                        "{} no es una unidad de compilación: jdtls y javac no entienden un snippet",
                        c.id
                    );
                    assert!(
                        !texto.contains("package "),
                        "{} declara `package`: JShell no lo acepta y el árbol no tiene carpetas de paquete",
                        c.id
                    );
                }
            }
            if c.lenguaje != "python" {
                continue;
            }
            for (ruta, texto) in c.semilla {
                if !ruta.ends_with(".py") {
                    continue;
                }
                let importado: Vec<&str> = texto
                    .lines()
                    .filter(|l| l.starts_with("from ore import "))
                    .flat_map(|l| l.trim_start_matches("from ore import ").split(", "))
                    .map(str::trim)
                    .collect();
                for nombre in PUESTOS {
                    // Usado como llamada o como decorador, no en la prosa.
                    let usa = texto.contains(&format!("{nombre}("))
                        || texto.contains(&format!("@{nombre}("));
                    if usa {
                        assert!(
                            importado.contains(&nombre),
                            "{} usa `{nombre}` y no lo importa: un editor lo subrayaría en rojo",
                            c.id
                        );
                    }
                }
            }
        }
    }

    /// ⭐ Ninguna plantilla es ya un cartel: todas traen código (⑧b), y las de
    ///   Python traen además el fichero donde se declara su entorno.
    #[test]
    fn ninguna_plantilla_es_un_cartel() {
        for c in CLASES {
            if c.semilla.is_empty() {
                assert_eq!(c.id, "semantics", "sólo `semantics` nace sin ficheros");
                continue;
            }
            let codigo: usize = c
                .semilla
                .iter()
                .filter(|(r, _)| !r.ends_with(".toml") && !r.ends_with(".xml"))
                .flat_map(|(_, t)| t.lines())
                .filter(|l| {
                    let l = l.trim();
                    !l.is_empty()
                        && !l.starts_with('#')
                        && !l.starts_with("//")
                        && !l.starts_with("--")
                })
                .count();
            assert!(codigo >= 4, "{} tiene {codigo} líneas de código", c.id);
            // ⭐ 0037 ③c: y en Java lo mismo, con su fichero. Un repositorio
            //   sin dónde declarar se come la capa de la celda, o —como pasaba
            //   en la JVM hasta ③c— no puede usar ni una biblioteca.
            // `sql` no: una consulta no usa dependencias de Python (0038 P7).
            if let Some(donde) = match c.lenguaje {
                "python" => Some("pyproject.toml"),
                "java" => Some("pom.xml"),
                _ => None,
            } {
                assert!(
                    c.semilla.iter().any(|(r, _)| *r == donde),
                    "{} no trae `{donde}`: dónde declarar su entorno",
                    c.id
                );
            }
        }
    }

    /// ⭐ (0038 P7) Las semillas nombran en TRES partes, `base.schema.nombre`:
    ///   ni el `<paquete>.<dataset>` de antes —que no es un nombre y no corría—
    ///   ni dos partes, que son `default` y el SQL avisa (`ORE-SQL-2P`).
    #[test]
    fn las_semillas_nombran_en_tres_partes() {
        for c in CLASES {
            for (ruta, texto) in c.semilla {
                // Un documento generado no lleva comentarios: lo que enseña a
                // nombrar está en el código del que sale (0050 G1).
                if ruta.ends_with(".toml")
                    || ruta.ends_with(".xml")
                    || ore_code::emitir::es_generado(texto)
                {
                    continue;
                }
                assert!(!texto.contains("<paquete>"), "{} · {ruta}", c.id);
                assert!(
                    texto.contains("mi_base.mi_schema."),
                    "{} · {ruta} no nombra en tres partes",
                    c.id
                );
            }
        }
        // y la de SQL es un `.sql`, no una cadena dentro de Python
        let sql = de("transforms-sql").unwrap();
        assert!(
            sql.semilla
                .iter()
                .any(|(r, _)| *r == "transforms/ejemplo.sql")
        );
        assert!(!sql.semilla.iter().any(|(r, _)| r.ends_with(".py")));
        assert!(!sql.semilla.iter().any(|(r, _)| *r == "pyproject.toml"));
    }

    /// Cada clase pertenece a una familia que existe, y cada familia tiene
    /// al menos una clase: una tarjeta que agrupa nada no se puede pintar.
    #[test]
    fn las_familias_y_las_clases_se_corresponden() {
        for c in CLASES {
            assert!(familia(c.familia).is_some(), "{} sin familia", c.id);
        }
        for f in FAMILIAS {
            assert!(
                CLASES.iter().any(|c| c.familia == f.id),
                "la familia `{}` no tiene ninguna clase",
                f.id
            );
        }
    }

    #[test]
    fn el_entorno_sale_del_lenguaje() {
        assert_eq!(entorno_de(de("transforms-python").unwrap()), Some("python"));
        assert_eq!(entorno_de(de("transforms-java").unwrap()), Some("jvm"));
        // `sql` corre donde corre python: no hay imagen de SQL.
        assert_eq!(entorno_de(de("transforms-sql").unwrap()), Some("python"));
        assert_eq!(entorno_de(de("semantics").unwrap()), None);
        assert_eq!(
            entorno_de(de("functions-typescript").unwrap()),
            Some("node")
        );
    }

    #[test]
    fn las_clases_estan_y_ninguna_siembra_fuera_de_su_carpeta() {
        // 8 desde 0050: `functions-typescript`.
        assert_eq!(CLASES.len(), 8);
        for c in CLASES {
            assert!(de(c.id).is_some());
            for (ruta, _) in c.semilla {
                assert!(!ruta.starts_with('/') && !ruta.contains(".."), "{ruta}");
                assert!(
                    !ruta.eq_ignore_ascii_case("README.md"),
                    "la semilla no pisa el manifiesto"
                );
            }
        }
    }
}
