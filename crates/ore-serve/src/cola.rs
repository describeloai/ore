//! La cola de trabajo — encolar el catálogo de una fuente en el mismo acto del alta.
//!
//! # Por qué esto existe
//!
//! Medido el 2026-09-10: se da de alta una fuente a las 19:24 y el Job que lee el
//! origen no existe hasta las 20:17. No es lentitud — es que quien lo rendía era
//! la convergencia, y a la convergencia sólo la llamaba el cron.
//!
//! El webhook de la forja SÍ dispara al empujar el árbol, y el `Receiver` de Flux
//! reconcilia lo que lleve `ore.dev/rol: agente`. Pero eso apuntaba al
//! **compartimento**, que no cambia cuando se declara una fuente. Flux miraba,
//! veía lo mismo, y no hacía nada.
//!
//! ⇒ Con una cola propia, el alta la escribe, el webhook dispara, el `Receiver`
//!   la reconcilia y Flux crea el Job. Segundos, y ni un actor nuevo.
//!
//! # ⛔⛔ Y por qué NO se escribe en el compartimento
//!
//! Porque contiene el `Deployment` de este mismo proceso, sus `NetworkPolicy` y
//! a qué cuenta corre. Escribir ahí sería **el gobernado escribiendo su
//! gobierno**. La cola es un segundo repositorio con un segundo escritor, y su
//! `Kustomization` corre con una cuenta que sólo puede crear `Job`.
//!
//! # ⭐⭐ Y por qué aquí NO se renderiza de verdad
//!
//! El renderizado del inquilino —namespace, árbol, organización— vive en
//! `gen-inquilino.py` y tiene detrás una comprobación byte a byte. Copiarlo aquí
//! serían dos descripciones del mismo manifiesto, y la que se quedara vieja **no
//! daría error: daría un Job mal**.
//!
//! ⇒ El aprovisionador deja en la cola `plantilla-catalogo.txt`, ya rendida para
//!   ESTE inquilino y con el hueco de la fuente intacto. Aquí sólo se sustituyen
//!   dos cosas: el nombre de la fuente y el resumen del contenido.

use ore_core::digest;

/// Lo que el aprovisionador deja en la cola para que esto pueda encolar.
pub const PLANTILLA: &str = "plantilla-catalogo.txt";

/// La fuente y el resumen que la plantilla trae de fábrica, y que aquí se
/// sustituyen. Son los del fichero modelo de `malla/`, y si allí cambiaran esto
/// dejaría de sustituir nada — por eso la comprobación de más abajo los fija.
const FUENTE_MODELO: &str = "bq";
const RESUMEN_MODELO: &str = "00000000";

/// De un nombre de FUENTE al nombre de un objeto de Kubernetes.
///
/// ⚠️ Es la misma función que `gen-inquilino.py::nombre_de_objeto`, y eso es una
/// duplicación de verdad. Se acepta porque es **cerrada y comprobable**: no
/// depende del manifiesto ni crece con él, y las pruebas de abajo fijan los
/// mismos casos que las de allí. Lo que NO se duplica es el renderizado del
/// inquilino, que es lo que puede envejecer.
///
/// Cortado a 30 por la misma cuenta: `catalogo-` son 9, el resumen añade 9, y un
/// Job pone a sus pods otro sufijo de 6.
pub fn nombre_de_objeto(s: &str) -> String {
    let mut out = String::new();
    let mut guion = false;
    for c in s.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
            guion = false;
        } else if c == '-' || !guion {
            // ⚠️ Una tirada de caracteres inválidos da UN guion, no uno por
            //   carácter: `Ventas..2024` es `ventas-2024`, no `ventas--2024`.
            out.push('-');
            guion = true;
        }
    }
    let n: String = out.trim_matches('-').chars().take(30).collect();
    let n = n.trim_matches('-').to_string();
    if n.is_empty() { "sin-nombre".into() } else { n }
}

/// El Job de catálogo de una fuente: cómo se llama el fichero y qué lleva dentro.
///
/// ⛔ El resumen va EL ÚLTIMO, sobre todo lo demás ya sustituido. Un Job es
///   inmutable: con el nombre derivado del contenido, «mismo nombre» implica
///   «mismo contenido» y el conflicto no puede darse. Es la misma regla que
///   `gen-inquilino.py`, y por eso los dos producen **el mismo nombre** para la
///   misma fuente — encolar dos veces no crea dos Jobs.
#[cfg(test)]
pub fn rendir(plantilla: &str, fuente: &str) -> Result<(String, String), String> {
    rendir_corrida(plantilla, fuente, None, None)
}

/// **Volver a catalogar**: lo mismo con una corrida dentro. El nombre del Job
/// sale del contenido, así que sin ella «otra vez» sería el mismo Job —ya
/// `Failed`— y Flux no crearía ninguno. Con ella el fichero es el mismo
/// (`44-el-catalogo-<obj>.yaml`: Flux poda el Job anterior) y el Job es otro.
///
/// Con `dueno`, el Job lleva `DUENO` junto a `FUENTE`: el `owner` del paquete que
/// cree (0052 · Ownership: quien dio de alta el origen). Una plantilla que no
/// lo lea sigue con el suyo, y una que lo lea sin él, también: los dos órdenes del
/// despliegue valen.
pub fn rendir_corrida(
    plantilla: &str,
    fuente: &str,
    corrida: Option<&str>,
    dueno: Option<&str>,
) -> Result<(String, String), String> {
    if !plantilla.contains(&format!("catalogo-{FUENTE_MODELO}-{RESUMEN_MODELO}")) {
        // ⛔ Se niega en vez de escribir algo que no sustituye nada. Un Job
        //   llamado `catalogo-bq-00000000` en el namespace de un cliente sería
        //   silencioso y estaría mal.
        return Err(format!(
            "`{PLANTILLA}` no trae el hueco `catalogo-{FUENTE_MODELO}-{RESUMEN_MODELO}`: \
             o no es la plantilla, o `malla/44-el-catalogo.yaml` cambió sin que esto se \
             enterara"
        ));
    }
    let obj = nombre_de_objeto(fuente);
    let t = plantilla.replace(
        &format!("value: \"{FUENTE_MODELO}\""),
        &format!("value: \"{fuente}\""),
    );
    let t = match dueno {
        None => t,
        Some(d) => {
            let hueco = format!("{{ name: FUENTE, value: \"{fuente}\" }}");
            t.lines()
                .flat_map(|l| {
                    let mut v = vec![l.to_string()];
                    if l.trim_start().starts_with(&format!("- {hueco}")) {
                        v.push(l.replace(&hueco, &format!("{{ name: DUENO, value: \"{d}\" }}")));
                    }
                    v
                })
                .collect::<Vec<_>>()
                .join("\n")
                + if t.ends_with('\n') { "\n" } else { "" }
        }
    };
    // Un comentario y no un campo: no cambia lo que el Job hace, solo quién es.
    let t = match corrida {
        Some(c) => format!("# corrida: {c}\n{t}"),
        None => t,
    };
    let h = digest::de_bytes(t.as_bytes());
    // `de_bytes` devuelve `sha256:<64 hex>`; se toman los ocho primeros, que es
    // lo mismo que hace el renderizador con `hexdigest()[:8]`.
    let h = &h["sha256:".len().."sha256:".len() + 8];
    let t = t.replace(
        &format!("catalogo-{FUENTE_MODELO}-{RESUMEN_MODELO}"),
        &format!("catalogo-{obj}-{h}"),
    );
    Ok((format!("44-el-catalogo-{obj}.yaml"), t))
}

