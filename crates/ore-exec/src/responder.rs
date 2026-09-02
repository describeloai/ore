//! La fase ④ y la respuesta: **ensamblar sobre flujos ya reducidos**.
//!
//! Ya reducidos, y esa es toda la diferencia. Cada lectura vino con su
//! proyección y sus claves, así que ④ junta lo poco que llegó en vez de filtrar
//! lo mucho que podría haber llegado — que es la lectura amplificada de §2 con
//! otro nombre.
//!
//! # Los ejes que la respuesta lleva
//!
//! | | Qué contesta |
//! |---|---|
//! | **digest** | qué significaba |
//! | **marca de agua** | hasta cuándo era cierto |
//! | **instante** | cuándo se autorizó |
//!
//! Los dos primeros los fijó `05-ejecutor` §7; el tercero salió del
//! [ADR 0007](../../docs/decisions/0007-enlazar-el-evaluador-de-cedar.md): Cedar
//! tiene extensión `datetime`, así que una política puede ser **función del
//! tiempo**, y sin ese eje *«la misma pregunta devolvió cosas distintas»* no se
//! distingue de un fallo.
//!
//! # El motor no lee el reloj
//!
//! **El instante llega con la petición**, igual que los atributos del principal.
//! No es escrúpulo: es lo que hace que una respuesta sea reproducible a partir de
//! sus entradas, y lo que impide que el estado degradado dependa de cuándo se
//! ejecute la prueba.

use crate::motor::Motor;
use crate::plan::{Lectura, Plan};
use ore_core::frescura::{duracion, epoca};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct Respuesta {
    /// Las filas ya ensambladas, con **propiedades** por clave.
    pub filas: Vec<BTreeMap<String, String>>,
    /// **Qué significaba.**
    pub digest: String,
    /// **Hasta cuándo era cierto**, y es la **más vieja** de las que
    /// intervinieron. Ausente si no intervino nada materializado.
    pub marca: Option<String>,
    /// **Cuándo se autorizó.**
    pub instante: Option<String>,
    /// Por qué la respuesta está degradada, si lo está. `05-ejecutor` §7 obliga
    /// a declararlo: un dato viejo con aspecto de fresco es peor que un error.
    pub degradado: Option<String>,
    /// Qué hay que aplicar a cada propiedad — las obligaciones que el veredicto
    /// trajo. El ejecutor las transporta; aplicarlas con sujeto es otra pieza.
    pub obligaciones: BTreeMap<String, Vec<String>>,
    /// **De dónde salió cada lectura**, y si fue del origen, por qué no de la
    /// caché. Una respuesta que no lo distingue no se puede auditar, y *«¿esto
    /// vino del lago o del sistema de gestión?»* es la primera pregunta de
    /// cualquiera que revise un número raro.
    pub origenes: BTreeMap<String, String>,
}

// `epoca` y `duracion` se movieron a `ore_core::frescura` cuando el veredicto
// de la cache empezo a necesitarlas: dos implementaciones de «esto esta
// rancio» que se contesten distinto es la clase de divergencia que no tiene
// aspecto de fallo, porque las dos devuelven un dato.

impl Motor {
    /// `(tipo, url)` de una fuente declarada. La credencial sale de la variable
    /// que el manifiesto nombra — **nunca del documento**, que por eso es
    /// publicable.
    fn fuente(&self, nombre: &str) -> Result<(String, String), String> {
        let cfg = self
            .paquete
            .docs
            .iter()
            .find(|d| d.kind == ore_core::document::Kind::OntologyConfig)
            .ok_or("no hay manifiesto raíz")?;
        let ds = cfg
            .section("datasources")
            .ok_or("el manifiesto no declara ninguna fuente")?
            .items()
            .iter()
            .find(|d| d.get("name").and_then(|(_, v)| v.as_str()) == Some(nombre))
            .ok_or_else(|| format!("`{nombre}` no está declarada en el manifiesto"))?;
        let tipo = ds
            .get("type")
            .and_then(|(_, v)| v.as_str())
            .ok_or_else(|| format!("`{nombre}` no declara `type`"))?;
        let var = ds
            .get("connectionEnv")
            .and_then(|(_, v)| v.as_str())
            .ok_or_else(|| format!("`{nombre}` no declara `connectionEnv`"))?;
        let url = std::env::var(var)
            .map_err(|_| format!("la variable `{var}` de `{nombre}` no está puesta"))?;
        Ok((tipo.to_string(), url))
    }

