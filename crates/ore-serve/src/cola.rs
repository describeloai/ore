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

/// El hueco del modo programado en `malla/48-la-copia.yaml` (0063 S8).
pub const PROGRAMADA_DE_LA_COPIA: &str = "name: PROGRAMADA, value: \"\"";

/// Los ficheros que el refresco programado deja en la cola.
pub const PREFIJO_PROGRAMADA: &str = "48-la-copia-prog-";

/// ⭐ 0063 S8 · **La pasada programada**: el Job de la copia en `main` SÓLO
/// para `vistas` (las que van a incumplir su `freshness`) y con la semántica de
/// siempre —la señal de cambio, «ya está», las promesas—, no la de rehacer.
/// `PROGRAMADA` lleva la franja (ISO): dos vueltas de la misma franja con las
/// mismas vistas rinden el mismo fichero, y la siguiente franja, otro Job. Una
/// plantilla sin el hueco es un error con su remedio: converger el inquilino.
pub fn rendir_programada(
    plantilla: &str,
    vistas: &[String],
    franja: &str,
) -> Result<(String, String), String> {
    if !plantilla.contains(PROGRAMADA_DE_LA_COPIA) {
        return Err(format!(
            "`{PLANTILLA_COPIA}` no sabe del refresco programado (no trae el hueco \
             `PROGRAMADA`): hay que converger este inquilino"
        ));
    }
    let (_, t) = rendir_copia(plantilla, vistas, None)?;
    let t = t.replace(
        PROGRAMADA_DE_LA_COPIA,
        &format!("name: PROGRAMADA, value: \"{franja}\""),
    );
    let h = digest::de_bytes(t.as_bytes());
    let h = &h["sha256:".len().."sha256:".len() + 8];
    let mut salida = String::new();
    for linea in t.lines() {
        if linea.starts_with("  name: copiar-") {
            salida.push_str(&format!("  name: copiar-prog-{h}\n"));
        } else {
            salida.push_str(linea);
            salida.push('\n');
        }
    }
    Ok((format!("{PREFIJO_PROGRAMADA}{h}.yaml"), salida))
}

