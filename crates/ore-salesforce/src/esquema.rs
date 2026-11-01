//! **El esquema de un objeto**, de `GET /sobjects/<X>/describe` (X0 A2): cada
//! campo con lo que se puede hacer con él. A diferencia de MongoDB, aquí **sí
//! hay catálogo que preguntar**, y el propio `describe` dice si un campo se
//! filtra (`filterable`), se ordena (`sortable`) o se agrupa (`groupable`):
//! el traductor no adivina, consulta.
//!
//! # Columnas planas
//!
//! Las columnas de ORE son planas (`docs/products/0066`, «las columnas son
//! planas»). Por eso:
//!
//! - un **compuesto** (`address`, `location`: `BillingAddress`) no es una
//!   columna; **sus subcampos sí** (`BillingCity`), como en Fivetran (X0 B6);
//! - un `base64` (`ContentVersion.VersionData`) no se lee por SOQL en filas (C4);
//! - ⚠ **`Account.Name` dice `compoundFieldName: "Name"`, apuntando a sí
//!   mismo** (A3). «Tiene `compoundFieldName`» NO significa «es un subcampo»:
//!   se trata de compuesto solo lo que **es** de tipo `address` o `location`.

use ore_driver::catalogo::Columna;
use serde_json::Value;

/// Cómo se escribe y se compara el valor de un campo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Categoria {
    /// `string`, `picklist`, `textarea`, `email`, `phone`, `url`, `combobox`:
    /// SOQL **no distingue mayúsculas** (X0 B2).
    Texto,
    Numero,
    Logico,
    Fecha,
    FechaHora,
    /// `id`, `reference`.
    Id,
    /// Lo que no se sabe comparar (`multipicklist`, `time`, `anyType`…): se
    /// puede leer, no filtrar.
    Ninguna,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Campo {
    pub nombre: String,
    /// El `type` de `describe`, tal cual.
    pub tipo: String,
    pub filtrable: bool,
    pub ordenable: bool,
    pub nulable: bool,
    /// Una fórmula: su valor **no mueve `SystemModstamp`** cuando cambia lo
    /// que la alimenta (§2), así que no es incremental.
    pub calculado: bool,
    pub compuesto_de: Option<String>,
    pub precision: u32,
    pub escala: u32,
    pub valores: Vec<String>,
    pub etiqueta: String,
}

impl Campo {
    pub fn categoria(&self) -> Categoria {
        match self.tipo.as_str() {
            "string" | "picklist" | "textarea" | "email" | "phone" | "url" | "combobox" => {
                Categoria::Texto
            }
            "int" | "long" | "double" | "currency" | "percent" => Categoria::Numero,
            "boolean" => Categoria::Logico,
            "date" => Categoria::Fecha,
            "datetime" => Categoria::FechaHora,
            "id" | "reference" => Categoria::Id,
            _ => Categoria::Ninguna,
        }
    }

    /// Un compuesto: se lee por sus subcampos.
    pub fn es_compuesto(&self) -> bool {
        matches!(self.tipo.as_str(), "address" | "location")
    }

    /// Lo que no sale por SOQL en filas: el compuesto, el binario, y lo que
    /// Salesforce no entrega en una consulta normal.
    pub fn seleccionable(&self) -> bool {
        !self.es_compuesto()
            && !matches!(
                self.tipo.as_str(),
                "base64" | "complexvalue" | "datacategorygroupreference" | "encryptedstring"
            )
    }

