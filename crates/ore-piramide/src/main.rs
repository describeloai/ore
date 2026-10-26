//! **`ore-piramide`** (ADR 0072 F2·3·2·2): construye la pirámide de una capa y la sube.
//!
//!   ore-piramide < peticion.json        (o ORE_PIRAMIDE='{…}')
//!
//! La petición: `{"fuente": {"metadata_location", "geometria", "id", "crs", "credencial"?},
//! "capa", "suelo"? (sin él, el del tipo: 13, o 14 para líneas), "tipo"?, "destino": {"ubicacion": "gs://…/x.pmtiles", "credencial"?},
//! "limites": {"segundos", "celdas"}, "trabajo"}`. Sin credencial, la identidad del pod. Cuenta lo
//! que hace en líneas `PIRAMIDE|que|{json}`; si falla, `PIRAMIDE|falla|…` y sale con 1.

use std::io::Read;
use std::time::Instant;

use ore_base::json::Json;
use ore_piramide::{construir, cuenta_de, manifiesto, partir, peticion, subir, version_tippecanoe};

#[cfg(target_env = "musl")]
#[global_allocator]
static ASIGNADOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn di(que: &str, j: Json) {
    println!("PIRAMIDE|{que}|{}", j.jcs());
}

fn correr() -> Result<(), String> {
    let texto = match std::env::var("ORE_PIRAMIDE") {
        Ok(t) => t,
        Err(_) => {
            let mut t = String::new();
            std::io::stdin()
                .read_to_string(&mut t)
                .map_err(|e| e.to_string())?;
            t
        }
    };
    let p = peticion(&texto)?;
    let (cubo, _) = partir(&p.metadata_location)?;
    let fuente = cuenta_de(cubo, p.credencial.as_deref())?;
    let (cubo_d, clave_d) = partir(&p.destino.ubicacion)?;
    let destino = cuenta_de(cubo_d, p.destino.token.as_deref())?;
    // El manifiesto se sube lo último: si está, la pirámide está entera (el Job que Flux
    // recrea al caducar no rehace nada).
    let manifiesto_d = format!("{clave_d}.json");
    if destino.existe(&manifiesto_d)? {
        di(
            "ya",
            Json::obj([("destino", Json::s(p.destino.ubicacion.clone()))]),
        );
        return Ok(());
    }
    let tip = version_tippecanoe(&p.tippecanoe);
    di(
        "inicio",
        Json::obj([
            ("ore_piramide", Json::s(ore_piramide::VERSION)),
            ("tippecanoe", Json::s(tip.clone())),
            ("instantanea", Json::s(p.metadata_location.clone())),
            ("destino", Json::s(p.destino.ubicacion.clone())),
            (
                "suelo",
                p.suelo.map_or(Json::s("por tipo"), |z| Json::Int(z as i64)),
            ),
        ]),
    );
    std::fs::create_dir_all(&p.trabajo).map_err(|e| e.to_string())?;
    let salida = p.trabajo.join("piramide.pmtiles");
    let _ = std::fs::remove_file(&salida);
    let h = construir(&fuente, &p, &salida, &mut di)?;
    let t = Instant::now();
    let nuevo = subir(&destino, clave_d, &salida)?;
    let subir_s = t.elapsed().as_secs_f64();
    let m = manifiesto(&p, &h, &tip, subir_s);
    destino.sobrescribir(&manifiesto_d, m.jcs().as_bytes())?;
    let _ = std::fs::remove_file(&salida);
    di(
        "fin",
        Json::obj([("nuevo", Json::Bool(nuevo)), ("manifiesto", m)]),
    );
    Ok(())
}

fn main() {
    if let Err(e) = correr() {
        di("falla", Json::obj([("error", Json::s(e))]));
        std::process::exit(1);
    }
}
