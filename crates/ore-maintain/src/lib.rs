//! **El mantenedor delegado**: la sesión que corre el circuito Δ y sostiene el
//! estado parcial de una vista.
//!
//! El [ADR 0012](../../../docs/decisions/0012-el-estado-es-parcial-y-vive-en-el-cliente.md)
//! decidió **dónde** vive lo que el mantenimiento incremental tiene que
//! recordar, y dejó una pieza sin construir: *«sostener los bytes es de un
//! programa delegado, con la frontera de siempre: por stdin, y lo que devuelve
//! no se cree»*. Esto es ese programa.
//!
//! # Qué es, en una línea
//!
//! > **El Delta Compiler es la semántica y el Partial State Store es el
//! > contrato. Este es el único sitio donde los dos corren.**
//!
//! Y no reimplementa ni una regla de ninguno de los dos: compila el circuito
//! con [`Circuito`], sostiene el estado con [`StateStore`], decide con
//! [`decidir`] y reparte la *upquery* con el Pushdown Planner. Lo suyo es el
//! bucle, el protocolo y **tres decisiones**, que están abajo.
//!
//! # El protocolo
//!
//! El mismo transporte que el driver (`docs/decisions/0008`) y la misma forma:
//! stdin/stdout, NDJSON, un verbo explícito en `argv`. Lo que viaja es **el
//! plan y filas**, nunca un dialecto.
//!
//! ```text
//! ore-maintain mantener
//! ```
//!
//! La **primera línea** es la sesión; cada línea siguiente es una orden, y cada
//! orden tiene exactamente una respuesta. Al cerrarse stdin sale el informe.
//!
//! ```json
//! {"plan":{…},"clave":["pais"],"bundle":"sha256:…","capacidad":128,
//!  "capacidades":{"lago":{"predicatePushdown":["eq"],"fullScan":"forbidden"}}}
//! {"op":"leer","clave":[{"s":"ES"}]}
//! {"op":"rellenar","clave":[{"s":"ES"}],"filas":[…],"marca":7,"bundle":"sha256:…"}
//! {"op":"delta","marca":8,"hojas":[{"datasource":"lago","objeto":"pedidos","filas":[…]}]}
//! ```
//!
//! # Las tres decisiones que son de aquí
//!
//! ## 1 · Una vista que no se mantiene no abre sesión
//!
//! La sesión falla en la primera línea, con **todos** los motivos del Refresh
//! Analyzer. Es el análisis puesto en la puerta: quien intenta mantener un
//! `PROMEDIO` se entera antes de mandar una sola fila, no a la tercera hora.
//!
//! ## 2 · Un fallo devuelve una *upquery*, y si hay capacidades, **la petición**
//!
//! Leer una clave ausente devuelve el plan que la repone y —cuando la sesión
//! declaró qué sabe hacer cada origen— lo que el Pushdown Planner le pediría a
//! cada hoja. Eso convierte el *miss* en algo que se le puede dar a un
//! `ore-read-<tipo>` tal cual.
//!
//! **La URL no viaja.** Es lo mismo que hace el driver al revés: quien invoca
//! elige la identidad, y un mantenedor que conociera la credencial sería un
//! sitio más donde vive un secreto. La petición sale sin ella y la completa
//! quien la tiene.
//!
//! ## 3 · El dictamen viaja, y **no se obedece solo**
//!
//! Cada paso trae su dictamen del Cost Model, con las medidas que entraron. Y
//! el paso **se aplica igual**, aunque el dictamen diga `RECOMPUTAR`.
//!
//! No es dejadez: es que este proceso **no puede recomputar**. Recomputar es
//! releer la fuente, y la fuente es del cliente. Si el mantenedor saltara el
//! paso, sus integradores se quedarían sin ver ese Δ y **todos los pasos
//! siguientes darían mal** — una junta cuyo integrador se saltó una alta ya no
//! empareja. Así que mantiene, y quien recibe el dictamen decide si recomputa y
//! **abre otra sesión**, que es lo que un recómputo es aquí: un estado nuevo.
//!
//! > Un dictamen que se obedeciera a sí mismo a mitad de un circuito sería una
//! > optimización que corrompe el estado.
//!
//! # Lo que no hace
//!
//! No abre nada, no lee el reloj y no tiene credenciales. Y **no cree lo que le
//! dan**: un relleno bajo otro *bundle* se rechaza, uno que nadie pidió se
//! rechaza, y una fila que no cuadra con el esquema del plan se rechaza — las
//! tres reglas son del Partial State Store, y aquí solo se ejercen.

