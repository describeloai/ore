//! **El catálogo de una prueba de `ore`** (0079 ·5g·3 R3b): los punteros viven en el catálogo
//! transaccional, y sólo ahí. Como en un Job, `ore` habla con él por `ORE_PUNTEROS`
//! (`ore-catalogo punteros --esquema …`), y la prueba pone y lee los punteros por la misma puerta,
//! no como ficheros.
//!
//! Cada [`Catalogo`] es un esquema propio en el Postgres de `ORE_CATALOGO_PRUEBAS` (en el CI, el
//! del job), que se borra al soltarse. Hacen falta `psql` (para crear y borrar el esquema) y el
//! binario `ore-catalogo` junto a los de la prueba (`cargo build -p ore-catalogo`, o
//! `CATALOGO_BIN`); sin ellos, la prueba falla diciéndolo, no se salta.
//!
//! Lo comparten las pruebas de integración (`#[path]`) y las de dentro del binario.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

use ore_core::json::Json;
use ore_core::punteros::protocolo::Proceso;
use ore_core::punteros::{Cambio, Contexto, Esperado, Punteros, Que, Ramas};

pub struct Catalogo {
    url: String,
    pub esquema: String,
    bin: PathBuf,
}

impl Catalogo {
    /// Un esquema nuevo para la prueba `nombre`, con el catálogo aplicado (lo aplica la primera
    /// apertura de la puerta).
    pub fn nuevo(nombre: &str) -> Catalogo {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let url = std::env::var("ORE_CATALOGO_PRUEBAS")
            .ok()
            .filter(|u| !u.trim().is_empty())
            .unwrap_or_else(|| {
                panic!(
                    "⛔ sin ORE_CATALOGO_PRUEBAS: los punteros viven en el catálogo, y esta prueba necesita uno"
                )
            });
        let bin = std::env::var("CATALOGO_BIN")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                // Las pruebas viven en `target/<perfil>/deps/`; los binarios, un nivel arriba.
                let yo = std::env::current_exe().unwrap();
                yo.parent().unwrap().parent().unwrap().join("ore-catalogo")
            });
        assert!(
            bin.is_file(),
            "⛔ no hay binario de `ore-catalogo` en {} (cargo build -p ore-catalogo, o CATALOGO_BIN)",
            bin.display()
        );
        let limpio: String = nombre
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .collect();
        let esquema = format!(
            "t_c_{limpio}_{}_{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        );
        psql(&url, &format!("CREATE SCHEMA {esquema}"));
        Catalogo { url, esquema, bin }
    }

    /// La orden de la puerta, lo que va en `ORE_PUNTEROS`.
    pub fn orden(&self) -> String {
        format!(
            "env CATALOGO_URL={} {} punteros --esquema {}",
            self.url,
            self.bin.display(),
            self.esquema
        )
    }

    /// `ore` (o quien sea) con esta puerta.
    pub fn en<'a>(&self, c: &'a mut Command) -> &'a mut Command {
        c.env("ORE_PUNTEROS", self.orden())
    }

    /// La puerta, sobre `raiz` y `rama` (sin ella, `main`): la misma que abre `ore`.
    pub fn puerta(&self, raiz: &Path, rama: Option<&str>) -> Proceso {
        Proceso::hijo(&self.orden(), raiz, None, None, rama.map(String::from))
    }

    /// Pone el puntero de `nombre` (un objeto JSON) en `rama`.
    pub fn poner(&self, nombre: &str, texto: &str, rama: Option<&str>) {
        let n = ore_core::parse::parse(texto).unwrap_or_else(|e| panic!("{nombre}: {e:?}"));
        self.puerta(Path::new("."), rama)
            .escribir(
                &[Cambio {
                    nombre: nombre.into(),
                    que: Que::Sustituir(Json::de_node_fiel(&n)),
                    esperado: Esperado::Cualquiera,
                }],
                &Contexto {
                    autor: "prueba".into(),
                    operacion: "prueba".into(),
                    idempotencia: None,
                },
            )
            .unwrap_or_else(|e| panic!("no se pudo poner el puntero de {nombre}: {e}"));
    }

    /// El puntero de `nombre` en `rama`, como lo guarda el catálogo; `None` si no lo hay.
    pub fn leer(&self, nombre: &str, rama: Option<&str>) -> Option<String> {
        self.puerta(Path::new("."), rama)
            .uno(nombre)
            .unwrap_or_else(|e| panic!("no se pudo leer el puntero de {nombre}: {e}"))
            .map(|l| l.texto)
    }

    /// Un campo escalar del puntero de `nombre` en `main`, como texto.
    pub fn campo(&self, nombre: &str, k: &str) -> Option<String> {
        let n = ore_core::parse::parse(&self.leer(nombre, None)?).ok()?;
        n.get(k).and_then(|(_, v)| v.as_str().map(String::from))
    }

    /// El puntero de `nombre`, como JSON (sin `intento`, que lleva la hora).
    pub fn json(&self, nombre: &str, rama: Option<&str>) -> Option<Json> {
        let t = self.leer(nombre, rama)?;
        match Json::de_node_fiel(&ore_core::parse::parse(&t).unwrap()) {
            Json::Obj(mut m) => {
                m.remove("intento");
                Some(Json::Obj(m))
            }
            otro => Some(otro),
        }
    }

    /// Nace `rama` desde `main`.
    pub fn crear_rama(&self, rama: &str) {
        self.puerta(Path::new("."), None)
            .crear(rama, "main")
            .unwrap_or_else(|e| panic!("no se pudo crear la rama {rama}: {e}"));
    }
}

impl Drop for Catalogo {
    fn drop(&mut self) {
        let _ = Command::new("psql")
            .args([self.url.as_str(), "-qX", "-c"])
            .arg(format!("DROP SCHEMA IF EXISTS {} CASCADE", self.esquema))
            .output();
    }
}

fn psql(url: &str, sql: &str) {
    let s = Command::new("psql")
        .args([url, "-qX", "-v", "ON_ERROR_STOP=1", "-c", sql])
        .output()
        .unwrap_or_else(|e| panic!("⛔ sin psql: hace falta para el esquema de la prueba ({e})"));
    assert!(
        s.status.success(),
        "⛔ psql `{sql}`: {}",
        String::from_utf8_lossy(&s.stderr)
    );
}
