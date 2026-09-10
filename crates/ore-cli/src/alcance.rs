//! **Qué parte de una fuente entra en un paquete.** El alcance, escrito.
//!
//! # Por qué no es una decisión de la cola
//!
//! La tentación era añadir una clase más a `Clase` —`entra/<tabla>`, contestada
//! `si` u `omitir`— y encaja mal por dos motivos que se ven a la primera:
//!
//! 1. **La cola vale porque cada línea es una duda real.** Un origen de cien
//!    tablas daría cien preguntas y noventa y cinco serían «sí, obviamente».
//!    Eso no es una cola de decisiones: es un formulario, y un formulario no se
//!    contesta.
//! 2. **El alcance no es algo que la inducción no supiera decidir.** Es una
//!    entrada, previa a inducir, igual que `--name` o `--out`. Meterlo en la
//!    cola sería decir que el inductor lo intentó y no pudo.
//!
//! # ⛔⛔ Y por qué SÍ se escribe, en vez de solo aplicarse
//!
//! Porque `drift-detect` denuncia toda tabla del origen que el paquete no
//! declara —`deriva.rs`, el bucle del final— y tiene razón en hacerlo. Sin esto,
//! marcar cinco de cien produce **noventa y cinco derivas en cada pasada, para
//! siempre**.
//!
//! El problema de fondo no es el ruido: es que **«no lo elegí» y «no lo vi» son
//! la misma ausencia** y llevan a sitios opuestos. Una hay que dejarla en paz y
//! la otra hay que mirarla. Un alcance escrito es lo único que las separa.
//!
//! ⇒ Es la misma figura que ya separa `pendiente` de `desconocido` en el estado
//!   de una fuente, y «todavía no se catalogó» de «se catalogó y no había nada»
//!   en la ficha del catálogo.
//!
//! # La gramática de los tres ficheros
//!
//! ```text
//! discover.catalog.json    lo que el mundo dijo      entero, sin recortar
//! discover.scope.json      lo que nos llevamos       esto
//! discover.answers.json    lo que decidimos          la cola contestada
//! ```
//!
//! El catálogo se queda **entero** a propósito. Recortarlo ahí haría que el
//! fichero mintiera sobre lo que el origen tenía, y es justo el documento del
//! que sale la comparación cuando alguien pregunta qué se movió.

use ore_core::json::Json;
use ore_core::parse;
use ore_driver::catalogo::Catalogo;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Al lado del paquete, como sus dos hermanos.
pub const FICHERO: &str = "discover.scope.json";

pub struct Alcance {
    /// De qué fuente es. No es adorno: un alcance pegado del paquete de al lado
    /// recortaría este a cero tablas y el síntoma sería «no se indujo nada».
    fuente: String,
    objetos: BTreeSet<String>,
}

/// Lo que quedó fuera al aplicar un alcance, para poder decirlo.
pub struct Recorte {
    /// Tablas del catálogo que el alcance no nombra. Se cuentan, no se listan:
    /// noventa y cinco nombres no son un mensaje.
    pub fuera: usize,
    /// Nombres del alcance que el catálogo no tiene. **Estos sí se listan**: o
    /// es una errata, o una tabla que desapareció del origen, y las dos piden
    /// que alguien mire.
    pub sin_respaldo: Vec<String>,
}

impl Alcance {
    pub fn nuevo(fuente: &str, objetos: impl IntoIterator<Item = String>) -> Self {
        Alcance {
            fuente: fuente.to_string(),
            objetos: objetos.into_iter().collect(),
        }
    }

    /// Se analiza con el analizador de YAML porque **JSON es un subconjunto de
    /// YAML** y `ore-core` no lleva uno de JSON (ADR 0002). Es lo mismo que hace
    /// `Catalogo::leer`, y por el mismo motivo.
    pub fn leer(texto: &str) -> Result<Self, String> {
        let raiz = parse::parse(texto).map_err(|e| format!("el alcance no analiza: {e:?}"))?;
        let fuente = raiz
            .get("source")
            .and_then(|(_, v)| v.as_str())
            .ok_or("el alcance no dice de qué `source` es")?
            .to_string();
        let objetos: BTreeSet<String> = raiz
            .get("only")
            .map(|(_, v)| v.items())
            .unwrap_or(&[])
            .iter()
            .filter_map(|n| n.as_str())
            .map(String::from)
            .collect();
        if objetos.is_empty() {
            // ⛔ Un alcance vacío NO es «todo»: es «nada», y escribir un fichero
            //   para decir nada no lo hace nadie a propósito. Se niega en vez de
            //   inducir un paquete sin tablas y dejar que quien mire lo achaque
            //   al origen.
            return Err("el alcance no nombra ningún objeto: `only` está vacío".into());
        }
        Ok(Alcance { fuente, objetos })
    }

