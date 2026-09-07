//! `ore package new` — **crear un paquete**, que hasta hoy no hacía nadie.
//!
//! # El hueco, comprobado
//!
//! `ore init` crea el esqueleto del *workspace* —`lattices/`, `rulesets/`,
//! `policies/`…— y deja `packages/` **vacío**. El único que escribía un
//! `package.yaml` era el inductor, así que **un paquete solo nacía descubriendo
//! una fuente**. Agrupar vistas en un paquete nuevo no tenía dónde empezar.
//!
//! # Por qué es un verbo aparte de `move`
//!
//! Porque crear un paquete es un **acto de gobierno** —necesita dueño, versión y
//! estado— y mover un documento no. Fallan por separado, que es el mismo motivo
//! por el que `discover` se partió en `--source` y `--from`.
//!
//! # El emisor es uno
//!
//! No escribe YAML: llama a [`crate::inductor::documento_paquete`], que es el
//! mismo que usa la inducción. Un manifiesto escrito a mano y uno inducido
//! tienen que ser el mismo texto — es la misma disciplina que `ore view add` con
//! `documento_vista`.
//!
//! # Lo que decide, y lo que no
//!
//! **`status: draft`**, y no `active`. `01-package` §2.3 deriva de `status` la
//! `oos.maturity` **por defecto** de lo que el paquete contenga, y un paquete
//! recién creado no contiene nada: llamarlo `active` sería afirmar `STABLE`
//! sobre lo que no existe. Es la misma figura que el `DRAFT` con el que la
//! inducción escribe cada vista.
//!
//! **`owner` se pregunta y no se deriva**: es quién responde. Sin él se escribe
//! `cambiame`, que **no valida** — un handle inventado dejaría el paquete sin
//! nadie que responda aparentando lo contrario. Igual que el inductor.
//!
//! **`domain` por defecto es el nombre**, que es lo que ya hace la inducción. Es
//! obligatorio en el esquema, así que omitirlo no es una opción; poner el nombre
//! es la conjetura más pequeña, y `--domain` la corrige.
//!
//! # Y el nombre tiene que poder ser un espacio de nombres
//!
//! Salió construyendo esto: el esquema deja llamar `oos.dev` o `mi-paquete` a un
//! paquete —`packageName` admite puntos, guiones y barras porque el nombre **es
//! también la coordenada con la que otro lo importa**— y un `namespace` es un
//! `identifier`, que no admite ninguno de los tres. Un paquete así no puede
//! contener contenido gobernado (`OOS2030`), así que este mando **se niega antes
//! de crearlo** en vez de dejar un paquete donde no se puede poner nada.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Lo que un `namespace` admite: `^[a-zA-Z][a-zA-Z0-9_]*$`, el `identifier` del
/// esquema publicado. Se comprueba aquí porque el nombre del paquete **es** el
/// espacio de nombres de lo que contenga.
fn usable_como_namespace(n: &str) -> bool {
    let mut c = n.chars();
    c.next().is_some_and(|p| p.is_ascii_alphabetic())
        && c.all(|x| x.is_ascii_alphanumeric() || x == '_')
}

