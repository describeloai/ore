//! El plan: **un artefacto, y se rechaza sin abrir una conexión**.
//!
//! Las cuatro fases de `05-ejecutor` §3, en su orden, que es normativo:
//!
//! ```text
//! ① AUTORIZAR   Cedar sobre el principal → poda el plan
//! ② TRAVESÍA    el índice, en local → un conjunto de CLAVES
//! ③ CARGA ÚTIL  una petición por (fuente, entidad), POR CLAVE
//! ④ ENSAMBLAR   sobre flujos ya reducidos
//! ```
//!
//! # Por qué planificar es puro
//!
//! Nada de aquí lee una variable de entorno, abre un socket ni resuelve un
//! secreto: el plan sale del **bundle** más la **petición**. Eso no es una
//! comodidad — es lo que permite probarlo con la misma maquinaria que L0, y lo
//! que hace que *«el plan se rechaza sin abrir una conexión»* sea comprobable en
//! vez de prometido.
//!
//! # La forma canónica no se inventa
//!
//! El plan se renderiza con `Json::jcs()`, que es **la misma forma canónica del
//! bundle** — RFC 8785, claves ordenadas, sin espacios. Así *«mismas entradas →
//! mismo plan byte a byte»* es **G1 aplicado a L2**, y no una segunda definición
//! de determinismo que podría divergir de la primera.
//!
//! # Lo que la máscara hace aquí
//!
//! > **La forma más fuerte de aplicar una máscara es no pedir la columna.**
//!
//! Una propiedad cuya máscara es `redact` **desaparece de la proyección**.
//! Redactar después sería haber traído el valor, y entonces la máscara habría
//! dejado de ser una salvaguarda para ser un adorno con coste de red.

use crate::autorizar::{Identidad, Peticion, Veredicto};
use crate::motor::Motor;
use ore_core::json::Json;
use std::collections::{BTreeMap, BTreeSet};

/// Lo que se pide: una entidad, unas propiedades y —si se saben— unas claves.
///
/// En v1 las claves llegan con la consulta porque **no hay travesía**: la fase ②
/// es la que las produce, y su índice es de M3. Sin claves, el plan necesita un
/// recorrido completo y eso ya no lo decide el motor (§5).
#[derive(Debug, Clone)]
pub struct Consulta {
    pub quien: Identidad,
    pub accion: String,
    pub purpose: String,
    pub entidad: String,
    pub propiedades: Vec<String>,
    /// Los valores de clave, **como tuplas**: una clave compuesta es una tupla,
    /// y aplanarla habría vuelto a perder la aridad que `via` como secuencia
    /// costó cerrar.
    pub claves: Vec<Vec<String>>,
    /// La fase ②, si se pide: desde qué clave, por qué relación y cuántos
    /// saltos. Con ella las claves **se computan** en local en vez de llegar de
    /// fuera, que es lo que convierte un escaneo en una búsqueda por clave.
    pub travesia: Option<Travesia>,
    /// Cuándo se pregunta, y **por qué está aquí y no en `responder`**.
    ///
    /// Decidir si lo materializado está rancio necesita saberlo, y esa decisión
    /// es de planificación: es la que decide si se abre una conexión. No rompe
    /// la pureza del plan — el instante es una entrada más, como los atributos
    /// del principal. Lo que la rompería sería **leerlo**, no recibirlo.
    pub instante: Option<String>,
    /// El `freshnessSLA` que aplique. Sin él no hay rancio: nadie declaró
    /// cuánto se tolera, e inventarlo fallaría en una de las dos direcciones.
    pub sla: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Travesia {
    pub relacion: String,
    pub desde: String,
    pub saltos: usize,
}

/// Las condiciones de `05-ejecutor` §9. **No son códigos de documento**: un
/// rechazo en tiempo de consulta no es un defecto de un fichero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rechazo {
    /// La petición no existe: no satisface el esquema, o su emisor no es el
    /// declarado. Se rechaza **antes** de ①.
    PeticionInvalida(String),
    /// La política podó el plan hasta dejarlo vacío.
    NoAutorizado { porque: Vec<String> },
    /// El plan exige una operación que las capacidades no autorizan.
    PlanRechazado {
        binding: String,
        campo: String,
        porque: String,
    },
    /// Una propiedad no la mapea ningún binding: no hay de dónde leerla.
    SinBinding { propiedad: String },
    /// Hay más de una lectura que ensamblar y la clave no está autorizada.
    /// Juntar por un campo vacío produciría una fila que mezcla dos personas.
    SinClaveParaEnsamblar { propiedad: String },
    /// Se pidió una travesía y no hay índice de topología cargado. **Denegar
    /// sería correcto y callar por qué no**: sin índice no es que no haya
    /// vecinos, es que no se pudo mirar.
    TravesiaNoDisponible { relacion: String },
}

