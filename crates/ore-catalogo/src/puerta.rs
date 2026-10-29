//! **La puerta sobre el catálogo** (0079 §19.2, E1a·5c·2): `Catalogo`, la segunda implementación
//! de [`Punteros`] y [`Ramas`], sobre el [`Almacen`] de un inquilino y abierta sobre **una ref**.
//!
//! Se comporta como [`ore_core::punteros::Ficheros`] sobre los mismos punteros, y lo prueba un
//! guion de conformidad (`tests/la_conformidad_de_las_dos_puertas.rs`) que corre contra las dos:
//!
//! - **Por ruta, como un directorio.** El catálogo guarda cada puntero por su ruta en el árbol (lo
//!   que dejó el andamio): un nombre `b.s.n` es `[datasets/b/s/n.json, datasets/b_n.json]`, la de
//!   su sitio primero y la de antes después (sólo en `default`). Escribir deja la de su sitio y
//!   retira la de antes; [`Punteros::todos`] dice cada ruta con [`crudo::clave_de`] y, si un nombre
//!   tiene las dos, manda la de su sitio (lo de [`crudo::todos_en`]).
//! - **Una transacción por `escribir`**: el *compare-and-set* contra lo efectivo (lo heredado de
//!   `main` cuenta), la historia y el evento, todo o nada (`escribir` de la v2).
//! - **`reclamados` es de todas las refs** (`vivos`), lápidas fuera. `resultados/` no está en el
//!   catálogo: los informes de las funciones siguen en git (§11.5 punto 15). Con
//!   [`Catalogo::con_arbol`], los de `<raiz>/resultados` se suman igual que en `Ficheros`.
//!
//! **Lo que no es igual, a sabiendas** (y por qué no lo ve quien llama):
//!
//! - Un `Sustituir` que no es un objeto: `Ficheros` lo descubre escribiendo y deja escritos los
//!   cambios de antes; aquí se rechaza antes de tocar nada, con el mismo fallo.
//! - `Contexto.idempotencia`: la misma clave dos veces es una escritura (ADR 0031). `Ficheros` no
//!   la mira.
//! - `Poner` sobre un campo nulo o decimal: el catálogo lo conserva (`jsonb`); `Ficheros` lo
//!   degrada a cadena (`Json::de_node`). Lo fija el guion de conformidad.
//! - Una ref que no está no ve nada (como una rama que git no tiene) y no admite escribir (503).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use ore_core::json::Json;
use ore_core::parse::Node;
use ore_core::punteros::protocolo::{Apertura, Puerta, Servidor};
use ore_core::punteros::{
    Cambio, Contexto, De, Esperado, Fallo, Leido, Punteros, Que, Ramas, Reclamados, Version, crudo,
    dataset_de_ubicacion, partes,
};

use crate::almacen::{
    Almacen, Aplicada, CambioRuta, EsperadoMl, FalloAplicar, FalloCerrar, FalloEscritura,
    PlanFusion, Propio, QueRuta,
};

/// **La puerta sobre el catálogo.** Ver el módulo.
pub struct Catalogo {
    almacen: Mutex<Almacen>,
    rama: String,
    /// La raíz del árbol, por sus `resultados/` (lo que sigue en git).
    arbol: Option<PathBuf>,
}

impl Catalogo {
    /// Conecta al esquema del inquilino y abre la puerta sobre `rama` (`None`: `main`).
    pub fn abrir(url: &str, esquema: &str, rama: Option<&str>) -> Result<Catalogo, Fallo> {
        Ok(Catalogo::sobre(
            Almacen::conectar(url, esquema).map_err(Fallo::Almacen)?,
            rama,
        ))
    }

    /// Sobre una conexión ya abierta (la de un pool).
    pub fn sobre(almacen: Almacen, rama: Option<&str>) -> Catalogo {
        Catalogo {
            almacen: Mutex::new(almacen),
            rama: rama.filter(|r| !r.is_empty()).unwrap_or("main").to_string(),
            arbol: None,
        }
    }

    /// Con la raíz del árbol: [`Punteros::reclamados`] suma lo que nombran sus `resultados/`.
    pub fn con_arbol(mut self, raiz: &Path) -> Catalogo {
        self.arbol = Some(raiz.to_path_buf());
        self
    }

