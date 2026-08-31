//! Lectura estructural de políticas Cedar.
//!
//! # Qué es y qué no es
//!
//! **No es un intérprete de Cedar, y no debe serlo.** Cedar tiene su propio
//! motor, su propia semántica de autorización y su propia implementación de
//! referencia; reimplementarla sería exactamente el error que OOS evita al no
//! definir un lenguaje de políticas propio.
//!
//! Lo que hace falta aquí es más pequeño y muy distinto: comparar **dos
//! versiones del mismo conjunto de políticas** para decir si la segunda concede
//! más que la primera. Eso no exige evaluar nada — exige leer la forma: qué
//! políticas hay, con qué efecto, con qué condiciones y con qué obligaciones.
//!
//! La distinción marca el límite: si algún día `ore` necesitara *decidir* una
//! autorización, la respuesta sería enlazar `cedar-policy`, no ampliar este
//! fichero.
//!
//! Registro: `docs/decisions/0003-lectura-estructural-de-cedar.md`

use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Permit,
    Forbid,
}

#[derive(Debug, Clone)]
pub struct Policy {
    /// De `@id("…")`. Es la identidad de la política a través de versiones: sin
    /// ella, mover una política de línea parecería borrarla y crear otra.
    pub id: String,
    pub effect: Effect,
    /// De `@obligation("…")`, en orden de aparición.
    pub obligations: Vec<String>,
    /// Las conjunciones de `when { … }`, normalizadas en espacios. Quitar una
    /// es ampliar el acceso.
    pub conditions: Vec<String>,
    /// Finalidades admitidas, leídas de `context.purpose`.
    pub purposes: BTreeSet<String>,
    /// De `@oosMask("<ruleset cualificado>#<id>")`. La anotación **nombra** una
    /// máscara declarada en un `Ruleset`; no la declara. Es lo que mantiene la
    /// definición en un solo sitio, con dueño y con descenso verificado.
    pub masks: Vec<String>,
    /// De `@oosScope("<ruleset cualificado>#<id>")`. Igual que `masks`, y por la
    /// misma razon: la anotacion **nombra** un ambito de fila declarado en un
    /// `Ruleset`. Una mascara recorta el valor; un ambito recorta la fila.
    pub scopes: Vec<String>,
    /// Las etiquetas que la política menciona — `Label::"gdpr.sensitivity:high"`
    /// en el ámbito o en las condiciones.
    ///
    /// Es lo que permite responder *«¿hay una política sobre esta propiedad?»*
    /// sin evaluar nada: la proyección a esquema Cedar convierte cada nivel en
    /// un tipo de entidad, así que mencionarlo **es** apuntar a la clasificación.
    pub labels: BTreeSet<String>,
    /// Los roles que la politica menciona — `Role::"hr_analyst"`.
    ///
    /// Se leen por la misma razon que las etiquetas: para poder responder si la
    /// politica **puede casar con algo**. Un rol no se declara en ninguna parte
    /// —son cadenas que trae la capa de identidad— asi que lo comprobable no es
    /// CUAL, sino **si pueden llegar roles**: sin `subject.roles` declarado, la
    /// politica no casa nunca.
    pub roles: BTreeSet<String>,
    /// Las propiedades que la politica nombra DIRECTAMENTE —
    /// `resource == Property::"hr.Employee.nationalId"`.
    ///
    /// Faltaban, y se vio al usar `alcance()` para explicar una denegacion: el
    /// `forbid` sobre el DNI del ejemplo salia como *«no alcanza ninguna
    /// propiedad todavia»* siendo la politica mas contundente del fichero. La
    /// lectura miraba solo las etiquetas, asi que la enumeracion —que es la
    /// otra mitad de la proyeccion— no contaba.
    pub properties: BTreeSet<String>,
}

impl Policy {
    /// El umbral de `aggregate:minGroupSize=N`, si lo declara.
    pub fn min_group_size(&self) -> Option<u32> {
        self.obligations.iter().find_map(|o| {
            o.strip_prefix("aggregate:")?
                .split_once('=')
                .filter(|(k, _)| k.trim() == "minGroupSize")
                .and_then(|(_, v)| v.trim().parse().ok())
        })
    }

    /// Desclasificadores declarados: el nombre, sin sus parámetros.
    pub fn declassifiers(&self) -> BTreeSet<String> {
        self.obligations
            .iter()
            .map(|o| o.split(':').next().unwrap_or(o).trim().to_string())
            .collect()
    }
}