    /// El tipo de OOS, o `None` si no se sabe traducir (`time`, `anyType`…):
    /// no es un hueco a rellenar, es la conjetura que esta pieza no toma.
    pub fn oos(&self) -> Option<String> {
        Some(match self.tipo.as_str() {
            "int" | "long" => "Integer".to_string(),
            "double" | "currency" | "percent" => {
                if self.precision > 0 {
                    format!("Decimal<{}, {}>", self.precision, self.escala)
                } else {
                    "Float".to_string()
                }
            }
            "boolean" => "Boolean".to_string(),
            "date" => "Date".to_string(),
            // Un instante de Salesforce es UTC.
            "datetime" => "DateTimeTz".to_string(),
            "string" | "picklist" | "multipicklist" | "textarea" | "email" | "phone" | "url"
            | "combobox" | "id" | "reference" => "String".to_string(),
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Objeto {
    pub nombre: String,
    pub consultable: bool,
    pub replicable: bool,
    pub propio: bool,
    pub campos: Vec<Campo>,
}

impl Objeto {
    pub fn de_describe(texto: &str) -> Result<Objeto, String> {
        let v: Value =
            serde_json::from_str(texto).map_err(|e| format!("el `describe` no es JSON: {e}"))?;
        let nombre = v["name"]
            .as_str()
            .ok_or("el `describe` no trae `name`")?
            .to_string();
        let campos = v["fields"]
            .as_array()
            .ok_or_else(|| format!("el `describe` de `{nombre}` no trae `fields`"))?
            .iter()
            .map(|f| Campo {
                nombre: f["name"].as_str().unwrap_or_default().to_string(),
                tipo: f["type"].as_str().unwrap_or_default().to_string(),
                filtrable: f["filterable"].as_bool().unwrap_or(false),
                ordenable: f["sortable"].as_bool().unwrap_or(false),
                nulable: f["nillable"].as_bool().unwrap_or(true),
                calculado: f["calculated"].as_bool().unwrap_or(false),
                compuesto_de: f["compoundFieldName"].as_str().map(str::to_string),
                precision: f["precision"].as_u64().unwrap_or(0) as u32,
                escala: f["scale"].as_u64().unwrap_or(0) as u32,
                valores: f["picklistValues"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter(|p| p["active"].as_bool().unwrap_or(true))
                            .filter_map(|p| p["value"].as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
                etiqueta: f["label"].as_str().unwrap_or_default().to_string(),
            })
            .collect();
        Ok(Objeto {
            nombre,
            consultable: v["queryable"].as_bool().unwrap_or(false),
            replicable: v["replicateable"].as_bool().unwrap_or(false),
            propio: v["custom"].as_bool().unwrap_or(false),
            campos,
        })
    }

    pub fn campo(&self, nombre: &str) -> Option<&Campo> {
        self.campos.iter().find(|c| c.nombre == nombre)
    }

    /// Las columnas planas: lo que se puede leer.
    pub fn columnas(&self) -> impl Iterator<Item = &Campo> {
        self.campos.iter().filter(|c| c.seleccionable())
    }

    /// El cursor de los cambios (§1.8): `SystemModstamp` si el objeto lo
    /// tiene; si no, `LastModifiedDate`. Un objeto sin ninguno **se reimporta**.
    pub fn cursor(&self) -> Option<&str> {
        ["SystemModstamp", "LastModifiedDate"]
            .into_iter()
            .find(|n| self.campo(n).is_some_and(|c| c.filtrable && c.ordenable))
    }

    /// Lo que `catalogo` cuenta de cada columna.
    pub fn columna_del_catalogo(c: &Campo) -> Columna {
        Columna {
            nombre: c.nombre.clone(),
            tipo: c.oos(),
            origen: Some(c.tipo.clone()),
            obligatoria: !c.nulable,
            valores: c.valores.clone(),
            descripcion: if c.calculado {
                Some(format!(
                    "{} (fórmula: no mueve SystemModstamp cuando cambia lo que la alimenta)",
                    c.etiqueta
                ))
            } else if c.etiqueta.is_empty() {
                None
            } else {
                Some(c.etiqueta.clone())
            },
        }
    }
}

#[cfg(test)]
pub(crate) mod pruebas {
    use super::*;

    /// Un `describe` de `Account` recortado, con la forma que dio la org en X0.
    pub const ACCOUNT: &str = r#"{
      "name": "Account", "queryable": true, "replicateable": true, "custom": false,
      "fields": [
        {"name":"Id","type":"id","filterable":true,"sortable":true,"nillable":false,"label":"Account ID"},
        {"name":"IsDeleted","type":"boolean","filterable":true,"sortable":true,"nillable":false,"label":"Deleted"},
        {"name":"Name","type":"string","filterable":true,"sortable":true,"nillable":false,"label":"Account Name","compoundFieldName":"Name"},
        {"name":"Industry","type":"picklist","filterable":true,"sortable":true,"nillable":true,"label":"Industry",
         "picklistValues":[{"value":"Banking","active":true},{"value":"Energy","active":true},{"value":"Viejo","active":false}]},
        {"name":"NumberOfEmployees","type":"int","filterable":true,"sortable":true,"nillable":true,"label":"Employees"},
        {"name":"AnnualRevenue","type":"currency","filterable":true,"sortable":true,"nillable":true,"precision":18,"scale":0,"label":"Revenue"},
        {"name":"Description","type":"textarea","filterable":false,"sortable":false,"nillable":true,"label":"Description"},
        {"name":"BillingAddress","type":"address","filterable":false,"sortable":false,"nillable":true,"label":"Billing Address"},
        {"name":"BillingCity","type":"string","filterable":true,"sortable":true,"nillable":true,"label":"Billing City","compoundFieldName":"BillingAddress"},
        {"name":"CreatedDate","type":"datetime","filterable":true,"sortable":true,"nillable":false,"label":"Created Date"},
        {"name":"SystemModstamp","type":"datetime","filterable":true,"sortable":true,"nillable":false,"label":"System Modstamp"},
        {"name":"X0_Formula__c","type":"double","filterable":true,"sortable":true,"nillable":true,"calculated":true,"precision":18,"scale":0,"label":"X0 Formula"}
      ]}"#;

    pub fn cuenta() -> Objeto {
        Objeto::de_describe(ACCOUNT).unwrap()
    }

    #[test]
    fn el_compuesto_no_es_columna_y_sus_subcampos_si() {
        let o = cuenta();
        let cols: Vec<_> = o.columnas().map(|c| c.nombre.as_str()).collect();
        assert!(!cols.contains(&"BillingAddress"), "{cols:?}");
        assert!(cols.contains(&"BillingCity"), "{cols:?}");
    }

    /// X0 A3: `Account.Name` apunta a sí mismo y NO es un subcampo.
    #[test]
    fn name_dice_compound_de_si_mismo_y_sigue_siendo_una_columna() {
        let o = cuenta();
        let n = o.campo("Name").unwrap();
        assert_eq!(n.compuesto_de.as_deref(), Some("Name"));
        assert!(!n.es_compuesto() && n.seleccionable());
        assert!(o.columnas().any(|c| c.nombre == "Name"));
    }

    #[test]
    fn el_texto_largo_se_lee_pero_no_se_filtra() {
        let d = cuenta().campo("Description").unwrap().clone();
        assert!(d.seleccionable() && !d.filtrable && !d.ordenable);
    }

    #[test]
    fn los_tipos_de_oos_y_lo_que_no_se_traduce() {
        let o = cuenta();
        let t = |n: &str| o.campo(n).unwrap().oos();
        assert_eq!(t("Id").as_deref(), Some("String"));
        assert_eq!(t("NumberOfEmployees").as_deref(), Some("Integer"));
        assert_eq!(t("AnnualRevenue").as_deref(), Some("Decimal<18, 0>"));
        assert_eq!(t("IsDeleted").as_deref(), Some("Boolean"));
        assert_eq!(t("SystemModstamp").as_deref(), Some("DateTimeTz"));
        let hora = Campo {
            tipo: "time".into(),
            ..o.campo("Name").unwrap().clone()
        };
        assert_eq!(hora.oos(), None, "`time` no se traduce: no se conjetura");
    }

    #[test]
    fn la_picklist_trae_sus_valores_activos_en_orden() {
        assert_eq!(
            cuenta().campo("Industry").unwrap().valores,
            ["Banking", "Energy"]
        );
    }

    #[test]
    fn el_cursor_es_system_modstamp() {
        assert_eq!(cuenta().cursor(), Some("SystemModstamp"));
    }

    #[test]
    fn la_formula_se_dice_en_la_descripcion() {
        let o = cuenta();
        let c = Objeto::columna_del_catalogo(o.campo("X0_Formula__c").unwrap());
        assert!(c.descripcion.unwrap().contains("fórmula"));
    }

    #[test]
    fn un_describe_roto_se_dice() {
        assert!(
            Objeto::de_describe("no es json")
                .unwrap_err()
                .contains("no es JSON")
        );
        assert!(Objeto::de_describe("{}").unwrap_err().contains("`name`"));
    }
}