/// La plantilla de la copia (0027 P1 I3), sin lista, que el aprovisionador
/// deja en la cola junto a la del catálogo.
pub const PLANTILLA_COPIA: &str = "plantilla-copia.txt";
const VISTAS_MODELO: &str = "olist.customers";

/// El hueco de la rama en `malla/48-la-copia.yaml` (0044 C.2 ④): vacío es
/// `main`, que es lo que la copia fue siempre.
const RAMA_DE_LA_COPIA: &str = "name: RAMA, value: \"\"";

/// Rinde el Job de la copia con la lista de vistas —`paquete.vista`, las que
/// son datasets mantenidos en todo el árbol (0033)— y el resumen del contenido en el
/// nombre. Es lo mismo que hace `gen-inquilino.py` con `--copias`, y por eso el
/// fichero se llama igual: dos rendidos de la misma lista son el mismo Job.
///
/// ⭐ **Con rama** (0044 C.2 ④) es otro Job y otro fichero —`copiar-rama-<h>`,
///   `48-la-copia-rama-<h>.yaml`, con la rama dentro del resumen—: construye en
///   la rama y empuja a ella, y no sustituye a la copia de `main`. Una plantilla
///   sin el hueco `RAMA` es un error, no una copia en `main`.
pub fn rendir_copia(
    plantilla: &str,
    vistas: &[String],
    rama: Option<&str>,
) -> Result<(String, String), String> {
    if !plantilla.contains(&format!("copiar-{RESUMEN_MODELO}")) {
        return Err(format!(
            "`{PLANTILLA_COPIA}` no trae el hueco `copiar-{RESUMEN_MODELO}`: o no es la \
             plantilla, o `malla/48-la-copia.yaml` cambió sin que esto se enterara"
        ));
    }
    let mut t = plantilla.replace(
        &format!("value: \"{VISTAS_MODELO}\""),
        &format!("value: \"{}\"", vistas.join(",")),
    );
    if let Some(r) = rama {
        if !t.contains(RAMA_DE_LA_COPIA) {
            return Err(format!(
                "`{PLANTILLA_COPIA}` no sabe de ramas todavía (no trae el hueco `RAMA`): \
                 hay que converger este inquilino"
            ));
        }
        t = t.replace(RAMA_DE_LA_COPIA, &format!("name: RAMA, value: \"{r}\""));
    }
    let h = digest::de_bytes(t.as_bytes());
    let h = &h["sha256:".len().."sha256:".len() + 8];
    let (job, fichero) = match rama {
        Some(_) => (
            format!("copiar-rama-{h}"),
            format!("48-la-copia-rama-{h}.yaml"),
        ),
        None => (format!("copiar-{h}"), "48-la-copia.yaml".to_string()),
    };
    let t = t.replace(&format!("copiar-{RESUMEN_MODELO}"), &job);
    Ok((fichero, t))
}

/// **Rehacer** (0030 W1): el mismo Job de la copia, con `REHACER` puesto al
/// instante de la petición y `VISTAS` a las del paquete. El fichero y el
/// nombre llevan el instante: dos peticiones son dos Jobs, como en la
/// invocación, porque «rehaz ahora» no es idempotente por contenido.
pub fn rendir_rehacer(
    plantilla: &str,
    vistas: &[String],
    instante: &str,
    rama: Option<&str>,
) -> Result<(String, String), String> {
    if !plantilla.contains("name: REHACER, value: \"\"") {
        return Err(format!(
            "`{PLANTILLA_COPIA}` no trae el hueco `REHACER`: o no es la plantilla, o \
             `malla/48-la-copia.yaml` cambió sin que esto se enterara"
        ));
    }
    let (_, t) = rendir_copia(plantilla, vistas, rama)?;
    let t = t.replace(
        "name: REHACER, value: \"\"",
        &format!("name: REHACER, value: \"{instante}\""),
    );
    let h = digest::de_bytes(t.as_bytes());
    let h = &h["sha256:".len().."sha256:".len() + 8];
    // El nombre que `rendir_copia` puso lleva el resumen de la lista; el de
    // rehacer lleva el de la pasada entera, instante incluido.
    let mut salida = String::new();
    for linea in t.lines() {
        if let Some(resto) = linea.strip_prefix("  name: copiar-") {
            let _ = resto;
            salida.push_str(&format!("  name: copiar-rehacer-{h}\n"));
        } else {
            salida.push_str(linea);
            salida.push('\n');
        }
    }
    Ok((format!("48-la-copia-rehacer-{h}.yaml"), salida))
}

/// La plantilla de la invocación (0029 F4a I3), sin función, que el
/// aprovisionador deja en la cola junto a las otras dos.
pub const PLANTILLA_INVOCACION: &str = "plantilla-invocacion.txt";
const FUNCION_MODELO: &str = "ventas.clasificar";
const PUERTA_MODELO: &str = "http://10.10.0.100:8000/v1";
const ID_MODELO: &str = "modelo-id";
const CORRIDA_MODELO: &str = "00000000T000000Z";

/// Lo que el Job de una invocación necesita saber, resuelto aquí al encolar.
#[derive(Clone, Copy)]
pub struct Invocacion<'a> {
    /// `<paquete>.<nombre>`.
    pub funcion: &'a str,
    /// La puerta (`GET /modelos/{n}` → `url`) y el id servido (→ `model`).
    pub puerta: &'a str,
    pub modelo: &'a str,
    /// El instante de la petición: dos peticiones son dos Jobs.
    pub corrida: &'a str,
}

/// Rinde el Job de UNA invocación. El fichero lleva la función dentro
/// (`49-la-invocacion-<objeto>.yaml`): la siguiente invocación de la misma
/// función lo sustituye —Flux retira el Job anterior y crea el nuevo—, y dos
/// funciones no se pisan. El nombre del Job lleva la corrida en el resumen.
pub fn rendir_invocacion(plantilla: &str, i: &Invocacion) -> Result<(String, String), String> {
    if !plantilla.contains(&format!("invocar-{RESUMEN_MODELO}")) {
        return Err(format!(
            "`{PLANTILLA_INVOCACION}` no trae el hueco `invocar-{RESUMEN_MODELO}`: o no es la \
            plantilla, o `malla/49-la-invocacion.yaml` cambió sin que esto se enterara"
        ));
    }
    for (de, a) in [
        (FUNCION_MODELO, i.funcion),
        (PUERTA_MODELO, i.puerta),
        (ID_MODELO, i.modelo),
        (CORRIDA_MODELO, i.corrida),
    ] {
        if !plantilla.contains(&format!("value: \"{de}\"")) {
            return Err(format!(
                "`{PLANTILLA_INVOCACION}` no trae el hueco `value: \"{de}\"`: `malla/49-la-invocacion.yaml` cambió sin que esto se enterara"
            ));
        }
        if a.contains('"') || a.contains('\n') {
            return Err(format!("`{a}` no puede ir en un valor del Job"));
        }
    }
    let t = plantilla
        .replace(
            &format!("value: \"{FUNCION_MODELO}\""),
            &format!("value: \"{}\"", i.funcion),
        )
        .replace(
            &format!("value: \"{PUERTA_MODELO}\""),
            &format!("value: \"{}\"", i.puerta),
        )
        .replace(
            &format!("value: \"{ID_MODELO}\""),
            &format!("value: \"{}\"", i.modelo),
        )
        .replace(
            &format!("value: \"{CORRIDA_MODELO}\""),
            &format!("value: \"{}\"", i.corrida),
        );
    let h = digest::de_bytes(t.as_bytes());
    let h = &h["sha256:".len().."sha256:".len() + 8];
    let obj = nombre_de_objeto(i.funcion);
    let t = t.replace(
        &format!("invocar-{RESUMEN_MODELO}"),
        &format!("invocar-{obj}-{h}"),
    );
    Ok((format!("49-la-invocacion-{obj}.yaml"), t))
}

