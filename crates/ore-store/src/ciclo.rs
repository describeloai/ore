//! **El ciclo del almacén delegado**, fuera del compilador — lo que `ore-store-r2`
//! y `ore-store-gcs` tienen en común: todo menos el transporte.
//!
//! Normativo: [ADR 0015](../../../docs/decisions/0015-el-protocolo-del-almacen.md),
//! revisado por [0031 §10](../../../docs/decisions/0031-el-puesto.md) (W3.6a,
//! 2026-09-20: **la copia es un dataset**). Es la **tercera** vez que este
//! árbol delega, y por la misma razón que las dos anteriores: `ore` no puede
//! abrir un socket, y no por promesa — `ore-cli/tests/dependencias.rs` lee el
//! `Cargo.lock` y falla si aparece una crate de red, de TLS o de FFI en su
//! cierre.
//!
//! | | qué delega | ADR |
//! |---|---|---|
//! | `ore-read-<tipo>` | leer filas de un origen | 0008 |
//! | `ore-maintain` | correr el circuito Δ | 0013 |
//! | **`ore-store-<tipo>`** | **escribir el dataset y devolver su puntero** | **0015 · 0031 §10** |
//!
//! # El protocolo
//!
//! Hereda la línea de 0008 —*«la petición es un fragmento del plan, no SQL»*—
//! llevada a su sitio:
//!
//! > **Lo que viaja no son llamadas al almacén: son las filas y el puntero.**
//!
//! - **stdin**: la petición en JSON canónico, **una línea**, y después las filas,
//!   **una por línea**, como objetos JSON de cadenas. Por stdin y no por `argv`
//!   por lo mismo de siempre: `argv` lo lee cualquier proceso, y una fila es un
//!   dato;
//! - **stdout**: una línea JSON con el `metadata_location` nuevo, el snapshot y
//!   las cuentas;
//! - **stderr**: lo que haya que contar.
//!
//! Este programa **no sabe qué es una entidad, ni un conducto, ni una vista.**
//! Recibe una cabecera y un flujo de filas, y escribe una tabla Iceberg.
//!
//! # Lo que cambió con el dataset (W3.6a)
//!
//! Hasta aquí el almacén guardaba **el estado**: un recibo por cabecera decía
//! qué artefacto era el vigente, y `buscar`/`anterior` lo consultaban. Desde
//! W3.6a el estado vive **en el árbol** —`copias/<p>_<v>.json` apunta al
//! `metadata.json` de la tabla— y el árbol es el catálogo: el commit del Job
//! es el *swap* y la forja el *compare-and-set*. Así que:
//!
//! - **`buscar`** ya no busca un recibo: comprueba que el puntero que el árbol
//!   trae sigue existiendo en el bucket;
//! - **`anterior`** desaparece: sobre qué fundir y desde dónde leer lo sabe el
//!   puntero, y lo decide `ore`;
//! - **`sellar`** recibe `dataset` y, si la tabla ya existe, `base` (su
//!   `metadata_location`) y `fundir`; devuelve el `metadata_location` nuevo.
//!   Rehacer es sobrescribir, y la historia queda en los snapshots;
//! - **`recoger`** expira snapshots y retira lo que ningún snapshot nombra;
//!   **`recoger-huerfanas`** retira los datasets que ningún puntero reclama y
//!   los sobres heredados que ningún puntero sigue nombrando;
//! - **`leer`** abre la tabla por `metadata_location` (o el sobre por `clave`,
//!   mientras quede alguno) y devuelve la cabecera y las filas, como siempre.

use crate::almacen::Almacen;
use crate::lago::{self, Lago, Operacion};
use crate::{carga, sobre};
use ore_core::json::Json;
use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::sync::Arc;

