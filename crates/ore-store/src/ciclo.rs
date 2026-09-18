//! **El ciclo del almacén delegado**, fuera del compilador — lo que `ore-store-r2`
//! y `ore-store-gcs` tienen en común: todo menos el transporte.
//!
//! Normativo: [ADR 0015](../../../docs/decisions/0015-el-protocolo-del-almacen.md).
//! Es la **tercera** vez que este árbol delega, y por la misma razón que las dos
//! anteriores: `ore` no puede abrir un socket, y no por promesa —
//! `ore-cli/tests/dependencias.rs` lee el `Cargo.lock` y falla si aparece una
//! crate de red, de TLS o de FFI en su cierre.
//!
//! | | qué delega | ADR |
//! |---|---|---|
//! | `ore-read-<tipo>` | leer filas de un origen | 0008 |
//! | `ore-maintain` | correr el circuito Δ | 0013 |
//! | **`ore-store-<tipo>`** | **sellar y subir el artefacto** | **0015** |
//!
//! # El protocolo
//!
//! Hereda la línea de 0008 —*«la petición es un fragmento del plan, no SQL»*—
//! llevada a su sitio:
//!
//! > **Lo que viaja no son llamadas al almacén: es el artefacto.**
//!
//! - **stdin**: la cabecera del sobre en JSON canónico, **una línea**, y después
//!   las filas, **una por línea**, como objetos JSON de cadenas. Por stdin y no
//!   por `argv` por lo mismo de siempre: `argv` lo lee cualquier proceso, y una
//!   fila es un dato;
//! - **stdout**: una línea JSON con el nombre, el digest, el tamaño y **si hizo
//!   falta subir**;
//! - **stderr**: lo que haya que contar.
//!
//! Este programa **no sabe qué es una entidad, ni un conducto, ni una vista.**
//! Recibe una cabecera y un flujo de filas.
//!
//! # Y lo que decide si sube
//!
//! El nombre **es** el contenido, así que:
//!
//! - re-materializar con el mismo testigo da el mismo nombre y **no sube ni un
//!   byte** — lo dice `subido: false`;
//! - dos escritores que lleguen a la vez escriben los mismos bytes, así que la
//!   carrera es inofensiva.

use crate::almacen::Almacen;
use crate::{carga, sobre};
use std::collections::BTreeMap;
use std::io::Read;