pub fn nuevo(raiz: &Path, nombre: &str, owner: Option<&str>, dominio: Option<&str>) -> ExitCode {
    if !usable_como_namespace(nombre) {
        eprintln!("error: `{nombre}` no puede ser un espacio de nombres");
        eprintln!("  El nombre de un paquete es el `namespace` de todo lo que contenga, y un");
        eprintln!("  `namespace` admite letras, dígitos y `_`, empezando por letra. Un nombre");
        eprintln!("  con puntos o guiones es legal como COORDENADA de importación, pero un");
        eprintln!("  paquete así no puede contener contenido gobernado — `OOS2030`.");
        return ExitCode::from(64); // EX_USAGE
    }

    let destino = raiz.join("packages").join(nombre);
    let manifiesto = destino.join("package.yaml");
    if manifiesto.exists() {
        eprintln!("error: `{}` ya existe", manifiesto.display());
        eprintln!("  Sobrescribirlo perdería su dueño, su versión y lo que exporte.");
        return ExitCode::from(65); // EX_DATAERR
    }
    if let Err(e) = std::fs::create_dir_all(&destino) {
        eprintln!("error: no se pudo crear `{}`: {e}", destino.display());
        return ExitCode::from(73); // EX_CANTCREAT
    }

    let texto = crate::inductor::documento_paquete(
        nombre,
        owner.unwrap_or("cambiame"),
        "draft",
        dominio.unwrap_or(nombre),
    );
    if let Err(e) = std::fs::write(&manifiesto, &texto) {
        eprintln!("error: no se pudo escribir `{}`: {e}", manifiesto.display());
        return ExitCode::from(73);
    }

    println!("  ✓ {}", manifiesto.display());
    println!("  ✓ `{nombre}` · en DRAFT: un paquete vacío no es nada todavía");
    println!();
    // Y no se crean `views/`, `tables/` ni `entities/`: un directorio vacío no
    // viaja en git y no significa nada. Los crea quien escribe el primer
    // documento, que es como ya funciona la inducción.
    println!("  ore discover --from <catálogo> --out packages/{nombre}");
    println!("  ore view add <nombre> --from <tabla>");
    if owner.is_none() {
        println!();
        println!("  · `owner: cambiame` — NO valida, y es a propósito");
        println!("    De él heredan las políticas. Un handle inventado dejaría el paquete");
        println!("    sin nadie que responda aparentando lo contrario.");
    }

    // Lo que salga del árbol entero, como hace `ore view add`: escribir un
    // documento que rompe el árbol y callarlo es media herramienta.
    let diags = ore_core::validate_package(raiz);
    if diags.is_empty() {
        println!();
        println!("  ok · sin errores");
        return ExitCode::SUCCESS;
    }
    println!();
    println!("  {} diagnóstico(s) en el árbol:", diags.len());
    for d in diags.iter().take(5) {
        for l in d.render(raiz).lines().take(2) {
            println!("    {l}");
        }
    }
    if diags.len() > 5 {
        println!("    … y {} más. `ore validate`", diags.len() - 5);
    }
    ExitCode::SUCCESS
}

// ── `ore package move` ──────────────────────────────────────────────────────