/// El `main` de los dos binarios: lee la petición, elige el verbo, y contesta
/// una línea. Quién guarda los bytes lo decide el que llama.
pub fn principal(cuenta: Arc<dyn Almacen>) -> std::process::ExitCode {
    let verbo = std::env::args().nth(1).unwrap_or_else(|| "sellar".into());
    match correr(&verbo, cuenta) {
        Ok(linea) => {
            println!("{linea}");
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn correr(verbo: &str, cuenta: Arc<dyn Almacen>) -> Result<String, String> {
    let mut texto = String::new();
    std::io::stdin()
        .read_to_string(&mut texto)
        .map_err(|e| format!("no se pudo leer la entrada: {e}"))?;

    let mut lineas = texto.lines().filter(|l| !l.trim().is_empty());
    let primera = lineas
        .next()
        .ok_or("la entrada está vacía: se esperaba la petición en la primera línea")?;
    let n =
        ore_core::parse::parse(primera).map_err(|e| format!("la petición no analiza: {e:?}"))?;
    let campo = |k: &str| -> Option<String> {
        n.get(k)
            .and_then(|(_, v)| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    let bandera = |k: &str| campo(k).is_some_and(|v| v == "true");
    let lago = Lago::nuevo(cuenta.clone());

    match verbo {
        // **El paso 4.** El puntero lo trae el árbol; aquí sólo se comprueba
        // que lo que apunta sigue en el bucket (un HEAD), para no decir «ya
        // está» de una copia que alguien vació.
        "buscar" => {
            let ml = campo("metadata_location")
                .ok_or("a `buscar` le falta `metadata_location`: el puntero del árbol")?;
            let clave = lago.clave(&ml).map_err(|e| e.to_string())?;
            Ok(Json::obj([("existe", Json::Bool(cuenta.existe(&clave)?))]).jcs())
        }
        "sellar" => {
            let cab = leer_cabecera(primera)?;
            let dataset = campo("dataset").ok_or(
                "a `sellar` le falta `dataset`: bajo qué nombre vive la tabla (`copias/<p>_<v>`)",
            )?;
            sellar(
                &lago,
                &cab,
                &dataset,
                campo("base").as_deref(),
                bandera("fundir"),
                lineas,
            )
        }
        "recoger" | "recoger-seco" => {
            let dataset = campo("dataset").ok_or("a `recoger` le falta `dataset`")?;
            let ml = campo("metadata_location")
                .ok_or("a `recoger` le falta `metadata_location`: el puntero vigente")?;
            let edad = campo("edad_ms").and_then(|v| v.parse::<i64>().ok());
            recoger(&lago, &dataset, &ml, edad, verbo == "recoger-seco")
        }
        "recoger-huerfanas" => recoger_huerfanas(&lago, &n),
        "leer" => leer(&lago, &n),
        otro => Err(format!(
            "verbo desconocido `{otro}`: hace `buscar`, `sellar`, `recoger`, `recoger-seco`, \
             `recoger-huerfanas` y `leer`"
        )),
    }
}

/// **Sella el dataset**: las filas que llegan, tipadas con el contrato de 0032,
/// van a la tabla Iceberg de `dataset`. Tres caminos, y el puntero decide cuál:
///
/// - **sin `base`**: la tabla no existe (primera copia, o un puntero heredado
///   que apunta a un sobre): se crea y se escribe en UN `metadata.json`;
/// - **con `base` y `fundir`**: un refresco. Lo que había se lee de la tabla,
///   se funde con el incremento por la clave, y el resultado **sobrescribe**
///   (un snapshot que sustituye todos los ficheros: los 10 M de la medida
///   tardan 3,8 s en reescribirse, y el snapshot anterior sigue ahí);
/// - **con `base` y sin `fundir`**: rehacer, o un plan que cambió. Las filas
///   que llegan son la copia entera y sobrescriben.
///
/// Si el esquema del lote no es el de la tabla (una columna nueva, una que se
/// fue, una que cambió de tipo), la tabla lo adopta antes de escribir.
///
/// La cabecera —plan, esquema, testigo, clave, conducto— va como propiedades
/// del snapshot, y `leer` la devuelve tal cual: el dataset es autodescriptivo
/// como lo era el sobre.
fn sellar<'a>(
    lago: &Lago,
    cab: &sobre::Cabecera,
    dataset: &str,
    base: Option<&str>,
    fundir: bool,
    filas: impl Iterator<Item = &'a str>,
) -> Result<String, String> {
    let llegadas: Vec<carga::Fila> = filas
        .map(objeto_plano)
        .collect::<Result<Vec<_>, String>>()?;

    let previa = match base {
        Some(b) => Some(lago.abrir(b, dataset)?),
        None => None,
    };

    // **La fusión, y la trampa de determinismo que trae debajo.** Sin clave no
    // hay con qué fundir, así que las filas van tal cual y una copia solo se
    // puede rehacer entera — la otra cara de `OOS2023`. Con clave se funde
    // siempre que haya base: `fundir` ordena por clave, y así una copia
    // rehecha entera y una refrescada dan los mismos ficheros para el mismo
    // estado.
    let filas = if fundir {
        if cab.clave.is_empty() {
            return Err(
                "se pidió fundir sobre la copia anterior y la cabecera no declara `clave`: sin \
                 ella no se sabe qué fila sustituye a cuál"
                    .into(),
            );
        }
        let Some(t) = &previa else {
            return Err("se pidió fundir y no hay `base` sobre la que fundir".into());
        };
        carga::fundir(lago.filas(t)?, llegadas, &cab.clave)
    } else if cab.clave.is_empty() {
        llegadas
    } else {
        carga::fundir(Vec::new(), llegadas, &cab.clave)
    };

    // **Cuántas filas traen cada columna.** El informe decía «32 951 filas,
    // copiada» de una copia con 2 de 9 columnas vacías (medida W1 §B). Se
    // cuenta por columna, y lo que salga vacío se ve en el informe.
    let columnas = Json::Obj(
        cab.esquema
            .keys()
            .map(|c| {
                let n = filas.iter().filter(|f| f.contains_key(c)).count();
                (c.clone(), Json::Int(n as i64))
            })
            .collect(),
    );
    let carga::Lote {
        lote,
        sin_estrechar,
    } = carga::lote(&cab.esquema, &filas)?;

    // El esquema que el lote pide, con los ids de la tabla si la hay.
    let deseado = lago::esquema_deseado(
        &lago::columnas_de(&lote),
        previa
            .as_ref()
            .map(|t| t.metadata().current_schema().as_ref()),
    )?;
    let mut propiedades_snapshot = HashMap::from([
        (lago::PROP_CABECERA.to_string(), cab.jcs()),
        (lago::PROP_PLAN.to_string(), cab.plan.clone()),
        (
            lago::PROP_TESTIGO_MODO.to_string(),
            cab.testigo.modo.clone(),
        ),
    ]);
    if let Some(v) = &cab.testigo.valor {
        propiedades_snapshot.insert(lago::PROP_TESTIGO_VALOR.to_string(), v.clone());
    }

    let (tabla, operacion, esquema_cambiado) = match previa {
        Some(t) => {
            let (t, cambiado) = lago.esquema(&t, deseado)?;
            (t, Operacion::Sobrescribir, cambiado)
        }
        None => {
            let t = lago.crear(
                dataset,
                deseado,
                HashMap::from([
                    (lago::PROP_CABECERA.to_string(), cab.jcs()),
                    ("ore.conducto".to_string(), cab.conducto.clone()),
                ]),
            )?;
            (t, Operacion::Anexar, false)
        }
    };
    let escrito = lago.instantanea(&tabla, lote, operacion, propiedades_snapshot)?;
    let t = &escrito.tabla;

    Ok(Json::obj([
        ("bytes", Json::Int(escrito.bytes as i64)),
        ("columnas", columnas),
        ("esquema_cambiado", Json::Bool(esquema_cambiado)),
        ("ficheros", Json::Int(escrito.ficheros as i64)),
        ("filas", Json::Int(escrito.filas as i64)),
        (
            "metadata_location",
            Json::s(t.metadata_location().unwrap_or_default()),
        ),
        (
            "operacion",
            Json::s(match (operacion, base) {
                (Operacion::Anexar, _) => "creada",
                (Operacion::Sobrescribir, _) if fundir => "refrescada",
                (Operacion::Sobrescribir, _) => "sobrescrita",
            }),
        ),
        ("retirados", Json::Int(escrito.retirados as i64)),
        // Las columnas que la tabla de 0032 quería estrechar y se quedaron como
        // texto porque un valor no analizó, con el porqué. Vacío es lo que el
        // contrato promete; lo que haya va al informe tal cual.
        (
            "sin_estrechar",
            Json::Obj(
                sin_estrechar
                    .iter()
                    .map(|(c, p)| (c.clone(), Json::s(p)))
                    .collect(),
            ),
        ),
        (
            "snapshot",
            Json::s(
                t.metadata()
                    .current_snapshot_id()
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
            ),
        ),
        ("ubicacion", Json::s(t.metadata().location())),
    ])
    .jcs())
}

/// **`leer`: la copia, de vuelta, fila a fila** (0029 ③ «traer», F4a·I1).
///
/// La entrada es **el puntero**: `{"metadata_location": "…"}` para un dataset,
/// `{"clave": "ore/v1/<sha256>"}` para un sobre heredado mientras quede alguno.
/// La salida es la cabecera en una línea —la misma que `ore` selló, que el
/// dataset guarda como propiedad de su snapshot— y después **las filas, una
/// por línea**, como objetos JSON de cadenas: el protocolo de 0008 en sentido
/// contrario. Quien lee sabe así **qué copia** leyó y puede dejarlo escrito en
/// lo que produzca.
fn leer(lago: &Lago, n: &ore_core::parse::Node) -> Result<String, String> {
    let campo = |k: &str| {
        n.get(k)
            .and_then(|(_, v)| v.as_str())
            .filter(|c| !c.is_empty())
            .map(String::from)
    };
    let (cabecera, filas) = if let Some(ml) = campo("metadata_location") {
        let dataset = campo("dataset").unwrap_or_else(|| "dataset".into());
        let t = lago.abrir(&ml, &dataset)?;
        let cab = Lago::propiedad(&t, lago::PROP_CABECERA)
            .ok_or_else(|| format!("`{ml}` no lleva la cabecera de ORE (`{}`): no es un dataset que este programa haya sellado", lago::PROP_CABECERA))?;
        (cab, lago.filas(&t)?)
    } else if let Some(clave) = campo("clave") {
        let cuenta = lago_cuenta(lago)?;
        let bytes = cuenta
            .leer_bytes(&clave)?
            .ok_or_else(|| format!("`{clave}` no está en el almacén"))?;
        let (cabecera, payload) = sobre::abrir(&bytes)?;
        (cabecera, carga::leer(payload)?)
    } else {
        return Err(
            "a la petición le falta `metadata_location` (el puntero del dataset) o `clave` (un sobre heredado)"
                .into(),
        );
    };
    let mut out = String::with_capacity(cabecera.len() + filas.len() * 64);
    out.push_str(&cabecera);
    for f in &filas {
        out.push('\n');
        out.push_str(&Json::Obj(f.iter().map(|(k, v)| (k.clone(), Json::s(v))).collect()).jcs());
    }
    Ok(out)
}

fn lago_cuenta(lago: &Lago) -> Result<Arc<dyn Almacen>, String> {
    lago.cuenta_publica()
}

/// **La recogida de basura de un dataset**, y por qué es explícita.
///
/// Un snapshot superado **sigue siendo cierto hasta su marca**, y alguien puede
/// estar leyéndolo por su id (`iceberg_scan(…, snapshot_from_id)`). Así que se
/// expira cuando alguien lo pide, y `recoger-seco` dice antes qué se iría.
///
/// Dos pasos: expirar los snapshots que no son el vigente y son más viejos que
/// `edad_ms` (sin `edad_ms`, todos los superados: el sentido que `recoger`
/// tuvo siempre), y retirar del bucket lo que ningún snapshot que quede nombra
/// —ficheros de los expirados, y lo que dejó una pasada que no llegó a
/// apuntarse en el árbol—. Si expiró alguno hay un `metadata.json` nuevo, y se
/// devuelve: **el puntero tiene que moverse a él**.
fn recoger(
    lago: &Lago,
    dataset: &str,
    metadata_location: &str,
    edad_ms: Option<i64>,
    seco: bool,
) -> Result<String, String> {
    let tabla = lago.abrir(metadata_location, dataset)?;
    let antes = tabla.metadata().snapshots().count();
    let (tabla, expirados) = if seco {
        let corte = lago::ahora_ms() - edad_ms.unwrap_or(0);
        let cur = tabla.metadata().current_snapshot_id();
        let ids: Vec<i64> = tabla
            .metadata()
            .snapshots()
            .filter(|s| Some(s.snapshot_id()) != cur && s.timestamp_ms() <= corte)
            .map(|s| s.snapshot_id())
            .collect();
        (tabla, ids)
    } else {
        lago.expirar(&tabla, edad_ms.unwrap_or(0))?
    };
    let ficheros = lago.huerfanos(&tabla, seco)?;
    Ok(Json::obj([
        ("expirados", Json::Int(expirados.len() as i64)),
        ("ficheros", Json::Int(ficheros as i64)),
        (
            "metadata_location",
            Json::s(tabla.metadata_location().unwrap_or(metadata_location)),
        ),
        ("seco", Json::Bool(seco)),
        ("snapshots", Json::Int(antes as i64)),
    ])
    .jcs())
}

/// **Lo que ningún puntero reclama.** `recoger` limpia DENTRO de un dataset
/// vigente; esto retira los datasets **cuyo puntero ya no está en el árbol**
/// —la base se retiró, la vista dejó de declarar copia— y los sobres heredados
/// (`ore/v1/`) que ningún puntero sigue nombrando. La entrada es una línea JSON
/// con lo que el árbol reclama —TODOS los datasets con puntero, no sólo los de
/// esta pasada— y `seco` para decir qué se iría sin tocar nada:
///
/// ```text
/// {"datasets": ["copias/ventas_pedidos", …], "claves": ["ore/v1/…", …], "seco": false}
/// ```
fn recoger_huerfanas(lago: &Lago, n: &ore_core::parse::Node) -> Result<String, String> {
    let cuenta = lago_cuenta(lago)?;
    let seco = n
        .get("seco")
        .and_then(|(_, v)| v.as_str())
        .is_some_and(|v| v == "true");
    let lista = |k: &str| -> std::collections::BTreeSet<String> {
        n.get(k)
            .map(|(_, v)| v.items())
            .unwrap_or(&[])
            .iter()
            .filter_map(|p| p.as_str())
            .map(String::from)
            .collect()
    };
    let datasets = lista("datasets");
    let claves = lista("claves");

    // Los datasets del bucket: `ore/v2/<clase>/<nombre>/…` → `<clase>/<nombre>`.
    let raiz = format!("{}/", lago::RAIZ);
    let mut huerfanos: std::collections::BTreeSet<String> = Default::default();
    let mut objetos = 0usize;
    for k in cuenta.listar(&raiz)? {
        let resto = &k[raiz.len()..];
        let mut partes = resto.splitn(3, '/');
        let (Some(clase), Some(nombre)) = (partes.next(), partes.next()) else {
            continue;
        };
        let dataset = format!("{clase}/{nombre}");
        if datasets.contains(&dataset) {
            continue;
        }
        huerfanos.insert(dataset);
        if !seco {
            cuenta.borrar(&k)?;
        }
        objetos += 1;
    }
    // Y lo heredado: sobres y recibos de `ore/v1/`. Los recibos no los lee ya
    // nadie; un sobre sólo se queda si un puntero todavía lo nombra.
    let mut heredados = 0usize;
    for k in cuenta.listar("ore/v1/")? {
        if claves.contains(&k) {
            continue;
        }
        if !seco {
            cuenta.borrar(&k)?;
        }
        heredados += 1;
    }
    Ok(Json::obj([
        ("datasets", Json::Int(datasets.len() as i64)),
        ("heredados", Json::Int(heredados as i64)),
        ("huerfanos", Json::Int(huerfanos.len() as i64)),
        ("objetos", Json::Int(objetos as i64)),
        ("seco", Json::Bool(seco)),
    ])
    .jcs())
}

/// La cabecera, leída con el analizador del núcleo — el mismo que lee YAML, que
/// es un superconjunto de JSON. No entra un analizador más para esto. Los
/// campos de la petición que no son de la cabecera (`dataset`, `base`,
/// `fundir`) simplemente no se leen: la cabecera dice QUÉ CONTIENE la copia, y
/// no cómo se construyó.
fn leer_cabecera(linea: &str) -> Result<sobre::Cabecera, String> {
    let n = ore_core::parse::parse(linea).map_err(|e| format!("la cabecera no analiza: {e:?}"))?;
    let s = |k: &str| -> Result<String, String> {
        n.get(k)
            .and_then(|(_, v)| v.as_str())
            .map(String::from)
            .ok_or_else(|| format!("a la cabecera le falta `{k}`"))
    };
    let esquema = n
        .get("esquema")
        .map(|(_, e)| {
            e.entries()
                .iter()
                .filter_map(|(k, v)| Some((k.as_str()?.to_string(), v.as_str()?.to_string())))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    if esquema.is_empty() {
        return Err("la cabecera no declara `esquema`: sin él no hay tabla que escribir".into());
    }
    let testigo = n.get("testigo").map(|(_, t)| sobre::Testigo {
        modo: t
            .get("modo")
            .and_then(|(_, v)| v.as_str())
            .unwrap_or("none")
            .to_string(),
        valor: t
            .get("valor")
            .and_then(|(_, v)| v.as_str())
            .map(String::from),
    });
    Ok(sobre::Cabecera {
        plan: s("plan")?,
        esquema,
        testigo: testigo.unwrap_or_default(),
        clave: n
            .get("clave")
            .map(|(_, v)| {
                v.items()
                    .iter()
                    .filter_map(|i| i.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        conducto: s("conducto")?,
    })
}

/// Una fila: un objeto de cadenas y nada más. Un valor que no sea escalar es un
/// defecto de quien la produjo — y se dice, en vez de aplanarlo.
fn objeto_plano(linea: &str) -> Result<carga::Fila, String> {
    let n = ore_core::parse::parse(linea).map_err(|e| format!("una fila no analiza: {e:?}"))?;
    let mut out = BTreeMap::new();
    for (k, v) in n.entries() {
        let Some(nombre) = k.as_str() else { continue };
        match v.as_str() {
            Some(x) => {
                out.insert(nombre.to_string(), x.to_string());
            }
            None => {
                return Err(format!(
                    "`{nombre}` no es un escalar: una fila es un objeto plano, y aplanarlo aquí \
                     inventaría una codificación que nadie declaró"
                ));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Un almacén en memoria: lo justo para que el ciclo se pruebe sin red.
    #[derive(Default)]
    pub struct Memoria(pub Mutex<BTreeMap<String, Vec<u8>>>);

    impl Almacen for Memoria {
        fn base(&self) -> String {
            "memory://pruebas".into()
        }
        fn leer(&self, clave: &str) -> Result<Option<String>, String> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .get(clave)
                .map(|b| String::from_utf8_lossy(b).into_owned()))
        }
        fn existe(&self, clave: &str) -> Result<bool, String> {
            Ok(self.0.lock().unwrap().contains_key(clave))
        }
        // Como los de verdad: si estaba, no se toca (`If-None-Match: *`).
        fn subir(&self, clave: &str, cuerpo: &[u8]) -> Result<bool, String> {
            let mut m = self.0.lock().unwrap();
            if m.contains_key(clave) {
                return Ok(false);
            }
            m.insert(clave.to_string(), cuerpo.to_vec());
            Ok(true)
        }
        fn listar(&self, prefijo: &str) -> Result<Vec<String>, String> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .keys()
                .filter(|k| k.starts_with(prefijo))
                .cloned()
                .collect())
        }
        fn borrar(&self, clave: &str) -> Result<(), String> {
            self.0.lock().unwrap().remove(clave);
            Ok(())
        }
        fn leer_bytes(&self, clave: &str) -> Result<Option<Vec<u8>>, String> {
            Ok(self.0.lock().unwrap().get(clave).cloned())
        }
    }

    fn cabecera(testigo: &str) -> sobre::Cabecera {
        sobre::Cabecera {
            plan: "sha256:plan".into(),
            esquema: [
                ("id".to_string(), "Integer".to_string()),
                ("nombre".to_string(), "String".to_string()),
                ("total".to_string(), "Decimal".to_string()),
            ]
            .into(),
            testigo: sobre::Testigo {
                modo: "log".into(),
                valor: Some(testigo.into()),
            },
            clave: vec!["id".into()],
            conducto: "materialization.payload".into(),
        }
    }

    fn campo(linea: &str, k: &str) -> String {
        ore_core::parse::parse(linea)
            .unwrap()
            .get(k)
            .unwrap_or_else(|| panic!("falta `{k}` en {linea}"))
            .1
            .as_str()
            .unwrap()
            .to_string()
    }

    fn filas_de(lago: &Lago, ml: &str) -> Vec<carga::Fila> {
        lago.filas(&lago.abrir(ml, "copias/p_v").unwrap()).unwrap()
    }

    /// **El ciclo entero sobre un almacén en memoria**: crear, «ya está»,
    /// refrescar fundiendo, rehacer sobrescribiendo, leer, recoger. Es lo que
    /// `refresco.sh` mide contra un bucket de verdad, sin el bucket.
    #[test]
    fn crear_refrescar_sobrescribir_leer_y_recoger() {
        let cuenta: Arc<Memoria> = Arc::new(Memoria::default());
        let lago = Lago::nuevo(cuenta.clone());

        // ① crear: UN metadata.json, un fichero de datos, un manifiesto y su lista
        let s1 = sellar(
            &lago,
            &cabecera("7"),
            "copias/p_v",
            None,
            false,
            [
                "{\"id\":\"2\",\"nombre\":\"Bea\",\"total\":\"10.50\"}",
                "{\"id\":\"1\"}",
            ]
            .into_iter(),
        )
        .expect("sella");
        assert_eq!(campo(&s1, "operacion"), "creada");
        assert_eq!(campo(&s1, "filas"), "2");
        let ml1 = campo(&s1, "metadata_location");
        assert!(
            ml1.starts_with("memory://pruebas/ore/v2/copias/p_v/metadata/00000-"),
            "{ml1}"
        );
        assert_eq!(
            cuenta.0.lock().unwrap().len(),
            4,
            "{:?}",
            cuenta.0.lock().unwrap().keys()
        );
        let t1 = lago.abrir(&ml1, "copias/p_v").unwrap();
        assert_eq!(t1.metadata().snapshots().count(), 1);
        assert_eq!(
            Lago::propiedad(&t1, lago::PROP_TESTIGO_VALOR).as_deref(),
            Some("7")
        );
        let f = filas_de(&lago, &ml1);
        assert_eq!(f.len(), 2);
        assert_eq!(f[0]["id"], "1", "ordenadas por la clave");
        assert_eq!(f[1]["total"], "10.5", "el decimal vuelve canónico");

        // ② refrescar: 1 fila nueva y 1 que cambia, fundidas sobre la base
        let s2 = sellar(
            &lago,
            &cabecera("9"),
            "copias/p_v",
            Some(&ml1),
            true,
            [
                "{\"id\":\"3\",\"nombre\":\"Cai\"}",
                "{\"id\":\"2\",\"nombre\":\"Bea2\",\"total\":\"11\"}",
            ]
            .into_iter(),
        )
        .expect("refresca");
        assert_eq!(campo(&s2, "operacion"), "refrescada");
        assert_eq!(
            campo(&s2, "filas"),
            "3",
            "la copia entera, no el incremento"
        );
        assert_eq!(
            campo(&s2, "retirados"),
            "1",
            "el fichero anterior queda retirado"
        );
        let ml2 = campo(&s2, "metadata_location");
        assert!(ml2.contains("/metadata/00001-"), "{ml2}");
        let f = filas_de(&lago, &ml2);
        assert_eq!(f.len(), 3);
        assert_eq!(f[1]["nombre"], "Bea2");
        assert_eq!(f[1]["total"], "11");
        assert_eq!(f[2]["nombre"], "Cai");
        let t2 = lago.abrir(&ml2, "copias/p_v").unwrap();
        assert_eq!(t2.metadata().snapshots().count(), 2, "la historia se queda");
        assert_eq!(
            Lago::propiedad(&t2, lago::PROP_TESTIGO_VALOR).as_deref(),
            Some("9")
        );
        // y el snapshot anterior sigue siendo legible tal cual era
        let f1 = filas_de(&lago, &ml1);
        assert_eq!(f1.len(), 2);

        // ③ rehacer: las filas que llegan son la copia entera
        let s3 = sellar(
            &lago,
            &cabecera("9"),
            "copias/p_v",
            Some(&ml2),
            false,
            ["{\"id\":\"5\",\"nombre\":\"Eva\"}"].into_iter(),
        )
        .expect("rehace");
        assert_eq!(campo(&s3, "operacion"), "sobrescrita");
        assert_eq!(campo(&s3, "filas"), "1");
        let ml3 = campo(&s3, "metadata_location");
        assert_eq!(filas_de(&lago, &ml3).len(), 1);

        // ④ leer: la cabecera que se selló y las filas, una por línea
        let leido = leer(
            &lago,
            &ore_core::parse::parse(&format!("{{\"metadata_location\":\"{ml3}\"}}")).unwrap(),
        )
        .expect("lee");
        let lineas: Vec<&str> = leido.lines().collect();
        assert_eq!(lineas.len(), 2, "{leido}");
        assert_eq!(lineas[0], cabecera("9").jcs());
        assert_eq!(
            lineas[1], "{\"id\":\"5\",\"nombre\":\"Eva\"}",
            "el nulo no viaja"
        );

        // ⑤ recoger: expiran los dos snapshots superados y se van sus ficheros
        let antes = cuenta.0.lock().unwrap().len();
        let seco = recoger(&lago, "copias/p_v", &ml3, None, true).expect("seco");
        assert_eq!(campo(&seco, "expirados"), "2");
        assert_eq!(
            cuenta.0.lock().unwrap().len(),
            antes,
            "en seco no se toca nada"
        );
        let r = recoger(&lago, "copias/p_v", &ml3, None, false).expect("recoge");
        assert_eq!(campo(&r, "expirados"), "2");
        let ml4 = campo(&r, "metadata_location");
        assert_ne!(ml4, ml3, "expirar deja un metadata.json nuevo");
        let t4 = lago.abrir(&ml4, "copias/p_v").unwrap();
        assert_eq!(t4.metadata().snapshots().count(), 1);
        assert_eq!(filas_de(&lago, &ml4).len(), 1, "la vigente sigue entera");
        assert!(
            campo(&r, "ficheros").parse::<usize>().unwrap() >= 5,
            "los ficheros de los expirados se fueron: {r}"
        );
        // y recoger otra vez no mueve el puntero
        let r2 = recoger(&lago, "copias/p_v", &ml4, None, false).expect("recoge");
        assert_eq!(campo(&r2, "metadata_location"), ml4);
        assert_eq!(campo(&r2, "ficheros"), "0");
    }

    /// Un cambio de esquema —una columna nueva y otra que cambia de tipo— lo
    /// adopta la tabla antes de escribir, y la vieja sigue legible.
    #[test]
    fn el_esquema_evoluciona_con_el_lote() {
        let cuenta: Arc<Memoria> = Arc::new(Memoria::default());
        let lago = Lago::nuevo(cuenta);
        let s1 = sellar(
            &lago,
            &cabecera("1"),
            "copias/p_v",
            None,
            false,
            ["{\"id\":\"1\",\"nombre\":\"a\",\"total\":\"1\"}"].into_iter(),
        )
        .unwrap();
        let ml1 = campo(&s1, "metadata_location");
        let mut cab = cabecera("2");
        cab.esquema.insert("pais".into(), "String".into());
        cab.esquema.insert("total".into(), "Integer".into());
        let s2 = sellar(
            &lago,
            &cab,
            "copias/p_v",
            Some(&ml1),
            false,
            ["{\"id\":\"1\",\"nombre\":\"a\",\"total\":\"2\",\"pais\":\"ES\"}"].into_iter(),
        )
        .unwrap();
        assert_eq!(campo(&s2, "esquema_cambiado"), "true");
        let ml2 = campo(&s2, "metadata_location");
        let t2 = lago.abrir(&ml2, "copias/p_v").unwrap();
        let cols = lago::columnas_iceberg(&t2);
        assert_eq!(cols["pais"], "string");
        assert_eq!(cols["total"], "long");
        let esq = t2.metadata().current_schema();
        assert_eq!(esq.field_by_name("id").unwrap().id, 1, "conserva su id");
        assert!(
            esq.field_by_name("total").unwrap().id > 3,
            "otro tipo, otro id"
        );
        assert_eq!(t2.metadata().schemas_iter().count(), 2);
        let f = filas_de(&lago, &ml2);
        assert_eq!(f[0]["pais"], "ES");
        assert_eq!(f[0]["total"], "2");
    }

    /// Lo huérfano: un dataset sin puntero se va entero; el que se reclama, no.
    #[test]
    fn lo_que_ningun_puntero_reclama_se_va() {
        let cuenta: Arc<Memoria> = Arc::new(Memoria::default());
        let lago = Lago::nuevo(cuenta.clone());
        for d in ["copias/p_a", "copias/p_b"] {
            sellar(
                &lago,
                &cabecera("1"),
                d,
                None,
                false,
                ["{\"id\":\"1\"}"].into_iter(),
            )
            .unwrap();
        }
        cuenta.subir("ore/v1/viejo", b"ORECOPY1...").unwrap();
        cuenta.subir("ore/v1/plan/x/y", b"ore/v1/viejo").unwrap();
        let n = ore_core::parse::parse(
            "{\"datasets\":[\"copias/p_a\"],\"claves\":[\"ore/v1/viejo\"],\"seco\":false}",
        )
        .unwrap();
        let r = recoger_huerfanas(&lago, &n).unwrap();
        assert_eq!(campo(&r, "huerfanos"), "1");
        assert_eq!(
            campo(&r, "heredados"),
            "1",
            "el recibo se va, el sobre nombrado se queda"
        );
        let claves: Vec<String> = cuenta.0.lock().unwrap().keys().cloned().collect();
        assert!(
            claves.iter().all(|k| !k.contains("copias/p_b/")),
            "{claves:?}"
        );
        assert!(claves.iter().any(|k| k.contains("copias/p_a/")));
        assert!(claves.contains(&"ore/v1/viejo".to_string()));
        assert!(!claves.contains(&"ore/v1/plan/x/y".to_string()));
    }

    #[test]
    fn leer_lo_que_no_esta_lo_dice() {
        let lago = Lago::nuevo(Arc::new(Memoria::default()));
        let n = ore_core::parse::parse("{\"clave\":\"ore/v1/nadie\"}").unwrap();
        let e = leer(&lago, &n).unwrap_err();
        assert!(e.contains("no está en el almacén"), "{e}");
        let n = ore_core::parse::parse("{\"plan\":\"x\"}").unwrap();
        let e = leer(&lago, &n).unwrap_err();
        assert!(e.contains("le falta `metadata_location`"), "{e}");
    }
}
