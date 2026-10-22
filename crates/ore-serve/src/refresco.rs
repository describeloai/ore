//! ⭐ **El vigilante del refresco** (ADR 0063 S8, D-S8-1): cada `cadencia`,
//! mira en el índice de `main` qué copias con `freshness` van a vencerla
//! (`ore_core::refresco`) y, si alguna toca, encola UNA pasada de la copia
//! sólo para ellas, en modo programado (`cola::rendir_programada`): la señal de
//! cambio, «ya está» y las promesas, como siempre. Sin nada que vencer no se
//! encola nada: no hay pod, y no se gasta cuota.
//!
//! Apagado por defecto: lo enciende `--refrescar <segundos>` en un inquilino.
//! Sin cola, o con una plantilla que aún no trae el hueco `PROGRAMADA`, lo dice
//! en `GET /refresco` y en el registro, y no encola nada.
//!
//! Lo que encoló lo retira de la cola cuando ya pasó ([`RETIRAR`]): un Job que
//! terminó no vuelve a correr, y la cola sólo guarda lo vivo.

use crate::cola;
use crate::rutas::Servidor;
use ore_core::json::Json;
use ore_core::refresco::{Memoria, del_indice, observar, toca};
use ore_entrada::http::Respuesta;
use ore_entrada::identidad::Identidad;
use std::collections::BTreeMap;
use std::sync::Mutex;

/// Cuándo se retira de la cola un fichero programado: pasado el plazo de su
/// Job (una hora) y su arranque, con margen.
pub const RETIRAR: i64 = 2 * 3600;

/// Lo que el vigilante sabe entre vueltas.
#[derive(Default)]
pub struct Vigilante {
    /// Cada cuánto mira, en segundos.
    pub cadencia: i64,
    memoria: Mutex<BTreeMap<String, Memoria>>,
    /// Lo que dijo la última vuelta, para `GET /refresco`.
    ultima: Mutex<Option<Json>>,
    /// Cuándo encoló por última vez: pasado [`RETIRAR`], se retira de la cola
    /// aunque no toque nada más.
    ultima_encolada: Mutex<Option<i64>>,
}

impl Vigilante {
    pub fn nuevo(cadencia: i64) -> Vigilante {
        Vigilante {
            cadencia: cadencia.max(10),
            ..Default::default()
        }
    }
}

fn ore_serve() -> Identidad {
    Identidad {
        persona: "ore-serve".into(),
        agente: None,
        correo: None,
        nombre: None,
        tipo: None,
        usuario: None,
    }
}

impl Servidor {
    /// **Una vuelta del vigilante**, con la hora de fuera. Devuelve lo que hay
    /// que decir en el registro (vacío si no pasó nada).
    pub(crate) fn refrescar(&self, ahora: i64) -> String {
        let Some(v) = &self.refresco else {
            return String::new();
        };
        let r = self.assets(None, None);
        if r.codigo != 200 {
            return format!("NO miró: el índice de `main` contestó {}", r.codigo);
        }
        let texto = match &r.cuerpo {
            Json::Crudo(t) => t.clone(),
            otro => otro.jcs(),
        };
        let Ok(indice) = ore_core::parse::parse(&texto) else {
            return "NO miró: el índice no analiza".into();
        };
        let copias = del_indice(&indice);
        let mut memoria = v.memoria.lock().unwrap_or_else(|e| e.into_inner());
        memoria.retain(|k, _| copias.iter().any(|c| &c.vista == k));
        let mut tocan = Vec::new();
        let mut lista = Vec::new();
        for c in &copias {
            let m = memoria.entry(c.vista.clone()).or_default();
            observar(m, c);
            let (si, por_que) = match toca(c, m, ahora, v.cadencia) {
                Ok(p) => (true, p),
                Err(p) => (false, p),
            };
            if si {
                tocan.push(c.vista.clone());
            }
            lista.push(Json::obj([
                ("vista", Json::s(&c.vista)),
                ("freshness_s", Json::Int(c.limite)),
                (
                    "edad_s",
                    c.edad.map(Json::Int).unwrap_or(Json::Crudo("null".into())),
                ),
                ("toca", Json::Bool(si)),
                ("por_que", Json::s(por_que)),
                ("fallos", Json::Int(m.fallos as i64)),
            ]));
        }
        let mut dicho = String::new();
        let mut encolado = Json::Crudo("null".into());
        let mut ultima_encolada = v.ultima_encolada.lock().unwrap_or_else(|e| e.into_inner());
        if tocan.is_empty()
            && let Some(t) = *ultima_encolada
            && ahora - t >= RETIRAR
        {
            match self.retirar_de_la_cola(ahora) {
                Ok(d) => {
                    *ultima_encolada = None;
                    dicho = d;
                }
                Err(e) => dicho = format!("refresco · NO retirado: {e}"),
            }
        }
        if !tocan.is_empty() {
            match self.encolar_programada(&tocan, ahora, v.cadencia) {
                Ok((fichero, c)) => {
                    for t in &tocan {
                        if let Some(m) = memoria.get_mut(t) {
                            m.encolada = Some(ahora);
                        }
                    }
                    *ultima_encolada = Some(ahora);
                    dicho = format!(
                        "refresco · encolado `{fichero}` para {} · {c}",
                        tocan.join(", ")
                    );
                    encolado = Json::s(&fichero);
                }
                Err(e) => dicho = format!("refresco · NO encolado ({}): {e}", tocan.join(", ")),
            }
        }
        *v.ultima.lock().unwrap_or_else(|e| e.into_inner()) = Some(Json::obj([
            ("cuando", Json::s(ore_core::frescura::iso(ahora))),
            ("cadencia_s", Json::Int(v.cadencia)),
            ("copias", Json::Arr(lista)),
            ("encolado", encolado),
            ("dicho", Json::s(&dicho)),
        ]));
        dicho
    }