// ── La comprobación de acceso, antes del alta ───────────────────────────────

/// Lo que el aprovisionador deja en la cola para comprobar un origen (54).
pub const PLANTILLA_COMPROBACION: &str = "plantilla-comprobacion.txt";
const TIPO_MODELO: &str = "bigquery";
const URL_MODELO: &str = "bigquery://modelo/dataset";

/// Rinde el Job que comprueba si la cuenta de la celda llega a `url`. Una
/// ranura por URL (`54-la-comprobacion-<objeto>.yaml`): comprobar otra vez la
/// sustituye y Flux poda el Job anterior; el nombre del Job lleva la corrida en
/// el resumen, así que dos clics son dos Jobs.
///
/// ⛔ La URL viaja EN CLARO en la cola: solo se admite una sin credencial, y
///   eso lo decide quien llama ([`url_sin_secreto`]).
pub fn rendir_comprobacion(
    plantilla: &str,
    tipo: &str,
    url: &str,
    corrida: &str,
) -> Result<(String, String), String> {
    if !plantilla.contains(&format!("comprobar-{RESUMEN_MODELO}")) {
        return Err(format!(
            "`{PLANTILLA_COMPROBACION}` no trae el hueco `comprobar-{RESUMEN_MODELO}`: o no es la \
             plantilla, o `malla/54-la-comprobacion.yaml` cambió sin que esto se enterara"
        ));
    }
    for (de, a) in [
        (TIPO_MODELO, tipo),
        (URL_MODELO, url),
        (CORRIDA_MODELO, corrida),
    ] {
        if !plantilla.contains(&format!("value: \"{de}\"")) {
            return Err(format!(
                "`{PLANTILLA_COMPROBACION}` no trae el hueco `value: \"{de}\"`: \
                 `malla/54-la-comprobacion.yaml` cambió sin que esto se enterara"
            ));
        }
        if a.contains('"') || a.contains('\n') || a.contains('\'') || a.contains('$') {
            return Err(format!("`{a}` no puede ir en un valor del Job"));
        }
    }
    let t = plantilla
        .replace(
            &format!("value: \"{TIPO_MODELO}\""),
            &format!("value: \"{tipo}\""),
        )
        .replace(
            &format!("value: \"{URL_MODELO}\""),
            &format!("value: \"{url}\""),
        )
        .replace(
            &format!("value: \"{CORRIDA_MODELO}\""),
            &format!("value: \"{corrida}\""),
        );
    let h = digest::de_bytes(t.as_bytes());
    let h = &h["sha256:".len().."sha256:".len() + 8];
    let obj = nombre_de_objeto(url.split_once("://").map_or(url, |(_, r)| r));
    let t = t.replace(
        &format!("comprobar-{RESUMEN_MODELO}"),
        &format!("comprobar-{h}"),
    );
    Ok((format!("54-la-comprobacion-{obj}.yaml"), t))
}

/// ¿Puede esta URL viajar en claro por la cola? Solo las familias que leen con
/// la cuenta de la celda y no llevan credencial dentro: BigQuery, GCS, Azure
/// Blob, SharePoint, S3 por rol y SFTP con la clave de la celda.
pub fn url_sin_secreto(url: &str) -> Result<&'static str, String> {
    // ⭐ ADR 0061 O2·3 · Un bucket de GCS se lee con la cuenta de la celda, o
    //   suplantando una del cliente (`suplantar`): su URL no lleva secreto. Ni
    //   `endpoint` —el token es al portador y no se manda a otro servidor; eso
    //   lo vigila también `ore-gcs`— ni nada más.
    if let Some(resto) = url.strip_prefix("gs://") {
        let (camino, consulta) = resto.split_once('?').unwrap_or((resto, ""));
        if camino.contains('@') || url.contains('#') {
            return Err("la URL de GCS es `gs://<bucket>[/<prefijo>][?suplantar=<cuenta>]`".into());
        }
        if let Some(k) = consulta
            .split('&')
            .filter(|p| !p.is_empty())
            .map(|p| p.split_once('=').map_or(p, |(k, _)| k))
            .find(|k| *k != "suplantar")
        {
            return Err(format!(
                "la URL de GCS sólo admite `suplantar`, no `{k}`: no se comprueba otra cosa"
            ));
        }
        return Ok("gcs");
    }
    // ⭐ ADR 0061 O3·3 · Un contenedor de Azure se lee con la cuenta de la celda
    //   federada en la app de Entra del cliente: la URL nombra la app (`tenant`,
    //   `cliente`) y nada más —ni `endpoint`, ni una SAS, ni una clave—.
    if let Some(resto) = url.strip_prefix("az://") {
        let (camino, consulta) = resto.split_once('?').unwrap_or((resto, ""));
        if camino.contains('@') || url.contains('#') {
            return Err(
                "la URL de Azure es `az://<cuenta>/<contenedor>[/<prefijo>]?tenant=…&cliente=…`"
                    .into(),
            );
        }
        let claves: Vec<&str> = consulta
            .split('&')
            .filter(|p| !p.is_empty())
            .map(|p| p.split_once('=').map_or(p, |(k, _)| k))
            .collect();
        if let Some(k) = claves.iter().find(|k| !matches!(**k, "tenant" | "cliente")) {
            return Err(format!(
                "la URL de Azure sólo admite `tenant` y `cliente`, no `{k}`: no se comprueba otra cosa"
            ));
        }
        if !(claves.contains(&"tenant") && claves.contains(&"cliente")) {
            return Err("la URL de Azure nombra la app de Entra: `tenant` y `cliente`".into());
        }
        return Ok("azure");
    }
    // ⭐ ADR 0061 O5·3 · Una biblioteca de SharePoint, igual (D-O5): la misma
    //   federación, y la URL nombra el sitio, la biblioteca y la app.
    if let Some(resto) = url.strip_prefix("sharepoint://") {
        let (camino, consulta) = resto.split_once('?').unwrap_or((resto, ""));
        if camino.contains('@') || url.contains('#') {
            return Err("la URL de SharePoint es `sharepoint://<host>/[sites/<sitio>/]<biblioteca>[/<prefijo>]?tenant=…&cliente=…`".into());
        }
        let claves: Vec<&str> = consulta
            .split('&')
            .filter(|p| !p.is_empty())
            .map(|p| p.split_once('=').map_or(p, |(k, _)| k))
            .collect();
        if let Some(k) = claves.iter().find(|k| !matches!(**k, "tenant" | "cliente")) {
            return Err(format!(
                "la URL de SharePoint sólo admite `tenant` y `cliente`, no `{k}`: no se comprueba otra cosa"
            ));
        }
        if !(claves.contains(&"tenant") && claves.contains(&"cliente")) {
            return Err("la URL de SharePoint nombra la app de Entra: `tenant` y `cliente`".into());
        }
        return Ok("sharepoint");
    }
    // ⭐ ADR 0061 O4·3 · Un SFTP se lee con la clave de la celda (D-O4): su URL
    //   no lleva secreto salvo el recurso de una contraseña, y ésa no viaja en
    //   un Job (se comprueba al catalogarla, como una clave de S3).
    if let Some(resto) = url.strip_prefix("sftp://") {
        let (camino, consulta) = resto.split_once('?').unwrap_or((resto, ""));
        let autoridad = camino.split('/').next().unwrap_or("");
        let Some((quien, _)) = autoridad.rsplit_once('@') else {
            return Err(
                "la URL de un SFTP es `sftp://<usuario>@<host>[:<puerto>]/<ruta>?huella=…`".into(),
            );
        };
        if quien.contains(':') || url.contains('#') {
            return Err(
                "una URL de SFTP con contraseña no viaja en un Job: se comprueba al catalogarla \
                 (con la clave de la celda, sí)"
                    .into(),
            );
        }
        if let Some(k) = consulta
            .split('&')
            .filter(|p| !p.is_empty())
            .map(|p| p.split_once('=').map_or(p, |(k, _)| k))
            .find(|k| !matches!(*k, "huella" | "edad" | "legado"))
        {
            return Err(format!(
                "la URL de un SFTP sólo admite `huella`, `edad` y `legado`, no `{k}`"
            ));
        }
        return Ok("sftp");
    }
    // ⭐ 0046 E9b · Un bucket de S3 por ROL tampoco lleva secreto: su URL nombra el
    //   rol (`role_arn`), y la credencial la pide el driver en el Job, de una hora.
    if let Some(resto) = url.strip_prefix("s3://") {
        let (autoridad, consulta) = resto.split_once('?').unwrap_or((resto, ""));
        if autoridad.contains('@') || url.contains('#') {
            return Err("la URL de S3 es `s3://<bucket>[/<prefijo>]?region=…&role_arn=…`".into());
        }
        let claves: Vec<&str> = consulta
            .split('&')
            .filter_map(|p| p.split_once('=').map(|(k, _)| k))
            .collect();
        if !claves.contains(&"role_arn") {
            return Err(
                "solo se comprueba antes del alta un bucket por rol (`role_arn`): una clave de \
                 acceso no viaja en un Job; se comprueba al catalogarlo"
                    .into(),
            );
        }
        if let Some(k) = claves.iter().find(|k| {
            let k = k.to_ascii_lowercase();
            ["key", "secret", "token", "pass", "credential", "sig"]
                .iter()
                .any(|s| k.contains(s))
        }) {
            return Err(format!(
                "la URL de un rol no lleva `{k}`: no se comprueba una URL con secreto"
            ));
        }
        return Ok("s3");
    }
    let Some(resto) = url.strip_prefix("bigquery://") else {
        return Err(
            "solo se comprueba antes del alta un origen que no lleva credencial en su URL \
             (BigQuery, con la cuenta de la celda; S3 por rol); los demás se comprueban al \
             catalogarlos"
                .into(),
        );
    };
    if resto.contains('@') || resto.contains('?') || resto.contains('#') {
        return Err("la URL de BigQuery es `bigquery://<proyecto>/<dataset>`, sin nada más".into());
    }
    Ok("bigquery")
}