/// El `main` de los dos binarios: lee la cabecera, elige el verbo, y contesta
/// una línea. Quién guarda los bytes lo decide el que llama.
pub fn principal(cuenta: &dyn Almacen) -> std::process::ExitCode {
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

fn correr(verbo: &str, cuenta: &dyn Almacen) -> Result<String, String> {
    let mut texto = String::new();
    std::io::stdin()
        .read_to_string(&mut texto)
        .map_err(|e| format!("no se pudo leer la entrada: {e}"))?;

    let mut lineas = texto.lines().filter(|l| !l.trim().is_empty());
    let cabecera = lineas
        .next()
        .ok_or("la entrada está vacía: se esperaba la cabecera en la primera línea")?;
    // `leer` no lleva cabecera: lleva el nombre de lo que quiere leer.
    if verbo == "leer" {
        return leer(cuenta, cabecera);
    }
    // `recoger-huerfanas` tampoco: lleva los planes que el árbol reclama.
    if verbo == "recoger-huerfanas" {
        return recoger_huerfanas(cuenta, cabecera);
    }
    let cab = leer_cabecera(cabecera)?;
    let recibo = sobre::recibo(&cab);

    match verbo {
        // **El paso 4, y aquí sí ahorra la lectura.** La cabecera se conoce
        // antes de pedirle una fila a nadie, así que este `GET` de 71 bytes
        // decide si hay que leer el origen entero.
        "buscar" => {
            let hay = cuenta.leer(&recibo)?;
            Ok(ore_core::json::Json::obj([
                (
                    "clave",
                    match &hay {
                        Some(k) => ore_core::json::Json::s(k),
                        None => ore_core::json::Json::s(""),
                    },
                ),
                ("existe", ore_core::json::Json::Bool(hay.is_some())),
                ("recibo", ore_core::json::Json::s(&recibo)),
            ])
            .jcs())
        }
        "sellar" => {
            // `base` viaja en la línea de la petición y **no entra en la
            // cabecera**, y eso es deliberado: la cabecera dice QUÉ CONTIENE la
            // copia —plan y testigo— y no CÓMO se construyó. Si el camino
            // entrara, una copia rehecha entera y una refrescada tendrían
            // cabeceras distintas para el mismo estado, y el recibo dejaría de
            // reconocer que ya estaba.
            //
            // Se cae solo: `leer_cabecera` construye la `Cabecera` de campos
            // nombrados, así que un campo de más simplemente no se lee.
            let peticion = ore_core::parse::parse(cabecera).ok();
            let base = peticion.as_ref().and_then(|n| {
                n.get("base")
                    .and_then(|(_, v)| v.as_str())
                    .map(String::from)
            });
            // `rehacer` tampoco entra en la cabecera, por lo mismo: es cómo se
            // construyó, no qué contiene.
            let rehacer = peticion
                .as_ref()
                .and_then(|n| n.get("rehacer").and_then(|(_, v)| v.as_str()))
                .is_some_and(|v| v == "true");
            sellar(cuenta, cab, &recibo, base.as_deref(), rehacer, lineas)
        }
        "anterior" => anterior(cuenta, &cab, &recibo),
        "recoger" => recoger(cuenta, &cab, &recibo, false),
        "recoger-seco" => recoger(cuenta, &cab, &recibo, true),
        otro => Err(format!(
            "verbo desconocido `{otro}`: hace `buscar`, `anterior`, `sellar`, `recoger`, \n             `recoger-seco`, `recoger-huerfanas` y `leer`"
        )),
    }
}

/// **Lo que ninguna vista reclama.** `recoger` borra las copias SUPERADAS de un
/// plan vigente; esto borra las de los planes que **ya no tienen vista**: la
/// base se retiró, o la vista dejó de declarar copia (medido el 2026-09-18 en
/// `demo`: recibos de planes que el árbol no reclama y artefactos sin recibo,
/// que nadie iba a borrar nunca). La entrada es una línea JSON con los planes
/// que el árbol reclama —TODOS los de las vistas con copia, no sólo los de
/// esta pasada— y `seco` para decir qué se iría sin tocar nada:
///
/// ```text
/// {"planes": ["sha256:…", …], "seco": false}
/// ```
///
/// Se borra el recibo y luego su artefacto, como en `recoger`; y después los
/// artefactos a los que ningún recibo apunta (una subida que se cortó).
fn recoger_huerfanas(cuenta: &dyn Almacen, entrada: &str) -> Result<String, String> {
    let n = ore_core::parse::parse(entrada).map_err(|e| format!("la entrada no analiza: {e:?}"))?;
    let seco = n
        .get("seco")
        .and_then(|(_, v)| v.as_str())
        .is_some_and(|v| v == "true");
    let planes: std::collections::BTreeSet<String> = n
        .get("planes")
        .map(|(_, v)| v.items())
        .unwrap_or(&[])
        .iter()
        .filter_map(|p| p.as_str())
        .map(|p| sobre::prefijo_de_plan(p))
        .collect();

    let recibos = cuenta.listar("ore/v1/plan/")?;
    let mut huerfanas = 0usize;
    let mut apuntados: std::collections::BTreeSet<String> = Default::default();
    for r in &recibos {
        // `ore/v1/plan/<plan>/<cabecera>` → su prefijo de plan
        let Some(prefijo) = r.rsplit_once('/').map(|(p, _)| p.to_string()) else {
            continue;
        };
        let artefacto = cuenta.leer(r)?;
        if planes.contains(&prefijo) {
            if let Some(a) = artefacto {
                apuntados.insert(a);
            }
            continue;
        }
        if !seco {
            cuenta.borrar(r)?;
            if let Some(a) = &artefacto {
                cuenta.borrar(a)?;
            }
        }
        huerfanas += 1;
    }
    // Los artefactos que ningún recibo vigente apunta.
    let mut sueltos = 0usize;
    for a in cuenta.listar("ore/v1/")? {
        if a.starts_with("ore/v1/plan/") || apuntados.contains(&a) {
            continue;
        }
        if !seco {
            cuenta.borrar(&a)?;
        }
        sueltos += 1;
    }
    Ok(ore_core::json::Json::obj([
        ("recibos", ore_core::json::Json::Int(recibos.len() as i64)),
        ("planes", ore_core::json::Json::Int(planes.len() as i64)),
        ("huerfanas", ore_core::json::Json::Int(huerfanas as i64)),
        ("sueltos", ore_core::json::Json::Int(sueltos as i64)),
        ("seco", ore_core::json::Json::Bool(seco)),
    ])
    .jcs())
}

/// Los pasos 5 y 6: sella el artefacto, lo sube, y **deja el recibo**.
///
/// El recibo va DESPUÉS del artefacto, y el orden importa: si se escribiera
/// antes y la subida fallara, el paso 4 diría que la copia está y no estaría.
/// Al revés, lo peor que pasa es repetir el trabajo — que es lo que hace este
/// programa idempotente en vez de frágil.
///
/// **Con `rehacer`** (W1, el recibo que mentía): el artefacto nuevo se sube
/// como siempre, y el recibo **se sobrescribe** para apuntar a él. El
/// artefacto al que apuntaba antes —misma cabecera, otros bytes— se borra: no
/// lo nombra ningún recibo y `recoger` no lo encontraría, porque recoge
/// recibos de cabeceras superadas y esta cabecera sigue vigente. Si los bytes
/// son los mismos, no hay nada que sobrescribir ni que borrar, y se dice.
fn sellar<'a>(
    cuenta: &dyn Almacen,
    cab: sobre::Cabecera,
    recibo: &str,
    base: Option<&str>,
    rehacer: bool,
    filas: impl Iterator<Item = &'a str>,
) -> Result<String, String> {
    let llegadas: Vec<carga::Fila> = filas
        .map(objeto_plano)
        .collect::<Result<Vec<_>, String>>()?;

    // **La fusión, y la trampa de determinismo que trae debajo.**
    //
    // Sin clave no hay con qué fundir, así que las filas van tal cual y una
    // copia solo se puede rehacer entera — la otra cara de `OOS2023`.
    //
    // Con clave se funde **siempre**, también cuando no hay base. No es simetría
    // gratuita: `fundir` ordena por clave, y si solo ordenara el camino
    // incremental, una copia rehecha entera y una refrescada darían **bytes
    // distintos para el mismo estado**. El artefacto dejaría de poder nombrarse
    // por su digest, que es la propiedad de la que cuelga todo lo demás.
    let filas = if cab.clave.is_empty() {
        if base.is_some() {
            return Err(
                "se pidió fundir sobre una copia anterior y la cabecera no declara \
                        `clave`: sin ella no se sabe qué fila sustituye a cuál"
                    .into(),
            );
        }
        llegadas
    } else {
        let anteriores = match base {
            Some(b) => {
                let bytes = cuenta
                    .leer_bytes(b)?
                    .ok_or_else(|| format!("la copia base `{b}` no está en el almacén"))?;
                let (_, payload) = sobre::abrir(&bytes)?;
                carga::leer(payload)?
            }
            None => Vec::new(),
        };
        carga::fundir(anteriores, llegadas, &cab.clave)
    };

    // **Cuántas filas traen cada columna.** El informe decía «32 951 filas,
    // copiada» de una copia con 2 de 9 columnas vacías (medida W1 §B: el
    // driver de Postgres convertía en nulo lo que no sabía leer). Las filas no
    // bastan para saber qué hay: se cuenta por columna, y lo que salga vacío
    // se ve en el informe sin abrir el artefacto.
    let columnas = ore_core::json::Json::Obj(
        cab.esquema
            .keys()
            .map(|c| {
                let n = filas.iter().filter(|f| f.contains_key(c)).count();
                (c.clone(), ore_core::json::Json::Int(n as i64))
            })
            .collect(),
    );
    let parquet = carga::escribir(&cab.esquema, &filas)?;
    let artefacto = sobre::sellar(&cab, &parquet);
    let clave = sobre::clave(&artefacto);
    let digest = ore_core::digest::de_bytes(&artefacto);

    let subido = if cuenta.existe(&clave)? {
        false
    } else {
        cuenta.subir(&clave, &artefacto)?
    };
    // Y el recibo, que es lo que hace que la próxima vez no se lea el origen.
    // `If-None-Match` deja ganar al primero: si un segundo escritor llega con
    // otra carga bajo la misma cabecera, el recibo NO cambia — y eso es lo que
    // vuelve detectable que el testigo no fijaba el estado que decía fijar.
    let recibo_nuevo = cuenta.subir(recibo, clave.as_bytes())?;
    let mut superada = None;
    if rehacer && !recibo_nuevo {
        let anterior = cuenta.leer(recibo)?.unwrap_or_default();
        if anterior != clave {
            cuenta.sobrescribir(recibo, clave.as_bytes())?;
            if !anterior.is_empty() {
                cuenta.borrar(&anterior)?;
                superada = Some(anterior);
            }
        }
    }

    Ok(ore_core::json::Json::obj([
        ("rehecha", ore_core::json::Json::Bool(rehacer)),
        (
            "superada",
            ore_core::json::Json::s(superada.unwrap_or_default()),
        ),
        ("bytes", ore_core::json::Json::Int(artefacto.len() as i64)),
        ("clave", ore_core::json::Json::s(&clave)),
        ("columnas", columnas),
        ("digest", ore_core::json::Json::s(&digest)),
        ("filas", ore_core::json::Json::Int(filas.len() as i64)),
        ("recibo", ore_core::json::Json::s(recibo)),
        ("recibo_nuevo", ore_core::json::Json::Bool(recibo_nuevo)),
        ("subido", ore_core::json::Json::Bool(subido)),
    ])
    .jcs())
}