use std::collections::BTreeMap;

use ore_core::json::Json;
use ore_core::parse::Node;
use ore_view::capabilities::{self, Capacidades};
use ore_view::cost_model::{Decision, Medida, Politica, decidir};
use ore_view::delta_compiler::{Circuito, Zset};
use ore_view::plan::{Nodo, Valor};
use ore_view::refresh_analyzer::analizar;
use ore_view::state_store::{Identidades, Lectura, StateStore};
use ore_view::{Hoja, RefreshMode};

/// Una sesión de mantenimiento: **una vista, un circuito y un almacén**.
pub struct Sesion {
    plan: Nodo,
    circuito: Circuito,
    almacen: StateStore,
    capacidades: BTreeMap<String, Capacidades>,
    politica: Politica,
    pasos: u64,
}

impl Sesion {
    /// Abre la sesión desde su primera línea.
    ///
    /// Falla si la vista no se puede mantener, y entonces el error trae **todos
    /// los motivos**: el Refresh Analyzer en la puerta.
    pub fn abrir(n: &Node) -> Result<Sesion, String> {
        let plan = match n.get("plan") {
            Some((_, p)) => Nodo::de(p).map_err(|e| format!("`plan`: {e}"))?,
            None => return Err("la sesión no trae `plan`".into()),
        };

        // 1 · ¿Se puede mantener? Antes que nada, y con la lista entera.
        if let RefreshMode::Full { porque } = analizar(&plan) {
            return Err(format!(
                "esta vista no se mantiene incrementalmente: {}",
                porque
                    .iter()
                    .map(|m| m.como_texto())
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
        }

        let clave: Vec<String> = n
            .get("clave")
            .map(|(_, v)| {
                v.items()
                    .iter()
                    .filter_map(|c| c.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        if clave.is_empty() {
            return Err(
                "la sesión no trae `clave`: sin ella el almacén no sabe qué guardar".into(),
            );
        }

        let bundle = texto(n, "bundle").unwrap_or_default();
        if bundle.is_empty() {
            return Err(
                "la sesión no trae `bundle`: sin él un relleno de otra compilación no se \
                 puede distinguir de uno bueno"
                    .into(),
            );
        }
        let topologia = texto(n, "topologia");
        let capacidad = entero(n, "capacidad").unwrap_or(128).max(1) as usize;

        let almacen = StateStore::nuevo(plan.clone(), clave, bundle, topologia, capacidad)
            .map_err(|r| r.como_texto())?;

        let circuito = Circuito::compilar(&plan).map_err(|m| m.como_texto())?;

        let capacidades = n
            .get("capacidades")
            .map(|(_, v)| {
                v.entries()
                    .iter()
                    .filter_map(|(k, c)| Some((k.as_str()?.to_string(), Capacidades::de_oos(c))))
                    .collect()
            })
            .unwrap_or_default();

        // Sin política declarada, la que dice ser lo que es. **No se elige la de
        // Snowflake por defecto**: es su cifra, no la nuestra, y ponerla de
        // silencio la convertiría en nuestra.
        let politica = n
            .get("politica")
            .and_then(|(_, p)| politica_de(p))
            .unwrap_or_else(Politica::sin_medir);

        Ok(Sesion {
            plan,
            circuito,
            almacen,
            capacidades,
            politica,
            pasos: 0,
        })
    }

    /// Atiende una orden. **Nunca falla hacia fuera**: un error es una
    /// respuesta, porque una sesión que se cae en la orden tres pierde el
    /// estado de las dos primeras.
    pub fn atender(&mut self, n: &Node) -> Json {
        let op = n.get("op").and_then(|(_, v)| v.as_str()).unwrap_or("");
        match op {
            "leer" => self.leer(n),
            "rellenar" => self.rellenar(n),
            "delta" => self.delta(n),
            "desalojar" => self.desalojar(n),
            "estado" => self.estado(),
            "" => error("", "una orden sin `op`"),
            otro => error(
                "",
                &format!("`{otro}` no es una orden: leer · rellenar · delta · desalojar · estado"),
            ),
        }
    }

    fn leer(&mut self, n: &Node) -> Json {
        let clave = match clave_de(n) {
            Ok(c) => c,
            Err(e) => return error("leer", &e),
        };
        match self.almacen.leer(&clave) {
            Lectura::Presente { filas, marca } => Json::obj([
                ("op", Json::s("leer")),
                ("presente", Json::Bool(true)),
                ("marca", Json::Int(marca as i64)),
                ("filas", filas.json()),
            ]),
            Lectura::Ausente { upquery } => {
                // La upquery es un plan; con capacidades es además lo que se le
                // pide a cada hoja. Un fallo que el origen no sabe contestar se
                // dice **aquí**, antes de que nadie abra nada.
                let reparto = if self.capacidades.is_empty() {
                    None
                } else {
                    Some(capabilities::repartir(&upquery, &self.capacidades))
                };
                let mut campos = vec![
                    ("op", Json::s("leer")),
                    ("presente", Json::Bool(false)),
                    ("upquery", upquery.json()),
                ];
                match reparto {
                    None => {}
                    Some(Ok(r)) => campos.push((
                        "peticiones",
                        Json::Arr(
                            r.peticiones
                                .iter()
                                .map(|p| {
                                    Json::obj([
                                        ("datasource", Json::s(p.datasource.as_str())),
                                        ("objeto", Json::s(p.objeto.as_str())),
                                        (
                                            "filtros",
                                            Json::Arr(p.filtros.iter().map(|f| f.json()).collect()),
                                        ),
                                    ])
                                })
                                .collect(),
                        ),
                    )),
                    Some(Err(r)) => campos.push(("rechazo", Json::s(r.como_texto()))),
                }
                Json::obj(campos)
            }
        }
    }

    fn rellenar(&mut self, n: &Node) -> Json {
        let clave = match clave_de(n) {
            Ok(c) => c,
            Err(e) => return error("rellenar", &e),
        };
        let filas = match n.get("filas").map(|(_, v)| Zset::leer(v)) {
            Some(Ok(z)) => z,
            Some(Err(e)) => return error("rellenar", &e),
            None => return error("rellenar", "un relleno sin `filas`"),
        };
        // Las identidades vienen **con el relleno** y no de la sesión: lo que se
        // comprueba es bajo qué se computaron ESTAS filas, y quien las trae es
        // quien lo sabe.
        let identidades = Identidades {
            bundle: texto(n, "bundle").unwrap_or_default(),
            topologia: texto(n, "topologia"),
            marca: entero(n, "marca").unwrap_or(0),
        };
        match self.almacen.rellenar(&clave, filas, identidades) {
            Ok(()) => Json::obj([("op", Json::s("rellenar")), ("ok", Json::Bool(true))]),
            Err(r) => error("rellenar", &r.como_texto()),
        }
    }

    fn delta(&mut self, n: &Node) -> Json {
        let marca = entero(n, "marca").unwrap_or(0);
        let mut deltas: BTreeMap<Hoja, Zset> = BTreeMap::new();
        let mut filas_delta = 0u64;
        for h in n.get("hojas").map(|(_, v)| v.items()).unwrap_or(&[]) {
            let (Some(ds), Some(ob)) = (
                h.get("datasource").and_then(|(_, v)| v.as_str()),
                h.get("objeto").and_then(|(_, v)| v.as_str()),
            ) else {
                return error("delta", "una hoja sin `datasource` u `objeto`");
            };
            let z = match h.get("filas").map(|(_, v)| Zset::leer(v)) {
                Some(Ok(z)) => z,
                Some(Err(e)) => return error("delta", &e),
                None => Zset::nuevo(),
            };
            filas_delta += z.filas().count() as u64;
            deltas.insert((ds.to_string(), ob.to_string()), z);
        }

        // La medida, ANTES del paso: el dictamen habla de lo que había.
        let medida = Medida {
            filas_base: entero(n, "base").unwrap_or_else(|| self.almacen.filas()),
            filas_delta,
        };
        let dictamen = decidir(&self.plan, medida, &self.politica);

        let salida = match self.circuito.paso(&deltas) {
            Ok(s) => s,
            Err(e) => return error("delta", &e.como_texto()),
        };
        self.pasos += 1;

        let aplicacion = match self.almacen.aplicar(&salida, marca) {
            Ok(a) => a,
            Err(r) => return error("delta", &r.como_texto()),
        };

        Json::obj([
            ("op", Json::s("delta")),
            (
                "decision",
                Json::s(match dictamen.decision {
                    Decision::Incremental => "incremental",
                    Decision::Recomputar => "recomputar",
                }),
            ),
            ("porque", Json::s(dictamen.porque.as_str())),
            ("base", Json::Int(medida.filas_base as i64)),
            ("delta", Json::Int(medida.filas_delta as i64)),
            ("integradores", Json::Int(dictamen.integradores as i64)),
            ("salida", salida.json()),
            ("aplicadas", Json::Int(aplicacion.aplicadas as i64)),
            (
                "guardadasEnVuelo",
                Json::Int(aplicacion.guardadas_en_vuelo as i64),
            ),
            (
                "descartadasAusentes",
                Json::Int(aplicacion.descartadas_ausentes as i64),
            ),
            (
                "descartadasViejas",
                Json::Int(aplicacion.descartadas_viejas as i64),
            ),
        ])
    }

    fn desalojar(&mut self, n: &Node) -> Json {
        let clave = match clave_de(n) {
            Ok(c) => c,
            Err(e) => return error("desalojar", &e),
        };
        Json::obj([
            ("op", Json::s("desalojar")),
            ("desalojada", Json::Bool(self.almacen.desalojar(&clave))),
        ])
    }

    fn estado(&self) -> Json {
        let s = self.almacen.estadisticas();
        Json::obj([
            ("op", Json::s("estado")),
            ("aciertos", Json::Int(s.aciertos as i64)),
            ("fallos", Json::Int(s.fallos as i64)),
            ("desalojos", Json::Int(s.desalojos as i64)),
            ("rellenos", Json::Int(s.rellenos as i64)),
            ("calientes", Json::Int(self.almacen.claves().len() as i64)),
            ("filas", Json::Int(self.almacen.filas() as i64)),
        ])
    }

    /// El informe de cierre. Sale cuando stdin se cierra, y es lo que queda en
    /// un log cuando la sesión ya no está.
    pub fn fin(&self) -> Json {
        let s = self.almacen.estadisticas();
        Json::obj([
            ("op", Json::s("fin")),
            ("pasos", Json::Int(self.pasos as i64)),
            ("aciertos", Json::Int(s.aciertos as i64)),
            ("fallos", Json::Int(s.fallos as i64)),
            ("desalojos", Json::Int(s.desalojos as i64)),
            ("rellenos", Json::Int(s.rellenos as i64)),
            ("calientes", Json::Int(self.almacen.claves().len() as i64)),
            ("filas", Json::Int(self.almacen.filas() as i64)),
        ])
    }
}

// ── Lectura del protocolo ───────────────────────────────────────────────────

fn error(op: &str, porque: &str) -> Json {
    Json::obj([("op", Json::s(op)), ("error", Json::s(porque.to_string()))])
}

fn texto(n: &Node, k: &str) -> Option<String> {
    n.get(k)
        .and_then(|(_, v)| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
}

fn entero(n: &Node, k: &str) -> Option<u64> {
    n.get(k).and_then(|(_, v)| v.as_str()?.parse().ok())
}

fn clave_de(n: &Node) -> Result<Vec<Valor>, String> {
    let Some((_, v)) = n.get("clave") else {
        return Err("la orden no trae `clave`".into());
    };
    v.items()
        .iter()
        .map(|x| Valor::leer(x).ok_or_else(|| "una clave con un valor que no se lee".to_string()))
        .collect()
}

fn politica_de(n: &Node) -> Option<Politica> {
    if let Some((_, u)) = n.get("umbral") {
        return Some(Politica::Umbral {
            numerador: entero(u, "numerador")?,
            denominador: entero(u, "denominador")?,
        });
    }
    if let Some((_, c)) = n.get("coeficientes") {
        return Some(Politica::Coeficientes {
            por_fila_delta: entero(c, "porFilaDelta")?,
            por_fila_recomputo: entero(c, "porFilaRecomputo")?,
            por_integrador: entero(c, "porIntegrador")?,
        });
    }
    None
}