    pub fn escribir(&self) -> String {
        Json::obj([
            ("source", Json::s(&self.fuente)),
            (
                "only",
                Json::Arr(self.objetos.iter().map(Json::s).collect()),
            ),
        ])
        .pretty()
    }

    /// **Recorta el catálogo.** Devuelve el catálogo recortado y lo que sobró
    /// por cada lado, para que quien llame lo diga.
    ///
    /// ⚠️ El recorte se hace sobre el catálogo y NO sobre lo inducido, que es la
    /// misma regla que `inducir_con` sigue con las decisiones: lo que sale del
    /// inductor es siempre una inducción de algo, nunca una inducción retocada.
    pub fn aplicar(&self, cat: Catalogo) -> (Catalogo, Recorte) {
        let habia: BTreeSet<String> = cat.tablas.iter().map(|t| t.nombre.clone()).collect();
        let sin_respaldo: Vec<String> = self
            .objetos
            .iter()
            .filter(|o| !habia.contains(o.as_str()))
            .cloned()
            .collect();
        let Catalogo { fuente, tablas } = cat;
        let antes = tablas.len();
        let tablas: Vec<_> = tablas
            .into_iter()
            .filter(|t| self.objetos.contains(&t.nombre))
            .collect();
        let recorte = Recorte {
            fuera: antes - tablas.len(),
            sin_respaldo,
        };
        (Catalogo { fuente, tablas }, recorte)
    }

    /// ¿Entra este objeto?
    ///
    /// ⭐ Lo pregunta `drift-detect`, y es la única forma que tiene de distinguir
    /// **«no lo elegí» de «no lo vi»**. No sirve mirar `discover.catalog.json`
    /// en su lugar: un objeto retenido por una colisión sin resolver también
    /// está ahí, y ese NO está decidido — está pendiente, y tiene que seguir
    /// saliendo como deriva.
    pub fn nombra(&self, objeto: &str) -> bool {
        self.objetos.contains(objeto)
    }

    /// ⛔ Que el alcance sea de OTRA fuente no se arregla ignorándolo: recortaría
    /// a cero y el síntoma sería «el origen no tiene nada».
    pub fn comprueba_la_fuente(&self, cat: &Catalogo) -> Result<(), String> {
        if self.fuente != cat.fuente() {
            return Err(format!(
                "el alcance es de `{}` y el catálogo de `{}`",
                self.fuente,
                cat.fuente()
            ));
        }
        Ok(())
    }
}

pub fn ruta(raiz: &Path) -> PathBuf {
    raiz.join(FICHERO)
}

/// El alcance que un paquete ya tiene, si lo tiene.
///
/// ⭐ `None` y un alcance vacío son cosas distintas y por eso no se doblan: un
/// paquete sin fichero es uno que se llevó **la fuente entera**, y ese es el
/// caso normal. `leer` ya se niega a devolver uno vacío.
pub fn del_paquete(raiz: &Path) -> Result<Option<Alcance>, String> {
    let r = ruta(raiz);
    match std::fs::read_to_string(&r) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("no se pudo leer `{}`: {e}", r.display())),
        Ok(t) => Alcance::leer(&t)
            .map(Some)
            .map_err(|m| format!("`{}`: {m}", r.display())),
    }
}

/// **Lo que se eligió, y sobre qué se eligió.** Las dos mitades, juntas porque
/// por separado no contestan nada.
///
/// ⭐⭐ El alcance solo dice qué entra. Para saber si algo que NO entra fue una
/// decisión o un despiste hace falta la otra mitad: **qué había delante cuando
/// se eligió**, que es `discover.catalog.json`. Con las dos:
///
/// | en el alcance | estaba en el catálogo | qué es |
/// |---|---|---|
/// | sí | — | entra: se compara como siempre |
/// | no | sí | **lo miraron y dijeron que no.** No es deriva |
/// | no | no | **apareció después.** Nadie lo ha visto: sí es deriva |
///
/// La tercera fila es la que justifica que `discover.catalog.json` se guarde
/// ENTERO. Sin ella, elegir cinco de cien dejaría ciega la fuente para siempre
/// — que es cambiar noventa y cinco falsas alarmas por una señal perdida, y la
/// señal perdida es la que importa.
pub struct Eleccion {
    alcance: Alcance,
    /// Lo que el origen tenía cuando se eligió.
    delante: BTreeSet<String>,
}