/// Trocea el fichero en enunciados terminados en `;` a nivel superior.
///
/// A nivel superior de verdad: un `;` dentro de `{ … }`, `( … )`, de una cadena
/// o de un comentario no termina nada.
fn statements(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut actual = String::new();
    let mut profundidad = 0usize;
    let mut en_cadena = false;
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if en_cadena {
            actual.push(c);
            match c {
                '\\' => {
                    if let Some(n) = chars.next() {
                        actual.push(n);
                    }
                }
                '"' => en_cadena = false,
                _ => {}
            }
            continue;
        }
        if c == '/' && chars.peek() == Some(&'/') {
            for n in chars.by_ref() {
                if n == '\n' {
                    break;
                }
            }
            actual.push('\n');
            continue;
        }
        match c {
            '"' => en_cadena = true,
            '{' | '(' | '[' => profundidad += 1,
            '}' | ')' | ']' => profundidad = profundidad.saturating_sub(1),
            ';' if profundidad == 0 => {
                out.push(std::mem::take(&mut actual));
                continue;
            }
            _ => {}
        }
        actual.push(c);
    }
    if !actual.trim().is_empty() {
        out.push(actual);
    }
    out
}

/// El argumento de `@nombre("…")`, todas las apariciones.
fn annotations(s: &str, nombre: &str) -> Vec<String> {
    let marca = format!("@{nombre}(\"");
    let mut out = Vec::new();
    let mut resto = s;
    while let Some(i) = resto.find(&marca) {
        let tras = &resto[i + marca.len()..];
        match tras.find('"') {
            Some(j) => {
                out.push(tras[..j].to_string());
                resto = &tras[j..];
            }
            None => break,
        }
    }
    out
}

/// Las etiquetas que un enunciado menciona, por su forma proyectada.
///
/// `Label::"gdpr.sensitivity:high"` es lo que la proyección a esquema Cedar
/// emite para cada nivel de cada retículo, así que buscarlo literalmente no es
/// un atajo: es leer el vocabulario que nosotros mismos generamos.
fn etiquetas(s: &str) -> BTreeSet<String> {
    nombrados(s, "Label")
}

/// Los identificadores que la política nombra de un tipo de entidad dado:
/// `Tipo::"…"`.
///
/// Se generalizó al añadir los roles, y no por economía: **leer `Label::"…"` y
/// leer `Role::"…"` son la misma pregunta** —*¿con qué puede casar esta
/// política?*— sobre dos vocabularios. Dos analizadores para una pregunta
/// acaban divergiendo en el caso raro, que es donde importa.
fn nombrados(s: &str, tipo: &str) -> BTreeSet<String> {
    let marca = format!("{tipo}::\"");
    let mut out = BTreeSet::new();
    let mut resto = s;
    while let Some(i) = resto.find(&marca) {
        let tras = &resto[i + marca.len()..];
        match tras.find('"') {
            Some(j) => {
                out.insert(tras[..j].to_string());
                resto = &tras[j..];
            }
            None => break,
        }
    }
    out
}

/// Divide por `&&` a nivel superior y normaliza espacios.
fn conjunciones(cuerpo: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut actual = String::new();
    let mut profundidad = 0usize;
    let mut en_cadena = false;
    let cs: Vec<char> = cuerpo.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if en_cadena {
            actual.push(c);
            if c == '"' {
                en_cadena = false;
            }
            i += 1;
            continue;
        }
        match c {
            '"' => en_cadena = true,
            '{' | '(' | '[' => profundidad += 1,
            '}' | ')' | ']' => profundidad = profundidad.saturating_sub(1),
            '&' if profundidad == 0 && cs.get(i + 1) == Some(&'&') => {
                out.push(normalizar(&actual));
                actual.clear();
                i += 2;
                continue;
            }
            _ => {}
        }
        actual.push(c);
        i += 1;
    }
    if !actual.trim().is_empty() {
        out.push(normalizar(&actual));
    }
    out
}