// ── El puesto (0031 W3.1): la sesión viva de una persona ─────────────────────
pub const PLANTILLA_PUESTO: &str = "plantilla-puesto.txt";
const PUESTO_MODELO: &str = "puesto-modelo";
const RAMA_MODELO: &str = "rama-modelo";
const CAPA_MODELO: &str = "capa-modelo";
const ABIERTO_MODELO: &str = "abierto-modelo";
/// R1 · **La identidad del puesto, declarada.** La audiencia del token de
/// cuenta de servicio que el pod monta (`malla/51-el-puesto.yaml`): aquí se
/// escribe `ore-serve/puestos/<id>/<apertura>`, el clúster la firma y
/// `ore-serve` sabe qué puesto habla sin deducirlo de nada.
pub const AUDIENCIA_DE_PUESTOS: &str = "ore-serve/puestos";
const AUDIENCIA_MODELO: &str = "audience: ore-serve/puestos/puesto-modelo/abierto-modelo";
/// El hueco del trabajo (W3.7 ④): `<ruta>@<commit>` del fichero que el Job
/// corre como una sola celda y termina; vacío es una sesión.
const TRABAJO_MODELO: &str = "trabajo-modelo";
/// El hueco de la imagen del puesto: `puesto-entorno-modelo:1` →
/// `puesto-<entorno>:1` (W3.4: python, node o jvm; las tres con el agente
/// dentro y su `CMD`, por eso la plantilla no lleva `command`).
const ENTORNO_MODELO: &str = "puesto-entorno-modelo:1";
/// Y el NOMBRE del entorno, que `traer-la-capa` necesita para saber si baja
/// ruedas o jars (0037 ③c): el contenedor que baja la capa es el mismo para
/// los tres, porque el cliente de la nube vive en él.
const ENTORNO_VALOR_MODELO: &str = "entorno-modelo";
/// Los entornos que tienen imagen (`Dockerfile`, `cloudbuild.yaml`).
pub const ENTORNOS: [&str; 3] = ["python", "node", "jvm"];

/// La etiqueta que abre la salida al gateway de modelos a un trabajo del
/// puesto (0050 P4): la selecciona `salida-al-modelo-de-una-funcion`
/// (`malla/11`), junto con `ore.dev/rol: puesto`. La pone **solo** `ore-serve`,
/// y solo al trabajo de una función que declara `models`: una sesión
/// interactiva no la lleva nunca.
pub const ETIQUETA_USA_MODELO: &str = "ore.dev/usa-modelo";

/// La plantilla del puesto con la etiqueta de [`ETIQUETA_USA_MODELO`] en el
/// pod, antes de rendirla: así el nombre del Job, que resume el contenido,
/// la cuenta.
pub fn con_salida_al_modelo(plantilla: &str) -> Result<String, String> {
    let rol = "
        ore.dev/rol: puesto
";
    if plantilla.matches(rol).count() != 1 {
        return Err(format!(
            "`{PLANTILLA_PUESTO}` no trae (una vez) la etiqueta `ore.dev/rol: puesto` del pod: `malla/51-el-puesto.yaml` cambió sin que esto se enterara"
        ));
    }
    Ok(plantilla.replacen(
        rol,
        &format!(
            "{rol}        {ETIQUETA_USA_MODELO}: \"si\"
"
        ),
        1,
    ))
}