impl Eleccion {
    /// `None` si el paquete no tiene alcance: se llevó la fuente entera, que es
    /// el caso normal y el de siempre.
    pub fn del_paquete(raiz: &Path) -> Result<Option<Self>, String> {
        let Some(alcance) = del_paquete(raiz)? else {
            return Ok(None);
        };
        // ⚠️ Si el catálogo guardado no se puede leer, `delante` queda vacío y
        //   entonces TODO lo que no entra cuenta como aparecido después. Es el
        //   lado seguro: dice de más, no de menos.
        let delante = std::fs::read_to_string(raiz.join("discover.catalog.json"))
            .ok()
            .and_then(|t| Catalogo::leer(&t).ok())
            .map(|c| c.tablas.into_iter().map(|t| t.nombre).collect())
            .unwrap_or_default();
        Ok(Some(Eleccion { alcance, delante }))
    }

    /// Qué es un objeto del origen que el paquete no declara.
    pub fn juzgar(&self, objeto: &str) -> Juicio {
        if self.alcance.nombra(objeto) {
            // Está en el alcance y aun así no se declaró: lo retiene algo de la
            // cola —una colisión sin resolver, un `omitir`—, y eso está
            // pendiente, no decidido.
            return Juicio::Retenido;
        }
        if self.delante.contains(objeto) {
            Juicio::Descartado
        } else {
            Juicio::Nuevo
        }
    }
}

pub enum Juicio {
    /// Lo miraron y dijeron que no. No es deriva.
    Descartado,
    /// No estaba cuando se eligió: nadie lo ha visto. Sí es deriva.
    Nuevo,
    /// Se pidió y no salió. Sí es deriva, y es la de siempre.
    Retenido,
}

/// Los objetos de un fichero de lista: uno por línea.
///
/// Existe porque **un origen de cien tablas no cabe en una línea de órdenes**, y
/// menos en la de un `Job` que la lleva escrita en un manifiesto. Se ignoran las
/// líneas vacías y las que empiezan por `#`, para que una lista se pueda anotar.
pub fn de_fichero(p: &Path) -> Result<Vec<String>, String> {
    let t = std::fs::read_to_string(p)
        .map_err(|e| format!("no se pudo leer `{}`: {e}", p.display()))?;
    Ok(t.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect())
}

#[cfg(test)]
mod prueba {
    use super::*;
    use ore_driver::catalogo::Tabla;

    fn tabla(nombre: &str) -> Tabla {
        Tabla {
            nombre: nombre.into(),
            columnas: Vec::new(),
            clave: Vec::new(),
            unicas: Vec::new(),
            foraneas: Vec::new(),
            filas: None,
            clase: "table".into(),
            lee: None,
            cambia: None,
        }
    }

    fn cat() -> Catalogo {
        Catalogo {
            fuente: "ventas".into(),
            tablas: vec![
                tabla("public.clientes"),
                tabla("public.pedidos"),
                tabla("legacy.viejo"),
            ],
        }
    }

    #[test]
    fn recorta_y_cuenta_lo_que_deja_fuera() {
        let a = Alcance::nuevo("ventas", ["public.clientes".into()]);
        let (c, r) = a.aplicar(cat());
        assert_eq!(c.tablas.len(), 1);
        assert_eq!(c.tablas[0].nombre, "public.clientes");
        assert_eq!(r.fuera, 2);
        assert!(r.sin_respaldo.is_empty());
    }

    /// ⛔ Un nombre que el catálogo no tiene NO se traga: o es una errata, o la
    /// tabla se fue del origen.
    #[test]
    fn un_nombre_sin_respaldo_se_dice() {
        let a = Alcance::nuevo(
            "ventas",
            ["public.clientes".into(), "public.qlientes".into()],
        );
        let (c, r) = a.aplicar(cat());
        assert_eq!(c.tablas.len(), 1);
        assert_eq!(r.sin_respaldo, vec!["public.qlientes".to_string()]);
    }

    #[test]
    fn se_escribe_y_se_vuelve_a_leer_igual() {
        let a = Alcance::nuevo("ventas", ["b".into(), "a".into()]);
        let t = a.escribir();
        let b = Alcance::leer(&t).unwrap();
        assert_eq!(b.fuente, "ventas");
        // Ordenado: sale de un `BTreeSet`, y un alcance que cambia de orden
        // produce un fichero distinto para la misma selección.
        assert_eq!(b.escribir(), t);
        assert!(t.contains("\"a\""), "{t}");
    }

    #[test]
    fn un_alcance_vacio_se_niega() {
        assert!(Alcance::leer("{\"source\":\"ventas\",\"only\":[]}").is_err());
    }

    #[test]
    fn un_alcance_de_otra_fuente_se_niega() {
        let a = Alcance::nuevo("compras", ["public.clientes".into()]);
        assert!(a.comprueba_la_fuente(&cat()).is_err());
    }
}