/// La franja de un fichero programado (lo que lleva `PROGRAMADA`), para
/// retirar los que ya pasaron.
pub fn franja_de(texto: &str) -> Option<&str> {
    let i = texto.find("name: PROGRAMADA, value: \"")? + "name: PROGRAMADA, value: \"".len();
    let resto = &texto[i..];
    Some(&resto[..resto.find('"')?]).filter(|f| !f.is_empty())
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

// ── La pirámide de una capa (0072 F2·3·2·4) ─────────────────────────────────

/// Lo que el aprovisionador deja en la cola para construir una pirámide (59).
pub const PLANTILLA_PIRAMIDE: &str = "plantilla-piramide.txt";
const PETICION_MODELO: &str = "value: '{\"modelo\":\"piramide\"}'";

/// Rinde el Job que construye la pirámide de una instantánea: `peticion` es lo que
/// `ore-piramide` lee (JSON de una línea, sin credenciales: el Job usa la identidad del
/// driver). Una ranura por TABLA (`59-la-piramide-<resumen de la tabla>.yaml`): una
/// instantánea nueva sustituye al Job de la anterior y Flux lo poda; el nombre del Job
/// lleva el resumen de lo rendido, así que la misma petición es el mismo Job.
pub fn rendir_piramide(
    plantilla: &str,
    tabla: &str,
    peticion: &str,
) -> Result<(String, String), String> {
    if !plantilla.contains(&format!("piramide-{RESUMEN_MODELO}")) {
        return Err(format!(
            "`{PLANTILLA_PIRAMIDE}` no trae el hueco `piramide-{RESUMEN_MODELO}`: o no es la \
             plantilla, o `malla/59-la-piramide.yaml` cambió sin que esto se enterara"
        ));
    }
    if !plantilla.contains(PETICION_MODELO) {
        return Err(format!(
            "`{PLANTILLA_PIRAMIDE}` no trae el hueco `{PETICION_MODELO}`: \
             `malla/59-la-piramide.yaml` cambió sin que esto se enterara"
        ));
    }
    if peticion.contains('\n') || peticion.contains('\'') {
        return Err("la petición de la pirámide no puede ir en un valor del Job".into());
    }
    let t = plantilla.replace(PETICION_MODELO, &format!("value: '{peticion}'"));
    let h = digest::de_bytes(t.as_bytes());
    let h = &h["sha256:".len().."sha256:".len() + 8];
    let r = digest::de_bytes(tabla.as_bytes());
    let r = &r["sha256:".len().."sha256:".len() + 12];
    let t = t.replace(
        &format!("piramide-{RESUMEN_MODELO}"),
        &format!("piramide-{h}"),
    );
    Ok((format!("59-la-piramide-{r}.yaml"), t))
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
    //   `cliente`) y nada más —ni `endpoint`, ni una SAS, ni una clave—. O es
    //   público (D-O6): `publico=1`, sin app y sin identidad.
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
        if let Some(k) = claves
            .iter()
            .find(|k| !matches!(**k, "tenant" | "cliente" | "publico"))
        {
            return Err(format!(
                "an Azure URL only takes `tenant` and `cliente` (or `publico=1`), not `{k}`: nothing else is checked"
            ));
        }
        let app = claves.contains(&"tenant") && claves.contains(&"cliente");
        let publico = consulta.split('&').any(|p| p == "publico=1");
        if app == publico || (publico && claves.len() != 1) {
            return Err(
                "an Azure URL names the Entra app (`tenant` and `cliente`), or is `publico=1` for a public container"
                    .into(),
            );
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
/// Los entornos que tienen imagen (`Dockerfile`, el job `construir` de ci.yml).
/// `python-notebooks` (0064): Analytics y Models, con marimo — su familia
/// manda antes que su lenguaje (`ore_core::clases::entorno_de`).
pub const ENTORNOS: [&str; 5] = ["python", "node", "jvm", SPARK, "python-notebooks"];

/// ORE 0068 C2·3 · **El motor Heavyweight**: `puesto-spark:1`, el puesto de
/// Python más Spark 4.1 y Sedona (C2·2). Lo elige el `motor` del repositorio
/// para sus Builds, nunca un lenguaje: no hay celda «spark».
pub const SPARK: &str = "spark";

/// El entorno cuyas dependencias declaradas (la capa) lleva uno: `spark` sale
/// de `puesto-python` (`FROM puesto-python`), así que su capa es la de Python
/// —las mismas ruedas, resueltas contra el mismo intérprete—, y `traer-la-capa`
/// baja ruedas.
pub fn entorno_de_la_capa(entorno: &str) -> &str {
    if entorno == SPARK { "python" } else { entorno }
}

/// ⭐ Y los recursos del puesto pesado (0068 C2·3), sobre los de la plantilla:
/// `(lo que trae, lo que pone)`. Un `n2-highmem-8` (8 vCPU, 64 GB; decidido
/// 2026-10-18) deja ~57 GiB al pod tras lo que GKE reserva y sus DaemonSets:
/// 48 GiB son el pod entero, con margen. Pedir 7 CPU es lo que hace que Kueue
/// sólo lo admita en el *flavor* pesado (los demás tienen 1 y 1,5).
/// `ORE_MEMORIA_MB` dice lo mismo que `limits.memory` (`gen-inquilino` ⑱): el
/// arnés reparte desde ahí (pieza 0: la JVM ocupa el heap + ~0,8 GiB). Se
/// confirman en victor (pieza 6).
///
/// ⭐ (pieza 6, decidido 2026-10-19) **Y su cola es otra**: `cola-pesada`, que
/// lleva a la `ClusterQueue` del pool pesado y a nada más (`malla/11`). Con el
/// sabor pesado en la cola de siempre, un puesto ligero que no cupiera en
/// `system` ni en `jobs` encendería un nodo de 64 GB; y uno pesado no puede
/// caer en un nodo ligero.
///
/// ⭐ 0068 CE·6 · **Y su disco**: 250 Gi de `ephemeral-storage`, en la SSD local
/// del pool pesado (281 GiB asignables, medido el 2026-10-20). Sin pedirlo, un
/// *shuffle* grande llenaba el disco y el kubelet desalojaba el NODO entero
/// (C3·d, Q11); con el tope, si se pasa cae este pod y con un error que lo dice.
/// La `ClusterQueue` pesada lo cubre (`malla/11`), o Kueue no lo admitiría.
const RECURSOS_DEL_PUESTO_PESADO: [(&str, &str); 4] = [
    (
        "kueue.x-k8s.io/queue-name: cola\n",
        "kueue.x-k8s.io/queue-name: cola-pesada\n",
    ),
    (
        "requests: {cpu: \"100m\", memory: 768Mi}",
        "requests: {cpu: \"7\", memory: 48Gi, ephemeral-storage: 250Gi}",
    ),
    (
        "limits:   {cpu: \"2\", memory: 4Gi}",
        "limits:   {cpu: \"8\", memory: 48Gi, ephemeral-storage: 250Gi}",
    ),
    (
        "name: ORE_MEMORIA_MB, value: \"4096\"",
        "name: ORE_MEMORIA_MB, value: \"49152\"",
    ),
];

/// ⭐ Query Q1 · **Y el tope de `/trabajo`**, que la plantilla pone a 4 GiB para
/// una sesión (`malla/51`: una sesión llenó el disco del nodo del sistema el
/// 2026-10-23) y que el puesto pesado sube a 240 GiB, por debajo de los 250 GiB
/// de `ephemeral-storage` que pide: su derrame va a la SSD local y no se capa.
/// `ORE_DISCO_MB` dice lo mismo que el `sizeLimit` (`gen-inquilino` ㉑).
///
/// Opcional, a diferencia de los recursos de arriba: una celda sin converger
/// trae la plantilla de antes, sin tope, y entonces no hay nada que subir (el
/// binario va antes que la malla).
const DISCO_DEL_PUESTO_PESADO: [(&str, &str); 2] = [
    ("emptyDir: {sizeLimit: 4Gi}", "emptyDir: {sizeLimit: 240Gi}"),
    (
        "name: ORE_DISCO_MB, value: \"4096\"",
        "name: ORE_DISCO_MB, value: \"245760\"",
    ),
];

/// ⭐ 0068 C3·c · **El Build elástico.** Un Build pesado con entradas grandes
/// no es un pod de 7 CPU / 48 GiB con `local[*]`: es su DRIVER (este pod,
/// pequeño y fuera de *spot*: su cola `cola-driver` y el sabor `pesado-driver`,
/// `malla/22` y `malla/74`), que pide
/// a Kubernetes los ejecutores según la carga, cada uno a la cola de su celda
/// (`spark.py`, `_elastico`). Medido en victor (C3·c0): escala de 1 a 2 solo,
/// 200 M × 20 k en 79 s frente a 116 s en un pod; perder un ejecutor da las
/// mismas cuentas. Un Build pequeño sigue en un pod: en frío, el elástico
/// tardó 170 s frente a 115 s.
///
/// Lo que cambia sobre el trabajo pesado ya rendido: los recursos del driver,
/// su cuenta (`driver-pesado`: puede crear ejecutores, y la política de la
/// celda dice cuáles), su etiqueta de red y lo que el SDK necesita saber.
const DEL_PESADO_AL_DRIVER: [(&str, &str); 5] = [
    // Su cola (`malla/22`): el sabor `pesado-driver`, uno por celda.
    (
        "kueue.x-k8s.io/queue-name: cola-pesada\n",
        "kueue.x-k8s.io/queue-name: cola-driver\n",
    ),
    // El driver no vuelca *shuffle* (lo hacen sus ejecutores) y su cola no
    // cubre disco: vuelve a no pedirlo.
    (
        "requests: {cpu: \"7\", memory: 48Gi, ephemeral-storage: 250Gi}",
        "requests: {cpu: \"1500m\", memory: 12Gi}",
    ),
    (
        "limits:   {cpu: \"8\", memory: 48Gi, ephemeral-storage: 250Gi}",
        "limits:   {cpu: \"2\", memory: 12Gi}",
    ),
    (
        "serviceAccountName: puesto\n",
        "serviceAccountName: driver-pesado\n",
    ),
    (
        "\n        ore.dev/rol: puesto\n",
        "\n        ore.dev/rol: puesto\n        ore.dev/spark: driver\n",
    ),
];
/// El techo de ejecutores de un Build elástico: los nodos del pool `pesado`
/// (`malla/73`). Kueue garantiza uno por celda y presta el resto.
pub const TECHO_DE_EJECUTORES: u32 = 2;

/// El trabajo pesado ya rendido, como driver de un Build elástico con `techo`
/// ejecutores como mucho. Cada cambio tiene que encontrar su texto UNA vez:
/// si la plantilla cambia sin que esto se entere, se dice, no se lanza a medias.
pub fn al_driver_elastico(texto: &str, techo: u32) -> Result<String, String> {
    let mut t = texto.to_string();
    for (de, a) in DEL_PESADO_AL_DRIVER {
        if t.matches(de).count() != 1 {
            return Err(format!(
                "el trabajo pesado no trae (una vez) `{}`: no se puede hacer driver elástico",
                de.trim()
            ));
        }
        t = t.replacen(de, a, 1);
    }
    let memoria = "name: ORE_MEMORIA_MB, value: \"49152\" }";
    let Some(i) = t.find(memoria) else {
        return Err("el trabajo pesado no trae `ORE_MEMORIA_MB` de 48 GiB".into());
    };
    // La línea entera, para poner las nuevas con su sangría.
    let ini = t[..i].rfind('\n').map(|n| n + 1).unwrap_or(0);
    let sangria = t[ini..i].to_string(); // "            - { "
    let imagen = t
        .lines()
        .find_map(|l| {
            l.trim()
                .strip_prefix("image: ")
                .filter(|i| i.ends_with("/puesto-spark:1"))
        })
        .ok_or("el trabajo pesado no trae la imagen `puesto-spark:1`")?
        .to_string();
    let nuevas = format!(
        "{sangria}name: ORE_MEMORIA_MB, value: \"12288\" }}\n\
         {sangria}name: ORE_SPARK_ELASTICO, value: \"{techo}\" }}\n\
         {sangria}name: ORE_SPARK_IMAGEN, value: \"{imagen}\" }}\n\
         {sangria}name: ORE_POD_NOMBRE, valueFrom: {{fieldRef: {{fieldPath: metadata.name}}}} }}\n\
         {sangria}name: ORE_POD_IP, valueFrom: {{fieldRef: {{fieldPath: status.podIP}}}} }}"
    );
    let fin = i + memoria.len();
    t.replace_range(ini..fin, &nuevas);
    Ok(t)
}

/// ⭐ Elastic Sessions S1 · **Una sesión, al *session pool*** (`--session-pool`;
/// docs/decisions/0073-elastic-sessions.md §2.1). Sobre el puesto ya rendido:
/// - **su cola**: `cola-sesiones`, que sólo lleva al sabor `sesiones` (`malla/10`,
///   `malla/11`; el pool de `malla/75`, n2-highmem-8 con SSD local y
///   `singleProcessOOMKill`). Los trabajos y el puesto pesado no pasan por aquí;
/// - **crece en caliente**: `resizePolicy NotRequired` en memoria y CPU, para que
///   `kubectl patch --subresource resize` cambie el cgroup sin reiniciar (R0·2:
///   2–4 s). Nace igual que hoy (la base de C1, y `ORE_MEMORIA_MB` dice su
///   límite, ⑱): crecer es de S2–S3;
/// - **su `/trabajo`, en la SSD**: de 4 a 64 GiB, y `ORE_DISCO_MB` con él (㉑).
///   Sin pedir `ephemeral-storage`: 64 GiB por sesión reservados dejarían cuatro
///   por nodo. Si una se pasa del tope, el kubelet la desaloja a ella sola.
///
/// Opcional como en el pesado sólo el disco: una plantilla sin el tope de Q1 no
/// tiene nada que subir. La cola y los recursos tienen que estar una vez.
pub fn al_session_pool(texto: &str) -> Result<String, String> {
    let mut t = texto.to_string();
    for (de, a) in AL_SESSION_POOL {
        if t.matches(de).count() != 1 {
            return Err(format!(
                "el puesto no trae (una vez) `{}`: no se puede mandar al session pool",
                de.trim()
            ));
        }
        t = t.replacen(de, a, 1);
    }
    for (de, a) in DISCO_DEL_SESSION_POOL {
        t = t.replacen(de, a, 1);
    }
    Ok(t)
}

const AL_SESSION_POOL: [(&str, &str); 2] = [
    (
        "kueue.x-k8s.io/queue-name: cola\n",
        "kueue.x-k8s.io/queue-name: cola-sesiones\n",
    ),
    // El límite del contenedor del puesto (el `initContainer` tiene 2Gi): detrás,
    // con su misma sangría, la política de redimensionado.
    (
        "            limits:   {cpu: \"2\", memory: 4Gi}\n",
        "            limits:   {cpu: \"2\", memory: 4Gi}\n          resizePolicy: [{resourceName: memory, restartPolicy: NotRequired}, {resourceName: cpu, restartPolicy: NotRequired}]\n",
    ),
];

const DISCO_DEL_SESSION_POOL: [(&str, &str); 2] = [
    ("emptyDir: {sizeLimit: 4Gi}", "emptyDir: {sizeLimit: 64Gi}"),
    (
        "name: ORE_DISCO_MB, value: \"4096\"",
        "name: ORE_DISCO_MB, value: \"65536\"",
    ),
];

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
    if entorno == SPARK {
        for (de, _) in RECURSOS_DEL_PUESTO_PESADO {
            if plantilla.matches(de).count() != 1 {
                return Err(format!(
                    "`{PLANTILLA_PUESTO}` no trae (una vez) `{de}`: los recursos del puesto pesado se ponen sobre ellos, y `malla/51-el-puesto.yaml` cambió sin que esto se enterara"
                ));
            }
        }
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
        // `traer-la-capa` sólo distingue ruedas, jars y npm: el pesado baja ruedas.
        .replace(
            &format!("value: \"{ENTORNO_VALOR_MODELO}\""),
            &format!("value: \"{}\"", entorno_de_la_capa(entorno)),
        )
        .replace(ENTORNO_MODELO, &format!("puesto-{entorno}:1"))
        .replace(
            &format!("value: \"{TRABAJO_MODELO}\""),
            &format!("value: \"{trabajo}\""),
        );
    let t = if entorno == SPARK {
        RECURSOS_DEL_PUESTO_PESADO
            .iter()
            .chain(DISCO_DEL_PUESTO_PESADO.iter())
            .fold(t, |t, (de, a)| t.replacen(de, a, 1))
    } else if entorno == "python-notebooks" {
        // ⭐ 0064: `python-notebooks` sube SU petición de memoria (no el límite,
        //   que ya es 4Gi, ni `ORE_MEMORIA_MB`, que sigue repartiendo sobre el
        //   mismo tope): M0·d midió hasta 730 MiB con un DataFrame mediano, y
        //   768Mi de petición deja ese puesto como primer candidato a desalojo
        //   bajo presión de memoria. `assert!` en vez de un `.replace()` mudo:
        //   si el texto base cambia y deja de traer "memory: 768Mi", que la
        //   construcción lo diga en vez de dejar la petición sin subir en silencio.
        assert!(
            t.contains("memory: 768Mi"),
            "`{PLANTILLA_PUESTO}` no trae `memory: 768Mi`: la petición de memoria de `python-notebooks` no se puede subir"
        );
        t.replace("memory: 768Mi", "memory: 1536Mi")
    } else {
        t
    };
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
/// 0074 AR1·4: el hueco del token de leer de los registros de artefactos, en la
/// plantilla de la capa de la JVM. Opcional: una plantilla sin él se rinde
/// igual (binario antes que malla).
const ARTEFACTOS_MODELO: &str = "artefactos-modelo";
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
    // 0074 AR1·4 · El token de leer de los registros de artefactos, DESPUÉS de
    // nombrar el Job: el nombre sale del hash de la plantilla rendida y es lo
    // que impide encolar dos veces la misma capa; un token nuevo en cada
    // rendición lo cambiaría. En la JVM (Maven) y, AR2·3, en la de Python
    // (PyPI: Python, Spark y los notebooks); la dirección la trae la plantilla.
    let hueco = format!("value: \"{ARTEFACTOS_MODELO}\"");
    let t = if entorno != "node" && t.contains(&hueco) {
        let token = crate::artefactos::token_de_leer(&job);
        t.replace(&hueco, &format!("value: \"{token}\""))
    } else {
        t
    };
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

    /// 0063 S8: la pasada programada es otro Job por franja, con `PROGRAMADA`
    /// en todos los contenedores; la misma franja y las mismas vistas, el mismo
    /// fichero; y una plantilla sin el hueco se dice, no se adivina.
    #[test]
    fn la_pasada_programada_es_un_job_por_franja() {
        let p = "metadata:\n  name: copiar-00000000\nenv:\n  - { name: VISTAS, value: \"olist.customers\" }\n  - { name: REHACER, value: \"\" }\n  - { name: PROGRAMADA, value: \"\" }\n  - { name: RAMA, value: \"\" }\nenv2:\n  - { name: PROGRAMADA, value: \"\" }\n";
        let v = vec!["vara_pg.ore_vara.pedidos".to_string()];
        let (f, t) = rendir_programada(p, &v, "2026-10-16T08:00:00Z").unwrap();
        assert!(
            f.starts_with(PREFIJO_PROGRAMADA) && f.ends_with(".yaml"),
            "{f}"
        );
        assert!(t.contains("  name: copiar-prog-"), "{t}");
        assert_eq!(
            t.matches("name: PROGRAMADA, value: \"2026-10-16T08:00:00Z\"")
                .count(),
            2,
            "{t}"
        );
        assert!(
            t.contains("name: REHACER, value: \"\"") && t.contains("name: RAMA, value: \"\""),
            "{t}"
        );
        assert!(t.contains("value: \"vara_pg.ore_vara.pedidos\""), "{t}");
        assert_eq!(franja_de(&t), Some("2026-10-16T08:00:00Z"));
        assert_eq!(
            rendir_programada(p, &v, "2026-10-16T08:00:00Z").unwrap().0,
            f
        );
        assert_ne!(
            rendir_programada(p, &v, "2026-10-16T08:01:00Z").unwrap().0,
            f
        );
        let sin = p.replace("  - { name: PROGRAMADA, value: \"\" }\n", "");
        assert!(
            rendir_programada(&sin, &v, "x")
                .unwrap_err()
                .contains("converger")
        );
    }

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

    /// La pirámide (0072 F2·3·2·4), sobre la plantilla de verdad: la petición entra, el
    /// nombre lleva el resumen, la ranura es de la tabla y lo rendido sigue siendo YAML.
    #[test]
    fn la_piramide_rinde_su_job() {
        let p = include_str!("../../../malla/59-la-piramide.yaml");
        let pet = r#"{"capa":"v.trip","destino":{"ubicacion":"gs://c/t/_ore_piramides/00001-a-r1.pmtiles"}}"#;
        let (f, t) = rendir_piramide(p, "gs://c/t", pet).unwrap();
        assert!(
            f.starts_with("59-la-piramide-") && f.ends_with(".yaml"),
            "{f}"
        );
        assert!(t.contains(&format!("value: '{pet}'")), "{t}");
        assert!(!t.contains("piramide-00000000"), "{t}");
        let n = ore_core::parse::parse(&t).expect("lo rendido es YAML");
        let (_, c) = n
            .get("spec")
            .and_then(|(_, s)| s.get("template"))
            .and_then(|(_, s)| s.get("spec"))
            .and_then(|(_, s)| s.get("containers"))
            .unwrap();
        let (_, env) = c.items()[0].get("env").unwrap();
        let v = env
            .items()
            .iter()
            .find(|e| e.get("name").and_then(|(_, v)| v.as_str()) == Some("ORE_PIRAMIDE"))
            .and_then(|e| e.get("value").and_then(|(_, v)| v.as_str()))
            .unwrap();
        assert_eq!(v, pet);
        // la misma petición, el mismo Job; otra instantánea de la misma tabla, la misma ranura
        assert_eq!(rendir_piramide(p, "gs://c/t", pet).unwrap().1, t);
        let otra = pet.replace("00001", "00002");
        let (f2, t2) = rendir_piramide(p, "gs://c/t", &otra).unwrap();
        assert_eq!(f2, f);
        assert_ne!(t2, t);
        assert_ne!(rendir_piramide(p, "gs://c/u", pet).unwrap().0, f);
        assert!(rendir_piramide(p, "gs://c/t", "{\"a\":\"b'c\"}").is_err());
        assert!(rendir_piramide("metadata:\n  name: x\n", "gs://c/t", pet).is_err());
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
        // D-O6: un contenedor público, sin app; no se mezcla con una
        assert_eq!(
            url_sin_secreto("az://azureopendatastorage/mnist/?publico=1"),
            Ok("azure")
        );
        assert!(url_sin_secreto("az://cuenta/cubo?publico=1&tenant=t&cliente=c").is_err());
        assert!(url_sin_secreto("az://cuenta/cubo?publico=1&publico=1").is_err());
        assert!(url_sin_secreto("az://cuenta/cubo?publico=0").is_err());
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

    /// Elastic Sessions S1 · Una sesión al *session pool*, sobre la plantilla de
    /// verdad: su cola, la política de redimensionado en el contenedor del puesto
    /// (no en el que baja la capa) y su `/trabajo` en la SSD, con el disco que
    /// dice. La memoria con la que nace no cambia (⑱), tampoco la de notebooks.
    #[test]
    fn una_sesion_al_session_pool_lleva_su_cola_crece_en_caliente_y_derrama_en_la_ssd() {
        let p = include_str!("../../../malla/51-el-puesto.yaml");
        for entorno in ["python", "node", "jvm", "python-notebooks"] {
            let (_, t, _) =
                rendir_puesto(p, "puesto-ana-python", "", "", "1", entorno, "").unwrap();
            let s = al_session_pool(&t).unwrap();
            assert!(
                s.contains("kueue.x-k8s.io/queue-name: cola-sesiones\n"),
                "{s}"
            );
            assert!(!s.contains("kueue.x-k8s.io/queue-name: cola\n"), "{s}");
            let politica = "          resizePolicy: [{resourceName: memory, restartPolicy: NotRequired}, {resourceName: cpu, restartPolicy: NotRequired}]\n";
            assert_eq!(s.matches(politica).count(), 1, "{s}");
            // Justo detrás de los recursos del puesto, y antes de su seguridad.
            let i = s.find(politica).unwrap();
            assert!(s[..i].ends_with("            limits:   {cpu: \"2\", memory: 4Gi}\n"));
            assert!(s[i + politica.len()..].starts_with("          securityContext:"));
            assert!(s.contains("emptyDir: {sizeLimit: 64Gi}"), "{s}");
            assert!(s.contains("name: ORE_DISCO_MB, value: \"65536\""), "{s}");
            assert!(!s.contains("sizeLimit: 4Gi"), "{s}");
            assert_eq!(64 * 1024, 65536);
            // Nace como hoy: crecer es de S2–S3.
            assert!(s.contains("name: ORE_MEMORIA_MB, value: \"4096\""), "{s}");
            assert_eq!(s.matches("limits:   {cpu: \"2\", memory: 4Gi}").count(), 1);
            // Sólo cambia lo dicho.
            let quitado = s
                .replace("queue-name: cola-sesiones\n", "queue-name: cola\n")
                .replace(politica, "")
                .replace("sizeLimit: 64Gi", "sizeLimit: 4Gi")
                .replace("value: \"65536\"", "value: \"4096\"");
            assert_eq!(quitado, t);
        }
        // Una plantilla que no trae la cola de siempre no se adivina.
        let sin = p.replace("kueue.x-k8s.io/queue-name: cola\n", "");
        let (_, t, _) =
            rendir_puesto(&sin, "puesto-ana-python", "", "", "1", "python", "").unwrap();
        assert!(
            al_session_pool(&t)
                .unwrap_err()
                .contains("queue-name: cola")
        );
    }

    /// ORE 0068 C2·3 · El puesto pesado, sobre la plantilla de verdad: su
    /// imagen, la capa de Python (sale de `puesto-python`) y sus recursos, con
    /// `ORE_MEMORIA_MB` diciendo lo mismo que `limits.memory`. Los demás
    /// entornos no cambian; y una plantilla sin los recursos de siempre no se
    /// adivina.
    #[test]
    fn el_puesto_pesado_lleva_su_imagen_la_capa_de_python_y_sus_recursos() {
        let p = include_str!("../../../malla/51-el-puesto.yaml");
        let (f, t, job) = rendir_puesto(
            p,
            "trabajo-ana-1a2b3c4d",
            "",
            "capa-0123456789ab",
            "1",
            SPARK,
            "packages/geo/t/x.py@abc",
        )
        .unwrap();
        assert_eq!(f, "54-el-trabajo-ana-1a2b3c4d.yaml");
        assert!(job.starts_with("trabajo-ana-1a2b3c4d-"));
        assert!(t.contains("/ore/puesto-spark:1"), "{t}");
        assert!(t.contains("{ name: ENTORNO, value: \"python\" }"), "{t}");
        assert!(t.contains("requests: {cpu: \"7\", memory: 48Gi, ephemeral-storage: 250Gi}"));
        assert!(t.contains("limits:   {cpu: \"8\", memory: 48Gi, ephemeral-storage: 250Gi}"));
        assert!(t.contains("name: ORE_MEMORIA_MB, value: \"49152\""));
        assert!(!t.contains("memory: 4Gi}") && !t.contains("value: \"4096\""));
        // Pieza 6: su propia cola, la del pool pesado.
        assert!(
            t.contains("kueue.x-k8s.io/queue-name: cola-pesada\n"),
            "{t}"
        );
        assert!(!t.contains("kueue.x-k8s.io/queue-name: cola\n"), "{t}");
        // 48 GiB = 49152 MiB: lo que el pod dice que mide es lo que mide (⑱).
        assert_eq!(48 * 1024, 49152);

        // Query Q1: su `/trabajo` sube a 240 GiB, y el disco que dice, con él (㉑).
        assert!(t.contains("emptyDir: {sizeLimit: 240Gi}"), "{t}");
        assert!(t.contains("name: ORE_DISCO_MB, value: \"245760\""), "{t}");
        assert!(!t.contains("sizeLimit: 4Gi"), "{t}");
        assert_eq!(240 * 1024, 245760);

        let (_, tp, _) =
            rendir_puesto(p, "trabajo-ana-1a2b3c4d", "", "", "1", "python", "x.py@abc").unwrap();
        assert!(tp.contains("/ore/puesto-python:1") && tp.contains("value: \"4096\""));
        assert!(tp.contains("kueue.x-k8s.io/queue-name: cola\n"));
        assert!(!tp.contains("48Gi"));
        assert!(
            tp.contains("emptyDir: {sizeLimit: 4Gi}")
                && tp.contains("name: ORE_DISCO_MB, value: \"4096\"")
        );

        // Una celda sin converger trae la plantilla de antes, sin tope: el pesado
        // se rinde igual, sin nada que subir (el binario va antes que la malla).
        let vieja = p
            .replace("emptyDir: {sizeLimit: 4Gi}", "emptyDir: {}")
            .replace("- { name: ORE_DISCO_MB, value: \"4096\" }", "");
        let (_, tv, _) = rendir_puesto(
            &vieja,
            "trabajo-ana-1a2b3c4d",
            "",
            "",
            "1",
            SPARK,
            "x.py@abc",
        )
        .unwrap();
        assert!(
            tv.contains("ephemeral-storage: 250Gi") && !tv.contains("sizeLimit: "),
            "{tv}"
        );

        let sin = p.replace("memory: 768Mi}", "memory: 1Gi}");
        let e = rendir_puesto(&sin, "trabajo-ana-1a2b3c4d", "", "", "1", SPARK, "x.py@abc")
            .unwrap_err();
        assert!(e.contains("puesto pesado"), "{e}");
        assert_eq!(entorno_de_la_capa(SPARK), "python");
        assert_eq!(entorno_de_la_capa("jvm"), "jvm");
    }

    /// Query Q4·1a · **La sesión de una hoja en G**: el puesto de una persona
    /// (sin trabajo) en `spark` es el pesado entero —su cola, sus recursos,
    /// su disco y su imagen—, con la capa de Python y como sesión.
    #[test]
    fn la_sesion_de_una_hoja_en_g_es_el_puesto_pesado() {
        let p = include_str!("../../../malla/51-el-puesto.yaml");
        let (f, t, job) = rendir_puesto(
            p,
            "puesto-ana-spark",
            "",
            "capa-0123456789ab",
            "1",
            SPARK,
            "",
        )
        .unwrap();
        assert_eq!(f, "51-el-puesto-ana-spark.yaml");
        assert!(job.starts_with("puesto-ana-spark-"), "{job}");
        assert!(
            t.contains(
                "kueue.x-k8s.io/queue-name: cola-pesada
"
            ),
            "{t}"
        );
        assert!(t.contains("limits:   {cpu: \"8\", memory: 48Gi, ephemeral-storage: 250Gi}"));
        assert!(t.contains("name: ORE_MEMORIA_MB, value: \"49152\""));
        assert!(t.contains("/puesto-spark:1"));
        assert!(t.contains("value: \"python\""), "la capa es la de Python");
        assert!(t.contains("value: \"\"") && !t.contains(TRABAJO_MODELO));
        ore_core::parse::parse(&t).unwrap();
    }

    /// 0068 C3·c · El driver de un Build elástico, sobre la plantilla de
    /// verdad: pequeño, con su cuenta, su etiqueta de red en el POD, y lo que el
    /// SDK necesita (el techo, la imagen, su nombre y su IP); y su cola,
    /// `cola-driver` (`malla/22`). Sin el trabajo pesado
    /// de siempre, no se adivina.
    #[test]
    fn el_driver_elastico_es_el_trabajo_pesado_en_pequeno_y_con_su_cuenta() {
        let p = include_str!("../../../malla/51-el-puesto.yaml");
        let (_, t, _) = rendir_puesto(
            p,
            "trabajo-ana-1a2b3c4d",
            "",
            "capa-0123456789ab",
            "1",
            SPARK,
            "packages/geo/t/x.py@abc",
        )
        .unwrap();
        let d = al_driver_elastico(&t, TECHO_DE_EJECUTORES).unwrap();
        assert!(
            d.contains("requests: {cpu: \"1500m\", memory: 12Gi}"),
            "{d}"
        );
        assert!(d.contains("limits:   {cpu: \"2\", memory: 12Gi}"));
        assert!(!d.contains("48Gi") && !d.contains("49152"));
        assert!(!d.contains("ephemeral-storage"), "{d}");
        assert!(d.contains("name: ORE_MEMORIA_MB, value: \"12288\" }"));
        assert!(d.contains("name: ORE_SPARK_ELASTICO, value: \"2\" }"));
        assert!(d.contains("name: ORE_SPARK_IMAGEN, value: \"europe-west1-docker.pkg.dev/"));
        assert!(d.contains("/ore/puesto-spark:1\" }"));
        assert!(d.contains("name: ORE_POD_IP, valueFrom: {fieldRef: {fieldPath: status.podIP}} }"));
        assert!(d.contains("serviceAccountName: driver-pesado\n"));
        assert!(!d.contains("serviceAccountName: puesto\n"));
        assert!(d.contains("        ore.dev/rol: puesto\n        ore.dev/spark: driver\n"));
        assert!(d.contains("kueue.x-k8s.io/queue-name: cola-driver\n"));
        assert!(!d.contains("queue-name: cola-pesada"));
        // Es YAML que se lee.
        ore_core::parse::parse(&d).unwrap();
        // Lo ligero no se hace driver.
        let (_, tp, _) =
            rendir_puesto(p, "trabajo-ana-1a2b3c4d", "", "", "1", "python", "x.py@abc").unwrap();
        assert!(al_driver_elastico(&tp, 2).unwrap_err().contains("no trae"));
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

    /// 0064: `python-notebooks` es un entorno con imagen propia (no 500 al
    /// abrirlo) y sube SU petición de memoria sin tocar el límite ni los
    /// demás entornos.
    #[test]
    fn notebooks_tiene_su_imagen_y_sube_la_peticion_de_memoria() {
        let p = "name: puesto-00000000
image: registro/ore/puesto-entorno-modelo:1
env:
  - { name: PUESTO, value: \"puesto-modelo\" }
  - { name: RAMA, value: \"rama-modelo\" }
  - { name: CAPA, value: \"capa-modelo\" }
  - { name: ABIERTO, value: \"abierto-modelo\" }
  - { name: ENTORNO, value: \"entorno-modelo\" }
resources:
  requests: { cpu: \"100m\", memory: 768Mi }
  limits:   { cpu: \"2\", memory: 4Gi }
";
        let (f, t, job) =
            rendir_puesto(p, "puesto-noa", "", "", "1", "python-notebooks", "").unwrap();
        assert_eq!(f, "51-el-puesto-noa.yaml");
        assert!(job.starts_with("puesto-noa-"));
        assert!(t.contains("image: registro/ore/puesto-python-notebooks:1"));
        assert!(t.contains("memory: 1536Mi") && !t.contains("memory: 768Mi"));
        assert!(t.contains("memory: 4Gi"), "el límite no se toca");
        // Los demás entornos no se ven afectados.
        let (_, tp, _) = rendir_puesto(p, "puesto-ana", "", "", "1", "python", "").unwrap();
        assert!(tp.contains("memory: 768Mi") && !tp.contains("memory: 1536Mi"));
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

    #[test]
    fn la_capa_de_la_jvm_lleva_el_token_de_artifacts_y_no_cambia_de_nombre() {
        let p = "name: la-capa-jvm-00000000
env:
  - { name: CAPA, value: \"capa-modelo\" }
  - { name: RAMA, value: \"rama-modelo\" }
  - { name: ALCANCE, value: \"alcance-modelo\" }
  - { name: INTENTO, value: \"intento-modelo\" }
  - { name: ORE_ARTEFACTOS_TOKEN, value: \"artefactos-modelo\" }
";
        let (_, t, job) = rendir_capa(p, "jvm", "capa-0123456789ab", "", "", "1").unwrap();
        let token = t
            .lines()
            .find_map(|l| l.split("ORE_ARTEFACTOS_TOKEN, value: \"").nth(1))
            .and_then(|r| r.split('"').next())
            .unwrap();
        let carga = crate::artefactos::verificar(token).unwrap();
        assert_eq!(
            (carga.registro.as_str(), carga.verbo.as_str()),
            ("*", "leer")
        );
        // Dos rendiciones: tokens distintos (caducan), el mismo Job.
        let (_, _, job2) = rendir_capa(p, "jvm", "capa-0123456789ab", "", "", "1").unwrap();
        assert_eq!(job, job2);
        // Sin el hueco (una plantilla de antes), se rinde igual.
        let vieja = p.replace(
            "  - { name: ORE_ARTEFACTOS_TOKEN, value: \"artefactos-modelo\" }\n",
            "",
        );
        assert!(rendir_capa(&vieja, "jvm", "capa-0123456789ab", "", "", "1").is_ok());
    }

    /// AR2·3: la de Python (y Spark y los notebooks, que la comparten) lleva
    /// el mismo token, después de nombrar el Job.
    #[test]
    fn la_capa_de_python_lleva_el_token_de_artifacts_y_no_cambia_de_nombre() {
        let p = "name: la-capa-00000000
env:
  - { name: CAPA, value: \"capa-modelo\" }
  - { name: RAMA, value: \"rama-modelo\" }
  - { name: ALCANCE, value: \"alcance-modelo\" }
  - { name: INTENTO, value: \"intento-modelo\" }
  - { name: ORE_ARTEFACTOS_TOKEN, value: \"artefactos-modelo\" }
";
        for entorno in ["python", SPARK, "python-notebooks"] {
            let (_, t, job) = rendir_capa(p, entorno, "capa-0123456789ab", "", "", "1").unwrap();
            let token = t
                .lines()
                .find_map(|l| l.split("ORE_ARTEFACTOS_TOKEN, value: \"").nth(1))
                .and_then(|r| r.split('"').next())
                .unwrap();
            let carga = crate::artefactos::verificar(token).unwrap();
            assert_eq!(
                (carga.registro.as_str(), carga.verbo.as_str()),
                ("*", "leer"),
                "{entorno}"
            );
            assert_eq!(carga.persona, format!("capa:{job}"));
            let (_, _, job2) = rendir_capa(p, entorno, "capa-0123456789ab", "", "", "1").unwrap();
            assert_eq!(job, job2, "{entorno}");
        }
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