/// **`leer`: la copia, de vuelta, fila a fila** (0029 ③ «traer», F4a·I1).
///
/// Hasta aquí el almacén sólo releía para **fundir** (`anterior` → `sellar`
/// con `base`). Una función lee la copia para trabajar sobre ella, y eso es
/// otro verbo: la entrada es **el nombre del artefacto** —`{"clave": "ore/v1/
/// <sha256>"}`, el que el informe de la copia deja en el árbol— y la salida es
/// la cabecera del sobre en una línea y después **las filas, una por línea**,
/// como objetos JSON de cadenas: el mismo protocolo de 0008 en sentido
/// contrario. Quien lee sabe así **qué copia** leyó (su nombre es su digest) y
/// puede dejarlo escrito en lo que produzca.
///
/// Por nombre y no por plan, a propósito: por plan habría que elegir cuál de
/// los recibos es «la vigente», y eso lo sabe quien construyó la cabecera
/// (`ore`, con el testigo del origen), no el almacén. El informe ya lo dice.
fn leer(cuenta: &dyn Almacen, peticion: &str) -> Result<String, String> {
    let n =
        ore_core::parse::parse(peticion).map_err(|e| format!("la petición no analiza: {e:?}"))?;
    let clave = n
        .get("clave")
        .and_then(|(_, v)| v.as_str())
        .filter(|c| !c.is_empty())
        .ok_or("a la petición le falta `clave`: el nombre del artefacto que hay que leer")?;
    let bytes = cuenta
        .leer_bytes(clave)?
        .ok_or_else(|| format!("`{clave}` no está en el almacén"))?;
    let (cabecera, payload) = sobre::abrir(&bytes)?;
    let filas = carga::leer(payload)?;
    let mut out = String::with_capacity(bytes.len());
    out.push_str(&cabecera);
    for f in &filas {
        out.push('\n');
        out.push_str(
            &ore_core::json::Json::Obj(
                f.iter()
                    .map(|(k, v)| (k.clone(), ore_core::json::Json::s(v)))
                    .collect(),
            )
            .jcs(),
        );
    }
    Ok(out)
}