    /// La ref sobre la que está abierta.
    pub fn rama(&self) -> &str {
        &self.rama
    }

    /// La conexión, de vuelta (para devolverla a su pool).
    pub fn soltar(self) -> Almacen {
        self.almacen
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn a(&self) -> Result<MutexGuard<'_, Almacen>, Fallo> {
        self.almacen
            .lock()
            .map_err(|_| Fallo::Almacen("la conexión al catálogo quedó rota".into()))
    }

    /// **Sembrar una vez por árbol** (§19.2 P3, para las suites de ·5c·4): si el catálogo no tiene
    /// ya la siembra de `raiz`, carga en `main`, en un commit y sin historia, los punteros que hay
    /// hoy en `<raiz>/datasets` y `<raiz>/copias`. Devuelve si sembró.
    ///
    /// **Una vez, no en cada apertura:** lo que se escribe por la puerta no vuelve al directorio,
    /// y volver a sembrar lo pisaría con lo de antes. La marca es un commit con `git_sha =
    /// sembrado:<raíz canónica>`, el mismo `replicar_main` del andamio (idempotente por el sha) y
    /// bajo un cerrojo: dos que siembran a la vez siembran una.
    ///
    /// Un fichero que no analiza no se siembra (`Ficheros` tampoco lo lee). Lo que no está en el
    /// directorio y sí en el catálogo se queda: sembrar no borra.
    pub fn sembrar_si_hace_falta(&self, raiz: &Path) -> Result<bool, Fallo> {
        let raiz_c = std::fs::canonicalize(raiz).unwrap_or_else(|_| raiz.to_path_buf());
        let marca = format!("sembrado:{}", raiz_c.to_string_lossy().replace('\\', "/"));
        let mut a = self.a()?;
        if a.sembrado(&marca).map_err(Fallo::Almacen)? {
            return Ok(false);
        }
        let (mut rutas, mut textos) = (Vec::new(), Vec::new());
        for carpeta in crudo::CARPETAS_AL_DIA {
            let dir = raiz.join(carpeta);
            for f in crudo::ficheros(&dir) {
                let Some(n) = std::fs::read_to_string(&f)
                    .ok()
                    .and_then(|t| ore_core::parse::parse(&t).ok())
                else {
                    continue;
                };
                let Ok(rel) = f.strip_prefix(raiz) else {
                    continue;
                };
                rutas.push(rel.to_string_lossy().replace('\\', "/"));
                // Por `Json`: lo que analiza `ore_core::parse` y no es JSON estricto, en jsonb.
                textos.push(Json::de_node_fiel(&n).jcs());
            }
        }
        a.sembrar(&marca, &rutas, &textos).map_err(Fallo::Almacen)
    }
}

/// Las rutas de un nombre: la de su sitio y, en `default`, la de antes.
fn rutas_de(nombre: &str) -> Option<Vec<String>> {
    partes(nombre)?;
    let dir = Path::new(crudo::CARPETA);
    Some(
        [crudo::ruta_en(dir, nombre), crudo::legado_en(dir, nombre)]
            .into_iter()
            .flatten()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect(),
    )
}

fn de_de(de: &str) -> De {
    if de == "main" { De::Main } else { De::Aqui }
}

fn leido(de: &str, texto: String) -> Option<Leido> {
    let puntero = ore_core::parse::parse(&texto).ok()?;
    Some(Leido {
        puntero,
        texto,
        de: de_de(de),
    })
}

