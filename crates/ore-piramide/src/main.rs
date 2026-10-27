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
use ore_piramide::{
    construir, cuenta_de, manifiesto, partir, peticion, retirar_viejas, subir, version_tippecanoe,
};
use ore_store::almacen::Almacen;
use std::sync::Arc;

#[cfg(target_env = "musl")]
#[global_allocator]
static ASIGNADOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn di(que: &str, j: Json) {
    println!("PIRAMIDE|{que}|{}", j.jcs());
}

/// Dónde dejar el fallo, si se llegó a saber el destino (F2·3·2·6: el motor lo lee y el
/// descriptor lo dice).
type Destino = Option<(Arc<dyn Almacen>, String)>;

fn correr(destino_de_fallo: &mut Destino) -> Result<(), String> {
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
    *destino_de_fallo = Some((destino.clone(), clave_d.to_string()));
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
    // F2·3·2·6: un fallo de antes ya no vale, y las pirámides viejas de la tabla se retiran
    let _ = destino.borrar(&format!("{clave_d}.fallo.json"));
    match retirar_viejas(&destino, clave_d) {
        Ok(b) => di("retiradas", Json::Arr(b.into_iter().map(Json::s).collect())),
        Err(e) => di("aviso", Json::obj([("retirar", Json::s(e))])),
    }
    di(
        "fin",
        Json::obj([("nuevo", Json::Bool(nuevo)), ("manifiesto", m)]),
    );
    Ok(())
}

fn main() {
    let mut destino = None;
    if let Err(e) = correr(&mut destino) {
        let cuando = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let fallo = Json::obj([
            ("error", Json::s(e)),
            ("cuando", Json::Int(cuando as i64)),
            ("ore_piramide", Json::s(ore_piramide::VERSION)),
        ]);
        if let Some((d, clave)) = destino {
            let _ = d.sobrescribir(&format!("{clave}.fallo.json"), fallo.jcs().as_bytes());
        }
        di("falla", fallo);
        std::process::exit(1);
    }
}
