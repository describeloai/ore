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
# Cambia las dos referencias por las tuyas y dale a Run.

from ore import transform, over, write

ENTRADA = \"<paquete>.<dataset>\"
SALIDA = \"<paquete>.<resumen>\"


@transform(inputs=[ENTRADA], output=SALIDA)
def resumir():
    tabla = over(ENTRADA, como=\"arrow\")
    return write(SALIDA, tabla)


escrito = resumir()
print(\"filas\", escrito[\"filas\"])
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
// Cambia las dos referencias por las tuyas y dale a Run.
import static ore.Ore.*;

import java.util.List;
import java.util.Map;

public class Ejemplo {
    static final String ENTRADA = \"<paquete>.<dataset>\";
    static final String SALIDA = \"<paquete>.<resumen>\";

    public static void main(String[] args) throws Exception {
        Map<String, Object> escrito = transform(\"resumir\", List.of(ENTRADA), SALIDA,
                () -> write(SALIDA, over(ENTRADA)));
        System.out.println(\"filas \" + escrito.get(\"filas\"));
    }
}
";

/// El transform escrito en SQL. La consulta manda; lo que sale, se escribe.
///
/// ⛔ La consulta vive en una cadena y no en un `.sql` aparte, y no por gusto:
///   un trabajo del árbol se ejecuta como **una celda** —no hay fichero desde
///   el que leer al lado— y una celda SQL a secas **lee pero no escribe**. Un
///   `.sql` suelto que dijera ser un transform sería un cartel.
const TRANSFORMS_SQL: &str = "\
# Un transform escrito en SQL: la consulta manda, y lo que sale se escribe.
# Sigue DECLARANDO qué lee y qué escribe, y el servidor lo hace cumplir
# (ADR 0031 · W3.7).
#
# `transform`, `sql` y `write` son del SDK del puesto y ADEMAS los pone la
# sesión en el espacio de la celda: el import no cambia lo que corre — hace que
# el editor sepa de qué hablas (ADR 0037 ③a).
# Cambia las referencias por las tuyas y dale a Run.

from ore import transform, sql, write

ENTRADA = \"mi_paquete.mi_dataset\"
SALIDA = \"mi_paquete.mi_resumen\"

CONSULTA = f\"\"\"
select pais, count(*) as n
from {ENTRADA}
group by pais
\"\"\"


@transform(inputs=[ENTRADA], output=SALIDA)
def resumir():
    return write(SALIDA, sql(CONSULTA, como=\"arrow\"))


escrito = resumir()
print(\"filas\", escrito[\"filas\"])
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

FUENTE = \"<paquete>.<dataset>\"

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

ENTRADA = \"<paquete>.<dataset>\"
MODELO = \"<paquete>.<modelo>\"

filas = over(ENTRADA)
# ... entrenar con lo que declares en el pyproject.toml de este repositorio ...
digest = \"sha256:0000000000000000000000000000000000000000000000000000000000000000\"

print(declare({
    \"kind\": \"TrainedModel\",
    \"metadata\": {\"name\": MODELO.split(\".\")[-1], \"namespace\": MODELO.split(\".\")[0]},
    \"spec\": {\"owner\": \"team:cambiame\", \"trainedFrom\": [ENTRADA], \"digest\": digest},
}))
";

const FUNCTIONS_PY: &str = "\
# Una función de lectura: entra una fila, sale un valor, y sin efectos. Se
# declara como `Function` en `functions/` y se invoca con `ore invoke` (ADR
# 0029); su clase no escribe datos (0036 ⑤), y eso no es un aviso: es el techo.
#
# `over` es del SDK del puesto y ADEMAS lo pone la sesión en el espacio de la
# celda: el import no cambia lo que corre — hace que el editor sepa de qué
# hablas (ADR 0037 ③a).

from ore import over


FUENTE = \"<paquete>.<dataset>\"


def clasificar(fila):
    return {\"etiqueta\": \"alta\" if fila[\"total\"] > 100 else \"baja\"}


for fila in over(FUENTE)[:5]:
    print(fila, \"→\", clasificar(fila))
";

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
        version: 3,
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
        version: 3,
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
        version: 2,
        semilla: &[
            ("pyproject.toml", PYPROJECT_PY),
            ("transforms/ejemplo.py", TRANSFORMS_SQL),
        ],
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
        version: 3,
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
        version: 3,
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
        descripcion: "Write reusable code for pipelines, transforms and applications.",
        version: 3,
        semilla: &[
            ("pyproject.toml", PYPROJECT_PY),
            ("funciones/ejemplo.py", FUNCTIONS_PY),
        ],
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
        assert!(CLASES.iter().filter(|c| c.ejecuta).count() == 6);
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
                    !l.is_empty() && !l.starts_with('#') && !l.starts_with("//")
                })
                .count();
            assert!(codigo >= 4, "{} tiene {codigo} líneas de código", c.id);
            // ⭐ 0037 ③c: y en Java lo mismo, con su fichero. Un repositorio
            //   sin dónde declarar se come la capa de la celda, o —como pasaba
            //   en la JVM hasta ③c— no puede usar ni una biblioteca.
            if let Some(donde) = match c.lenguaje {
                "python" | "sql" => Some("pyproject.toml"),
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
    }

    #[test]
    fn las_clases_estan_y_ninguna_siembra_fuera_de_su_carpeta() {
        assert_eq!(CLASES.len(), 7);
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