/// **Mover un documento a otro paquete**, que son tres cosas y hay que hacer
/// las tres.
///
/// # Por qué es un mando y no tres pasos a mano
///
/// De los cinco pasos que un movimiento exige, cuatro los caza alguien
/// —`OOS5007` al hacer `diff`, `OOS2028` al compilar, `OOS2018` al validar— y
/// **cambiar el `namespace` no lo cazaba nadie**. Con `OOS2030` puesto ya sí,
/// pero queda lo otro que una persona no puede garantizar: **el orden**. Si se
/// anuncia el `moved` antes de mover, hay una ventana en la que el manifiesto
/// habla de algo que sigue donde estaba; si se hace al revés, una en la que el
/// nombre viejo vive en dos sitios. Aquí se calcula todo y se escribe al final.
///
/// # Qué hace, exactamente
///
/// 1. mueve el fichero al mismo subdirectorio del paquete destino;
/// 2. reescribe su `namespace`, que **con `OOS2030` es una sola cosa con la
///    anterior** — antes eran dos que se movían por separado;
/// 3. anuncia el `moved` en el manifiesto de **origen**, que es lo que impide
///    que el nombre que desaparece sea un `OOS5007`;
/// 4. y **reapunta lo que lo nombraba**, incluida la forma corta: un documento
///    que compartía espacio con él lo llamaba por su nombre a secas, y tras el
///    movimiento eso ya no resuelve.
///
/// # Y por qué NO toca `exports`
///
/// Porque `exports` es *«esto lo expongo a propósito»* —`01-package` §3.3— y un
/// mando que ensancha la superficie pública por su cuenta contradice la frase
/// para la que esa lista existe. Lo que hace es **decir** qué referencias pasan
/// a cruzar el límite de un paquete y qué línea haría falta; `OOS2028` lo
/// confirma al compilar, y ahí la decisión es de quien publica.
pub fn mover(raiz: &Path, qname: &str, destino: &str, since: Option<&str>) -> ExitCode {
    // **Sin exigir validez**: mover es justo lo que se hace para arreglar un
    // árbol, y exigir que compilase primero lo haría inútil en su caso típico.
    let pkg = ore_core::validate::cargar_paquete(raiz).0;
    let miembros = ore_core::link::miembros(&pkg);

    let Some(doc) = pkg
        .docs
        .iter()
        .find(|d| d.qname().as_deref() == Some(qname))
    else {
        eprintln!("error: no hay ningún documento que se llame `{qname}`");
        return ExitCode::from(65); // EX_DATAERR
    };
    if !ore_core::pertenencia::DEL_PAQUETE.contains(&doc.kind) {
        eprintln!(
            "error: un `{}` no es de un paquete: su nombre es el de un vocabulario compartido",
            doc.kind.as_str()
        );
        eprintln!("  Tiene que significar lo mismo desde todos los paquetes, así que moverlo");
        eprintln!("  entre ellos no querría decir nada. Vive en la raíz del workspace.");
        return ExitCode::from(65);
    }

    // El destino, por su manifiesto y no por su ruta: un paquete puede estar
    // donde quiera, y `miembros()` ya sabe dónde.
    let Some(manifiesto_destino) = pkg
        .docs
        .iter()
        .filter(|d| d.kind == ore_core::document::Kind::Package)
        .find(|d| d.meta("name").and_then(|n| n.as_str()) == Some(destino))
    else {
        eprintln!("error: no hay ningún paquete que se llame `{destino}`");
        eprintln!("  ore package new {destino} --owner <handle>");
        return ExitCode::from(65);
    };
    if !usable_como_namespace(destino) {
        eprintln!("error: `{destino}` no puede ser un espacio de nombres");
        eprintln!("  Un paquete así puede contener vocabulario compartido, no contenido");
        eprintln!("  gobernado — `OOS2030`.");
        return ExitCode::from(65);
    }

    let (Some(dir_destino), Some(dir_origen)) = (
        manifiesto_destino.path.parent(),
        ore_core::link::miembro_de(&miembros, &doc.path),
    ) else {
        eprintln!("error: no se pudo situar el documento o el paquete destino");
        return ExitCode::from(70); // EX_SOFTWARE
    };
    if dir_origen == dir_destino {
        eprintln!("error: `{qname}` ya está en `{destino}`");
        return ExitCode::from(65);
    }

    let nombre = doc
        .meta("name")
        .and_then(|n| n.as_str())
        .unwrap_or_default()
        .to_string();
    let nuevo_qname = format!("{destino}.{nombre}");
    if pkg
        .docs
        .iter()
        .any(|d| d.qname().as_deref() == Some(&nuevo_qname))
    {
        eprintln!("error: `{destino}` ya tiene un documento llamado `{nombre}`");
        eprintln!("  Mover este encima perdería el que hay, y eso no lo decide este mando.");
        return ExitCode::from(65);
    }

    // El mismo subdirectorio: el reparto en carpetas lo decide el árbol que ya
    // hay, no este mando.
    let Ok(relativa) = doc.path.strip_prefix(dir_origen) else {
        eprintln!("error: `{}` no cuelga de su paquete", doc.path.display());
        return ExitCode::from(70);
    };
    let destino_path = dir_destino.join(relativa);

    // ── Todo se calcula antes de escribir nada ──────────────────────────────
    let mut ediciones: BTreeMap<PathBuf, String> = BTreeMap::new();

    // ① el documento, con su espacio de nombres nuevo
    let Ok(texto) = std::fs::read_to_string(&doc.path) else {
        eprintln!("error: no se pudo leer `{}`", doc.path.display());
        return ExitCode::from(66); // EX_NOINPUT
    };
    let viejo_ns = doc
        .meta("namespace")
        .and_then(|n| n.as_str())
        .unwrap_or_default()
        .to_string();
    ediciones.insert(
        destino_path.clone(),
        texto.replace(
            &format!("namespace: {viejo_ns}"),
            &format!("namespace: {destino}"),
        ),
    );

    // ② quien lo nombraba, reapuntado — incluida la forma corta
    let mut reapuntados: Vec<String> = Vec::new();
    let mut cruzan: Vec<String> = Vec::new();
    for d in &pkg.docs {
        if d.path == doc.path {
            continue;
        }
        let refs: Vec<_> = ore_core::exporta::referencias(d)
            .into_iter()
            .filter(|r| r.destino == qname && r.kind == doc.kind)
            .collect();
        if refs.is_empty() {
            continue;
        }
        let Ok(t) = std::fs::read_to_string(&d.path) else {
            continue;
        };
        let mut lineas: Vec<String> = t.lines().map(String::from).collect();
        for r in &refs {
            match sustituir_en(&lineas, r.pos, &nuevo_qname) {
                Some((i, l)) => {
                    lineas[i] = l;
                    reapuntados.push(format!("{}  ·  {}", d.qname().unwrap_or_default(), r.clase));
                }
                None => {
                    eprintln!(
                        "error: no se pudo reapuntar `{}` en `{}`",
                        r.clase,
                        d.path.display()
                    );
                    eprintln!("  Nada se ha movido: o se hacen los tres pasos o ninguno.");
                    return ExitCode::from(70);
                }
            }
        }
        // ¿Pasa a cruzar el límite del paquete? Entonces hace falta un
        // `exports`, y esa decisión no es de aquí.
        if ore_core::link::miembro_de(&miembros, &d.path) != Some(dir_destino) {
            cruzan.push(d.qname().unwrap_or_default());
        }
        let mut nuevo = lineas.join("\n");
        if t.ends_with('\n') {
            nuevo.push('\n');
        }
        ediciones.insert(d.path.clone(), nuevo);
    }

    // ③ el anuncio en el manifiesto de ORIGEN
    let Some(manifiesto_origen) = pkg
        .docs
        .iter()
        .filter(|d| d.kind == ore_core::document::Kind::Package)
        .find(|d| d.path.parent() == Some(dir_origen))
    else {
        eprintln!("error: `{qname}` no está dentro de ningún paquete");
        return ExitCode::from(65);
    };
    let version = manifiesto_origen
        .meta("version")
        .and_then(|n| n.as_str())
        .unwrap_or("0.1.0")
        .to_string();
    let desde = since.unwrap_or(&version);
    let Ok(t) = std::fs::read_to_string(&manifiesto_origen.path) else {
        eprintln!("error: no se pudo leer el manifiesto de origen");
        return ExitCode::from(66);
    };
    let anuncio = match anunciar(&t, qname, &nuevo_qname, desde) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("error: no se pudo anunciar el movimiento: {e}");
            eprintln!("  Nada se ha movido. Sin el anuncio, el nombre que desaparece es un");
            eprintln!("  `OOS5007` y el movimiento se cobra como una supresión.");
            return ExitCode::from(70);
        }
    };
    ediciones.insert(manifiesto_origen.path.clone(), anuncio);

    // ── Y ahora sí, se escribe ──────────────────────────────────────────────
    if let Some(d) = destino_path.parent()
        && let Err(e) = std::fs::create_dir_all(d)
    {
        eprintln!("error: no se pudo crear `{}`: {e}", d.display());
        return ExitCode::from(73);
    }
    for (ruta, contenido) in &ediciones {
        if let Err(e) = std::fs::write(ruta, contenido) {
            eprintln!("error: no se pudo escribir `{}`: {e}", ruta.display());
            return ExitCode::from(73);
        }
    }
    if let Err(e) = std::fs::remove_file(&doc.path) {
        eprintln!("error: no se pudo retirar `{}`: {e}", doc.path.display());
        return ExitCode::from(73);
    }

    println!("  ✓ {qname} → {nuevo_qname}");
    println!("  ✓ {}", destino_path.display());
    println!(
        "  ✓ anunciado en `{}` · since {desde}",
        manifiesto_origen.path.display()
    );
    if !reapuntados.is_empty() {
        println!();
        println!("  {} referencia(s) reapuntada(s):", reapuntados.len());
        for r in &reapuntados {
            println!("    {r}");
        }
    }
    if since.is_none() {
        println!();
        println!("  · `since: {desde}` es la versión que el paquete declara HOY");
        println!("    `ore diff` calcula el salto que este cambio exige. Si publicas con");
        println!("    uno mayor, ajústala: `--since` la fija.");
    }
    if !cruzan.is_empty() {
        println!();
        println!(
            "  · ahora {} referencia(s) cruzan a `{destino}`:",
            cruzan.len()
        );
        for c in &cruzan {
            println!("      {c}");
        }
        println!("    `exports` es «esto lo expongo a propósito», así que este mando no lo");
        println!("    toca. En el manifiesto de `{destino}`:");
        println!("      spec: {{ …, exports: [{nuevo_qname}] }}");
    }

    let diags = ore_core::validate_package(raiz);
    if diags.is_empty() {
        println!();
        println!("  ok · sin errores");
        return ExitCode::SUCCESS;
    }
    println!();
    println!("  {} diagnóstico(s) en el árbol:", diags.len());
    for d in diags.iter().take(5) {
        for l in d.render(raiz).lines().take(2) {
            println!("    {l}");
        }
    }
    if diags.len() > 5 {
        println!("    … y {} más. `ore validate`", diags.len() - 5);
    }
    ExitCode::SUCCESS
}

