//! **Los 14 casos** de `docs/federation.md` §2, contra un conector y un banco.
//!
//! Cada caso dice `pasa`, `falla` o `no aplica`, y siempre por qué, con las
//! cifras que midió. Un conector sin `capacidades` (los v1) se prueba igual
//! —lo que hace se mide— pero los casos que dependen de lo declarado fallan
//! por no declararlo: es la línea de base de F2·1.

use crate::bancos::Banco;
use crate::conector::{Conector, Salida, corto};
use crate::respuesta::{self, Forma, tipo_arrow};
use crate::semilla::{self, Derecha, GRANDE, GRANDE_COLUMNAS, PRUEBAS, TIPOS, Tabla};
use ore_core::json::Json;
use ore_core::tipos::{Fisico, Valor};
use ore_driver::Codigo;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufReader, Write};
use std::time::{Duration, Instant};

/// Cómo quedó un caso.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Estado {
    Pasa,
    Falla,
    NoAplica,
}

impl Estado {
    pub fn as_str(self) -> &'static str {
        match self {
            Estado::Pasa => "pasa",
            Estado::Falla => "falla",
            Estado::NoAplica => "no aplica",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Resultado {
    pub caso: u8,
    pub nombre: &'static str,
    pub estado: Estado,
    pub detalle: String,
}

/// Los catorce, con el nombre de §2.
pub const CASOS: [(u8, &str); 14] = [
    (1, "proyección"),
    (2, "cada operador declarado"),
    (3, "un operador no declarado"),
    (4, "limit y orderBy"),
    (5, "tipos"),
    (6, "tabla vacía"),
    (7, "flujo"),
    (8, "timeoutMs"),
    (9, "sólo lectura"),
    (10, "cancelar"),
    (11, "servir"),
    (12, "secretos"),
    (13, "capacidades"),
    (14, "estimar"),
];

/// El techo de memoria del caso 7: 10⁶ filas en flujo no pueden pedir más.
/// M3 midió 551 MB con el conector v1, que lo juntaba todo.
pub const TECHO_KIB: u64 = 128 * 1024;

const PLAZO: Duration = Duration::from_secs(60);

/// Lo que el conector declara (`capacidades`).
#[derive(Debug, Clone, Default)]
pub struct Declarado {
    pub protocolo: i64,
    pub operadores: Vec<String>,
    pub limit: bool,
    pub order_by: bool,
    pub estimar: bool,
    pub servir: bool,
}

fn declarado(c: &Conector) -> Result<Declarado, String> {
    let s = c.correr("capacidades", "{}", Duration::from_secs(10), None);
    if !s.ok {
        return Err(format!("`capacidades` no contesta ({})", s.resumen()));
    }
    let texto = String::from_utf8_lossy(&s.stdout);
    let n = ore_core::parse::parse(texto.trim())
        .map_err(|e| format!("`capacidades` no es JSON: {e:?}"))?;
    let si = |k: &str| n.get(k).and_then(|(_, v)| v.as_str()) == Some("true");
    Ok(Declarado {
        protocolo: n
            .get("protocolo")
            .and_then(|(_, v)| v.as_str())
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
        operadores: n
            .get("operadores")
            .map(|(_, v)| {
                v.items()
                    .iter()
                    .filter_map(|o| o.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        limit: si("limit"),
        order_by: si("orderBy"),
        estimar: si("estimar"),
        servir: si("servir"),
    })
}

/// Un filtro de la petición.
fn filtro(columna: &str, operador: &str, valor: &Derecha) -> Json {
    let mut o = BTreeMap::new();
    o.insert("columna".to_string(), Json::s(columna));
    o.insert("operador".to_string(), Json::s(operador));
    match valor {
        Derecha::Uno(v) => {
            o.insert("valor".to_string(), Json::s(*v));
        }
        Derecha::Lista(l) => {
            o.insert(
                "valor".to_string(),
                Json::Arr(l.iter().map(|v| Json::s(*v)).collect()),
            );
        }
        Derecha::Ninguno => {}
    }
    Json::Obj(o)
}

fn fisico_de(tabla: Tabla, columna: &str) -> Fisico {
    tabla
        .columnas()
        .iter()
        .find(|c| c.nombre == columna)
        .map(|c| c.fisico())
        .unwrap_or(Fisico::Texto)
}

struct Kit<'a> {
    c: &'a Conector,
    b: &'a mut dyn Banco,
    caps: Result<Declarado, String>,
}

/// Una petición en construcción.
type Pet = BTreeMap<String, Json>;

impl Kit<'_> {
    fn base(&self, tabla: Tabla, proy: &[(&str, &str)]) -> Pet {
        let mut m = Pet::new();
        m.insert("url".into(), Json::s(self.b.url()));
        m.insert("objeto".into(), Json::s(self.b.objeto(tabla)));
        m.insert(
            "proyeccion".into(),
            Json::Obj(
                proy.iter()
                    .map(|(p, c)| (p.to_string(), Json::s(*c)))
                    .collect(),
            ),
        );
        m.insert("formato".into(), Json::s("arrow"));
        self.b.completar(tabla, &mut m);
        m
    }

    fn leer(&self, m: &Pet, plazo: Duration) -> Salida {
        self.c
            .correr("leer", &Json::Obj(m.clone()).jcs(), plazo, None)
    }

    /// Lee y devuelve los `id`, ordenados o no.
    fn ids(&self, m: &Pet, ordenar: bool) -> Result<Vec<i64>, String> {
        let s = self.leer(m, PLAZO);
        if !s.ok {
            return Err(s.resumen());
        }
        let r = respuesta::leer(&s.stdout, &[("id", Fisico::Entero)])?;
        let mut ids = r.enteros("id")?;
        if ordenar {
            ids.sort();
        }
        Ok(ids)
    }

    fn sin_caps(&self) -> Option<String> {
        self.caps.as_ref().err().cloned()
    }

    fn declara(&self, op: &str) -> bool {
        match &self.caps {
            Ok(d) => d.operadores.iter().any(|o| o == op),
            // Sin declaración, se prueba todo: lo que hace se mide igual.
            Err(_) => true,
        }
    }

    // ── 1 ──────────────────────────────────────────────────────────────────
    fn proyeccion(&mut self) -> (Estado, String) {
        let m = self.base(
            Tabla::Tipos,
            &[("clave", "id"), ("nombre", "texto"), ("otra", "texto")],
        );
        let s = self.leer(&m, PLAZO);
        if !s.ok {
            return (Estado::Falla, s.resumen());
        }
        let esperado = [
            ("clave", Fisico::Entero),
            ("nombre", Fisico::Texto),
            ("otra", Fisico::Texto),
        ];
        let r = match respuesta::leer(&s.stdout, &esperado) {
            Ok(r) => r,
            Err(e) => return (Estado::Falla, e),
        };
        let nombres: BTreeSet<&str> = r.campos.iter().map(|(n, _)| n.as_str()).collect();
        if nombres != BTreeSet::from(["clave", "nombre", "otra"]) {
            return (
                Estado::Falla,
                format!("salen los campos {nombres:?}, no clave, nombre y otra"),
            );
        }
        let semilla = semilla::valores();
        let (ic, ino, iot) = (
            r.campo("clave").unwrap_or(0),
            r.campo("nombre").unwrap_or(0),
            r.campo("otra").unwrap_or(0),
        );
        let mut mal = Vec::new();
        for f in &r.filas {
            let Some(Valor::Entero(id)) = &f[ic] else {
                mal.push("una fila sin `clave`".to_string());
                continue;
            };
            let debido = semilla
                .iter()
                .find(|s| s[0] == Some(Valor::Entero(*id)))
                .map(|s| s[4].clone());
            if debido.as_ref() != Some(&f[ino]) || f[ino] != f[iot] {
                mal.push(format!("id {id}"));
            }
        }
        if r.filas.len() != semilla.len() {
            mal.push(format!("{} filas de {}", r.filas.len(), semilla.len()));
        }
        if mal.is_empty() {
            (
                Estado::Pasa,
                format!(
                    "3 campos con sus nombres, dos de la misma columna ({})",
                    if r.forma == Forma::Arrow {
                        "Arrow"
                    } else {
                        "texto"
                    }
                ),
            )
        } else {
            (
                Estado::Falla,
                format!("valores que no casan: {}", mal.join(", ")),
            )
        }
    }

    // ── 2 ──────────────────────────────────────────────────────────────────
    fn operadores(&mut self) -> (Estado, String) {
        let (mut bien, mut rechaza, mut mal) = (0, BTreeSet::new(), Vec::new());
        for p in PRUEBAS.iter().filter(|p| self.declara(p.operador)) {
            let mut m = self.base(Tabla::Tipos, &[("id", "id")]);
            m.insert(
                "filtros".into(),
                Json::Arr(vec![filtro(p.columna, p.operador, &p.valor)]),
            );
            match self.ids(&m, true) {
                Err(_) => {
                    rechaza.insert(p.operador);
                }
                Ok(ids) if ids == semilla::esperado(p) => bien += 1,
                Ok(ids) => mal.push(format!(
                    "{} {} {:?} → {:?} (debía {:?})",
                    p.columna,
                    p.operador,
                    p.valor,
                    ids,
                    semilla::esperado(p)
                )),
            }
        }
        let probadas = PRUEBAS.iter().filter(|p| self.declara(p.operador)).count();
        let mut d = format!("{bien}/{probadas} pruebas dan sus filas");
        if !rechaza.is_empty() {
            d.push_str(&format!(
                "; rechaza {}",
                rechaza.into_iter().collect::<Vec<_>>().join(", ")
            ));
        }
        if !mal.is_empty() {
            d.push_str(&format!("; **OTRAS FILAS**: {}", mal.join(" | ")));
        }
        match self.sin_caps() {
            Some(e) => (Estado::Falla, format!("{e}; probados los diez: {d}")),
            None if bien == probadas => (Estado::Pasa, d),
            None => (Estado::Falla, d),
        }
    }

    // ── 3 ──────────────────────────────────────────────────────────────────
    fn no_declarado(&mut self) -> (Estado, String) {
        let Ok(d) = &self.caps else {
            return (
                Estado::Falla,
                format!(
                    "{}: no hay con qué comprobarlo",
                    self.sin_caps().unwrap_or_default()
                ),
            );
        };
        let fuera: Vec<&str> = ore_driver::OPERADORES
            .iter()
            .copied()
            .filter(|o| !d.operadores.iter().any(|x| x == o))
            .collect();
        if fuera.is_empty() {
            return (Estado::Pasa, "declara los diez".into());
        }
        let mut mal = Vec::new();
        for op in &fuera {
            let Some(p) = PRUEBAS.iter().find(|p| p.operador == *op) else {
                continue;
            };
            let mut m = self.base(Tabla::Tipos, &[("id", "id")]);
            m.insert(
                "filtros".into(),
                Json::Arr(vec![filtro(p.columna, p.operador, &p.valor)]),
            );
            let s = self.leer(&m, PLAZO);
            match (s.ok, s.fallo()) {
                (false, Some(f)) if f.codigo == Codigo::Operador => {}
                (false, _) => mal.push(format!(
                    "`{op}` se rechaza sin `operador` ({})",
                    s.resumen()
                )),
                (true, _) => mal.push(format!("`{op}` no declarado y se sirve")),
            }
        }
        if mal.is_empty() {
            (
                Estado::Pasa,
                format!("rechaza con `operador` los {} que no declara", fuera.len()),
            )
        } else {
            (Estado::Falla, mal.join("; "))
        }
    }

    // ── 4 ──────────────────────────────────────────────────────────────────
    fn limite_y_orden(&mut self) -> (Estado, String) {
        let (limit, orden) = match &self.caps {
            Ok(d) => (Some(d.limit), Some(d.order_by)),
            Err(_) => (None, None),
        };
        let mut partes = Vec::new();
        let mut falla = self.sin_caps().is_some();

        let mut m = self.base(Tabla::Tipos, &[("id", "id")]);
        m.insert("limit".into(), Json::Int(3));
        match (limit, self.ids(&m, false)) {
            (Some(false), Err(_)) => partes.push("no declara `limit` y lo rechaza".to_string()),
            (Some(false), Ok(ids)) => {
                falla = true;
                partes.push(format!(
                    "no declara `limit` y lo sirve ({} filas)",
                    ids.len()
                ));
            }
            (_, Ok(ids)) if ids.len() == 3 => partes.push("limit 3 → 3 filas".into()),
            (_, Ok(ids)) => {
                falla = true;
                partes.push(format!("limit 3 → {} filas", ids.len()));
            }
            (_, Err(e)) => {
                falla = true;
                partes.push(format!("limit: {e}"));
            }
        }

        for (col, desc, n) in [("entero", true, 3usize), ("importe", false, 10)] {
            let mut m = self.base(Tabla::Tipos, &[("id", "id")]);
            m.insert("limit".into(), Json::Int(n as i64));
            m.insert(
                "orderBy".into(),
                Json::Arr(vec![Json::obj([
                    ("columna", Json::s(col)),
                    ("direccion", Json::s(if desc { "desc" } else { "asc" })),
                ])]),
            );
            let debido = semilla::primeros(col, desc, n);
            let que = format!("{col} {} limit {n}", if desc { "desc" } else { "asc" });
            match (orden, self.ids(&m, false)) {
                (Some(false), Err(_)) => {
                    partes.push(format!("no declara `orderBy` y rechaza {que}"))
                }
                (Some(false), Ok(_)) => {
                    falla = true;
                    partes.push(format!("no declara `orderBy` y sirve {que}"));
                }
                (_, Ok(ids)) if ids == debido => partes.push(format!("{que} en su orden")),
                (_, Ok(ids)) => {
                    falla = true;
                    partes.push(format!("{que} → {ids:?} (debía {debido:?})"));
                }
                (_, Err(e)) => {
                    falla = true;
                    partes.push(format!("{que}: {e}"));
                }
            }
        }
        let mut d = partes.join("; ");
        if let Some(e) = self.sin_caps() {
            d = format!("{e}; {d}");
        }
        (if falla { Estado::Falla } else { Estado::Pasa }, d)
    }

    // ── 5 ──────────────────────────────────────────────────────────────────
    fn tipos(&mut self) -> (Estado, String) {
        let proy: Vec<(&str, &str)> = TIPOS.iter().map(|c| (c.nombre, c.nombre)).collect();
        let s = self.leer(&self.base(Tabla::Tipos, &proy), PLAZO);
        if !s.ok {
            return (Estado::Falla, s.resumen());
        }
        let esperado: Vec<(&str, Fisico)> = TIPOS.iter().map(|c| (c.nombre, c.fisico())).collect();
        let r = match respuesta::leer(&s.stdout, &esperado) {
            Ok(r) => r,
            Err(e) => return (Estado::Falla, format!("no se lee en sus tipos: {e}")),
        };
        let mut mal = Vec::new();
        if r.forma == Forma::Texto {
            mal.push("contesta en texto, no en Arrow".to_string());
        } else {
            for c in TIPOS {
                let debido = tipo_arrow(&c.fisico());
                match r.campos.iter().find(|(n, _)| n == c.nombre) {
                    Some((_, Some(t))) if respuesta::mismo_tipo(t, &debido) => {}
                    Some((_, t)) => mal.push(format!("`{}` es {t:?}, no {debido}", c.nombre)),
                    None => mal.push(format!("falta `{}`", c.nombre)),
                }
            }
        }
        let semilla = semilla::valores();
        let posiciones: Vec<Option<usize>> = TIPOS.iter().map(|c| r.campo(c.nombre)).collect();
        let mut valores_mal = Vec::new();
        for f in &r.filas {
            let Some(Some(Valor::Entero(id))) = posiciones[0].map(|i| &f[i]) else {
                continue;
            };
            let Some(debida) = semilla.iter().find(|s| s[0] == Some(Valor::Entero(*id))) else {
                valores_mal.push(format!("id {id} no es de la semilla"));
                continue;
            };
            for (k, c) in TIPOS.iter().enumerate() {
                let Some(i) = posiciones[k] else { continue };
                if f[i] != debida[k] {
                    valores_mal.push(format!(
                        "{}[{id}] = {:?}, no {:?}",
                        c.nombre, f[i], debida[k]
                    ));
                }
            }
        }
        if r.filas.len() != semilla.len() {
            valores_mal.push(format!("{} filas de {}", r.filas.len(), semilla.len()));
        }
        if !valores_mal.is_empty() {
            mal.push(format!("valores: {}", valores_mal.join("; ")));
        }
        if mal.is_empty() {
            (
                Estado::Pasa,
                "los 9 tipos con su tipo Arrow de 0032 y sus valores exactos".into(),
            )
        } else {
            (Estado::Falla, mal.join("; "))
        }
    }

    // ── 6 ──────────────────────────────────────────────────────────────────
    fn vacia(&mut self) -> (Estado, String) {
        let proy: Vec<(&str, &str)> = TIPOS.iter().map(|c| (c.nombre, c.nombre)).collect();
        let s = self.leer(&self.base(Tabla::Vacia, &proy), PLAZO);
        if !s.ok {
            return (Estado::Falla, s.resumen());
        }
        if s.stdout.iter().all(u8::is_ascii_whitespace) {
            return (
                Estado::Falla,
                "no devuelve nada: en texto una tabla vacía no dice sus columnas".into(),
            );
        }
        let esperado: Vec<(&str, Fisico)> = TIPOS.iter().map(|c| (c.nombre, c.fisico())).collect();
        match respuesta::leer(&s.stdout, &esperado) {
            Ok(r)
                if r.forma == Forma::Arrow
                    && r.filas.is_empty()
                    && r.campos.len() == TIPOS.len() =>
            {
                (
                    Estado::Pasa,
                    format!("esquema de {} campos, 0 filas", r.campos.len()),
                )
            }
            Ok(r) => (
                Estado::Falla,
                format!(
                    "{:?} con {} campos y {} filas",
                    r.forma,
                    r.campos.len(),
                    r.filas.len()
                ),
            ),
            Err(e) => (Estado::Falla, e),
        }
    }

    // ── 7 ──────────────────────────────────────────────────────────────────
    fn flujo(&mut self) -> (Estado, String) {
        let proy: Vec<(&str, &str)> = GRANDE_COLUMNAS
            .iter()
            .map(|c| (c.nombre, c.nombre))
            .collect();
        let s = self.leer(&self.base(Tabla::Grande, &proy), Duration::from_secs(180));
        if !s.ok {
            return (Estado::Falla, s.resumen());
        }
        let esperado: Vec<(&str, Fisico)> = GRANDE_COLUMNAS
            .iter()
            .map(|c| (c.nombre, fisico_de(Tabla::Grande, c.nombre)))
            .collect();
        let r = match respuesta::leer(&s.stdout, &esperado) {
            Ok(r) => r,
            Err(e) => return (Estado::Falla, e),
        };
        let ids = r.enteros("id").unwrap_or_default();
        let suma: i128 = ids.iter().map(|&i| i128::from(i)).sum();
        let debida = i128::from(GRANDE) * (i128::from(GRANDE) + 1) / 2;
        let mib = |k: u64| k as f64 / 1024.0;
        let mut d = format!(
            "{} filas en {} ms, {:.0} MB por stdout, {} lote(s), primer byte a los {} ms",
            r.filas.len(),
            s.ms,
            s.stdout.len() as f64 / 1e6,
            r.lotes,
            s.primer_byte_ms.unwrap_or(0)
        );
        let mut mal = Vec::new();
        match s.pico_kib {
            Some(k) => {
                d.push_str(&format!(", pico {:.0} MiB", mib(k)));
                if k > TECHO_KIB {
                    mal.push(format!("pasa del techo de {:.0} MiB", mib(TECHO_KIB)));
                }
            }
            None => d.push_str(", pico sin medir (sin /proc)"),
        }
        if ids.len() as u64 != GRANDE || suma != debida {
            mal.push("no son las 10⁶ filas de la semilla".into());
        }
        if r.forma != Forma::Arrow || r.lotes < 2 {
            mal.push("no sale en lotes Arrow".into());
        }
        if s.primer_byte_ms.is_none_or(|p| p * 2 > s.ms) {
            mal.push("el primer byte sale pasada la mitad: lo junta antes de escribir".into());
        }
        if mal.is_empty() {
            (Estado::Pasa, d)
        } else {
            (Estado::Falla, format!("{d} — {}", mal.join("; ")))
        }
    }

    // ── 8 ──────────────────────────────────────────────────────────────────
    fn tiempo(&mut self) -> (Estado, String) {
        let lenta = self.b.lenta();
        let (mut m, ms) = match &lenta {
            Some(o) => {
                let mut m = self.base(Tabla::Tipos, &[("id", "id")]);
                m.insert("objeto".into(), Json::s(o.as_str()));
                (m, 1000)
            }
            None => (
                self.base(Tabla::Grande, &[("id", "id"), ("nota", "nota")]),
                1,
            ),
        };
        m.insert("timeoutMs".into(), Json::Int(ms));
        let s = self.leer(&m, Duration::from_millis(ms as u64 + 5000));
        let vivas = lenta.as_deref().and_then(|o| {
            std::thread::sleep(Duration::from_millis(300));
            self.b.consultas_vivas(o)
        });
        self.b.limpiar();
        let que = format!(
            "timeoutMs {ms} sobre {}",
            lenta.as_deref().unwrap_or("grande")
        );
        match (s.ok, s.fallo(), vivas) {
            (false, Some(f), v) if f.codigo == Codigo::Tiempo && v.unwrap_or(0) == 0 => (
                Estado::Pasa,
                format!("{que}: `tiempo` a los {} ms, nada vivo en el origen", s.ms),
            ),
            (false, Some(f), Some(v)) if f.codigo == Codigo::Tiempo => (
                Estado::Falla,
                format!("{que}: `tiempo`, pero {v} consulta(s) siguen vivas en el origen"),
            ),
            (true, _, _) => (
                Estado::Falla,
                format!("{que}: termina bien en {} ms, sin cortar", s.ms),
            ),
            _ => (Estado::Falla, format!("{que}: {}", s.resumen())),
        }
    }

    // ── 9 ──────────────────────────────────────────────────────────────────
    fn solo_lectura(&mut self) -> (Estado, String) {
        let Some(escribe) = self.b.que_escribe() else {
            return (
                Estado::NoAplica,
                format!(
                    "{}: una lectura no puede escribir (no hay SQL)",
                    self.b.familia()
                ),
            );
        };
        self.b.limpiar();
        let mut mal = Vec::new();
        let tipos = self.b.objeto(Tabla::Tipos);
        let intentos: Vec<Pet> = vec![
            {
                let mut m = self.base(Tabla::Tipos, &[("id", "id")]);
                m.insert(
                    "objeto".into(),
                    Json::s(format!("{tipos}; DROP TABLE kit.marcas")),
                );
                m
            },
            self.base(Tabla::Tipos, &[("id", "id\"; DROP TABLE kit.marcas; --")]),
            self.base(
                Tabla::Tipos,
                &[("id", "id FROM kit.tipos; DROP TABLE kit.marcas; --")],
            ),
        ];
        for m in &intentos {
            let s = self.leer(m, PLAZO);
            if s.ok {
                mal.push("un nombre con `;` se sirve".to_string());
            }
        }
        if let Some(Err(e)) = self.b.escrituras() {
            mal.push(format!("**un nombre inyectó SQL**: {e}"));
        }
        let mut m = self.base(Tabla::Tipos, &[("id", "id")]);
        m.insert("objeto".into(), Json::s(escribe.as_str()));
        let s = self.leer(&m, PLAZO);
        let escritas = self.b.escrituras();
        match (s.ok, escritas) {
            (_, Some(Ok(n))) if n > 0 => {
                mal.push(format!("**leer `{escribe}` escribió {n} fila(s)**"))
            }
            (true, _) => mal.push(format!("leer `{escribe}` se sirve")),
            _ => {}
        }
        self.b.limpiar();
        if mal.is_empty() {
            (
                Estado::Pasa,
                format!(
                    "3 nombres con `;` rechazados; leer `{escribe}` no escribe ({})",
                    corto(&s.resumen())
                ),
            )
        } else {
            (Estado::Falla, mal.join("; "))
        }
    }

    // ── 10 ─────────────────────────────────────────────────────────────────
    fn cancelar(&mut self) -> (Estado, String) {
        let Some(lenta) = self.b.lenta() else {
            return (
                Estado::NoAplica,
                format!("{}: no hay nada lento que cancelar", self.b.familia()),
            );
        };
        let mut m = self.base(Tabla::Tipos, &[("id", "id")]);
        m.insert("objeto".into(), Json::s(lenta.as_str()));
        let mut hijo = match self.c.lanzar("leer") {
            Ok(h) => h,
            Err(e) => return (Estado::Falla, format!("no arranca: {e}")),
        };
        if let Some(mut stdin) = hijo.stdin.take() {
            let _ = stdin.write_all(Json::Obj(m).jcs().as_bytes());
        }
        std::thread::sleep(Duration::from_millis(1500));
        let antes = self.b.consultas_vivas(&lenta).unwrap_or(0);
        let _ = std::process::Command::new("kill")
            .args(["-TERM", &hijo.id().to_string()])
            .status();
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(3) {
            if hijo.try_wait().ok().flatten().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = hijo.kill();
        let _ = hijo.wait();
        if let Some(mut e) = hijo.stderr.take() {
            let mut s = String::new();
            let _ = std::io::Read::read_to_string(&mut e, &mut s);
            self.c.anotar(s);
        }
        let mut despues = antes;
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(3) {
            despues = self.b.consultas_vivas(&lenta).unwrap_or(0);
            if despues == 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let ms = t.elapsed().as_millis();
        self.b.limpiar();
        match (antes, despues) {
            (0, _) => (
                Estado::Falla,
                "a los 1,5 s no había consulta en el origen".into(),
            ),
            (_, 0) => (
                Estado::Pasa,
                format!("SIGTERM: la consulta deja el origen en {ms} ms"),
            ),
            (_, n) => (
                Estado::Falla,
                format!("SIGTERM: {n} consulta(s) siguen vivas en el origen 3 s después"),
            ),
        }
    }

    // ── 11 ─────────────────────────────────────────────────────────────────
    fn servir(&mut self) -> (Estado, String) {
        match &self.caps {
            Err(e) => return (Estado::Falla, format!("{e}: no declara `servir`")),
            Ok(d) if !d.servir => return (Estado::Falla, "no declara `servir`".into()),
            Ok(_) => {}
        }
        let mut hijo = match self.c.lanzar("servir") {
            Ok(h) => h,
            Err(e) => return (Estado::Falla, format!("no arranca: {e}")),
        };
        let mut stdin = hijo.stdin.take().expect("tubería");
        let mut stdout = BufReader::new(hijo.stdout.take().expect("tubería"));
        let antes = self.b.sesiones();
        let mut mal = Vec::new();
        let t = Instant::now();
        let mut pedir = |m: &mut Pet, id: String| -> Result<(), String> {
            m.insert("id".into(), Json::s(id.as_str()));
            writeln!(stdin, "{}", Json::Obj(m.clone()).jcs()).map_err(|e| e.to_string())?;
            stdin.flush().map_err(|e| e.to_string())?;
            match ore_driver::servir::leer_respuesta(&mut stdout, &mut std::io::sink())? {
                Some(ore_driver::servir::Fin::Ok { id: r, .. }) if r == id => Ok(()),
                otro => Err(format!("la respuesta a `{id}` es {otro:?}")),
            }
        };
        for i in 0..100 {
            let mut m = self.base(Tabla::Tipos, &[("id", "id")]);
            m.insert(
                "filtros".into(),
                Json::Arr(vec![filtro("texto", "eq", &Derecha::Uno("ana"))]),
            );
            if let Err(e) = pedir(&mut m, format!("p{i}")) {
                mal.push(e);
                break;
            }
        }
        let ms = t.elapsed().as_millis();
        let tras = self.b.sesiones();
        if let (Some(a), Some(d)) = (antes, tras)
            && d > a + 1
        {
            mal.push(format!("100 peticiones abrieron {} sesiones", d - a));
        }
        if let Some(otra) = self.b.url_alternativa() {
            let mut m = self.base(Tabla::Tipos, &[("id", "id")]);
            m.insert("url".into(), Json::s(otra));
            if let Err(e) = pedir(&mut m, "otra".into()) {
                mal.push(e);
            }
            if let (Some(a), Some(d)) = (antes, self.b.sesiones())
                && d != a + 2
            {
                mal.push(format!("otra credencial deja {} sesiones, no 2", d - a));
            }
        }
        drop(pedir);
        let _ = hijo.kill();
        let _ = hijo.wait();
        self.b.limpiar();
        if mal.is_empty() {
            let sesiones = match (antes, tras) {
                (Some(_), Some(_)) => "por una sesión; otra credencial, otra",
                _ => "(el origen no dice sus sesiones)",
            };
            (
                Estado::Pasa,
                format!("100 peticiones en {ms} ms {sesiones}"),
            )
        } else {
            (Estado::Falla, mal.join("; "))
        }
    }

    // ── 12 ─────────────────────────────────────────────────────────────────
    fn secretos(&mut self) -> (Estado, String) {
        let mut mal = Vec::new();
        let mut urls = vec![("la de lectura", self.b.url())];
        let credencial = match self.b.url_mala() {
            None => "el origen de pruebas no comprueba credenciales".to_string(),
            Some(mala) => {
                let mut m = self.base(Tabla::Tipos, &[("id", "id")]);
                m.insert("url".into(), Json::s(mala.as_str()));
                urls.push(("la mala", mala));
                let s = self.leer(&m, PLAZO);
                match (s.ok, s.fallo()) {
                    (true, _) => {
                        mal.push("una credencial mala se sirve".to_string());
                        String::new()
                    }
                    (false, Some(f)) if f.codigo == Codigo::Credencial => {
                        "la mala sale como `credencial`".into()
                    }
                    (false, _) => {
                        if self.caps.is_ok() {
                            mal.push(format!(
                                "la credencial mala no sale como `credencial` ({})",
                                s.resumen()
                            ));
                        }
                        format!("la mala, sin tipar ({})", corto(&s.resumen()))
                    }
                }
            }
        };
        if let Some(o) = self.b.url_alternativa() {
            urls.push(("la otra", o));
        }
        let registro = self.c.registro();
        for (que, url) in &urls {
            for secreto in ore_driver::fallo::secretos(url) {
                let n = registro.iter().filter(|t| t.contains(&secreto)).count();
                if n > 0 {
                    let parte = if &secreto == url {
                        "la URL entera"
                    } else {
                        "su credencial"
                    };
                    mal.push(format!("**{parte} de {que} sale en {n} salida(s)**"));
                }
            }
        }
        mal.dedup();
        if mal.is_empty() {
            (
                Estado::Pasa,
                format!(
                    "ninguna URL ni credencial en {} salidas; {credencial}",
                    registro.len()
                ),
            )
        } else {
            (Estado::Falla, mal.join("; "))
        }
    }

    // ── 13 ─────────────────────────────────────────────────────────────────
    fn capacidades(&mut self, previos: &[Resultado]) -> (Estado, String) {
        let d = match &self.caps {
            Err(e) => return (Estado::Falla, e.clone()),
            Ok(d) => d.clone(),
        };
        let mut mal = Vec::new();
        if d.protocolo != i64::from(ore_driver::capacidades::PROTOCOLO) {
            mal.push(format!("protocolo {}", d.protocolo));
        }
        for o in &d.operadores {
            if !ore_driver::OPERADORES.contains(&o.as_str()) {
                mal.push(format!("declara `{o}`, que no es un operador"));
            }
        }
        for r in previos.iter().filter(|r| [2, 3, 4].contains(&r.caso)) {
            if r.estado == Estado::Falla {
                mal.push(format!("lo que declara no lo cumple el caso {}", r.caso));
            }
        }
        let d_txt = format!(
            "protocolo {}, {} operadores, limit {}, orderBy {}, estimar {}, servir {}",
            d.protocolo,
            d.operadores.len(),
            d.limit,
            d.order_by,
            d.estimar,
            d.servir
        );
        if mal.is_empty() {
            (Estado::Pasa, d_txt)
        } else {
            (Estado::Falla, format!("{d_txt} — {}", mal.join("; ")))
        }
    }

    // ── 14 ─────────────────────────────────────────────────────────────────
    fn estimar(&mut self) -> (Estado, String) {
        match &self.caps {
            Ok(d) if d.estimar => {}
            Ok(_) => return (Estado::NoAplica, "no declara `estimar`".into()),
            Err(_) => return (Estado::NoAplica, "no declara capacidades".into()),
        }
        let _ = self.b.facturado();
        let m = self.base(Tabla::Grande, &[("id", "id")]);
        let s = self.c.correr("estimar", &Json::Obj(m).jcs(), PLAZO, None);
        if !s.ok {
            return (Estado::Falla, s.resumen());
        }
        let texto = String::from_utf8_lossy(&s.stdout).to_string();
        let n = match ore_core::parse::parse(texto.trim()) {
            Ok(n) => n,
            Err(_) => return (Estado::Falla, format!("no es JSON: {}", corto(&texto))),
        };
        let filas = n
            .get("filas")
            .and_then(|(_, v)| v.as_str())
            .map(String::from);
        let bytes = n
            .get("bytes")
            .and_then(|(_, v)| v.as_str())
            .map(String::from);
        if filas.is_none() && bytes.is_none() {
            return (
                Estado::Falla,
                format!("no dice `filas` ni `bytes`: {}", corto(&texto)),
            );
        }
        match self.b.facturado() {
            Some(f) if f > 0 => (Estado::Falla, format!("estimar facturó {f} bytes")),
            f => (
                Estado::Pasa,
                format!(
                    "filas {}, bytes {} en {} ms{}",
                    filas.unwrap_or("?".into()),
                    bytes.unwrap_or("?".into()),
                    s.ms,
                    if f.is_some() { ", sin facturar" } else { "" }
                ),
            ),
        }
    }
}

/// **Pasa los casos** (`solo`, o los catorce) y devuelve sus resultados en
/// orden. El 12 va el último aunque se pida antes: busca secretos en todo lo
/// que los demás hicieron decir al conector.
pub fn correr(c: &Conector, b: &mut dyn Banco, solo: &[u8]) -> Vec<Resultado> {
    let caps = declarado(c).map_err(|e| format!("no declara capacidades ({e})"));
    let mut kit = Kit { c, b, caps };
    let quiere = |n: u8| solo.is_empty() || solo.contains(&n);
    let mut hechos: Vec<Resultado> = Vec::new();
    let mut orden: Vec<u8> = (1..=14).filter(|n| *n != 12).collect();
    orden.push(12);
    for n in orden.into_iter().filter(|n| quiere(*n)) {
        let (estado, detalle) = match n {
            1 => kit.proyeccion(),
            2 => kit.operadores(),
            3 => kit.no_declarado(),
            4 => kit.limite_y_orden(),
            5 => kit.tipos(),
            6 => kit.vacia(),
            7 => kit.flujo(),
            8 => kit.tiempo(),
            9 => kit.solo_lectura(),
            10 => kit.cancelar(),
            11 => kit.servir(),
            12 => kit.secretos(),
            13 => kit.capacidades(&hechos),
            14 => kit.estimar(),
            _ => unreachable!(),
        };
        let nombre = CASOS[usize::from(n) - 1].1;
        eprintln!("  {n:>2} · {nombre}: {}", estado.as_str());
        hechos.push(Resultado {
            caso: n,
            nombre,
            estado,
            detalle,
        });
    }
    hechos.sort_by_key(|r| r.caso);
    hechos
}