/// Rinde el Job del puesto de `id` (`puesto-<persona>-<entorno>`), en `rama`
/// (vacía = `main`), con la `capa` (el digest de `entorno.rs`, o vacía: sin
/// capa) y sobre la imagen del `entorno`. Devuelve `(fichero, texto, nombre
/// del Job)`. Un mismo puesto en la misma rama y con la misma capa es el mismo
/// fichero: abrirlo dos veces no crea dos Jobs.
pub fn rendir_puesto(
    plantilla: &str,
    id: &str,
    rama: &str,
    capa: &str,
    abierto: &str,
    entorno: &str,
    trabajo: &str,
) -> Result<(String, String, String), String> {
    if !plantilla.contains(&format!("puesto-{RESUMEN_MODELO}")) {
        return Err(format!(
            "`{PLANTILLA_PUESTO}` no trae el hueco `puesto-{RESUMEN_MODELO}`: o no es la plantilla, o `malla/51-el-puesto.yaml` cambió sin que esto se enterara"
        ));
    }
    if !ENTORNOS.contains(&entorno) {
        return Err(format!(
            "`{entorno}` no es un entorno con imagen: {}",
            ENTORNOS.join(", ")
        ));
    }
    if plantilla.matches(ENTORNO_MODELO).count() != 1 {
        return Err(format!(
            "`{PLANTILLA_PUESTO}` no trae (una vez) el hueco `{ENTORNO_MODELO}`: `malla/51-el-puesto.yaml` cambió sin que esto se enterara"
        ));
    }
    if !trabajo.is_empty() && !plantilla.contains(&format!("value: \"{TRABAJO_MODELO}\"")) {
        return Err(format!(
            "`{PLANTILLA_PUESTO}` no trae el hueco `value: \"{TRABAJO_MODELO}\"`: hay que converger este inquilino para correr un trabajo"
        ));
    }
    for (de, a) in [
        (PUESTO_MODELO, id),
        (RAMA_MODELO, rama),
        (CAPA_MODELO, capa),
        (ABIERTO_MODELO, abierto),
        (ENTORNO_VALOR_MODELO, entorno),
    ] {
        if !plantilla.contains(&format!("value: \"{de}\"")) {
            return Err(format!(
                "`{PLANTILLA_PUESTO}` no trae el hueco `value: \"{de}\"`: `malla/51-el-puesto.yaml` cambió sin que esto se enterara"
            ));
        }
        if a.contains('"') || a.contains('\n') {
            return Err(format!("`{a}` no puede ir en un valor del Job"));
        }
    }
    let t = plantilla
        // R1 · Opcional: una plantilla anterior no trae el hueco, y su pod no
        //   declara puesto (habla como el agente de la celda, como antes).
        .replace(
            AUDIENCIA_MODELO,
            &format!("audience: {AUDIENCIA_DE_PUESTOS}/{id}/{abierto}"),
        )
        .replace(
            &format!("value: \"{PUESTO_MODELO}\""),
            &format!("value: \"{id}\""),
        )
        .replace(
            &format!("value: \"{RAMA_MODELO}\""),
            &format!("value: \"{rama}\""),
        )
        .replace(
            &format!("value: \"{CAPA_MODELO}\""),
            &format!("value: \"{capa}\""),
        )
        .replace(
            &format!("value: \"{ABIERTO_MODELO}\""),
            &format!("value: \"{abierto}\""),
        )
        .replace(
            &format!("value: \"{ENTORNO_VALOR_MODELO}\""),
            &format!("value: \"{entorno}\""),
        )
        .replace(ENTORNO_MODELO, &format!("puesto-{entorno}:1"))
        .replace(
            &format!("value: \"{TRABAJO_MODELO}\""),
            &format!("value: \"{trabajo}\""),
        );
    let h = digest::de_bytes(t.as_bytes());
    let h = &h["sha256:".len().."sha256:".len() + 8];
    // Un trabajo (`trabajo-<persona>-<hex>`) se llama por su id: no es «el
    // puesto de ana en python», es una corrida, y cada una es un fichero.
    if let Some(quien) = id.strip_prefix("trabajo-") {
        let job = format!("trabajo-{quien}-{h}");
        let t = t.replace(&format!("puesto-{RESUMEN_MODELO}"), &job);
        return Ok((format!("54-el-trabajo-{quien}.yaml"), t, job));
    }
    let quien = id.strip_prefix("puesto-").unwrap_or(id);
    let job = format!("puesto-{quien}-{h}");
    let t = t.replace(&format!("puesto-{RESUMEN_MODELO}"), &job);
    Ok((format!("51-el-puesto-{quien}.yaml"), t, job))
}

// ── La capa (0031 §3, W3.2): las dependencias del árbol, resueltas ──────────
pub const PLANTILLA_CAPA: &str = "plantilla-capa.txt";
/// Y la de la JVM (0037 ③c): otro Job, otra imagen y otro resolvedor —Maven,
/// no `pip`—, así que otra plantilla. Lo que comparten es todo lo demás: el
/// digest, el alcance, el informe y el bucket.
pub const PLANTILLA_CAPA_JVM: &str = "plantilla-capa-jvm.txt";
/// Y la de Node (0050 R3 T5b): `npm`, en `capa-node:1`.
pub const PLANTILLA_CAPA_NODE: &str = "plantilla-capa-node.txt";

/// La plantilla que resuelve la capa de un entorno.
pub fn plantilla_capa_de(entorno: &str) -> &'static str {
    match entorno {
        "jvm" => PLANTILLA_CAPA_JVM,
        "node" => PLANTILLA_CAPA_NODE,
        _ => PLANTILLA_CAPA,
    }
}
const INTENTO_MODELO: &str = "intento-modelo";
const ALCANCE_MODELO: &str = "alcance-modelo";

/// Rinde el Job que resuelve la capa `digest` (`capa-<12 hex>`, de
/// `entorno::digest_de`) en `rama`. `alcance` es la carpeta del repositorio
/// (0036 ③) o vacío para la celda entera: el Job suma los `pyproject.toml` que
/// le tocan y **escribe el informe con el nombre del digest**, para que dos
/// alcances no se pisen. `intento` distingue un reintento tras un error (un Job
/// con el mismo nombre no se vuelve a correr): `"1"` la primera vez. Devuelve
/// `(fichero, texto, nombre del Job)`.
pub fn rendir_capa(
    plantilla: &str,
    entorno: &str,
    digest: &str,
    rama: &str,
    alcance: &str,
    intento: &str,
) -> Result<(String, String, String), String> {
    // Cada entorno, su plantilla, su mote y su número en la malla: dos Jobs
    // con el mismo nombre serían el mismo Job.
    let (nombre, mote, numero) = match entorno {
        "jvm" => (PLANTILLA_CAPA_JVM, "la-capa-jvm", "55"),
        "node" => (PLANTILLA_CAPA_NODE, "la-capa-node", "57"),
        _ => (PLANTILLA_CAPA, "la-capa", "52"),
    };
    if !plantilla.contains(&format!("{mote}-{RESUMEN_MODELO}")) {
        return Err(format!(
            "`{nombre}` no trae el hueco `{mote}-{RESUMEN_MODELO}`: o no es la plantilla, o `malla/{numero}-{mote}.yaml` cambió sin que esto se enterara"
        ));
    }
    if !digest.starts_with("capa-")
        || digest.len() != 17
        || !digest[5..].chars().all(|c| c.is_ascii_hexdigit())
    {
        return Err(format!(
            "`{digest}` no es el digest de una capa (`capa-<12 hex>`)"
        ));
    }
    for (de, a) in [
        (CAPA_MODELO, digest),
        (RAMA_MODELO, rama),
        (ALCANCE_MODELO, alcance),
        (INTENTO_MODELO, intento),
    ] {
        if !plantilla.contains(&format!("value: \"{de}\"")) {
            return Err(format!(
                "`{nombre}` no trae el hueco `value: \"{de}\"`: `malla/{numero}-{mote}.yaml` cambió sin que esto se enterara"
            ));
        }
        if a.contains('"') || a.contains('\n') {
            return Err(format!("`{a}` no puede ir en un valor del Job"));
        }
    }
    let t = plantilla
        .replace(
            &format!("value: \"{CAPA_MODELO}\""),
            &format!("value: \"{digest}\""),
        )
        .replace(
            &format!("value: \"{RAMA_MODELO}\""),
            &format!("value: \"{rama}\""),
        )
        .replace(
            &format!("value: \"{ALCANCE_MODELO}\""),
            &format!("value: \"{alcance}\""),
        )
        .replace(
            &format!("value: \"{INTENTO_MODELO}\""),
            &format!("value: \"{intento}\""),
        );
    let h = digest::de_bytes(t.as_bytes());
    let h = &h["sha256:".len().."sha256:".len() + 8];
    let corto = &digest[5..];
    let job = format!("{mote}-{corto}-{h}");
    let t = t.replace(&format!("{mote}-{RESUMEN_MODELO}"), &job);
    Ok((format!("{numero}-{mote}-{corto}.yaml"), t, job))
}

