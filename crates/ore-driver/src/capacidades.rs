//! **Lo que un conector sabe hacer** (`docs/federation.md` §1.4): el verbo
//! `capacidades` y la comprobación que va con él.
//!
//! Lo que se empuja a un origen es la intersección de esto con el `reads` de la
//! tabla (v1alpha24 §3), y quien la calcula es el coordinador. Pero un conector
//! no se fía: si le llega lo que no declaró, **se niega** con `operador` en vez
//! de servir de más ([`Capacidades::admite`]). Es la regla de 0008 —un operador
//! que no se sabe expresar no se ignora— extendida a `limit` y `orderBy`.

use crate::fallo::Fallo;
use crate::{OPERADORES, Peticion};
use ore_core::json::Json;

/// El protocolo de la petición que esta biblioteca habla.
pub const PROTOCOLO: u32 = 2;

/// Lo que un conector declara.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capacidades {
    /// `ore-read-<tipo>`.
    pub conector: &'static str,
    pub version: &'static str,
    /// Un subconjunto de [`OPERADORES`].
    pub operadores: &'static [&'static str],
    pub limit: bool,
    pub order_by: bool,
    pub estimar: bool,
    pub servir: bool,
}

impl Capacidades {
    /// La respuesta del verbo `capacidades`, en una línea. `agregados` y
    /// `juntas` salen siempre, y en falso: son de F9.
    pub fn json(&self) -> String {
        self.objeto(None)
    }

    /// Lo mismo, con **lo que sabe un origen de objetos** (ADR 0061, decisión
    /// 3) en `objetos`: cómo fija una lectura, si firma, qué huella da, si su
    /// credencial es corta. `objetos` es su JSON canónico. Quien lee
    /// `capacidades` ignora lo que no conoce: la pasarela y el kit siguen igual.
    pub fn json_con_objetos(&self, objetos: &str) -> String {
        self.objeto(Some(objetos))
    }

    fn objeto(&self, objetos: Option<&str>) -> String {
        debug_assert!(
            self.operadores.iter().all(|o| OPERADORES.contains(o)),
            "`{}` declara un operador que la petición no sabe llevar",
            self.conector
        );
        Json::obj(
            [
                ("conector", Json::s(self.conector)),
                ("version", Json::s(self.version)),
                ("protocolo", Json::Int(PROTOCOLO as i64)),
                (
                    "operadores",
                    Json::Arr(self.operadores.iter().map(|o| Json::s(*o)).collect()),
                ),
                ("limit", Json::Bool(self.limit)),
                ("orderBy", Json::Bool(self.order_by)),
                ("estimar", Json::Bool(self.estimar)),
                ("servir", Json::Bool(self.servir)),
                ("agregados", Json::Bool(false)),
                ("juntas", Json::Bool(false)),
            ]
            .into_iter()
            .chain(objetos.map(|o| ("objetos", Json::Crudo(o.to_string())))),
        )
        .jcs()
    }

    /// **Si esta petición se puede servir tal cual**, o el `operador` que lo
    /// impide. Lo que el conector no declaró no se ignora: ignorar un filtro o
    /// un `limit` devolvería otras filas de las pedidas sin fallar, e ignorar
    /// un `orderBy` con `limit`, otras n.
    pub fn admite(&self, p: &Peticion) -> Result<(), Fallo> {
        for f in &p.filtros {
            if !self.operadores.contains(&f.operador.as_str()) {
                return Err(Fallo::operador(format!(
                    "`{}` sobre `{}`: {} no lo sabe poner (sabe {})",
                    f.operador,
                    f.columna,
                    self.conector,
                    self.operadores.join(", ")
                )));
            }
        }
        if p.limit.is_some() && !self.limit {
            return Err(Fallo::operador(format!(
                "`limit`: {} no sabe parar a las n filas",
                self.conector
            )));
        }
        if !p.orden.is_empty() && !self.order_by {
            return Err(Fallo::operador(format!(
                "`orderBy`: {} no sabe ordenar",
                self.conector
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Filtro, Orden};

    const S3_V1: Capacidades = Capacidades {
        conector: "ore-read-s3",
        version: "1",
        operadores: &["eq"],
        limit: false,
        order_by: false,
        estimar: false,
        servir: false,
    };

    #[test]
    fn la_declaracion_es_una_linea_canonica() {
        assert_eq!(
            S3_V1.json(),
            r#"{"agregados":false,"conector":"ore-read-s3","estimar":false,"juntas":false,"limit":false,"operadores":["eq"],"orderBy":false,"protocolo":2,"servir":false,"version":"1"}"#
        );
    }

    /// ADR 0061: lo de un origen de objetos va en `objetos`, en su sitio de JCS.
    #[test]
    fn lo_de_un_origen_de_objetos_va_en_su_sitio() {
        assert_eq!(
            S3_V1.json_con_objetos(r#"{"fija":"version"}"#),
            r#"{"agregados":false,"conector":"ore-read-s3","estimar":false,"juntas":false,"limit":false,"objetos":{"fija":"version"},"operadores":["eq"],"orderBy":false,"protocolo":2,"servir":false,"version":"1"}"#
        );
    }

    /// **Lo que no se declaró se rechaza**, filtro a filtro, y también
    /// `limit` y `orderBy`.
    #[test]
    fn lo_no_declarado_se_rechaza_con_operador() {
        let mut p = Peticion {
            filtros: vec![Filtro::uno("pais", "eq", "ES")],
            ..Default::default()
        };
        assert_eq!(S3_V1.admite(&p), Ok(()));

        p.filtros.push(Filtro::uno("alta", "ge", "2026-01-01"));
        let e = S3_V1.admite(&p).expect_err("ge");
        assert_eq!(e.codigo, crate::Codigo::Operador);
        assert!(e.mensaje.contains("`ge` sobre `alta`"), "{e}");

        p.filtros.pop();
        p.limit = Some(10);
        assert!(
            S3_V1
                .admite(&p)
                .expect_err("limit")
                .mensaje
                .contains("`limit`")
        );

        p.limit = None;
        p.orden = vec![Orden {
            columna: "a".into(),
            descendente: false,
        }];
        assert!(
            S3_V1
                .admite(&p)
                .expect_err("orden")
                .mensaje
                .contains("`orderBy`")
        );
    }
}