/// Sustituye el token que empieza en `pos` por `nuevo`.
///
/// Por posición y no por búsqueda de texto: la referencia puede estar escrita
/// **en forma corta** —un documento que compartía espacio de nombres lo llamaba
/// `iberia` a secas— y buscar esa palabra por el fichero cambiaría cualquier
/// otra cosa que se llame igual. `referencias()` da la posición exacta, y eso
/// es la mitad del valor de que exista.
fn sustituir_en(
    lineas: &[String],
    pos: ore_core::diag::Pos,
    nuevo: &str,
) -> Option<(usize, String)> {
    let i = pos.line.checked_sub(1)?;
    let l = lineas.get(i)?;
    let cs: Vec<char> = l.chars().collect();
    let mut a = pos.col.checked_sub(1)?;
    if a >= cs.len() {
        return None;
    }
    // Una referencia entrecomillada empieza en la comilla o justo detrás,
    // según quién apunte: se admite lo uno y lo otro.
    let comilla = matches!(cs.get(a), Some('"') | Some('\''));
    if comilla {
        a += 1;
    }
    let mut b = a;
    while b < cs.len() && !matches!(cs[b], ' ' | ',' | '}' | ']' | '"' | '\'' | '\t') {
        b += 1;
    }
    if b == a {
        return None;
    }
    let mut out: String = cs[..a].iter().collect();
    out.push_str(nuevo);
    out.extend(cs[b..].iter());
    Some((i, out))
}