fn texto_de(n: &Node, k: &str) -> Option<String> {
    n.get(k)
        .and_then(|(_, v)| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// La `metadata_location` de un puntero leído (`None` si es nula o no está).
fn ml_de(texto: &str) -> Option<String> {
    let n = ore_core::parse::parse(texto).ok()?;
    match n.get("metadata_location") {
        Some((_, Node::Scalar { raw, style, .. }))
            if !(raw == "null" && *style == ore_core::parse::Style::Plain) =>
        {
            Some(raw.clone()).filter(|s| !s.is_empty())
        }
        _ => None,
    }
}

/// **Lo que reclama un puntero**, exactamente como en `Ficheros::reclamados`: su `dataset` o, si
/// no lo dice, **los dos** nombres (el de sus bytes y el de siempre, `<clase>/<stem>`); el de su
/// `resultado`, su `clave`, sus ubicaciones y, si es una colección, el par.
fn reclamar(r: &mut Reclamados, clase: &str, stem: Option<&str>, n: &Node) {
    let ml = texto_de(n, "metadata_location");
    let de_siempre = stem.map(|s| format!("{clase}/{s}"));
    let d = texto_de(n, "dataset").or_else(|| {
        if let Some(s) = &de_siempre {
            r.datasets.insert(s.clone());
        }
        ml.as_deref().and_then(dataset_de_ubicacion).or(de_siempre)
    });
    if let Some(d) = d {
        if let Some(ml) = ml {
            if texto_de(n, "kind").as_deref() == Some("MediaCollection") {
                r.colecciones.insert((d.clone(), ml.clone()));
            }
            r.ubicaciones.entry(d.clone()).or_default().insert(ml);
        }
        r.datasets.insert(d);
    }
    if let Some(d) = n.get("resultado").and_then(|(_, x)| texto_de(x, "dataset")) {
        r.datasets.insert(d);
    }
    if let Some(c) = texto_de(n, "clave") {
        r.claves.insert(c);
    }
}

fn en_ruta(c: &Cambio, rutas: Vec<String>) -> Result<CambioRuta, Fallo> {
    let que = match &c.que {
        Que::Retirar => QueRuta::Retirar,
        Que::Sustituir(j @ Json::Obj(_)) => QueRuta::Sustituir(j.clone()),
        Que::Sustituir(_) => return Err(no_es_un_objeto(&c.nombre)),
        Que::Poner { campos, quitar } => QueRuta::Poner {
            // Como `Ficheros`: el último de una clave repetida gana.
            campos: campos.iter().cloned().collect(),
            quitar: quitar.clone(),
        },
    };
    Ok(CambioRuta {
        rutas,
        esperado: match &c.esperado {
            Esperado::Cualquiera => EsperadoMl::Cualquiera,
            Esperado::Nada => EsperadoMl::Nada,
            Esperado::Ml(m) => EsperadoMl::Ml(m.clone()),
        },
        que,
    })
}

fn no_es_un_objeto(nombre: &str) -> Fallo {
    Fallo::Almacen(format!("el puntero de `{nombre}` no es un objeto"))
}

impl Catalogo {
    /// El *compare-and-set* de `cambios`, sólo leyendo: para decir, ante un cambio que no puede
    /// ser, el fallo que diría `Ficheros` (un conflicto de antes gana). No escribe nada.
    fn comprobar(&self, a: &mut Almacen, cambios: &[(&Cambio, Vec<String>)]) -> Result<(), Fallo> {
        for (c, rutas) in cambios {
            let hoy = a.efectivo(&self.rama, rutas).map_err(Fallo::Almacen)?;
            let ml = hoy.as_ref().and_then(|(_, t)| ml_de(t));
            let cumple = match &c.esperado {
                Esperado::Cualquiera => true,
                Esperado::Nada => hoy.is_none(),
                Esperado::Ml(m) => ml.as_deref() == Some(m.as_str()),
            };
            if !cumple {
                return Err(Fallo::Conflicto {
                    nombre: c.nombre.clone(),
                    esperado: c.esperado.clone(),
                    hay: ml,
                });
            }
        }
        Ok(())
    }
}

impl Catalogo {
    /// Los cambios por ruta, con el fallo que diría `Ficheros` ante uno que no puede ser.
    fn en_rutas(&self, a: &mut Almacen, cambios: &[Cambio]) -> Result<Vec<CambioRuta>, Fallo> {
        let mut con_rutas = Vec::with_capacity(cambios.len());
        for c in cambios {
            match rutas_de(&c.nombre) {
                Some(r) => con_rutas.push((c, r)),
                None => {
                    // `Ficheros` comprueba en orden: un conflicto de antes gana al nombre.
                    self.comprobar(a, &con_rutas)?;
                    return Err(Fallo::Nombre(c.nombre.clone()));
                }
            }
        }
        let mut en_rutas = Vec::with_capacity(cambios.len());
        for (c, r) in &con_rutas {
            match en_ruta(c, r.clone()) {
                Ok(x) => en_rutas.push(x),
                Err(f) => {
                    // `Ficheros` lo descubre al escribir, tras comprobarlos todos.
                    self.comprobar(a, &con_rutas)?;
                    return Err(f);
                }
            }
        }
        Ok(en_rutas)
    }
}

/// Un fallo de escribir, en los términos de la puerta.
fn fallo_de(cambios: &[Cambio], f: FalloEscritura) -> Fallo {
    match f {
        FalloEscritura::Conflicto { indice, hay } => {
            let c = cambios.get(indice).unwrap_or(&cambios[0]);
            Fallo::Conflicto {
                nombre: c.nombre.clone(),
                esperado: c.esperado.clone(),
                hay,
            }
        }
        FalloEscritura::SinRama(r) => Fallo::Almacen(format!(
            "la rama `{r}` no está en el catálogo: no se puede escribir en ella"
        )),
        // ore-core no tiene un 429: lo cerrado por una fusión se reintenta como un 503.
        FalloEscritura::Cerrado { indice } => {
            let c = cambios.get(indice).unwrap_or(&cambios[0]);
            Fallo::Almacen(format!(
                "`{}` está en una fusión en curso: reintenta en unos segundos",
                c.nombre
            ))
        }
        FalloEscritura::Otro(m) => Fallo::Almacen(m),
    }
}

// ── Las escrituras de definición (E1a·5c·5c·2 y ·3): fuera de la puerta, para ore-serve ───────

/// **Las rutas de un nombre** en el catálogo (la de su sitio y, en `default`, la de antes): lo
/// que piden los cerrojos y el plan de fusión, que van por ruta. `None` si no es un nombre.
pub fn rutas_del_nombre(nombre: &str) -> Option<Vec<String>> {
    rutas_de(nombre)
}

/// **El nombre corto de una ruta de puntero** (`datasets/b/s/n.json` → `b.s.n`; uno de antes, por
/// lo que dice su puntero): lo que dicen los `datos` de una fusión. Si no es una ruta de puntero,
/// la ruta tal cual.
pub fn nombre_de_la_ruta(ruta: &str, puntero: Option<&str>) -> String {
    let dentro: PathBuf = Path::new(ruta).components().skip(1).collect();
    let n = puntero.and_then(|t| ore_core::parse::parse(t).ok());
    crudo::clave_de(&dentro, n.as_ref()).unwrap_or_else(|| ruta.to_string())
}

impl Catalogo {
    /// **Aplica una definición** (retirar, renombrar) que git confirmó en `sha`, en la ref de la
    /// puerta: [`Punteros::escribir`] con `idempotencia = git:<sha>` (la segunda vez no hace nada).
    /// `dueno`: lo que la definición cerró mientras grababa, que se suelta en la misma transacción.
    pub fn aplicar_definicion(
        &self,
        sha: &str,
        ctx: &Contexto,
        cambios: &[Cambio],
        dueno: Option<&str>,
    ) -> Result<(), Fallo> {
        if cambios.is_empty() {
            if let Some(d) = dueno {
                self.soltar_de(d)?;
            }
            return Ok(());
        }
        let mut a = self.a()?;
        let en_rutas = self.en_rutas(&mut a, cambios)?;
        a.aplicar_definicion(
            &self.rama,
            sha,
            &ctx.autor,
            &ctx.operacion,
            &en_rutas,
            dueno,
        )
        .map(|_| ())
        .map_err(|f| fallo_de(cambios, f))
    }

    /// **Cierra** `rutas` en `refs` para `dueno` durante `plazo` (ver [`Almacen::cerrar`]).
    pub fn cerrar(
        &self,
        refs: &[String],
        rutas: &[String],
        dueno: &str,
        plazo: std::time::Duration,
    ) -> Result<(), FalloCerrar> {
        self.a()
            .map_err(|f| FalloCerrar::Otro(f.to_string()))?
            .cerrar(refs, rutas, dueno, plazo)
    }

    /// **El reconciliador**: ¿está ya aplicado lo del commit de git `sha`?
    pub fn aplicado(&self, sha: &str) -> Result<bool, Fallo> {
        self.a()?.aplicado(sha).map_err(Fallo::Almacen)
    }

    /// Suelta todo lo de `dueno`.
    pub fn soltar_de(&self, dueno: &str) -> Result<usize, Fallo> {
        self.a()?.soltar(dueno).map_err(Fallo::Almacen)
    }

    /// **El plan de fusión** de la rama de la puerta en `main` (ver [`Almacen::plan_fusion`]).
    /// `rutas`: el alcance (`None`, todo lo propio). `elegidos`, **por nombre** (`b.s.n → rama |
    /// main`, lo de hoy): cada uno vale para sus dos rutas.
    pub fn plan_fusion(
        &self,
        rutas: Option<&[String]>,
        elegidos: &BTreeMap<String, String>,
    ) -> Result<PlanFusion, Fallo> {
        let por_ruta: BTreeMap<String, String> = elegidos
            .iter()
            .flat_map(|(n, lado)| {
                rutas_de(n)
                    .unwrap_or_default()
                    .into_iter()
                    .map(move |r| (r, lado.clone()))
            })
            .collect();
        self.a()?
            .plan_fusion(&self.rama, rutas, &por_ruta)
            .map_err(Fallo::Almacen)
    }

    /// **Aplica el plan** que viajó en el commit de git `sha` (ver [`Almacen::aplicar_plan`]).
    pub fn aplicar_plan(
        &self,
        sha: &str,
        sujeto: &str,
        plan: &str,
        tardia: bool,
    ) -> Result<Aplicada, FalloAplicar> {
        self.a()
            .map_err(|f| FalloAplicar::Otro(f.to_string()))?
            .aplicar_plan(&self.rama, sha, sujeto, plan, tardia)
    }

    /// Lo propio de la rama de la puerta (ver [`Almacen::propios`]).
    pub fn propios(&self) -> Result<Vec<Propio>, Fallo> {
        self.a()?.propios(&self.rama).map_err(Fallo::Almacen)
    }
}

impl Punteros for Catalogo {
    fn uno(&self, nombre: &str) -> Result<Option<Leido>, Fallo> {
        let rutas = rutas_de(nombre).ok_or_else(|| Fallo::Nombre(nombre.into()))?;
        let e = self
            .a()?
            .efectivo(&self.rama, &rutas)
            .map_err(Fallo::Almacen)?;
        Ok(e.and_then(|(de, t)| leido(&de, t)))
    }

    fn todos(&self) -> Result<BTreeMap<String, Leido>, Fallo> {
        let prefijo = format!("{}/", crudo::CARPETA);
        let mut filas: Vec<(PathBuf, String, String)> = self
            .a()?
            .vista_entera(&self.rama)
            .map_err(Fallo::Almacen)?
            .into_iter()
            .filter_map(|(ruta, de, t)| Some((PathBuf::from(ruta.strip_prefix(&prefijo)?), de, t)))
            .collect();
        // En el orden de `crudo::ficheros` (por componentes): el mismo ganador que `todos_en`.
        filas.sort_by(|x, y| x.0.cmp(&y.0));
        let mut out: BTreeMap<String, Leido> = BTreeMap::new();
        for (rel, de, t) in filas {
            let Some(l) = leido(&de, t) else { continue };
            let Some(clave) = crudo::clave_de(&rel, Some(&l.puntero)) else {
                continue;
            };
            if rel.components().count() == 1 && out.contains_key(&clave) {
                continue;
            }
            out.insert(clave, l);
        }
        Ok(out)
    }

    fn historia(&self, nombre: &str, hasta: usize) -> Result<Vec<Version>, Fallo> {
        let rutas = rutas_de(nombre).ok_or_else(|| Fallo::Nombre(nombre.into()))?;
        if hasta == 0 {
            return Ok(Vec::new());
        }
        let vs = self
            .a()?
            .historia(&self.rama, &rutas, hasta)
            .map_err(Fallo::Almacen)?;
        Ok(vs
            .into_iter()
            .filter_map(|v| {
                Some(Version {
                    puntero: ore_core::parse::parse(&v.texto).ok()?,
                    id: v.id,
                    cuando: v.cuando,
                    quien: v.quien.filter(|q| !q.is_empty()),
                    texto: v.texto,
                })
            })
            .collect())
    }

    fn escribir(&self, cambios: &[Cambio], ctx: &Contexto) -> Result<(), Fallo> {
        if cambios.is_empty() {
            return Ok(());
        }
        let mut a = self.a()?;
        let en_rutas = self.en_rutas(&mut a, cambios)?;
        a.escribir(
            &self.rama,
            &ctx.autor,
            &ctx.operacion,
            ctx.idempotencia.as_deref(),
            &en_rutas,
        )
        .map(|_| ())
        .map_err(|f| fallo_de(cambios, f))
    }

    fn reclamados(&self) -> Result<Reclamados, Fallo> {
        let mut r = Reclamados::default();
        for (_, ruta, t) in self.a()?.vivos().map_err(Fallo::Almacen)? {
            let Ok(n) = ore_core::parse::parse(&t) else {
                continue;
            };
            let p = Path::new(&ruta);
            let clase = ruta.split('/').next().unwrap_or(crudo::CARPETA);
            reclamar(&mut r, clase, p.file_stem().and_then(|s| s.to_str()), &n);
        }
        // Los informes de las funciones siguen en git.
        if let Some(raiz) = &self.arbol {
            for f in crudo::ficheros(&raiz.join("resultados")) {
                let Some(n) = std::fs::read_to_string(&f)
                    .ok()
                    .and_then(|t| ore_core::parse::parse(&t).ok())
                else {
                    continue;
                };
                reclamar(
                    &mut r,
                    "resultados",
                    f.file_stem().and_then(|s| s.to_str()),
                    &n,
                );
            }
        }
        Ok(r)
    }

    fn version(&self) -> Result<String, Fallo> {
        self.a()?.version(&self.rama).map_err(Fallo::Almacen)
    }
}

/// Las ramas del catálogo: su fila en `refs` (E1a·3).
impl Ramas for Catalogo {
    fn crear(&self, rama: &str, desde: &str) -> Result<(), Fallo> {
        self.a()?.crear_rama(rama, desde).map_err(Fallo::Almacen)
    }
    fn borrar(&self, rama: &str) -> Result<(), Fallo> {
        self.a()?.borrar_rama(rama).map_err(Fallo::Almacen)
    }
    fn traer(&self, rama: &str) -> Result<(), Fallo> {
        // Como un merge de git: lo propio que ya es igual a `main` deja de ser de la rama.
        self.a()?.traer_main_limpiando(rama).map_err(Fallo::Almacen)
    }
}

/// **El servidor del protocolo sobre el catálogo** (`ore-catalogo punteros`): cada `abrir` conecta
/// al esquema y abre la puerta en su `ref` (sin ella, `main`), con la `raiz` por sus `resultados/`.
/// Los directorios `carpeta` y `ajenos` no se usan: el catálogo es la carpeta, y `reclamados` ya
/// ve todas las refs. Con `sembrar`, la primera apertura de cada árbol lo siembra (las pruebas).
pub fn servidor(url: String, esquema: String, sembrar: bool) -> Servidor {
    Servidor::nuevo(Box::new(move |a: &Apertura| {
        let c = Catalogo::abrir(&url, &esquema, a.rama.as_deref())?.con_arbol(&a.raiz);
        if sembrar {
            c.sembrar_si_hace_falta(&a.raiz)?;
        }
        Ok(Box::new(c) as Box<dyn Puerta>)
    }))
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn las_rutas_de_un_nombre() {
        assert_eq!(
            rutas_de("ventas.pedidos").unwrap(),
            vec![
                "datasets/ventas/default/pedidos.json",
                "datasets/ventas_pedidos.json"
            ]
        );
        assert_eq!(
            rutas_de("ventas.eu.pedidos").unwrap(),
            vec!["datasets/ventas/eu/pedidos.json"]
        );
        assert!(rutas_de("sin_base").is_none());
        assert!(rutas_de("a..b").is_none());
    }

    #[test]
    fn la_ml_de_un_texto() {
        assert_eq!(
            ml_de("{\"metadata_location\": \"m\"}").as_deref(),
            Some("m")
        );
        assert_eq!(ml_de("{\"metadata_location\": null}"), None);
        assert_eq!(ml_de("{\"metadata_location\": \"\"}"), None);
        assert_eq!(ml_de("{}"), None);
    }
}
