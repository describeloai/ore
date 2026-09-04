//! **La `Propuesta`**: lo que una función devuelve, y bajo qué lo decidió.
//!
//! > *Una función no aplica, propone.* — [`docs/functions.md`](../../../docs/functions.md) §3
//!
//! Un `Plan` entra, una `Propuesta` sale. La función es **pura**: recibe
//! valores, no una conexión; no lee durante la ejecución y no escribe durante
//! la ejecución. Lo que devuelve no se aplica: se **coteja**, y lo que quede
//! fuera de lo declarado se rechaza.
//!
//! # Por qué es un documento y no un `struct` enlazado
//!
//! Porque tres de sus cinco identidades **no se pueden alcanzar desde aquí**, y
//! eso no es una carencia: es la doctrina de delegación funcionando.
//!
//! | identidad | de dónde viene | ¿la alcanza `ore-cli`? |
//! |---|---|---|
//! | digest del bundle | [`crate::digest::bundle`] | sí |
//! | digest del plan de la vista | `ore-view`, `Nodo::digest` | sí |
//! | versión de topología | quien construya el índice | **no** — nadie lo construye hoy |
//! | marcas de agua | `ore-store-r2` | **no** — trae Parquet, TLS |
//! | el `Plan` | quien planifique la lectura | **no** |
//!
//! `ore-cli` depende de `ore-core` y `ore-view`, y de nada más, porque
//! `tests/dependencias.rs` lee el `Cargo.lock` y lo hace cumplir. Así que las
//! tres de abajo **llegan por el protocolo**, en JSON canónico, igual que llega
//! la cabecera de un sobre. Una `Propuesta` es un artefacto, no un grafo de
//! tipos.
//!
//! # Lo que este módulo comprueba, y lo que no puede
//!
//! Comprueba lo que se puede contestar **sin abrir nada**: que la función
//! exista, que cada edit caiga dentro de la superficie que `effects:` declara,
//! que nombre la fila con la clave de su entidad, y que el bundle bajo el que
//! se decidió siga siendo este.
//!
//! No comprueba las tres identidades delegadas —nadie aquí puede—, y **no las
//! calla**: viajan dentro del sello, así que un auditor con el artefacto y los
//! programas delegados las puede contrastar. Decir «no verificado» es una
//! respuesta; omitirlo no lo es.
//!
//! El digest del plan de la vista sí se puede recontrastar, pero no aquí: hace
//! falta el motor de vistas, y quien tiene los dos es `ore-cli`. Lo hace
//! `ore verify`.

use std::collections::BTreeMap;

use crate::document::Kind;
use crate::json::Json;
use crate::link::Package;
use crate::parse::Node;

/// **Bajo qué se decidió.** Las cinco identidades de `functions.md` §3.1.
///
/// Con las cinco dentro, una propuesta se contesta sola: *¿se puede
/// reproducir?*, *¿se computó sobre dato rancio?*, *¿el significado sigue
/// vigente?*, *¿por dónde iba a entrar?*
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Bajo {
    /// Bajo qué significado se decidió — y si sigue vigente.
    pub bundle: String,
    /// Con qué correspondencia se resolvieron las claves.
    pub topologia: String,
    /// Hasta cuándo era cierto el dato que se leyó, por objeto.
    pub testigos: BTreeMap<String, String>,
    /// Qué se leyó, qué se podó y por qué. Se cita por digest, y el par
    /// `(Plan, Propuesta)` es la historia entera **porque los dos se nombran por
    /// su contenido**.
    ///
    /// **Hoy no hay quien produzca ese plan.** El que lo hacía era el ejecutor
    /// del paradigma de bindings y se retiró; quién planifica una lectura en el
    /// paradigma de vistas está sin decidir. El campo se queda porque la
    /// identidad hace falta, no porque haya quien la rellene.
    pub plan: String,
    /// **Por dónde se va a escribir, y bajo qué recorte.**
    ///
    /// La quinta, y la que sale del sustrato: una vista recorta filas, y una
    /// propuesta que escribe a través de ella solo puede tocar las que la vista
    /// deja ver. Sin esto, *«¿podía esta función tocar esta fila?»* no tiene
    /// respuesta local.
    pub vista: String,
}