/// ⭐⭐ EL FALLO DE UN CATÁLOGO, tal como el Job lo deja en el árbol
/// (`.fallos/<fuente>.catalogo.txt`, `malla/44-el-catalogo.yaml`).
///
/// ⛔ Antes vivía solo en el log del pod, y el informador lo subía mientras el
///   pod existiera. Medido el 2026-09-27 en `victor`: una hora después del
///   fallo el Job seguía `Failed` y sus pods ya no estaban —nodos spot—, así
///   que la ficha decía «falló sin decir por qué» de un Job que SÍ lo dijo.
///   Lo que el cliente tiene que leer horas después no puede vivir en un pod.
///
/// Formato: `job: <nombre>`, `fin: <iso>`, una línea `---` y el log.
pub struct Fallo {
    pub job: String,
    pub fin: String,
    pub log: String,
}

pub fn fallo_de(texto: &str) -> Option<Fallo> {
    let (cabecera, log) = texto.split_once("\n---\n")?;
    let campo = |k: &str| {
        cabecera
            .lines()
            .find_map(|l| l.strip_prefix(k).map(|v| v.trim().to_string()))
    };
    Some(Fallo {
        job: campo("job:")?,
        fin: campo("fin:").unwrap_or_default(),
        log: log.to_string(),
    })
}

/// El nombre del Job que hay en un fichero de la cola: `name: catalogo-…`.
pub fn job_de(texto: &str) -> Option<&str> {
    texto
        .lines()
        .find_map(|l| l.trim().strip_prefix("name: catalogo-").map(|_| l.trim()))
        .and_then(|l| l.strip_prefix("name: "))
}

#[cfg(test)]
mod prueba {
    use super::*;

    /// **La copia en una rama** (0044 C.2 ④): otro Job y otro fichero, con la
    /// rama dentro; sin rama, lo de siempre; y una plantilla sin el hueco
    /// `RAMA` es un error, no una copia en `main`.
    #[test]
    fn la_copia_en_una_rama_es_otro_job() {
        let p = "metadata:\n  name: copiar-00000000\nenv:\n  - { name: VISTAS, value: \"olist.customers\" }\n  - { name: REHACER, value: \"\" }\n  - { name: RAMA, value: \"\" }\n";
        let v = vec!["ventas.copiaBase".to_string()];
        let (f0, t0) = rendir_copia(p, &v, None).unwrap();
        assert_eq!(f0, "48-la-copia.yaml");
        assert!(t0.contains("name: RAMA, value: \"\""), "{t0}");
        let (f1, t1) = rendir_copia(p, &v, Some("bea/datos")).unwrap();
        assert!(
            f1.starts_with("48-la-copia-rama-") && f1.ends_with(".yaml"),
            "{f1}"
        );
        assert!(t1.contains("name: RAMA, value: \"bea/datos\""), "{t1}");
        assert!(t1.contains("name: copiar-rama-"), "{t1}");
        let (f2, _) = rendir_copia(p, &v, Some("otra")).unwrap();
        assert_ne!(f1, f2, "dos ramas, dos Jobs");
        let (fr, tr) = rendir_rehacer(p, &v, "20260930T100000Z", Some("bea/datos")).unwrap();
        assert!(fr.starts_with("48-la-copia-rehacer-"), "{fr}");
        assert!(tr.contains("name: RAMA, value: \"bea/datos\""), "{tr}");
        let sin = "metadata:\n  name: copiar-00000000\nenv:\n  - { name: VISTAS, value: \"olist.customers\" }\n";
        assert!(rendir_copia(sin, &v, None).is_ok());
        assert!(rendir_copia(sin, &v, Some("bea/datos")).is_err());
    }