    fn leer_una(&self, l: &Lectura) -> Result<Vec<BTreeMap<String, String>>, String> {
        let (tipo, url) = self.fuente(&l.datasource)?;
        let salida = std::process::Command::new(format!("ore-read-{tipo}"))
            .args(["leer", &l.datasource])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut h| {
                use std::io::Write as _;
                h.stdin
                    .as_mut()
                    .expect("stdin")
                    .write_all(l.peticion(&url).as_bytes())?;
                h.wait_with_output()
            })
            .map_err(|e| format!("no se pudo invocar `ore-read-{tipo}`: {e}"))?;
        if !salida.status.success() {
            return Err(String::from_utf8_lossy(&salida.stderr).trim().to_string());
        }
        Ok(String::from_utf8_lossy(&salida.stdout)
            .lines()
            .filter(|x| !x.trim().is_empty())
            .filter_map(|x| ore_core::parse::parse(x).ok())
            .map(|n| {
                n.entries()
                    .iter()
                    .filter_map(|(k, v)| Some((k.as_str()?.to_string(), v.as_str()?.to_string())))
                    .collect()
            })
            .collect())
    }

    /// Ejecuta un plan y devuelve la respuesta ensamblada.
    ///
    /// `instante` es **cuándo se pregunta**, y llega de fuera: el motor no lee el
    /// reloj. `sla` es el `freshnessSLA` que aplique; si se supera, la respuesta
    /// sale **degradada y diciéndolo**.
    pub fn responder(
        &self,
        p: &Plan,
        instante: Option<&str>,
        sla: Option<&str>,
    ) -> Result<Respuesta, String> {
        // ④ · ensamblar por la clave, sobre lo que ya vino reducido.
        let mut por_clave: BTreeMap<Vec<String>, BTreeMap<String, String>> = BTreeMap::new();
        let mut sueltas: Vec<BTreeMap<String, String>> = Vec::new();

        for l in &p.lecturas {
            for fila in self.leer_una(l)? {
                if p.ensamblar_por.is_empty() {
                    sueltas.push(fila);
                    continue;
                }
                let clave: Vec<String> = p
                    .ensamblar_por
                    .iter()
                    .map(|k| fila.get(k).cloned().unwrap_or_default())
                    .collect();
                // Una fila sin clave no se puede ensamblar, y meterla igual
                // juntaría cosas que no son la misma.
                if clave.iter().any(String::is_empty) {
                    continue;
                }
                por_clave.entry(clave).or_default().extend(fila);
            }
        }
        let filas: Vec<BTreeMap<String, String>> = if p.ensamblar_por.is_empty() {
            sueltas
        } else {
            por_clave.into_values().collect()
        };

        // **La respuesta es tan fresca como su parte más rancia.**
        //
        // Dejó de ser la marca del índice el día que la caché entró en el plan:
        // ahora interviene más de una cosa materializada. Componerla de otra
        // forma —la más nueva, o la del índice a secas— declararía una frescura
        // que ninguna de las partes tiene, que es justo la mentira que este
        // proyecto existe para acotar.
        let mut marcas: Vec<String> = self.topologia.iter().map(|t| t.marca.clone()).collect();
        marcas.extend(p.lecturas.iter().filter_map(|l| match &l.origen {
            crate::plan::Origen::Cache { marca } => Some(marca.clone()),
            crate::plan::Origen::Fuente { .. } => None,
        }));
        // Una marca que no se sabe fechar ordena PRIMERO, es decir, gana como la
        // más vieja. Es la dirección conservadora: esconderla detrás de una
        // fresca sería afirmar una frescura que nadie ha comprobado.
        let marca = mas_vieja(marcas);
        let degradado = match (&marca, instante, sla) {
            (Some(m), Some(ahora), Some(sla)) => match (epoca(m), epoca(ahora), duracion(sla)) {
                (Some(m0), Some(t0), Some(d)) if t0 > m0 + d => Some(format!(
                    "la marca de agua es `{m}` y el `freshnessSLA` es `{sla}`: lo \
                         materializado lleva {} s de retraso",
                    t0 - m0
                )),
                _ => None,
            },
            _ => None,
        };

        Ok(Respuesta {
            filas,
            digest: ore_core::digest::bundle(&self.paquete),
            marca,
            instante: instante.map(String::from),
            degradado,
            obligaciones: p.autorizadas.clone(),
            origenes: p
                .lecturas
                .iter()
                .map(|l| {
                    (
                        format!("{}·{}", l.datasource, l.objeto),
                        match &l.origen {
                            crate::plan::Origen::Cache { marca } => format!("cache ({marca})"),
                            crate::plan::Origen::Fuente { porque: Some(x) } => {
                                format!("fuente — {x}")
                            }
                            crate::plan::Origen::Fuente { porque: None } => "fuente".into(),
                        },
                    )
                })
                .collect(),
        })
    }
}