/// Un cambio propuesto: qué propiedad, de qué fila, y a qué valor.
///
/// **Nombra la propiedad, no la columna.** Es el idioma de la ontología, y la
/// ontología no debe saber en qué columna cae — eso lo decide la vista, y por
/// eso `datasourceRef` desapareció del efecto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    /// El nombre cualificado de la propiedad: `hr.Employee.estado`.
    pub escribe: String,
    /// Qué fila. En propiedades, no en columnas, y **la clave entera**.
    pub fila: BTreeMap<String, String>,
    pub valor: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Propuesta {
    /// Quién la propone. Es lo que da la superficie contra la que se coteja.
    pub funcion: String,
    pub bajo: Bajo,
    pub edits: Vec<Edit>,
}

impl Propuesta {
    /// La forma canónica: **los mismos bytes que produciría cualquier
    /// implementación conforme**, porque es la del bundle.
    ///
    /// Es lo que hace que *«mismas identidades + mismos valores → misma
    /// propuesta»* sea comprobable en vez de prometido, y con ello el replay
    /// para un auditor.
    pub fn canonica(&self) -> String {
        Json::obj([
            ("funcion", Json::s(&self.funcion)),
            (
                "bajo",
                Json::obj([
                    ("bundle", Json::s(&self.bajo.bundle)),
                    ("plan", Json::s(&self.bajo.plan)),
                    (
                        "testigos",
                        Json::Obj(
                            self.bajo
                                .testigos
                                .iter()
                                .map(|(k, v)| (k.clone(), Json::s(v.as_str())))
                                .collect(),
                        ),
                    ),
                    ("topologia", Json::s(&self.bajo.topologia)),
                    ("vista", Json::s(&self.bajo.vista)),
                ]),
            ),
            (
                "edits",
                Json::Arr(
                    self.edits
                        .iter()
                        .map(|e| {
                            Json::obj([
                                ("escribe", Json::s(&e.escribe)),
                                (
                                    "fila",
                                    Json::Obj(
                                        e.fila
                                            .iter()
                                            .map(|(k, v)| (k.clone(), Json::s(v.as_str())))
                                            .collect(),
                                    ),
                                ),
                                ("valor", Json::s(&e.valor)),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
        .jcs()
    }

    /// El nombre de la propuesta **es** su contenido, como el de una copia.
    ///
    /// Con él, *«¿esto ya se aplicó?»* pasa de ser una pregunta sin sitio donde
    /// contestarse a una búsqueda por clave.
    pub fn digest(&self) -> String {
        crate::digest::de_bytes(self.canonica().as_bytes())
    }

    /// Lee lo que devolvió un programa delegado.
    ///
    /// **Lo que devuelve un delegado no se cree**, así que esto solo lo
    /// convierte en estructura; quien decide si vale es [`cotejar`]. Es lo
    /// mismo que hace `ore pack` con una firma: primero se lee, después se
    /// comprueba, y las dos cosas no son la misma.
    pub fn abrir(n: &Node) -> Result<Propuesta, String> {
        let s = |nodo: &Node, k: &str| -> Option<String> {
            nodo.get(k).and_then(|(_, v)| v.as_str()).map(String::from)
        };
        let mapa = |nodo: &Node, k: &str| -> BTreeMap<String, String> {
            nodo.get(k)
                .map(|(_, v)| {
                    v.entries()
                        .iter()
                        .filter_map(|(a, b)| Some((a.as_str()?.to_string(), b.as_str()?.to_string())))
                        .collect()
                })
                .unwrap_or_default()
        };

        let funcion = s(n, "funcion").ok_or("una propuesta sin `funcion` no se puede cotejar")?;
        let bajo_n = n
            .get("bajo")
            .map(|(_, v)| v)
            .ok_or("una propuesta sin `bajo` no dice bajo qué se decidió")?;
        let bajo = Bajo {
            bundle: s(bajo_n, "bundle").unwrap_or_default(),
            topologia: s(bajo_n, "topologia").unwrap_or_default(),
            testigos: mapa(bajo_n, "testigos"),
            plan: s(bajo_n, "plan").unwrap_or_default(),
            vista: s(bajo_n, "vista").unwrap_or_default(),
        };

        let mut edits = Vec::new();
        for e in n.get("edits").map(|(_, v)| v.items()).unwrap_or(&[]) {
            edits.push(Edit {
                escribe: s(e, "escribe").ok_or("un edit sin `escribe` no dice qué toca")?,
                fila: mapa(e, "fila"),
                valor: s(e, "valor").unwrap_or_default(),
            });
        }
        Ok(Propuesta {
            funcion,
            bajo,
            edits,
        })
    }
}

/// Por qué una propuesta no se aplica.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rechazo {
    /// La función que dice proponerla no está en el paquete.
    FuncionDesconocida(String),
    /// **El cotejo.** Un edit sobre algo que la función no declaró escribir.
    ///
    /// Es la razón de ser de este módulo: lo que devuelve un delegado no se
    /// cree. Una función que declaró tocar `status` y devuelve un edit sobre
    /// `salario` no es un error de programación que se descubra en producción;
    /// es una propuesta que no se aplica.
    FueraDeLaSuperficie {
        escribe: String,
        declarados: Vec<String>,
    },
    /// El edit no nombra la fila con la clave de su entidad.
    ///
    /// Sobra o falta: las dos son el mismo defecto, porque las dos dejan sin
    /// contestar *«qué fila»*.
    FilaSinClave {
        escribe: String,
        esperaba: Vec<String>,
        traia: Vec<String>,
    },
    /// El significado cambió desde que se decidió.
    ///
    /// No es un fallo de la propuesta: es que ya no es sobre este paquete. Se
    /// dice con los dos digests para que quien mire sepa cuál tiene delante.
    BundleCambiado { decidido: String, ahora: String },
}

impl Rechazo {
    pub fn como_texto(&self) -> String {
        match self {
            Rechazo::FuncionDesconocida(f) => {
                format!("`{f}` no es una función de este paquete")
            }
            Rechazo::FueraDeLaSuperficie {
                escribe,
                declarados,
            } => format!(
                "escribe `{escribe}`, que no está en sus `effects:` — declara {}",
                if declarados.is_empty() {
                    "nada".to_string()
                } else {
                    format!("`{}`", declarados.join("`, `"))
                }
            ),
            Rechazo::FilaSinClave {
                escribe,
                esperaba,
                traia,
            } => format!(
                "el edit sobre `{escribe}` nombra la fila con [{}] y su entidad se identifica \
                 con [{}]",
                traia.join(", "),
                esperaba.join(", ")
            ),
            Rechazo::BundleCambiado { decidido, ahora } => format!(
                "se decidió bajo el bundle {decidido} y este paquete es {ahora}: el significado \
                 cambió, así que la propuesta no es sobre él"
            ),
        }
    }
}

/// **El cotejo.** Lo que se puede contestar sin abrir nada.
///
/// Devuelve todos los rechazos y no el primero: quien recibe una propuesta mala
/// quiere saber cuánto tiene de mala, no cuál fue el primer edit que falló.
pub fn cotejar(pkg: &Package, p: &Propuesta) -> Vec<Rechazo> {
    let mut out = Vec::new();

    let ahora = crate::digest::bundle(pkg);
    if !p.bajo.bundle.is_empty() && p.bajo.bundle != ahora {
        out.push(Rechazo::BundleCambiado {
            decidido: p.bajo.bundle.clone(),
            ahora,
        });
    }

    let Some(f) = pkg
        .of(Kind::Function)
        .find(|f| f.qname().as_deref() == Some(p.funcion.as_str()))
    else {
        out.push(Rechazo::FuncionDesconocida(p.funcion.clone()));
        return out;
    };

    let declarados = crate::effect::destinos(f);
    for e in &p.edits {
        if !declarados.contains(&e.escribe) {
            out.push(Rechazo::FueraDeLaSuperficie {
                escribe: e.escribe.clone(),
                declarados: declarados.clone(),
            });
            continue;
        }
        // Y que nombre la fila. La clave es la de la ENTIDAD —propiedades— y no
        // la de la tabla —columnas—: quien propone habla el idioma de arriba, y
        // bajarlo es de quien aplica.
        let Some(esperaba) = clave_de(pkg, &e.escribe) else {
            continue; // la entidad no resuelve: es `OOS2005`, y falla antes
        };
        let traia: Vec<String> = e.fila.keys().cloned().collect();
        if traia != esperaba {
            out.push(Rechazo::FilaSinClave {
                escribe: e.escribe.clone(),
                esperaba,
                traia,
            });
        }
    }
    out
}

/// La `primaryKey` de la entidad a la que pertenece una propiedad, ordenada.
///
/// Ordenada porque se compara con las claves de un `BTreeMap`, y dos claves que
/// solo difieran en el orden en que se escribieron son la misma clave.
fn clave_de(pkg: &Package, propiedad_qn: &str) -> Option<Vec<String>> {
    let (entidad_qn, _) = propiedad_qn.rsplit_once('.')?;
    let e = pkg
        .of(Kind::Entity)
        .find(|e| e.qname().as_deref() == Some(entidad_qn))?;
    let mut k: Vec<String> = e
        .section("primaryKey")?
        .items()
        .iter()
        .filter_map(|i| i.as_str())
        .map(String::from)
        .collect();
    k.sort_unstable();
    Some(k)
}

/// El sello que acompaña a una propuesta cuando se enseña.
///
/// Deliberadamente **no** es JSON: es lo que un humano lee antes de decidir. El
/// artefacto es [`Propuesta::canonica`].
pub fn imprimir(p: &Propuesta, verificado: &[(&str, bool)]) -> String {
    let mut s = String::new();
    s.push_str(&format!("propuesta {}\n", p.digest()));
    s.push_str(&format!("  funcion   {}\n", p.funcion));
    for (nombre, ok) in verificado {
        s.push_str(&format!(
            "  {nombre:<9} {}\n",
            if *ok { "coincide" } else { "NO coincide" }
        ));
    }
    for e in &p.edits {
        let fila: Vec<String> = e.fila.iter().map(|(k, v)| format!("{k}={v}")).collect();
        s.push_str(&format!(
            "  edit      {} [{}] ← {}\n",
            e.escribe,
            fila.join(", "),
            e.valor
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;

    fn p() -> Propuesta {
        Propuesta {
            funcion: "hr.activar".to_string(),
            bajo: Bajo {
                bundle: "sha256:bbb".to_string(),
                topologia: "1".to_string(),
                testigos: [("erp.employees".to_string(), "42".to_string())]
                    .into_iter()
                    .collect(),
                plan: "sha256:ppp".to_string(),
                vista: "sha256:vvv".to_string(),
            },
            edits: vec![Edit {
                escribe: "hr.Employee.estado".to_string(),
                fila: [("employeeId".to_string(), "E-1".to_string())]
                    .into_iter()
                    .collect(),
                valor: "ACTIVO".to_string(),
            }],
        }
    }

    /// **Digiere igual dos veces**, que es la primera mitad del «listo cuando».
    #[test]
    fn la_misma_propuesta_da_el_mismo_digest() {
        assert_eq!(p().digest(), p().digest());
        assert!(p().digest().starts_with("sha256:"));
    }

    /// **Y cambiar la vista lo cambia**, que es la segunda y la que importa:
    /// es lo que hace auditable *por dónde se iba a entrar*.
    ///
    /// Lo único que se toca entre las dos es la quinta identidad. Si el digest
    /// no cambiara, la vista estaría viajando fuera del sello y dos propuestas
    /// que entran por sitios distintos serían indistinguibles.
    #[test]
    fn cambiar_la_vista_cambia_el_digest() {
        let a = p();
        let mut b = p();
        b.bajo.vista = "sha256:otra".to_string();
        assert_ne!(a.digest(), b.digest());

        // Y las otras cuatro, por lo mismo.
        for cambiar in [
            (|x: &mut Propuesta| x.bajo.bundle = "sha256:otro".into()) as fn(&mut Propuesta),
            |x: &mut Propuesta| x.bajo.plan = "sha256:otro".into(),
            |x: &mut Propuesta| x.bajo.topologia = "2".into(),
            |x: &mut Propuesta| {
                x.bajo.testigos.insert("erp.employees".into(), "43".into());
            },
        ] {
            let mut c = p();
            cambiar(&mut c);
            assert_ne!(a.digest(), c.digest(), "una identidad fuera del sello");
        }
    }

    /// Ida y vuelta por el protocolo: lo que se escribe es lo que se lee.
    #[test]
    fn una_propuesta_sobrevive_al_viaje_por_el_protocolo() {
        let texto = p().canonica();
        let leida = Propuesta::abrir(&parse(&texto).expect("json")).expect("abrir");
        assert_eq!(leida, p());
        assert_eq!(leida.digest(), p().digest());
    }

    #[test]
    fn el_orden_en_que_se_escriben_las_marcas_no_cambia_el_digest() {
        let mut a = p();
        a.bajo.testigos.insert("z.tabla".into(), "1".into());
        a.bajo.testigos.insert("a.tabla".into(), "2".into());
        let mut b = p();
        b.bajo.testigos.insert("a.tabla".into(), "2".into());
        b.bajo.testigos.insert("z.tabla".into(), "1".into());
        assert_eq!(a.digest(), b.digest());
    }
}