/// Escribe el `moved` en el manifiesto, en la forma que el manifiesto ya tenga.
///
/// Las dos existen en el corpus —`spec: { owner: … }` en una línea y `spec:` en
/// bloque— y editar la que hay es mejor que imponer una: reescribir el
/// manifiesto entero perdería lo que no sabemos emitir, que es casi todo lo que
/// `01-package` admite.
fn anunciar(texto: &str, de: &str, a: &str, since: &str) -> Result<String, String> {
    let item = format!("{{ from: {de}, to: {a}, since: {since} }}");
    let mut lineas: Vec<String> = texto.lines().map(String::from).collect();
    let i = lineas
        .iter()
        .position(|l| l.trim_start().starts_with("spec:"))
        .ok_or("el manifiesto no tiene `spec`")?;

    let resto = lineas[i]
        .split_once("spec:")
        .map(|(_, r)| r.trim())
        .unwrap_or("");
    if resto.starts_with('{') {
        // Flujo, en una línea. Si ya hay `moved: [ … ]`, se añade dentro.
        let l = lineas[i].clone();
        let nueva = match l.find("moved:") {
            Some(_) => {
                let cierre = l.rfind(']').ok_or("`moved` en flujo sin cerrar")?;
                format!("{}, {item}{}", l[..cierre].trim_end(), &l[cierre..])
            }
            None => {
                let cierre = l.rfind('}').ok_or("`spec` en flujo sin cerrar")?;
                format!(
                    "{}, moved: [{item}] {}",
                    l[..cierre].trim_end(),
                    l[cierre..].trim_start()
                )
            }
        };
        lineas[i] = nueva;
        return Ok(terminar(lineas, texto));
    }

    // Bloque. El final de `spec` es la primera línea con contenido que no está
    // indentada, o el final del fichero.
    let fin = lineas
        .iter()
        .enumerate()
        .skip(i + 1)
        .find(|(_, l)| !l.trim().is_empty() && !l.starts_with(' ') && !l.starts_with('\t'))
        .map(|(j, _)| j)
        .unwrap_or(lineas.len());
    match lineas[i + 1..fin]
        .iter()
        .position(|l| l.trim_start().starts_with("moved:"))
    {
        Some(rel) => {
            let m = i + 1 + rel;
            // Detrás del último elemento de la lista.
            let mut j = m + 1;
            while j < fin && lineas[j].trim_start().starts_with('-') {
                j += 1;
            }
            let sangria = lineas
                .get(m + 1)
                .map(|l| l.len() - l.trim_start().len())
                .unwrap_or(4);
            lineas.insert(j, format!("{}- {item}", " ".repeat(sangria)));
        }
        None => {
            lineas.insert(fin, format!("  moved:\n    - {item}"));
        }
    }
    Ok(terminar(lineas, texto))
}