/// **La respuesta es tan fresca como su parte más rancia.**
///
/// Es una función aparte porque es una regla, no un detalle: componerla de otra
/// forma —la más nueva, o la del índice a secas— declararía una frescura que
/// ninguna de las partes tiene.
///
/// Una marca que no se sabe fechar ordena **primero**, es decir, gana como la
/// más vieja. Es la dirección conservadora: esconderla detrás de una fresca
/// sería afirmar una frescura que nadie ha comprobado.
pub fn mas_vieja(mut marcas: Vec<String>) -> Option<String> {
    marcas.sort_by_key(|m| epoca(m).unwrap_or(i64::MIN));
    marcas.into_iter().next()
}

impl Motor {
    /// `(connectionEnv, refreshEnv)` de una fuente.
    ///
    /// Dos variables porque son **dos identidades**: el que refresca necesita
    /// lectura amplia y programada; el que responde, lectura por clave y por
    /// petición. `05-ejecutor` §6.2 lo exige, y hasta que hubo dónde declararlo
    /// la exigencia no era comprobable.
    pub fn variables_de(&self, nombre: &str) -> Result<(String, Option<String>), String> {
        let cfg = self
            .paquete
            .docs
            .iter()
            .find(|d| d.kind == ore_core::document::Kind::OntologyConfig)
            .ok_or("no hay manifiesto raíz")?;
        let ds = cfg
            .section("datasources")
            .ok_or("el manifiesto no declara ninguna fuente")?
            .items()
            .iter()
            .find(|d| d.get("name").and_then(|(_, v)| v.as_str()) == Some(nombre))
            .ok_or_else(|| format!("`{nombre}` no está declarada en el manifiesto"))?;
        Ok((
            ds.get("connectionEnv")
                .and_then(|(_, v)| v.as_str())
                .ok_or_else(|| format!("`{nombre}` no declara `connectionEnv`"))?
                .to_string(),
            ds.get("refreshEnv")
                .and_then(|(_, v)| v.as_str())
                .map(String::from),
        ))
    }