/// La cabecera, leída con el analizador del núcleo — el mismo que lee YAML, que
/// es un superconjunto de JSON. No entra un analizador más para esto.
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
        return Err("la cabecera no declara `esquema`: sin él no hay Parquet que escribir".into());
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
        bundle: s("bundle")?,
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

/// **La recogida de basura, y por qué el criterio no es el que el ADR escribió.**
///
/// [ADR 0015](../../../docs/decisions/0015-el-protocolo-del-almacen.md) la dejó
/// abierta diciendo *«una copia cuyo digest ya no está en ningún bundle es
/// basura»*. **Eso no se puede calcular**: el bundle no nombra copias, así que
/// por ese criterio todas serían basura, incluida la vigente.
///
/// Lo que sí se puede calcular es **superada**: bajo el prefijo de un plan hay
/// N recibos, y exactamente uno corresponde a la cabecera que `ore` construye
/// **ahora**. Los otros son refrescos anteriores. Ese criterio no exige nada que
/// no exista ya, y es el que se usa.
///
/// # Por qué es explícita y no automática
///
/// Porque una copia superada **sigue siendo cierta hasta su marca**, y alguien
/// puede estar leyéndola por su digest. El ADR argumentaba que borrar es seguro
/// *«porque nada la referencia por nombre mutable»* — cierto para quien la
/// encuentre por casualidad, y falso para quien se guardó el digest. Así que se
/// borra cuando alguien lo pide, y `recoger-seco` dice antes qué se iría.
///
/// # El orden
///
/// Primero el recibo, después el artefacto. Al revés, una interrupción dejaría
/// un recibo apuntando a algo que ya no está — y el paso ④ del ciclo diría *«ya
/// está»* de una copia borrada.
fn recoger(
    cuenta: &dyn Almacen,
    cab: &sobre::Cabecera,
    vigente: &str,
    seco: bool,
) -> Result<String, String> {
    let prefijo = sobre::prefijo_de_plan(&cab.plan);
    let recibos = cuenta.listar(&prefijo)?;

    let mut borrados = 0usize;
    let mut sin_artefacto = 0usize;
    for r in recibos.iter().filter(|r| *r != vigente) {
        // El artefacto al que apunta, antes de quitarle el puntero.
        let artefacto = cuenta.leer(r)?;
        if !seco {
            cuenta.borrar(r)?;
            match &artefacto {
                Some(a) => cuenta.borrar(a)?,
                None => sin_artefacto += 1,
            }
        } else if artefacto.is_none() {
            sin_artefacto += 1;
        }
        borrados += 1;
    }

    Ok(ore_core::json::Json::obj([
        ("recibos", ore_core::json::Json::Int(recibos.len() as i64)),
        ("seco", ore_core::json::Json::Bool(seco)),
        ("superadas", ore_core::json::Json::Int(borrados as i64)),
        // Un recibo sin artefacto es una subida que se cortó en medio. Se cuenta
        // porque no debería pasar, y contarlo es cómo se sabe si pasa.
        (
            "sin_artefacto",
            ore_core::json::Json::Int(sin_artefacto as i64),
        ),
        ("vigente", ore_core::json::Json::s(vigente)),
    ])
    .jcs())
}