fn normalizar(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Las finalidades que admite una condición sobre `context.purpose`.
///
/// `== "x"` y `in ["a", "b"]` se leen igual: lo que importa es el conjunto
/// resultante, no la forma sintáctica de escribirlo.
fn purposes(conds: &[String]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for c in conds {
        if !c.contains("context.purpose") {
            continue;
        }
        let mut resto = c.as_str();
        while let Some(i) = resto.find('"') {
            let tras = &resto[i + 1..];
            match tras.find('"') {
                Some(j) => {
                    out.insert(tras[..j].to_string());
                    resto = &tras[j + 1..];
                }
                None => break,
            }
        }
    }
    out
}

/// El efecto de un enunciado: la primera palabra tras sus anotaciones.
///
/// **Se salta las anotaciones contando paréntesis, no líneas.** Cedar no obliga
/// a poner `@id(…)` en su propia línea, y una implementación que lo diera por
/// hecho no leería mal la política: la haría **desaparecer**. Y desaparecer es
/// el peor resultado posible aquí — un `forbid` que se reformatea en una línea
/// se leería como un `forbid` eliminado (`OOS5014` espurio), y un `permit` al
/// que se le relaja una condición no se leería en absoluto.
///
/// Ninguna de las dos cosas la detecta la suite de conformidad, porque sus
/// políticas están todas escritas en varias líneas. La atrapó un test unitario.
fn efecto(s: &str) -> Option<Effect> {
    let mut resto = s.trim_start();
    while let Some(tras_arroba) = resto.strip_prefix('@') {
        let abre = tras_arroba.find('(')?;
        let mut nivel = 0usize;
        let mut en_cadena = false;
        let mut fin = None;
        let mut chars = tras_arroba[abre..].char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                '\\' if en_cadena => {
                    chars.next();
                }
                '"' => en_cadena = !en_cadena,
                '(' if !en_cadena => nivel += 1,
                ')' if !en_cadena => {
                    nivel -= 1;
                    if nivel == 0 {
                        fin = Some(abre + i + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        resto = tras_arroba[fin?..].trim_start();
    }
    // Sin efecto no hay política: es otra cosa.
    match resto {
        _ if resto.starts_with("permit") => Some(Effect::Permit),
        _ if resto.starts_with("forbid") => Some(Effect::Forbid),
        _ => None,
    }
}

pub fn read(text: &str) -> Vec<Policy> {
    statements(text)
        .iter()
        .filter_map(|s| {
            let effect = efecto(s)?;

            let conditions = match (s.find("when"), s.rfind('}')) {
                (Some(i), Some(j)) if j > i => {
                    let cuerpo = &s[i + 4..j];
                    conjunciones(cuerpo.trim_start().trim_start_matches('{'))
                }
                _ => Vec::new(),
            };

            Some(Policy {
                id: annotations(s, "id").first().cloned().unwrap_or_default(),
                effect,
                obligations: annotations(s, "obligation"),
                masks: annotations(s, "oosMask"),
                scopes: annotations(s, "oosScope"),
                labels: etiquetas(s),
                roles: nombrados(s, "Role"),
                properties: nombrados(s, "Property"),
                purposes: purposes(&conditions),
                conditions,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: &str = r#"
// Un comentario con un ; dentro no termina nada.
@id("manager-reads-own-chain")
@obligation("mask:LAST4")
permit (
    principal in Role::"people_manager",
    action == Action::"read",
    resource in Label::"gdpr.sensitivity:critical"
) when {
    context.purpose == "compensation_review" &&
    resource.owner in principal.directReports
};

@id("forbid-agent")
forbid ( principal in Role::"ai_agent", action, resource );
"#;

    #[test]
    fn lee_identidad_efecto_y_condiciones() {
        let ps = read(P);
        assert_eq!(ps.len(), 2);
        assert_eq!(ps[0].id, "manager-reads-own-chain");
        assert_eq!(ps[0].effect, Effect::Permit);
        assert_eq!(ps[0].obligations, ["mask:LAST4"]);
        assert_eq!(ps[0].conditions.len(), 2);
        assert_eq!(
            ps[0].conditions[1],
            "resource.owner in principal.directReports"
        );
        assert_eq!(ps[1].effect, Effect::Forbid);
        assert!(ps[1].conditions.is_empty());
    }

    #[test]
    fn lee_las_finalidades_en_ambas_formas() {
        let una = read(
            r#"@id("a") permit (principal, action, resource) when { context.purpose == "x" };"#,
        );
        assert_eq!(una[0].purposes, ["x".to_string()].into());

        let varias = read(
            r#"@id("a") permit (principal, action, resource) when { context.purpose in ["x", "y"] };"#,
        );
        assert_eq!(varias[0].purposes.len(), 2);
    }

    /// Regresion: `@id(...)` en la misma linea que `permit`.
    ///
    /// La version anterior filtraba por lineas que empiezan por `@`, asi que
    /// una politica escrita en una sola linea desaparecia entera. Para `diff`
    /// eso no es leer mal: es leer que la politica no existe.
    #[test]
    fn una_politica_en_una_sola_linea_no_desaparece() {
        let ps = read(r#"@id("a") @obligation("mask") forbid (principal, action, resource);"#);
        assert_eq!(ps.len(), 1, "la politica desaparecio");
        assert_eq!(ps[0].effect, Effect::Forbid);
        assert_eq!(ps[0].id, "a");
        assert_eq!(ps[0].obligations, ["mask"]);
    }

    /// Un enunciado que no es una politica sigue sin serlo, y una anotacion que
    /// contenga la palabra `permit` no lo convierte en una.
    #[test]
    fn lo_que_no_es_politica_se_ignora() {
        assert!(read(r#"entity Label;"#).is_empty());
        assert!(read(r#"@doc("permit nothing") entity Role;"#).is_empty());
    }

    #[test]
    fn lee_el_umbral_de_aggregate() {
        let ps = read(
            r#"@id("a") @obligation("aggregate:minGroupSize=8") permit (principal, action, resource);"#,
        );
        assert_eq!(ps[0].min_group_size(), Some(8));
        assert!(ps[0].declassifiers().contains("aggregate"));
    }
}