    /// La columna física de la propiedad que ordena el avance del refresco.
    ///
    /// Sin ella el refresco **no sabe desde dónde continuar** y solo puede
    /// recargar entero — que es exactamente lo que `05-ejecutor` §7 dice.
    ///
    /// # Dos sitios, y uno de ellos es donde siempre debió estar
    ///
    /// En v1alpha1 sale de `materialization.topology.watermark` del binding,
    /// pasada por su mapeo: una PROPIEDAD, traducida a columna.
    ///
    /// En v1alpha8 sale de `changes.field` de la tabla, y llega **ya en columna
    /// física**. No es un traslado cosmético: qué columna ordena el avance es un
    /// hecho del objeto —lo sabe quien lo sondeó, no quien lo consulta— y por
    /// eso vive en la cara `D`. El binding lo tenía dentro de una decisión de
    /// materialización, que es de la consulta; la tabla lo tiene donde ocurre.
    ///
    /// Y es el primer consumidor real de `witness: field`: hasta aquí el testigo
    /// se declaraba y nadie lo leía.
    pub fn columna_de_marca(&self, relacion: &str) -> Option<String> {
        let entidad = relacion.rsplit_once('.')?.0;

        // v1alpha8 · la cara `D` de la raíz. Va primero porque es la forma
        // vigente; si el paquete no la usa, se cae al camino del binding.
        if let Some(e) = self.paquete.entity(entidad)
            && let Some(v) = ore_core::vistas::respaldo(&self.paquete, e)
            && let Ok(raiz) = ore_core::vistas::raiz(&self.paquete, v)
            && let Some(qn) = raiz.tabla.as_deref()
            && let Some(tabla) = self.paquete.table(qn)
            && let Some(cambios) = tabla.section("changes")
            && cambios.get("witness").and_then(|(_, w)| w.as_str()) == Some("field")
            && let Some((_, campo)) = cambios.get("field")
        {
            return campo.as_str().map(String::from);
        }

        for b in self.paquete.docs.iter().filter(|d| {
            d.kind == ore_core::document::Kind::Binding
                && d.section("targetEntity").and_then(|t| t.as_str()) == Some(entidad)
        }) {
            let prop = b
                .section("materialization")
                .and_then(|m| m.get("topology").map(|(_, v)| v))
                .and_then(|t| t.get("watermark").map(|(_, v)| v))
                .and_then(|w| w.as_str())?;
            if let Some((_, c)) = b.section("properties").and_then(|p| p.get(prop)) {
                return c.as_str().map(String::from).or_else(|| {
                    c.get("column")
                        .and_then(|(_, x)| x.as_str())
                        .map(String::from)
                });
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::mas_vieja;

    fn v(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| (*s).to_string()).collect()
    }

    /// El criterio, y la prueba falla si se coge la otra.
    #[test]
    fn la_marca_de_la_respuesta_es_la_mas_vieja_de_las_que_intervinieron() {
        assert_eq!(
            mas_vieja(v(&["2026-09-01T10:00:00Z", "2026-08-15T04:00:00Z"])),
            Some("2026-08-15T04:00:00Z".to_string())
        );
        // Y no depende del orden de llegada: si dependiera, la frescura
        // declarada seria un accidente de como se recorrieron las lecturas.
        assert_eq!(
            mas_vieja(v(&["2026-08-15T04:00:00Z", "2026-09-01T10:00:00Z"])),
            Some("2026-08-15T04:00:00Z".to_string())
        );
    }

    /// Una marca que no se sabe fechar gana como la mas vieja. La direccion
    /// importa: al reves, un dato sin fechar se serviria con la frescura de
    /// otro.
    #[test]
    fn lo_que_no_se_sabe_fechar_no_se_esconde_detras_de_lo_fresco() {
        assert_eq!(
            mas_vieja(v(&["2026-09-01T10:00:00Z", "snapshot-91827"])),
            Some("snapshot-91827".to_string())
        );
    }

    /// Sin nada materializado no hay marca, y eso NO es lo mismo que una marca
    /// vacia: una respuesta que no lee nada materializado no tiene de que
    /// declarar frescura.
    #[test]
    fn sin_nada_materializado_no_hay_marca() {
        assert_eq!(mas_vieja(Vec::new()), None);
    }
}