fn terminar(lineas: Vec<String>, original: &str) -> String {
    let mut s = lineas.join("\n");
    if original.ends_with('\n') {
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Las dos formas del manifiesto**, que las dos estan en el corpus.
    ///
    /// Se edita la que hay en vez de imponer una: reescribir el manifiesto
    /// entero perderia lo que no sabemos emitir, que es casi todo lo que
    /// `01-package` admite.
    #[test]
    fn el_anuncio_respeta_la_forma_del_manifiesto() {
        let flujo = "kind: Package
spec: { owner: \"team:d\" }
";
        let r = anunciar(flujo, "a.x", "b.x", "1.0.0").unwrap();
        assert!(
            r.contains("moved: [{ from: a.x, to: b.x, since: 1.0.0 }]"),
            "{r}"
        );
        assert!(
            r.contains("owner: \"team:d\","),
            "sin espacio suelto:
{r}"
        );

        let bloque = "kind: Package
spec:
  owner: team:d
";
        let r = anunciar(bloque, "a.x", "b.x", "1.0.0").unwrap();
        assert!(
            r.contains(
                "  moved:
    - { from: a.x"
            ),
            "{r}"
        );

        // Y con una lista que ya existe, se anade detras del ultimo.
        let con = "kind: Package
spec:
  owner: team:d
  moved:
    - { from: a.y, to: b.y, since: 0.1.0 }
";
        let r = anunciar(con, "a.x", "b.x", "1.0.0").unwrap();
        assert_eq!(r.matches("- { from:").count(), 2, "{r}");
        assert!(
            r.find("a.y").unwrap() < r.find("a.x").unwrap(),
            "en orden:
{r}"
        );
    }

    /// **La forma corta**, que es la que obliga a sustituir por POSICION.
    ///
    /// Un documento que compartia espacio de nombres lo llamaba `iberia` a
    /// secas; buscar esa palabra por el fichero cambiaria cualquier otra cosa
    /// que se llame igual — empezando por su propio `name`.
    #[test]
    fn se_sustituye_por_posicion_y_no_por_texto() {
        let l: Vec<String> = vec![
            "metadata: { name: iberia }".into(),
            "  backedBy: iberia".into(),
            "  otro: \"iberia\"".into(),
        ];
        let pos = |line, col| ore_core::diag::Pos { line, col };

        let (i, r) = sustituir_en(&l, pos(2, 13), "eu.iberia").unwrap();
        assert_eq!(i, 1);
        assert_eq!(r, "  backedBy: eu.iberia");

        // Entrecomillada: se sustituye lo de dentro y las comillas se quedan.
        let (_, r) = sustituir_en(&l, pos(3, 10), "eu.iberia").unwrap();
        assert_eq!(r, "  otro: \"eu.iberia\"");

        // Y la linea 1 no se toca nunca: nadie apunta ahi.
        assert!(sustituir_en(&l, pos(9, 1), "x").is_none());
    }

    #[test]
    fn el_nombre_tiene_que_poder_ser_un_espacio_de_nombres() {
        assert!(usable_como_namespace("ventas"));
        assert!(usable_como_namespace("ventas_eu"));
        assert!(usable_como_namespace("v2"));
        // Legales como nombre de paquete —`packageName` los admite— e
        // imposibles como `namespace`. Es el hueco que destapó construir esto.
        assert!(!usable_como_namespace("oos.dev"));
        assert!(!usable_como_namespace("mi-paquete"));
        assert!(!usable_como_namespace("acme/hr"));
        // Y lo que no es un nombre.
        assert!(!usable_como_namespace(""));
        assert!(!usable_como_namespace("2ventas"));
    }
}
