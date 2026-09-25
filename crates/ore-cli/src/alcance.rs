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
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Al lado del paquete, como sus dos hermanos.
pub const FICHERO: &str = "discover.scope.json";

pub struct Alcance {
    /// De qué fuente es. No es adorno: un alcance pegado del paquete de al lado
    /// recortaría este a cero tablas y el síntoma sería «no se indujo nada».
    fuente: String,
    objetos: BTreeSet<String>,
    /// **La clase de la base** (ORE 0027 P1 I4): `standard` copia a la celda
    /// todo lo que entra; ausente = `foreign`, un espejo, lo que toda base era.
    /// Va aquí y no en las vistas porque es una REGLA sobre lo que entre
    /// después, y porque `review` re-induce las vistas y esto no se re-induce:
    /// se aplica.
    tipo: Option<String>,
    /// **Qué tablas están modeladas** (ORE 0027 P1 C1): las que tienen `Entity`
    /// y su cola de modelado. `None` = todas — lo que era antes de que el
    /// catálogo y la ontología fueran dos cosas, y por eso los paquetes de
    /// entonces no cambian—; `Some([])` = ninguna, lo que un alta del
    /// catálogo escribe. `ore model` añade una.
    entidades: Option<BTreeSet<String>>,
    /// **Qué tablas se copian una a una** en una base foránea (`copies`): la
    /// excepción a la regla de la clase. En una estándar no hace falta —se
    /// copia todo— y no se escribe.
    copias: BTreeSet<String>,
    /// **Los schemas renombrados** (0038 P6): schema del origen → el nombre
    /// que tiene en el paquete. `discover` deja lo del origen en la carpeta
    /// del schema del origen; alguien lo renombró (`ore package schema
    /// rename`), y la siguiente inducción —`review`, `model`, `copy`— lo
    /// tiene que emitir con el nombre nuevo o lo desharía: dos carpetas, las
    /// mismas tablas. Es una regla que alguien declaró, como las de arriba.
    schemas: BTreeMap<String, String>,
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
            tipo: None,
            entidades: None,
            copias: BTreeSet::new(),
            schemas: BTreeMap::new(),
        }
    }

    /// Los schemas renombrados: el del origen → el del paquete.
    pub fn schemas(&self) -> &BTreeMap<String, String> {
        &self.schemas
    }

    /// **Renombrar un schema** en la regla: lo que se llamaba `viejo` en el
    /// paquete se llama `nuevo`. Si `viejo` ya era un renombrado, se sigue la
    /// cadena hasta el origen; si `nuevo` es el del origen, la entrada sobra.
    /// Un schema que no sale del origen (creado a mano) también entra: no
    /// casa con ninguna tabla y no cambia nada, y así no hay que saberlo.
    pub fn renombrar_schema(&mut self, viejo: &str, nuevo: &str) {
        let origen = self
            .schemas
            .iter()
            .find(|(_, v)| v.as_str() == viejo)
            .map(|(k, _)| k.clone())
            .unwrap_or_else(|| viejo.to_string());
        if origen == nuevo {
            self.schemas.remove(&origen);
        } else {
            self.schemas.insert(origen, nuevo.to_string());
        }
    }

    /// ¿Esta tabla se copia? Por la clase de la base, o una a una. (El
    /// inductor lo decide con su `Regla`; esto es para leerlo.)
    #[cfg(test)]
    pub fn copia(&self, objeto: &str) -> bool {
        self.estandar() || self.copias.contains(objeto)
    }

    pub fn copiadas(&self) -> &BTreeSet<String> {
        &self.copias
    }

    /// **Copiar** una tabla de una base foránea. `Err` si no está en el alcance,
    /// si la base es estándar (ya se copia todo) o si ya estaba.
    pub fn copiar(&mut self, objeto: &str) -> Result<(), String> {
        if !self.objetos.contains(objeto) {
            return Err(format!(
                "`{objeto}` no está en el alcance de esta base ({} objetos)",
                self.objetos.len()
            ));
        }
        if self.estandar() {
            return Err("la base es estándar: ya se copia todo lo que entra".into());
        }
        if !self.copias.insert(objeto.to_string()) {
            return Err(format!("`{objeto}` ya se copia"));
        }
        Ok(())
    }

    /// Con las tablas modeladas dichas: `Some(vec![])` es «ninguna».
    pub fn con_entidades(mut self, e: Option<Vec<String>>) -> Self {
        self.entidades = e.map(|v| v.into_iter().collect());
        self
    }

    /// ¿Esta tabla se modela (lleva `Entity` y sus decisiones)?
    pub fn modela(&self, objeto: &str) -> bool {
        self.entidades.as_ref().is_none_or(|e| e.contains(objeto))
    }

    /// Las modeladas, para el inductor: `None` = todas.
    pub fn modeladas(&self) -> Option<&BTreeSet<String>> {
        self.entidades.as_ref()
    }

    /// **Modelar** una tabla del alcance: la añade a las modeladas. `Err` si no
    /// está en el alcance, o si ya lo estaba (para decirlo, no para fallar).
    pub fn modelar(&mut self, objeto: &str) -> Result<(), String> {
        if !self.objetos.contains(objeto) {
            return Err(format!(
                "`{objeto}` no está en el alcance de esta base ({} objetos)",
                self.objetos.len()
            ));
        }
        if self.modela(objeto) {
            return Err(format!("`{objeto}` ya está modelada"));
        }
        self.entidades
            .get_or_insert_with(BTreeSet::new)
            .insert(objeto.to_string());
        Ok(())
    }

    /// Con la clase declarada. Sólo `standard` se escribe: `foreign` es lo que
    /// significa no decir nada, y escribirlo inventaría una migración.
    pub fn con_tipo(mut self, tipo: &str) -> Self {
        self.tipo = (tipo == "standard").then(|| tipo.to_string());
        self
    }

    /// ¿Copia a la celda todo lo que entra?
    pub fn estandar(&self) -> bool {
        self.tipo.as_deref() == Some("standard")
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
        let tipo = raiz
            .get("type")
            .and_then(|(_, v)| v.as_str())
            .map(String::from);
        if let Some(t) = &tipo
            && t != "standard"
            && t != "foreign"
        {
            return Err(format!("`type` es `standard` o `foreign`, no `{t}`"));
        }
        let entidades = raiz.get("entities").map(|(_, v)| {
            v.items()
                .iter()
                .filter_map(|n| n.as_str())
                .map(String::from)
                .collect::<BTreeSet<String>>()
        });
        let copias = raiz
            .get("copies")
            .map(|(_, v)| {
                v.items()
                    .iter()
                    .filter_map(|n| n.as_str())
                    .map(String::from)
                    .collect::<BTreeSet<String>>()
            })
            .unwrap_or_default();
        let schemas = raiz
            .get("schemas")
            .map(|(_, v)| {
                v.entries()
                    .iter()
                    .filter_map(|(k, v)| Some((k.as_str()?.to_string(), v.as_str()?.to_string())))
                    .collect::<BTreeMap<String, String>>()
            })
            .unwrap_or_default();
        Ok(Alcance {
            fuente,
            objetos,
            tipo: tipo.filter(|t| t == "standard"),
            entidades,
            copias,
            schemas,
        })
    }

    pub fn escribir(&self) -> String {
        let mut campos = vec![
            ("source", Json::s(&self.fuente)),
            (
                "only",
                Json::Arr(self.objetos.iter().map(Json::s).collect()),
            ),
        ];
        if let Some(t) = &self.tipo {
            campos.push(("type", Json::s(t)));
        }
        if let Some(e) = &self.entidades {
            campos.push(("entities", Json::Arr(e.iter().map(Json::s).collect())));
        }
        if !self.copias.is_empty() {
            campos.push((
                "copies",
                Json::Arr(self.copias.iter().map(Json::s).collect()),
            ));
        }
        let mut j = Json::obj(campos);
        if !self.schemas.is_empty()
            && let Json::Obj(m) = &mut j
        {
            m.insert(
                "schemas".into(),
                Json::Obj(
                    self.schemas
                        .iter()
                        .map(|(k, v)| (k.clone(), Json::s(v)))
                        .collect(),
                ),
            );
        }
        j.pretty()
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
    fn la_clase_se_escribe_solo_si_es_estandar_y_vuelve_igual() {
        let a = Alcance::nuevo("crm", vec!["public.clientes".to_string()]);
        assert!(!a.estandar());
        assert!(!a.escribir().contains("type"), "foreign no se escribe");
        let e = a.con_tipo("standard");
        assert!(e.estandar());
        let t = e.escribir();
        assert!(t.contains("\"type\": \"standard\""), "{t}");
        assert!(Alcance::leer(&t).unwrap().estandar());
        assert!(
            !Alcance::nuevo("crm", vec!["x".to_string()])
                .con_tipo("foreign")
                .estandar()
        );
        assert!(Alcance::leer(r#"{"source":"crm","only":["x"],"type":"raro"}"#).is_err());
    }

    #[test]
    fn las_modeladas_viajan_y_ninguna_es_distinto_de_todas() {
        let a = Alcance::nuevo("crm", vec!["public.a".to_string(), "public.b".to_string()]);
        assert!(a.modela("public.a"), "sin `entities`, todas");
        assert!(!a.escribir().contains("entities"));
        let mut n = Alcance::nuevo("crm", vec!["public.a".to_string(), "public.b".to_string()])
            .con_entidades(Some(vec![]));
        assert!(!n.modela("public.a"), "con `entities: []`, ninguna");
        assert!(
            n.escribir().contains("\"entities\": []"),
            "{}",
            n.escribir()
        );
        assert!(n.modelar("public.zzz").is_err(), "fuera del alcance");
        n.modelar("public.a").unwrap();
        assert!(n.modelar("public.a").is_err(), "ya modelada");
        let t = n.escribir();
        let v = Alcance::leer(&t).unwrap();
        assert!(v.modela("public.a") && !v.modela("public.b"), "{t}");
        assert!(
            Alcance::nuevo("crm", vec!["x".to_string()])
                .modelar("x")
                .is_err(),
            "con todas, modelar es no-op"
        );
    }

    #[test]
    fn una_foranea_copia_tablas_una_a_una_y_una_estandar_no_lo_necesita() {
        let mut f = Alcance::nuevo("crm", vec!["public.a".to_string(), "public.b".to_string()]);
        assert!(!f.copia("public.a"));
        f.copiar("public.a").unwrap();
        assert!(f.copia("public.a") && !f.copia("public.b"));
        assert!(f.copiar("public.a").is_err(), "ya se copia");
        assert!(f.copiar("public.zzz").is_err(), "fuera del alcance");
        let t = f.escribir();
        assert!(
            t.contains("\"copies\": [\n    \"public.a\"")
                || t.contains("\"copies\": [\"public.a\"]"),
            "{t}"
        );
        assert!(Alcance::leer(&t).unwrap().copia("public.a"));
        let mut e = Alcance::nuevo("crm", vec!["public.a".to_string()]).con_tipo("standard");
        assert!(e.copia("public.a"), "estándar: todo");
        assert!(e.copiar("public.a").is_err(), "estándar: no hace falta");
        assert!(!e.escribir().contains("copies"));
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