/// De dónde sale una lectura.
///
/// Que esto sea parte del plan y no un detalle de ejecución es el punto: **una
/// respuesta que no distingue caché de origen no se puede auditar**, y *«¿esto
/// vino del lago o del sistema de gestión?»* es la primera pregunta de
/// cualquiera que revise un número raro.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origen {
    /// Del origen declarado en el binding.
    ///
    /// `porque` distingue tres cosas que producen la misma lectura y **no
    /// significan lo mismo**: que no había manifiesto que consultar (`None`),
    /// que había uno y estaba rancio, y que había uno escrito **bajo otra
    /// regla**. La tercera es la que alguien tiene que ver.
    Fuente { porque: Option<String> },
    /// De la caché, con hasta cuándo era cierta.
    Cache { marca: String },
}

/// Una petición de la fase ③: **una por (fuente, objeto), y por clave**.
#[derive(Debug, Clone)]
pub struct Lectura {
    pub datasource: String,
    pub objeto: String,
    /// Propiedad → columna física. Es **la proyección**, y lo que no está aquí
    /// no se pide.
    pub proyeccion: BTreeMap<String, String>,
    /// Las columnas **físicas** de la clave primaria, en el orden que la entidad
    /// declara. Sin ellas el driver tendría los valores y no sabría contra qué
    /// compararlos.
    pub clave_columnas: Vec<String>,
    pub claves: Vec<Vec<String>>,
    /// Los predicados que salen de los **ámbitos** de las políticas que
    /// autorizaron: `<columna> = <valor de la reclamación>`. Viajan al origen.
    pub filtros: Vec<Filtro>,
    /// De dónde sale esta lectura, y si es del origen, por qué no de la caché.
    pub origen: Origen,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filtro {
    pub columna: String,
    /// La propiedad de la que salió esa columna, en corto.
    ///
    /// Viaja al lado porque **la caché nombra sus columnas como las
    /// propiedades**, así que servirse de ella exige poder reescribir el
    /// predicado. Derivarla de la columna sería imposible: el mapeo va en un
    /// sentido y dos propiedades pueden apuntar a la misma columna.
    pub propiedad: String,
    /// `eq` o `gt`. Un ámbito solo produce `eq`; `gt` es de la marca de agua,
    /// que no tiene principal.
    pub operador: String,
    pub valor: String,
    /// Qué ámbito lo produjo, para que el filtro sea rastreable hasta su regla.
    pub ambito: String,
}

#[derive(Debug, Clone)]
pub struct Plan {
    /// ① — qué propiedades sobrevivieron, y qué hay que aplicarles.
    pub autorizadas: BTreeMap<String, Vec<String>>,
    /// ① — y las que no, con el motivo. Un plan que no dice qué podó no se
    /// puede auditar.
    pub podadas: BTreeMap<String, String>,
    /// ② — en v1 no hay travesía: las claves llegan con la consulta.
    pub claves: Vec<Vec<String>>,
    /// ③
    pub lecturas: Vec<Lectura>,
    /// ④ — por qué propiedades se ensambla. Con una sola lectura no hay nada
    /// que ensamblar, y decirlo es mejor que omitirlo.
    pub ensamblar_por: Vec<String>,
}

/// Una lista de tuplas, en forma canónica.
fn tuplas(v: &[Vec<String>]) -> Json {
    Json::Arr(
        v.iter()
            .map(|t| Json::Arr(t.iter().map(|x| Json::s(x.as_str())).collect()))
            .collect(),
    )
}

impl Lectura {
    /// Servirse de la caché: **cambiarle a esta lectura la fuente y el objeto**.
    ///
    /// Nada más, y eso no es una simplificación mía — es lo que el
    /// [ADR 0006](../../docs/decisions/0006-el-artefacto-de-topologia.md) §3
    /// decidió sin que nadie lo leyera así: *«el escaneo columnar lo hace el
    /// mismo camino que ya lee cualquier tabla del cliente: la caché entra por
    /// la puerta que ya existe»*. Una tabla Iceberg en el lago del cliente es una
    /// tabla, y ya hay un protocolo para leer tablas.
    ///
    /// # Por qué la proyección se vuelve la identidad
    ///
    /// Un binding mapea propiedad → columna **del origen**. La caché no tiene por
    /// qué llamarlas igual, y la convención es que **la columna se llama como la
    /// propiedad**. Dos razones: la caché la escribimos nosotros, así que
    /// elegimos los nombres; y una tabla cuyas columnas son propiedades es
    /// autodescriptiva y no puede desviarse de un binding que alguien edite
    /// mañana. Con nombres del origen habría dos mapas del mismo hecho y ninguno
    /// diría cuál manda.
    fn desde_la_cache(&mut self, e: &ore_core::cache::Entrada, clave: &[String]) {
        self.datasource = e.datasource.clone();
        self.objeto = e.tabla.clone();
        let props: Vec<String> = self.proyeccion.keys().cloned().collect();
        self.proyeccion = props.into_iter().map(|p| (p.clone(), p)).collect();
        self.clave_columnas = clave.to_vec();
        for f in &mut self.filtros {
            f.columna = f.propiedad.clone();
        }
        self.origen = Origen::Cache {
            marca: e.marca.clone(),
        };
    }