/// **Sobre qué copia se puede construir la siguiente**, si sobre alguna.
///
/// Es el otro lado del recibo. `buscar` contesta *«¿está ya esta?»*; esto
/// contesta *«¿hay una anterior de la que partir?»*, y las dos preguntas usan el
/// mismo prefijo por plan.
///
/// # Por qué devuelve el testigo y no solo la clave
///
/// Porque quien pregunta necesita las dos cosas para la misma decisión: la clave
/// dice **sobre qué fundir** y el testigo dice **desde dónde leer**. Pedirlas por
/// separado sería dos enumeraciones para una respuesta.
///
/// # Y por qué el modo decide si hay respuesta
///
/// Solo se puede partir de una copia anterior si el testigo **ordena**. `log` y
/// `snapshot` nombran una posición de confirmación; `field` es una columna que
/// se compara. Pero `snapshot` **no ordena**: dos digests de Iceberg no se
/// comparan, se identifican — y por eso un origen con snapshots se lee entero en
/// su versión, que es lo que Iceberg y Delta hacen al omitir el inicio.
///
/// Así que aquí solo contestan `log` y `field`, y para `snapshot` la respuesta
/// correcta es *no hay de dónde partir*. No es una limitación: es lo que ese modo
/// significa.
fn anterior(cuenta: &dyn Almacen, cab: &sobre::Cabecera, vigente: &str) -> Result<String, String> {
    let ordena = matches!(cab.testigo.modo.as_str(), "log" | "field");
    let mut mejor: Option<(String, String)> = None;

    if ordena && !cab.clave.is_empty() {
        let actual = cab.testigo.valor.as_deref().unwrap_or("");
        for r in cuenta.listar(&sobre::prefijo_de_plan(&cab.plan))? {
            if r == vigente {
                continue;
            }
            let Some(clave) = cuenta.leer(&r)? else {
                continue;
            };
            // El testigo de esa copia sale de su propia cabecera: es la copia la
            // que sabe hasta cuándo fue cierta, no el recibo.
            let Some(bytes) = cuenta.leer_bytes(&clave)? else {
                continue;
            };
            let (cabecera, _) = sobre::abrir(&bytes)?;
            let Some(t) = ore_core::parse::parse(&cabecera)
                .ok()
                .and_then(|n| n.get("testigo").map(|(_, x)| x.clone()))
                .and_then(|x| {
                    x.get("valor")
                        .and_then(|(_, v)| v.as_str())
                        .map(String::from)
                })
            else {
                continue;
            };
            // La mayor de las que quedan por debajo de la actual. Una copia con
            // un testigo POSTERIOR no es una base: sería leer hacia atrás.
            if t.as_str() < actual && mejor.as_ref().is_none_or(|(_, m)| t > *m) {
                mejor = Some((clave, t));
            }
        }
    }

    Ok(ore_core::json::Json::obj([
        (
            "clave",
            ore_core::json::Json::s(mejor.as_ref().map(|(c, _)| c.as_str()).unwrap_or("")),
        ),
        ("hay", ore_core::json::Json::Bool(mejor.is_some())),
        (
            "testigo",
            ore_core::json::Json::s(mejor.as_ref().map(|(_, t)| t.as_str()).unwrap_or("")),
        ),
    ])
    .jcs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// Un almacén en memoria: lo justo para que el ciclo se pruebe sin red.
    #[derive(Default)]
    struct Memoria(RefCell<BTreeMap<String, Vec<u8>>>);

    impl Almacen for Memoria {
        fn leer(&self, clave: &str) -> Result<Option<String>, String> {
            Ok(self
                .0
                .borrow()
                .get(clave)
                .map(|b| String::from_utf8_lossy(b).into_owned()))
        }
        fn existe(&self, clave: &str) -> Result<bool, String> {
            Ok(self.0.borrow().contains_key(clave))
        }
        // Como los de verdad: si estaba, no se toca (`If-None-Match: *`).
        fn subir(&self, clave: &str, cuerpo: &[u8]) -> Result<bool, String> {
            let mut m = self.0.borrow_mut();
            if m.contains_key(clave) {
                return Ok(false);
            }
            m.insert(clave.to_string(), cuerpo.to_vec());
            Ok(true)
        }
        fn listar(&self, prefijo: &str) -> Result<Vec<String>, String> {
            Ok(self
                .0
                .borrow()
                .keys()
                .filter(|k| k.starts_with(prefijo))
                .cloned()
                .collect())
        }
        fn borrar(&self, clave: &str) -> Result<(), String> {
            self.0.borrow_mut().remove(clave);
            Ok(())
        }
        fn leer_bytes(&self, clave: &str) -> Result<Option<Vec<u8>>, String> {
            Ok(self.0.borrow().get(clave).cloned())
        }
    }

    fn sellada(cuenta: &Memoria) -> String {
        let cab = sobre::Cabecera {
            plan: "sha256:plan".into(),
            esquema: [
                ("id".to_string(), "String".to_string()),
                ("nombre".to_string(), "String".to_string()),
            ]
            .into(),
            testigo: sobre::Testigo {
                modo: "log".into(),
                valor: Some("7".into()),
            },
            clave: vec!["id".into()],
            conducto: "materialization.payload".into(),
            bundle: "sha256:bundle".into(),
        };
        let filas: Vec<carga::Fila> = vec![
            [
                ("id".to_string(), "b".to_string()),
                ("nombre".to_string(), "Bea".to_string()),
            ]
            .into(),
            [("id".to_string(), "a".to_string())].into(),
        ];
        let parquet = carga::escribir(&cab.esquema, &filas).expect("parquet");
        let artefacto = sobre::sellar(&cab, &parquet);
        let clave = sobre::clave(&artefacto);
        cuenta.subir(&clave, &artefacto).expect("sube");
        clave
    }

    /// **Rehacer**: misma cabecera, otros bytes. El recibo pasa a apuntar al
    /// artefacto nuevo y el viejo se borra; con los mismos bytes no hay nada
    /// que cambiar. Sin `rehacer`, el recibo no se mueve: gana el primero.
    #[test]
    fn rehacer_mueve_el_recibo_y_borra_lo_superado() {
        let cuenta = Memoria::default();
        let cab = || sobre::Cabecera {
            plan: "sha256:plan".into(),
            esquema: [("id".to_string(), "String".to_string())].into(),
            testigo: sobre::Testigo {
                modo: "snapshot".into(),
                valor: None,
            },
            clave: vec![],
            conducto: "materialization.payload".into(),
            bundle: "sha256:bundle".into(),
        };
        let recibo = sobre::recibo(&cab());
        let primera = sellar(
            &cuenta,
            cab(),
            &recibo,
            None,
            false,
            ["{\"id\":\"a\"}"].into_iter(),
        )
        .expect("sella");
        let k1 = ore_core::parse::parse(&primera)
            .unwrap()
            .get("clave")
            .unwrap()
            .1
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(cuenta.leer(&recibo).unwrap().as_deref(), Some(k1.as_str()));

        // sin rehacer, otros bytes NO mueven el recibo
        let segunda = sellar(
            &cuenta,
            cab(),
            &recibo,
            None,
            false,
            ["{\"id\":\"b\"}"].into_iter(),
        )
        .expect("sella");
        let k2 = ore_core::parse::parse(&segunda)
            .unwrap()
            .get("clave")
            .unwrap()
            .1
            .as_str()
            .unwrap()
            .to_string();
        assert_ne!(k1, k2);
        assert_eq!(
            cuenta.leer(&recibo).unwrap().as_deref(),
            Some(k1.as_str()),
            "gana el primero"
        );
        cuenta.borrar(&k2).unwrap();

        // con rehacer, el recibo apunta al nuevo y el viejo se borra
        let tercera = sellar(
            &cuenta,
            cab(),
            &recibo,
            None,
            true,
            ["{\"id\":\"b\"}"].into_iter(),
        )
        .expect("sella");
        let n = ore_core::parse::parse(&tercera).unwrap();
        assert_eq!(n.get("clave").unwrap().1.as_str(), Some(k2.as_str()));
        assert_eq!(n.get("rehecha").unwrap().1.as_str(), Some("true"));
        assert_eq!(n.get("superada").unwrap().1.as_str(), Some(k1.as_str()));
        assert_eq!(cuenta.leer(&recibo).unwrap().as_deref(), Some(k2.as_str()));
        assert!(!cuenta.existe(&k1).unwrap(), "la superada se fue");
        assert!(cuenta.existe(&k2).unwrap());

        // rehacer con los mismos bytes: nada que mover ni borrar
        let cuarta = sellar(
            &cuenta,
            cab(),
            &recibo,
            None,
            true,
            ["{\"id\":\"b\"}"].into_iter(),
        )
        .expect("sella");
        let n = ore_core::parse::parse(&cuarta).unwrap();
        assert_eq!(n.get("superada").unwrap().1.as_str(), Some(""));
        assert_eq!(cuenta.leer(&recibo).unwrap().as_deref(), Some(k2.as_str()));
    }

    /// `leer` devuelve la cabecera del sobre y las filas una por línea; un
    /// nulo es la propiedad ausente, como en el protocolo del driver.
    #[test]
    fn leer_devuelve_la_cabecera_y_las_filas_por_nombre() {
        let cuenta = Memoria::default();
        let clave = sellada(&cuenta);
        let salida = leer(&cuenta, &format!("{{\"clave\":\"{clave}\"}}")).expect("lee");
        let lineas: Vec<&str> = salida.lines().collect();
        assert_eq!(lineas.len(), 3, "cabecera + 2 filas: {salida}");
        assert!(
            lineas[0].contains("\"plan\":\"sha256:plan\""),
            "{}",
            lineas[0]
        );
        assert_eq!(lineas[1], "{\"id\":\"b\",\"nombre\":\"Bea\"}");
        assert_eq!(lineas[2], "{\"id\":\"a\"}", "el nulo no viaja");
    }

    #[test]
    fn leer_lo_que_no_esta_lo_dice() {
        let cuenta = Memoria::default();
        let e = leer(&cuenta, "{\"clave\":\"ore/v1/nadie\"}").unwrap_err();
        assert!(e.contains("no está en el almacén"), "{e}");
        let e = leer(&cuenta, "{\"plan\":\"x\"}").unwrap_err();
        assert!(e.contains("le falta `clave`"), "{e}");
    }
}
