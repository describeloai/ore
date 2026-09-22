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
//! # Lo que una clase NO puede
//!
//! **Conceder.** Una clase ajusta hacia abajo —«este repositorio no escribe
//! datasets»— y nunca hacia arriba: lo que gobierna sigue siendo la etiqueta
//! (el conducto), la declaración (`@transform`) y quién escribió. El techo se
//! aplica donde ya se aplica el gobierno, no aquí.

/// Una clase de repositorio, tal como el producto la trae hoy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clase {
    /// La clave que el manifiesto guarda.
    pub id: &'static str,
    /// Cómo se llama en la consola.
    pub titulo: &'static str,
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

const TRANSFORMS: &str = "\
# Un transform: declara qué lee y qué escribe, y el servidor lo sabe.
#
#   from ore import transform
#
#   @transform(inputs=[\"ventas.pedidos\"], output=\"ventas.resumen\")
#   def resumir(pedidos):
#       return pedidos.group_by(\"pais\").len()
#
# Mientras corre, la sesión sólo resuelve sus `inputs` y sólo escribe su
# `output` (ADR 0031, W3.7 gobierno).
";

const ANALYTICS: &str = "\
# Un análisis: lee, y no declara nada — leer no escribe.
#
#   from ore import datos
#
#   pedidos = datos(\"ventas.pedidos\")
#   print(pedidos.head())
#
# Lo que esta sesión alcanza lo decide el conducto por la etiqueta del dato,
# no esta carpeta.
";

const MODELS: &str = "\
# El entrenamiento de un modelo. Lo que salga se declara como `TrainedModel`
# con su `trainedFrom`, que es lo que hace que el linaje no se corte.
#
#   from ore import datos
#
#   filas = datos(\"ventas.pedidos\")
#   # ... entrenar ...
";

const FUNCTIONS: &str = "\
# Una función de lectura: `over`, `output`, y sin efectos. Se declara como
# `Function` en `functions/` y se invoca con `ore invoke` (ADR 0029).
#
#   def clasificar(fila):
#       return {\"etiqueta\": \"alta\" if fila[\"total\"] > 100 else \"baja\"}
";

/// Las cinco clases de hoy. Añadir una es una fila más, y subir su `version`
/// es lo que hace que un repositorio se pueda actualizar.
pub const CLASES: &[Clase] = &[
    Clase {
        id: "transforms",
        escribe: true,
        ejecuta: true,
        perfil: None,
        titulo: "Transforms",
        version: 1,
        semilla: &[("transforms/ejemplo.py", TRANSFORMS)],
    },
    Clase {
        id: "analytics",
        escribe: false,
        ejecuta: true,
        perfil: None,
        titulo: "Analytics",
        version: 1,
        semilla: &[("analisis/ejemplo.py", ANALYTICS)],
    },
    Clase {
        id: "models",
        escribe: true,
        ejecuta: true,
        perfil: None,
        titulo: "Models",
        version: 1,
        semilla: &[("modelos/entrenar.py", MODELS)],
    },
    Clase {
        id: "functions",
        escribe: false,
        ejecuta: true,
        perfil: None,
        titulo: "Functions",
        version: 1,
        semilla: &[("funciones/ejemplo.py", FUNCTIONS)],
    },
    // `semantics` no siembra código: lo suyo son documentos del árbol, y
    // sembrar una `Entity` a medias sería sembrar algo que no compila.
    Clase {
        id: "semantics",
        escribe: false,
        ejecuta: false,
        perfil: None,
        titulo: "Semantics",
        version: 1,
        semilla: &[],
    },
];

pub fn de(id: &str) -> Option<&'static Clase> {
    CLASES.iter().find(|c| c.id == id)
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
        assert!(de("transforms").unwrap().escribe);
        assert!(de("models").unwrap().escribe);
        assert!(!de("analytics").unwrap().escribe);
        assert!(!de("functions").unwrap().escribe);
        // Y lo que no ejecuta es lo que sólo edita documentos.
        assert!(!de("semantics").unwrap().ejecuta);
        assert!(CLASES.iter().filter(|c| c.ejecuta).count() == 4);
    }

    #[test]
    fn las_cinco_clases_estan_y_ninguna_siembra_fuera_de_su_carpeta() {
        assert_eq!(CLASES.len(), 5);
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