    /// La petición que viaja al driver: **un fragmento del plan, no SQL**
    /// (`docs/decisions/0008-el-protocolo-del-driver.md`).
    ///
    /// La URL entra aquí y no en el plan a propósito: **el plan es puro**, y un
    /// secreto dentro de él lo volvería no imprimible. Quien invoca elige la
    /// identidad, que es lo que §6.2 pide al separar el proceso que refresca del
    /// que responde.
    pub fn peticion(&self, url: &str) -> String {
        Json::obj([
            ("url", Json::s(url)),
            ("objeto", Json::s(self.objeto.as_str())),
            (
                "proyeccion",
                Json::Obj(
                    self.proyeccion
                        .iter()
                        .map(|(p, c)| (p.clone(), Json::s(c.as_str())))
                        .collect(),
                ),
            ),
            (
                "claveColumnas",
                Json::Arr(
                    self.clave_columnas
                        .iter()
                        .map(|c| Json::s(c.as_str()))
                        .collect(),
                ),
            ),
            ("claves", tuplas(&self.claves)),
            (
                "filtros",
                Json::Arr(
                    self.filtros
                        .iter()
                        .map(|f| {
                            Json::obj([
                                ("columna", Json::s(f.columna.as_str())),
                                ("operador", Json::s(f.operador.as_str())),
                                ("valor", Json::s(f.valor.as_str())),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
        .jcs()
    }
}

impl Plan {
    /// La forma canónica del plan: **los mismos bytes que produciría cualquier
    /// implementación conforme**, porque es la del bundle.
    pub fn canonico(&self) -> String {
        let j = Json::obj([
            (
                "autorizadas",
                Json::Obj(
                    self.autorizadas
                        .iter()
                        .map(|(k, v)| {
                            (
                                k.clone(),
                                Json::Arr(v.iter().map(|x| Json::s(x.as_str())).collect()),
                            )
                        })
                        .collect(),
                ),
            ),
            (
                "podadas",
                Json::Obj(
                    self.podadas
                        .iter()
                        .map(|(k, v)| (k.clone(), Json::s(v.as_str())))
                        .collect(),
                ),
            ),
            ("claves", tuplas(&self.claves)),
            (
                "lecturas",
                Json::Arr(
                    self.lecturas
                        .iter()
                        .map(|l| {
                            Json::obj([
                                ("datasource", Json::s(l.datasource.as_str())),
                                ("objeto", Json::s(l.objeto.as_str())),
                                (
                                    "proyeccion",
                                    Json::Obj(
                                        l.proyeccion
                                            .iter()
                                            .map(|(p, c)| (p.clone(), Json::s(c.as_str())))
                                            .collect(),
                                    ),
                                ),
                                ("claves", tuplas(&l.claves)),
                                // Un plan que leyera de dos sitios distintos y
                                // saliera igual habria dejado de describir lo
                                // que paso.
                                (
                                    "origen",
                                    match &l.origen {
                                        Origen::Cache { marca } => Json::obj([
                                            ("de", Json::s("cache")),
                                            ("marca", Json::s(marca.as_str())),
                                        ]),
                                        Origen::Fuente { porque } => Json::obj([
                                            ("de", Json::s("fuente")),
                                            ("porque", Json::s(porque.as_deref().unwrap_or(""))),
                                        ]),
                                    },
                                ),
                                (
                                    "claveColumnas",
                                    Json::Arr(
                                        l.clave_columnas
                                            .iter()
                                            .map(|c| Json::s(c.as_str()))
                                            .collect(),
                                    ),
                                ),
                                (
                                    "filtros",
                                    Json::Arr(
                                        l.filtros
                                            .iter()
                                            .map(|f| {
                                                Json::obj([
                                                    ("ambito", Json::s(f.ambito.as_str())),
                                                    ("columna", Json::s(f.columna.as_str())),
                                                    ("operador", Json::s(f.operador.as_str())),
                                                    ("valor", Json::s(f.valor.as_str())),
                                                ])
                                            })
                                            .collect(),
                                    ),
                                ),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "ensamblarPor",
                Json::Arr(
                    self.ensamblar_por
                        .iter()
                        .map(|k| Json::s(k.as_str()))
                        .collect(),
                ),
            ),
        ]);
        j.jcs()
    }
}

impl Motor {
    /// Resuelve un `@oosMask("<ruleset>#<id>")` a su desclasificador.
    fn declasificador(&self, referencia: &str) -> Option<String> {
        let (rs, id) = referencia.split_once('#')?;
        let regla = self.paquete.docs.iter().find(|d| {
            d.kind == ore_core::document::Kind::Ruleset && d.qname().as_deref() == Some(rs)
        })?;
        regla
            .section("masks")?
            .items()
            .iter()
            .find(|m| m.get("id").and_then(|(_, v)| v.as_str()) == Some(id))
            .and_then(|m| m.get("declassifier"))
            .and_then(|(_, v)| v.as_str())
            .map(str::to_string)
    }

    /// Resuelve un `@oosScope("<ruleset>#<id>")` a `(property, matches)`.
    fn ambito(&self, referencia: &str) -> Option<(String, String)> {
        let (rs, id) = referencia.split_once('#')?;
        let regla = self.paquete.docs.iter().find(|d| {
            d.kind == ore_core::document::Kind::Ruleset && d.qname().as_deref() == Some(rs)
        })?;
        let s = regla
            .section("scopes")?
            .items()
            .iter()
            .find(|s| s.get("id").and_then(|(_, v)| v.as_str()) == Some(id))?;
        Some((
            s.get("property")?.1.as_str()?.to_string(),
            s.get("matches")?.1.as_str()?.to_string(),
        ))
    }

    /// Las lecturas que hacen falta para **construir el índice**: por cada
    /// relación con `via`, la clave de la entidad y la columna del enlace.
    ///
    /// Y lo interesante es que **no hay protocolo nuevo**: la proyección se
    /// nombra `desde` y `hasta`, y como el driver devuelve las filas con los
    /// nombres de propiedad, lo que sale ya es una arista. El driver no se
    /// entera de que esto es un índice, que es la prueba de que el protocolo de
    /// la fase ③ era el correcto.
    ///
    /// Devuelve también las relaciones que **no puede** leer, y por qué: una
    /// `via` compuesta es una clave de destino en tupla, y aplanarla aquí
    /// inventaría una codificación que nadie declaró.
    pub fn lecturas_de_aristas(&self) -> Vec<(String, Lectura)> {
        let mut out = Vec::new();
        for e in self.paquete.entities() {
            let Some(qn) = e.qname() else { continue };
            let Some(rels) = e.section("relations") else {
                continue;
            };
            let clave: Vec<String> = e
                .section("primaryKey")
                .map(|k| {
                    k.items()
                        .iter()
                        .filter_map(|i| i.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            if clave.len() != 1 {
                continue;
            }
            for (rk, rv) in rels.entries() {
                let Some(nombre) = rk.as_str() else { continue };
                let via: Vec<String> = rv
                    .get("via")
                    .map(|(_, v)| {
                        v.items()
                            .iter()
                            .filter_map(|i| i.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                if via.len() != 1 {
                    continue;
                }
                for b in self.paquete.docs.iter().filter(|d| {
                    d.kind == ore_core::document::Kind::Binding
                        && d.section("targetEntity").and_then(|t| t.as_str()) == Some(qn.as_str())
                }) {
                    let mapa = columnas(b);
                    let (Some(cd), Some(ch)) = (mapa.get(&clave[0]), mapa.get(&via[0])) else {
                        continue;
                    };
                    let Some(ds) = b.section("datasourceRef").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    out.push((
                        format!("{qn}.{nombre}"),
                        Lectura {
                            datasource: ds.to_string(),
                            objeto: b
                                .section("source")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            proyeccion: BTreeMap::from([
                                ("desde".to_string(), cd.clone()),
                                ("hasta".to_string(), ch.clone()),
                            ]),
                            clave_columnas: Vec::new(),
                            claves: Vec::new(),
                            filtros: Vec::new(),
                        },
                    ));
                }
            }
        }
        out
    }

    pub fn planificar(&self, c: &Consulta) -> Result<Plan, Rechazo> {
        // ── ① AUTORIZAR ──────────────────────────────────────────────────────
        //
        // Primero, y el orden es normativo: autorizar al final sería haber
        // abierto ya la conexión.
        let mut autorizadas: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut podadas: BTreeMap<String, String> = BTreeMap::new();
        let mut ambitos: BTreeSet<String> = BTreeSet::new();
        let mut redactadas: BTreeSet<String> = BTreeSet::new();

        for prop in &c.propiedades {
            let p = Peticion {
                quien: c.quien.clone(),
                accion: c.accion.clone(),
                propiedad: prop.clone(),
                purpose: c.purpose.clone(),
            };
            match self.autorizar(&p) {
                Veredicto::Invalida(m) => return Err(Rechazo::PeticionInvalida(m)),
                Veredicto::Denegado { porque, .. } => {
                    podadas.insert(prop.clone(), format!("{porque:?}"));
                }
                Veredicto::Permitido {
                    politicas,
                    obligaciones,
                    mascaras,
                    ambitos: ams,
                } => {
                    // La forma más fuerte de aplicar una máscara es no pedir la
                    // columna: `redact` saca la propiedad de la proyección.
                    if mascaras
                        .iter()
                        .filter_map(|m| self.declasificador(m))
                        .any(|d| d == "redact")
                    {
                        redactadas.insert(prop.clone());
                        podadas.insert(
                            prop.clone(),
                            "redactada: la máscara se aplica no pidiendo la columna".into(),
                        );
                        continue;
                    }
                    ambitos.extend(ams.iter().cloned());
                    let mut aplicar = obligaciones;
                    aplicar.extend(politicas.iter().map(|p| format!("por:{p}")));
                    autorizadas.insert(prop.clone(), aplicar);
                }
            }
        }

        if autorizadas.is_empty() {
            return Err(Rechazo::NoAutorizado {
                porque: podadas.values().cloned().collect(),
            });
        }

        // ── ② TRAVESÍA ───────────────────────────────────────────────────────
        //
        // **En local, sobre las aristas materializadas.** Cuando el motor abra
        // una conexión ya sabrá exactamente qué claves pide, y por eso puede
        // permitirse no compensar lo que la fuente no sabe hacer.
        let claves = match (&c.travesia, &self.topologia) {
            (None, _) => c.claves.clone(),
            (Some(t), None) => {
                return Err(Rechazo::TravesiaNoDisponible {
                    relacion: t.relacion.clone(),
                });
            }
            (Some(t), Some(indice)) => {
                let mut ks: Vec<Vec<String>> = indice
                    .travesia(&t.relacion, &t.desde, t.saltos)
                    .into_iter()
                    .map(|k| vec![k])
                    .collect();
                ks.extend(c.claves.iter().cloned());
                ks.sort();
                ks.dedup();
                ks
            }
        };

        // ── ③ CARGA ÚTIL ─────────────────────────────────────────────────────
        let bindings: Vec<&ore_core::link::Loaded> = self
            .paquete
            .docs
            .iter()
            .filter(|d| d.kind == ore_core::document::Kind::Binding)
            .filter(|d| {
                d.section("targetEntity").and_then(|t| t.as_str()) == Some(c.entidad.as_str())
            })
            .collect();

        let mut lecturas: Vec<Lectura> = Vec::new();
        for b in &bindings {
            let Some(ds) = b.section("datasourceRef").and_then(|v| v.as_str()) else {
                continue;
            };
            let objeto = b.section("source").and_then(|v| v.as_str()).unwrap_or("");
            let mapa = columnas(b);

            let proyeccion: BTreeMap<String, String> = autorizadas
                .keys()
                .filter_map(|prop| {
                    let corta = prop.rsplit('.').next().unwrap_or(prop);
                    mapa.get(corta).map(|col| (corta.to_owned(), col.clone()))
                })
                .collect();
            if proyeccion.is_empty() {
                continue;
            }

            // Los filtros de los ámbitos, con el valor que trajo el principal.
            let mut filtros: Vec<Filtro> = Vec::new();
            for a in &ambitos {
                let Some((prop, reclamacion)) = self.ambito(a) else {
                    continue;
                };
                let corta = prop.rsplit('.').next().unwrap_or(&prop).to_string();
                let (Some(col), Some(valor)) = (mapa.get(&corta), c.quien.claims.get(&reclamacion))
                else {
                    continue;
                };
                filtros.push(Filtro {
                    columna: col.clone(),
                    propiedad: corta.clone(),
                    operador: "eq".into(),
                    valor: valor.clone(),
                    ambito: a.clone(),
                });
            }
            filtros.sort_by(|x, y| (&x.ambito, &x.columna).cmp(&(&y.ambito, &y.columna)));

            // Sin claves y sin filtros, esto es un recorrido completo — y eso lo
            // autoriza el binding, no el motor (§5).
            if claves.is_empty() && filtros.is_empty() {
                let caps = b.section("capabilities");
                let full = caps
                    .and_then(|x| x.get("fullScan"))
                    .and_then(|(_, v)| v.as_str());
                let nombre = b.qname().unwrap_or_default();
                match full {
                    // Sin `capabilities`, un binding sirve la búsqueda por clave
                    // y nada más — P4, y `05-ejecutor` §5.1.
                    None => {
                        return Err(Rechazo::PlanRechazado {
                            binding: nombre,
                            campo: "capabilities".into(),
                            porque: "sin capacidades declaradas un binding sirve la búsqueda por \
                                     clave y nada más, y este plan no trae claves"
                                .into(),
                        });
                    }
                    Some("forbidden") => {
                        return Err(Rechazo::PlanRechazado {
                            binding: nombre,
                            campo: "fullScan".into(),
                            porque: "`fullScan: forbidden` es la negativa: un plan que necesite \
                                     recorrido completo se rechaza"
                                .into(),
                        });
                    }
                    _ => {}
                }
            }

            // Las columnas de la clave: la `primaryKey` de la entidad, pasada
            // por el mapeo de ESTE binding.
            let clave_columnas: Vec<String> = self
                .paquete
                .entity(&c.entidad)
                .and_then(|e| e.section("primaryKey"))
                .map(|k| {
                    k.items()
                        .iter()
                        .filter_map(|i| i.as_str())
                        .filter_map(|p| mapa.get(p).cloned())
                        .collect()
                })
                .unwrap_or_default();

            lecturas.push(Lectura {
                datasource: ds.to_string(),
                objeto: objeto.to_string(),
                proyeccion,
                clave_columnas,
                claves: claves.clone(),
                filtros,
                // Del origen mientras nadie diga otra cosa: la cache se
                // consulta despues, cuando ya estan todas y ordenadas.
                origen: Origen::Fuente { porque: None },
            });
        }

        // Una propiedad autorizada que ningún binding mapea **desaparecería de
        // la proyección sin decirlo**, y el plan diría ✓ sobre un dato que nunca
        // va a llegar. Un binding no está obligado a mapearlo todo —eso es
        // legal— así que lo que no puede ser legal es callarlo.
        let servidas: BTreeSet<String> = lecturas
            .iter()
            .flat_map(|l| l.proyeccion.keys().cloned())
            .collect();
        let huerfanas: Vec<String> = autorizadas
            .keys()
            .filter(|p| !servidas.contains(p.rsplit('.').next().unwrap_or(p)))
            .cloned()
            .collect();
        for h in huerfanas {
            autorizadas.remove(&h);
            podadas.insert(
                h,
                "autorizada, y ningún binding de la entidad la mapea: no hay de dónde leerla"
                    .into(),
            );
        }

        if lecturas.is_empty() || autorizadas.is_empty() {
            return Err(Rechazo::SinBinding {
                propiedad: podadas.keys().next().cloned().unwrap_or_default(),
            });
        }
        lecturas.sort_by(|a, b| (&a.datasource, &a.objeto).cmp(&(&b.datasource, &b.objeto)));

        // ── ③ bis · LA CACHÉ ─────────────────────────────────────────────────
        //
        // **Después de ③ y antes de ④**, y es el único punto donde las dos cosas
        // que hacen falta son ciertas a la vez: las lecturas ya están completas
        // —así que se sabe qué propiedades necesita cada una— y todavía no se ha
        // abierto nada.
        let clave_props: Vec<String> = self
            .paquete
            .entity(&c.entidad)
            .and_then(|e| e.section("primaryKey"))
            .map(|k| {
                k.items()
                    .iter()
                    .filter_map(|i| i.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        // La versión del índice **solo si hubo travesía**: sin ella la topología
        // no resolvió ninguna clave, y exigir que casen dejaría la caché
        // inservible para la mitad de los planes por una razón inventada.
        let version = c
            .travesia
            .as_ref()
            .and(self.topologia.as_ref())
            .map(|t| t.version());
        if let Some(m) = &self.cache {
            let bundle = ore_core::digest::bundle(&self.paquete);
            for l in &mut lecturas {
                // Las de la proyección, **más las de la clave, más las de los
                // filtros**. Las tres hacen falta y ninguna es obvia: sin la
                // clave se tiene la tabla y no se sabe qué fila pedir, y sin la
                // columna de un ámbito el predicado que restringe lo que el
                // principal puede ver **no se puede aplicar** — que es la peor
                // de las tres, porque devolvería filas de más.
                let mut necesita: Vec<String> = l.proyeccion.keys().cloned().collect();
                necesita.extend(clave_props.iter().cloned());
                necesita.extend(l.filtros.iter().map(|f| f.propiedad.clone()));
                necesita.sort();
                necesita.dedup();
                let v = m.consultar(&ore_core::cache::Pregunta {
                    bundle: &bundle,
                    topologia: version.as_deref(),
                    entidad: &c.entidad,
                    propiedades: &necesita,
                    instante: c.instante.as_deref(),
                    sla: c.sla.as_deref(),
                });
                match m.entradas.iter().find(|e| e.entidad == c.entidad) {
                    Some(e) if v.sirve() => l.desde_la_cache(e, &clave_props),
                    // Y el motivo viaja. Tres cosas distintas producen esta
                    // misma lectura al origen —no hay caché, la hay y está
                    // rancia, la hay y se escribió bajo otra regla— y la tercera
                    // es la que alguien tiene que ver.
                    _ => {
                        l.origen = Origen::Fuente {
                            porque: Some(v.como_texto()),
                        }
                    }
                }
            }
            // Servirse de la caché le cambia la fuente y el objeto a una
            // lectura, así que el orden de antes ya no vale. Se reordena por lo
            // mismo de siempre: dos ejecuciones tienen que dar el mismo plan
            // byte a byte, y el orden es parte de los bytes.
            lecturas.sort_by(|a, b| (&a.datasource, &a.objeto).cmp(&(&b.datasource, &b.objeto)));
        }

        // ── ④ ENSAMBLAR ──────────────────────────────────────────────────────
        //
        // Sobre flujos ya reducidos, y por la clave primaria de la entidad. Con
        // una sola lectura no hay nada que ensamblar, y el plan lo dice en vez
        // de callarlo.
        // ④ necesita la clave, y pedirla sin que este autorizada seria devolver
        // un dato que ① no permitio. Asi que si hay que ensamblar y la clave no
        // esta entre lo pedido, el plan lo DICE en vez de juntar por un campo
        // vacio — que es lo que produciria una fila que mezcla dos personas.
        if lecturas.len() > 1 {
            for k in &clave_props {
                let cualificado = format!("{}.{k}", c.entidad);
                if !autorizadas.contains_key(&cualificado) {
                    return Err(Rechazo::SinClaveParaEnsamblar {
                        propiedad: cualificado,
                    });
                }
            }
        }

        let ensamblar_por = if lecturas.len() > 1 {
            clave_props.clone()
        } else {
            Vec::new()
        };

        Ok(Plan {
            autorizadas,
            podadas,
            claves,
            lecturas,
            ensamblar_por,
        })
    }
}

/// Propiedad → columna física. Admite la forma breve y la expandida: la
/// canónica es la expandida, y el compilador ya la materializa al normalizar.
fn columnas(b: &ore_core::link::Loaded) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(ps) = b.section("properties") else {
        return out;
    };
    for (k, v) in ps.entries() {
        let Some(nombre) = k.as_str() else { continue };
        let col = v.as_str().map(str::to_string).or_else(|| {
            v.get("column")
                .and_then(|(_, c)| c.as_str())
                .map(str::to_string)
        });
        if let Some(col) = col {
            out.insert(nombre.to_string(), col);
        }
    }
    out
}