    /// Volver a catalogar da OTRO Job en el MISMO fichero: sin la corrida, el
    /// nombre saldría igual y Flux no crearía nada.
    #[test]
    fn el_job_de_catalogo_lleva_el_dueno_de_quien_dio_de_alta_el_origen() {
        let p = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../malla/44-el-catalogo.yaml"),
        )
        .unwrap();
        let (_, sin) = rendir_corrida(&p, "ventas", None, None).unwrap();
        let (f, con) = rendir_corrida(&p, "ventas", None, Some("user:ana")).unwrap();
        assert_eq!(f, "44-el-catalogo-ventas.yaml");
        assert!(!sin.contains("name: DUENO"), "sin dueño no se inventa");
        let fuente = con
            .lines()
            .find(|l| l.contains("{ name: FUENTE, value: \"ventas\" }"))
            .unwrap();
        let dueno = con
            .lines()
            .find(|l| l.contains("{ name: DUENO, value: \"user:ana\" }"))
            .unwrap();
        // Con la misma sangría: es otra entrada de la misma lista `env`.
        assert_eq!(
            fuente.len() - fuente.trim_start().len(),
            dueno.len() - dueno.trim_start().len()
        );
        assert_ne!(job_de(&sin), job_de(&con), "otro contenido, otro Job");
        assert_eq!(con.lines().count(), sin.lines().count() + 1);
    }

    #[test]
    fn volver_a_catalogar_da_otro_job() {
        let p =
            "metadata:\n  name: catalogo-bq-00000000\nenv:\n  - { name: FUENTE, value: \"bq\" }\n";
        let (f0, t0) = rendir(p, "ventas").unwrap();
        let (f1, t1) = rendir_corrida(p, "ventas", Some("20260927T150000Z"), None).unwrap();
        let (_, t2) = rendir_corrida(p, "ventas", Some("20260927T150001Z"), None).unwrap();
        assert_eq!(f0, f1);
        let nombre = |t: &str| {
            t.lines()
                .find(|l| l.contains("name: catalogo-"))
                .unwrap()
                .to_string()
        };
        assert_ne!(nombre(&t0), nombre(&t1));
        assert_ne!(nombre(&t1), nombre(&t2));
    }

    /// El fallo se lee, y el Job de la cola se reconoce: si son el mismo, la
    /// fuente está `fallida`; si no, alguien pulsó «Reintentar».
    #[test]
    fn el_fallo_se_lee_y_se_empareja_con_la_cola() {
        let f = fallo_de("job: catalogo-bq-1a2b3c4d\nfin: 2026-09-27T12:37:47Z\n---\n  ✗ jobs · `roles/bigquery.jobUser` en proyecto p\n").unwrap();
        assert_eq!(f.job, "catalogo-bq-1a2b3c4d");
        assert_eq!(f.fin, "2026-09-27T12:37:47Z");
        assert!(f.log.contains("jobUser"));
        assert!(fallo_de("sin cabecera").is_none());
        let cola = "apiVersion: batch/v1\nkind: Job\nmetadata:\n  name: catalogo-bq-1a2b3c4d\n  namespace: t-demo\n";
        assert_eq!(job_de(cola), Some("catalogo-bq-1a2b3c4d"));
    }

    /// La comprobación: la URL y la corrida entran, el nombre lleva el
    /// resumen, y la ranura es por URL.
    #[test]
    fn la_comprobacion_rinde_su_job() {
        let p = "metadata:\n  name: comprobar-00000000\n  env:\n    - { name: TIPO, value: \"bigquery\" }\n    - { name: URL, value: \"bigquery://modelo/dataset\" }\n    - { name: CORRIDA, value: \"00000000T000000Z\" }\n";
        let (f, t) =
            rendir_comprobacion(p, "bigquery", "bigquery://acme/ventas", "20260927T100000Z")
                .unwrap();
        assert_eq!(f, "54-la-comprobacion-acme-ventas.yaml");
        assert!(t.contains("value: \"bigquery://acme/ventas\""), "{t}");
        assert!(!t.contains("comprobar-00000000"), "{t}");
        let (_, t2) =
            rendir_comprobacion(p, "bigquery", "bigquery://acme/ventas", "20260927T100001Z")
                .unwrap();
        assert_ne!(t, t2, "dos clics, dos Jobs");
        assert!(rendir_comprobacion(p, "bigquery", "bigquery://a/\"x", "c").is_err());
    }

    /// Solo viaja en claro una URL sin credencial.
    #[test]
    fn solo_se_comprueba_una_url_sin_secreto() {
        assert_eq!(url_sin_secreto("bigquery://acme/ventas"), Ok("bigquery"));
        assert!(url_sin_secreto("postgres://u:clave@h/db").is_err());
        assert!(url_sin_secreto("bigquery://u:x@acme/ventas").is_err());
        // ⭐ 0046 E9b: S3 por rol sí; con clave, o con un rol y además un secreto, no.
        let rol = "s3://cubo/p?region=eu-north-1&role_arn=arn:aws:iam::123456789012:role/r";
        assert_eq!(url_sin_secreto(rol), Ok("s3"));
        assert!(url_sin_secreto("s3://cubo?region=x&access_key_id=a&secret_access_key=b").is_err());
        assert!(url_sin_secreto(&format!("{rol}&secret_access_key=b")).is_err());
        assert!(url_sin_secreto(&format!("{rol}&session_token=t")).is_err());
        assert!(
            url_sin_secreto("s3://u:p@cubo?role_arn=arn:aws:iam::123456789012:role/r").is_err()
        );
        // ADR 0061 O2·3: GCS, con la cuenta de la celda o suplantando; nada más.
        assert_eq!(url_sin_secreto("gs://cubo/docs/"), Ok("gcs"));
        assert_eq!(
            url_sin_secreto("gs://cubo/?suplantar=l@c.iam.gserviceaccount.com"),
            Ok("gcs")
        );
        assert!(url_sin_secreto("gs://cubo/?endpoint=https://evil.io").is_err());
        assert!(url_sin_secreto("gs://cubo/?token=x").is_err());
        assert!(url_sin_secreto("gs://u@cubo/").is_err());
        // ADR 0061 O3·3: Azure, con la app de Entra del cliente y nada más.
        assert_eq!(
            url_sin_secreto("az://cuenta/cubo/docs/?tenant=t&cliente=c"),
            Ok("azure")
        );
        assert!(url_sin_secreto("az://cuenta/cubo?tenant=t").is_err());
        assert!(url_sin_secreto("az://cuenta/cubo?tenant=t&cliente=c&sig=x").is_err());
        assert!(url_sin_secreto("az://cuenta/cubo?tenant=t&cliente=c&endpoint=https://e").is_err());
        // ADR 0061 O5·3: SharePoint, igual.
        assert_eq!(
            url_sin_secreto(
                "sharepoint://contoso.sharepoint.com/sites/x/Documentos/?tenant=t&cliente=c"
            ),
            Ok("sharepoint")
        );
        assert!(
            url_sin_secreto("sharepoint://contoso.sharepoint.com/Documentos?cliente=c").is_err()
        );
        assert!(
            url_sin_secreto(
                "sharepoint://contoso.sharepoint.com/D?tenant=t&cliente=c&endpoint=http://e"
            )
            .is_err()
        );
        assert!(
            url_sin_secreto("sharepoint://u@contoso.sharepoint.com/D?tenant=t&cliente=c").is_err()
        );
        // ADR 0061 O4·3: SFTP con la clave de la celda; con contraseña, no.
        assert_eq!(
            url_sin_secreto("sftp://ore@sftp.cliente.com:2222/datos/?huella=SHA256:abc&edad=60"),
            Ok("sftp")
        );
        assert!(url_sin_secreto("sftp://ore:clave@h/datos/").is_err());
        assert!(url_sin_secreto("sftp://h/datos/").is_err());
        assert!(url_sin_secreto("sftp://ore@h/?clave=x").is_err());
    }

    /// 0050 P4: la etiqueta que abre la puerta va en el POD (que es lo que la
    /// `NetworkPolicy` selecciona), sobre la plantilla de verdad, y el Job sigue
    /// rindiéndose; sin la etiqueta del rol, no se adivina dónde ponerla.
    #[test]
    fn la_salida_al_modelo_va_en_el_pod_del_trabajo() {
        let p = include_str!("../../../malla/51-el-puesto.yaml");
        let con = con_salida_al_modelo(p).unwrap();
        assert_eq!(con.matches("ore.dev/usa-modelo: \"si\"").count(), 1);
        assert!(con.contains(
            "        ore.dev/rol: puesto
        ore.dev/usa-modelo: \"si\"
"
        ));
        assert!(
            !p.contains("usa-modelo: \"si\""),
            "la plantilla no la lleva: la pone ore-serve"
        );
        let (_, t, job) = rendir_puesto(
            &con,
            "trabajo-ana-1a2b3c4d",
            "",
            "",
            "1",
            "python",
            "x.py@abc",
        )
        .unwrap();
        let (_, _, job_sin) =
            rendir_puesto(p, "trabajo-ana-1a2b3c4d", "", "", "1", "python", "x.py@abc").unwrap();
        assert!(t.contains("ore.dev/usa-modelo"));
        assert_ne!(
            job, job_sin,
            "el nombre resume el contenido, etiqueta incluida"
        );
        assert!(con_salida_al_modelo("kind: Job").is_err());
    }

    #[test]
    fn el_puesto_lleva_id_rama_y_capa_y_el_mismo_puesto_es_el_mismo_fichero() {
        let p = "name: puesto-00000000
image: registro/ore/puesto-entorno-modelo:1
env:
  - { name: PUESTO, value: \"puesto-modelo\" }
  - { name: RAMA, value: \"rama-modelo\" }
  - { name: CAPA, value: \"capa-modelo\" }
  - { name: ABIERTO, value: \"abierto-modelo\" }
  - { name: ENTORNO, value: \"entorno-modelo\" }
";
        let (f, t, job) = rendir_puesto(
            p,
            "puesto-ana-python",
            "ana/x",
            "capa-0123456789ab",
            "1",
            "python",
            "",
        )
        .unwrap();
        assert_eq!(f, "51-el-puesto-ana-python.yaml");
        assert!(
            job.starts_with("puesto-ana-python-") && job.len() == "puesto-ana-python-".len() + 8
        );
        assert!(
            t.contains("value: \"puesto-ana-python\"")
                && t.contains("value: \"ana/x\"")
                && t.contains("value: \"capa-0123456789ab\"")
                && t.contains("image: registro/ore/puesto-python:1")
                // ⭐ 0037 ③c: y el NOMBRE del entorno, que es lo que
                //   `traer-la-capa` mira para bajar ruedas o jars.
                && t.contains("{ name: ENTORNO, value: \"python\" }")
        );
        assert!(t.contains(&format!("name: {job}")));
        // R1 · Y la credencial del pod DECLARA el puesto y la apertura.
        let con_audiencia = format!(
            "{p}token:
  audience: ore-serve/puestos/puesto-modelo/abierto-modelo
"
        );
        let (_, ta, _) = rendir_puesto(
            &con_audiencia,
            "puesto-ana-python",
            "",
            "",
            "1759",
            "python",
            "",
        )
        .unwrap();
        assert!(
            ta.contains(
                "audience: ore-serve/puestos/puesto-ana-python/1759
"
            ),
            "{ta}"
        );
        assert!(!ta.contains("puesto-modelo") && !ta.contains("abierto-modelo"));
        let (_, t2, _) = rendir_puesto(p, "puesto-ana-python", "", "", "1", "python", "").unwrap();
        assert!(t2.contains("value: \"\""));
        // Reabrir en otro instante es OTRO Job (Flux retira el viejo): el fichero, el mismo.
        let (f3, _, job3) = rendir_puesto(
            p,
            "puesto-ana-python",
            "ana/x",
            "capa-0123456789ab",
            "2",
            "python",
            "",
        )
        .unwrap();
        assert_eq!(f3, f);
        assert_ne!(job3, job);
        // Otro entorno (W3.4): otra imagen, otro fichero, otro Job.
        let (f4, t4, job4) = rendir_puesto(p, "puesto-ana-node", "", "", "1", "node", "").unwrap();
        assert_eq!(f4, "51-el-puesto-ana-node.yaml");
        assert!(
            t4.contains("image: registro/ore/puesto-node:1")
                && job4.starts_with("puesto-ana-node-")
        );
        assert!(rendir_puesto(p, "puesto-ana-rust", "", "", "1", "rust", "").is_err());
        assert!(rendir_puesto("name: otra-cosa", "puesto-ana", "", "", "1", "python", "").is_err());
        // Un trabajo pide el hueco `TRABAJO`; con él, el Job se llama por su id
        // y el fichero es `54-el-trabajo-…`
        assert!(
            rendir_puesto(p, "trabajo-ana-1a2b3c4d", "", "", "1", "python", "x.py@abc").is_err()
        );
        let pt = format!("{p}  - {{ name: TRABAJO, value: \"trabajo-modelo\" }}\n");
        let (f5, t5, job5) = rendir_puesto(
            &pt,
            "trabajo-ana-1a2b3c4d",
            "",
            "",
            "1",
            "python",
            "packages/p/transforms/x.py@abc123",
        )
        .unwrap();
        assert_eq!(f5, "54-el-trabajo-ana-1a2b3c4d.yaml");
        assert!(job5.starts_with("trabajo-ana-1a2b3c4d-"), "{job5}");
        assert!(t5.contains("value: \"packages/p/transforms/x.py@abc123\""));
        assert!(t5.contains(&format!("name: {job5}")));
    }

    #[test]
    fn la_capa_lleva_digest_rama_alcance_e_intento_y_se_llama_por_el_digest() {
        let p = "name: la-capa-00000000
env:
  - { name: CAPA, value: \"capa-modelo\" }
  - { name: RAMA, value: \"rama-modelo\" }
  - { name: ALCANCE, value: \"alcance-modelo\" }
  - { name: INTENTO, value: \"intento-modelo\" }
";
        let (f, t, job) = rendir_capa(p, "python", "capa-0123456789ab", "", "", "1").unwrap();
        assert_eq!(f, "52-la-capa-0123456789ab.yaml");
        assert!(
            job.starts_with("la-capa-0123456789ab-")
                && job.len() == "la-capa-0123456789ab-".len() + 8
        );
        assert!(t.contains("value: \"capa-0123456789ab\"") && t.contains(&format!("name: {job}")));
        let (_, _, job2) = rendir_capa(p, "python", "capa-0123456789ab", "", "", "r2").unwrap();
        assert_ne!(job, job2);
        assert!(rendir_capa(p, "python", "no-es-un-digest", "", "", "1").is_err());
        // El alcance viaja al Job (0036 ③): es lo que hace que el Job sume los
        // `pyproject.toml` del repositorio y no los de toda la celda.
        let (_, t, _) =
            rendir_capa(p, "python", "capa-0123456789ab", "", "packages/hr/raw", "1").unwrap();
        assert!(t.contains("value: \"packages/hr/raw\""), "{t}");
    }

    /// Los MISMOS casos que fija `gen-inquilino.py`. Si los dos dejan de
    /// coincidir, el aprovisionador y el alta encolarían Jobs con nombres
    /// distintos para la misma fuente — y habría dos.
    #[test]
    fn el_nombre_de_objeto_coincide_con_el_renderizador() {
        assert_eq!(nombre_de_objeto("bq"), "bq");
        assert_eq!(
            nombre_de_objeto("postgresql_20260910_074550"),
            "postgresql-20260910-074550"
        );
        assert_eq!(nombre_de_objeto("Ventas.2024"), "ventas-2024");
        assert_eq!(nombre_de_objeto("---"), "sin-nombre");
        assert_eq!(nombre_de_objeto(""), "sin-nombre");
        // Cortado a 30, y sin dejar un guion colgando al final.
        assert_eq!(nombre_de_objeto(&"a".repeat(60)).len(), 30);
    }

    #[test]
    fn la_invocacion_lleva_funcion_puerta_id_y_corrida_y_dos_corridas_son_dos_jobs() {
        let p = "name: invocar-00000000
env:
  - { name: FUNCION, value: \"ventas.clasificar\" }
  - { name: MODELO_URL, value: \"http://10.10.0.100:8000/v1\" }
  - { name: MODELO_ID, value: \"modelo-id\" }
  - { name: CORRIDA, value: \"00000000T000000Z\" }
";
        let i = Invocacion {
            funcion: "olist_copia.traducirCategoria",
            puerta: "http://x:8000/v1",
            modelo: "deepseek-ai/DeepSeek-V2-Lite",
            corrida: "20260917T200000Z",
        };
        let (f, a) = rendir_invocacion(p, &i).unwrap();
        assert_eq!(f, "49-la-invocacion-olist-copia-traducircategoria.yaml");
        assert!(
            a.contains("value: \"olist_copia.traducirCategoria\"")
                && a.contains("value: \"http://x:8000/v1\"")
                && a.contains("value: \"deepseek-ai/DeepSeek-V2-Lite\"")
                && a.contains("value: \"20260917T200000Z\""),
            "{a}"
        );
        assert!(
            a.lines()
                .next()
                .unwrap()
                .starts_with("name: invocar-olist-copia-traducircategoria-"),
            "{a}"
        );
        let (_, b) = rendir_invocacion(
            p,
            &Invocacion {
                corrida: "20260917T200001Z",
                ..i
            },
        )
        .unwrap();
        assert_ne!(
            a.lines().next(),
            b.lines().next(),
            "otra corrida es otro Job"
        );
        assert!(
            rendir_invocacion(
                "name: x
", &i
            )
            .is_err()
        );
        assert!(
            rendir_invocacion(
                p,
                &Invocacion {
                    funcion: "a\"b",
                    ..i
                }
            )
            .is_err()
        );
    }

    /// ⛔ Una plantilla que no trae el hueco NO se rinde a medias.
    #[test]
    fn sin_hueco_se_niega() {
        assert!(rendir("apiVersion: batch/v1\nkind: Job\n", "bq").is_err());
    }

    #[test]
    fn el_nombre_lleva_el_resumen_y_es_determinista() {
        let p = "name: catalogo-bq-00000000\nenv:\n  - { name: FUENTE, value: \"bq\" }\n";
        let (f, a) = rendir(p, "ventas").unwrap();
        let (_, b) = rendir(p, "ventas").unwrap();
        assert_eq!(f, "44-el-catalogo-ventas.yaml");
        assert_eq!(a, b, "el mismo contenido tiene que dar el mismo nombre");
        assert!(a.contains("value: \"ventas\""));
        let n = a.lines().next().unwrap();
        assert!(n.starts_with("name: catalogo-ventas-"), "{n}");
        assert_eq!(n.len(), "name: catalogo-ventas-".len() + 8, "{n}");
        // Y una fuente distinta da un nombre distinto.
        let (_, c) = rendir(p, "compras").unwrap();
        assert!(c.contains("catalogo-compras-"));
    }
}