    /// El fichero de la pasada programada, a la cola, y fuera los que ya
    /// pasaron. La franja redondea `ahora` a la cadencia: dos vueltas de la
    /// misma franja con las mismas vistas son el mismo fichero.
    fn encolar_programada(
        &self,
        vistas: &[String],
        ahora: i64,
        cadencia: i64,
    ) -> Result<(String, String), String> {
        let forja = self
            .cola
            .as_ref()
            .ok_or("este servidor no sabe de ninguna cola (`--cola`)")?;
        let prestado = forja.clonar().map_err(|e| e.to_string())?;
        let dir = prestado.ruta();
        let plantilla = std::fs::read_to_string(dir.join(cola::PLANTILLA_COPIA)).map_err(|_| {
            format!(
                "la cola no trae `{}`: hay que converger este inquilino",
                cola::PLANTILLA_COPIA
            )
        })?;
        let franja = ore_core::frescura::iso(ahora - ahora.rem_euclid(cadencia.max(1)));
        let (fichero, texto) = cola::rendir_programada(&plantilla, vistas, &franja)?;
        let retirados = retirar_pasados(dir, ahora);
        std::fs::write(dir.join(&fichero), texto)
            .map_err(|e| format!("no se pudo escribir `{fichero}`: {e}"))?;
        if !forja.hay_cambios(dir) {
            return Ok((fichero, "ya estaba en la cola".into()));
        }
        let mensaje = format!(
            "Refresco programado: {} (0063 S8){}",
            vistas.join(", "),
            if retirados.is_empty() {
                String::new()
            } else {
                format!(" · retirados {}", retirados.join(", "))
            }
        );
        let c = forja
            .publicar(dir, &ore_serve(), &mensaje)
            .map_err(|e| e.to_string())?;
        Ok((fichero, format!("commit {c}")))
    }

    /// Fuera de la cola lo programado que ya pasó, sin encolar nada.
    fn retirar_de_la_cola(&self, ahora: i64) -> Result<String, String> {
        let forja = self
            .cola
            .as_ref()
            .ok_or("este servidor no sabe de ninguna cola (`--cola`)")?;
        let prestado = forja.clonar().map_err(|e| e.to_string())?;
        let dir = prestado.ruta();
        let fuera = retirar_pasados(dir, ahora);
        if fuera.is_empty() || !forja.hay_cambios(dir) {
            return Ok(String::new());
        }
        let que = format!(
            "Refresco programado: retirados {} (0063 S8)",
            fuera.join(", ")
        );
        let c = forja
            .publicar(dir, &ore_serve(), &que)
            .map_err(|e| e.to_string())?;
        Ok(format!("refresco · {que} · commit {c}"))
    }

    /// `GET /refresco`: qué copias declaran `freshness`, cuándo vencen, qué
    /// encoló la última vuelta y por qué. Sólo leer.
    pub(crate) fn estado_del_refresco(&self) -> Respuesta {
        let Some(v) = &self.refresco else {
            return Respuesta::ok(Json::obj([
                ("activo", Json::Bool(false)),
                (
                    "por_que",
                    Json::s("the scheduled refresh is off in this tenant (`--refrescar`)"),
                ),
            ]));
        };
        let ultima = v
            .ultima
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .unwrap_or(Json::Crudo("null".into()));
        Respuesta::ok(Json::obj([
            ("activo", Json::Bool(true)),
            ("cadencia_s", Json::Int(v.cadencia)),
            ("ultima", ultima),
        ]))
    }
}

/// Los ficheros programados cuya franja ya pasó hace [`RETIRAR`], fuera del
/// clon. Devuelve cuáles.
pub(crate) fn retirar_pasados(dir: &std::path::Path, ahora: i64) -> Vec<String> {
    let Ok(d) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut fuera = Vec::new();
    for e in d.flatten() {
        let nombre = e.file_name().to_string_lossy().to_string();
        if !nombre.starts_with(cola::PREFIJO_PROGRAMADA) {
            continue;
        }
        let pasada = std::fs::read_to_string(e.path())
            .ok()
            .and_then(|t| cola::franja_de(&t).and_then(ore_core::frescura::epoca))
            .is_none_or(|f| ahora - f >= RETIRAR);
        if pasada && std::fs::remove_file(e.path()).is_ok() {
            fuera.push(nombre);
        }
    }
    fuera.sort();
    fuera
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// Lo que ya pasó sale de la cola; lo de la franja de ahora, y lo que no
    /// es del refresco, se queda.
    #[test]
    fn se_retira_lo_programado_que_ya_paso() {
        let d = std::env::temp_dir().join(format!("ore-refresco-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let con = |f: &str| format!("env:\n  - {{ name: PROGRAMADA, value: \"{f}\" }}\n");
        std::fs::write(
            d.join("48-la-copia-prog-viejo.yaml"),
            con("2026-10-16T05:00:00Z"),
        )
        .unwrap();
        std::fs::write(
            d.join("48-la-copia-prog-nuevo.yaml"),
            con("2026-10-16T07:59:00Z"),
        )
        .unwrap();
        std::fs::write(
            d.join("48-la-copia-rehacer-x.yaml"),
            con("2026-10-16T05:00:00Z"),
        )
        .unwrap();
        let ahora = ore_core::frescura::epoca("2026-10-16T08:00:00Z").unwrap();
        assert_eq!(
            retirar_pasados(&d, ahora),
            vec!["48-la-copia-prog-viejo.yaml"]
        );
        assert!(d.join("48-la-copia-prog-nuevo.yaml").exists());
        assert!(d.join("48-la-copia-rehacer-x.yaml").exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
