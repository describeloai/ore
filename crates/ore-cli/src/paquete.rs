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

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ore_core::link::{Loaded, Package};

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

// ── El taller ───────────────────────────────────────────────────────────────

/// Los cambios en memoria hasta que se escriben todos.
///
/// Existe por lo que la medida del corte pedía: `move` es atómico por
/// documento, y **un bucle de `move` no lo es**. Si el tercero falla queda medio
/// partido, y medio partido no es un estado que nadie quiera revisar.
///
/// Y hace falta algo más que juntar escrituras: **cada movimiento tiene que ver
/// lo que hicieron los anteriores**. El manifiesto ya lleva un `moved` cuando se
/// añade el segundo; un documento que se movió y luego resulta que nombraba a
/// otro que también se mueve hay que reapuntarlo **en su ruta nueva**. Por eso
/// el taller lleva las dos cosas: el contenido y a dónde se fue cada fichero.
#[derive(Default)]
struct Taller {
    ficheros: BTreeMap<PathBuf, String>,
    /// ruta original → ruta actual, para los que ya se mudaron.
    mudados: BTreeMap<PathBuf, PathBuf>,
}

impl Taller {
    /// Dónde está ahora un fichero que originalmente estaba en `p`.
    fn ruta(&self, p: &Path) -> PathBuf {
        self.mudados
            .get(p)
            .cloned()
            .unwrap_or_else(|| p.to_path_buf())
    }

    /// El contenido actual: lo del taller si ya se tocó, y si no el del disco.
    fn leer(&self, p: &Path) -> Result<String, String> {
        let r = self.ruta(p);
        if let Some(t) = self.ficheros.get(&r) {
            return Ok(t.clone());
        }
        std::fs::read_to_string(&r).map_err(|e| format!("no se pudo leer `{}`: {e}", r.display()))
    }

    fn escribir(&mut self, original: &Path, texto: String) {
        let r = self.ruta(original);
        self.ficheros.insert(r, texto);
    }

    fn mudar(&mut self, de: &Path, a: PathBuf, texto: String) {
        self.ficheros.remove(&self.ruta(de));
        self.mudados.insert(de.to_path_buf(), a.clone());
        self.ficheros.insert(a, texto);
    }

    /// Y ahora sí. Se crean los directorios, se escribe todo y **al final** se
    /// retira lo que se mudó: si algo fallara antes, no se ha perdido nada.
    fn aplicar(&self) -> Result<(), String> {
        for (ruta, contenido) in &self.ficheros {
            if let Some(d) = ruta.parent() {
                std::fs::create_dir_all(d)
                    .map_err(|e| format!("no se pudo crear `{}`: {e}", d.display()))?;
            }
            std::fs::write(ruta, contenido)
                .map_err(|e| format!("no se pudo escribir `{}`: {e}", ruta.display()))?;
        }
        for de in self.mudados.keys() {
            std::fs::remove_file(de)
                .map_err(|e| format!("no se pudo retirar `{}`: {e}", de.display()))?;
        }
        Ok(())
    }
}

/// Lo que un movimiento produjo, para contarlo.
#[derive(Default)]
struct Rastro {
    reapuntados: Vec<String>,
    cruzan: Vec<String>,
}

// ── El plan de un movimiento ────────────────────────────────────────────────

/// Planifica mover **un** documento: no escribe nada, deja el taller listo.
///
/// Son cuatro cosas y hay que hacer las cuatro:
///
/// 1. el fichero, al mismo subdirectorio del destino —el reparto en carpetas lo
///    decide el árbol que ya hay, no este mando—;
/// 2. su `namespace`, que **con `OOS2030` es una sola cosa con lo anterior**;
/// 3. el `moved` en el manifiesto de **origen**, sin el cual el nombre que
///    desaparece es un `OOS5007`;
/// 4. y lo que lo nombraba, reapuntado **por posición** — ver [`sustituir_en`].
#[allow(clippy::too_many_arguments)]
fn planificar(
    pkg: &Package,
    miembros: &[PathBuf],
    taller: &mut Taller,
    doc: &Loaded,
    dir_origen: &Path,
    dir_destino: &Path,
    destino: &str,
    since: Option<&str>,
    tambien_se_mueven: &[String],
) -> Result<(String, Rastro), String> {
    let qname = doc.qname().unwrap_or_default();
    let nombre = doc
        .meta("name")
        .and_then(|n| n.as_str())
        .unwrap_or_default();
    let nuevo_qname = format!("{destino}.{nombre}");
    let mut rastro = Rastro::default();

    // ① y ② el fichero, con su espacio de nombres nuevo
    let relativa = doc
        .path
        .strip_prefix(dir_origen)
        .map_err(|_| format!("`{}` no cuelga de su paquete", doc.path.display()))?;
    let destino_path = dir_destino.join(relativa);
    let texto = taller.leer(&doc.path)?;
    let viejo_ns = doc
        .meta("namespace")
        .and_then(|n| n.as_str())
        .unwrap_or_default();
    let nuevo_texto = texto.replace(
        &format!("namespace: {viejo_ns}"),
        &format!("namespace: {destino}"),
    );
    taller.mudar(&doc.path, destino_path, nuevo_texto);

    // ④ quien lo nombraba
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
        let t = taller.leer(&d.path)?;
        let acaba_en_salto = t.ends_with('\n');
        let mut lineas: Vec<String> = t.lines().map(String::from).collect();
        for r in &refs {
            let (i, l) = sustituir_en(&lineas, r.pos, &nuevo_qname).ok_or_else(|| {
                format!(
                    "no se pudo reapuntar `{}` en `{}`",
                    r.clase,
                    d.path.display()
                )
            })?;
            lineas[i] = l;
            rastro
                .reapuntados
                .push(format!("{}  ·  {}", d.qname().unwrap_or_default(), r.clase));
        }
        // ¿Pasa a cruzar el límite? Lo que también se mueve, no.
        let suyo = ore_core::link::miembro_de(miembros, &d.path);
        if suyo != Some(dir_destino) && !tambien_se_mueven.contains(&d.qname().unwrap_or_default())
        {
            rastro.cruzan.push(d.qname().unwrap_or_default());
        }
        let mut nuevo = lineas.join("\n");
        if acaba_en_salto {
            nuevo.push('\n');
        }
        taller.escribir(&d.path, nuevo);
    }

    // ③ el anuncio, en el manifiesto de ORIGEN
    let manifiesto = pkg
        .docs
        .iter()
        .filter(|d| d.kind == ore_core::document::Kind::Package)
        .find(|d| d.path.parent() == Some(dir_origen))
        .ok_or_else(|| format!("`{qname}` no está dentro de ningún paquete"))?;
    let version = manifiesto
        .meta("version")
        .and_then(|n| n.as_str())
        .unwrap_or("0.1.0")
        .to_string();
    let desde = since.unwrap_or(&version).to_string();
    let t = taller.leer(&manifiesto.path)?;
    let anuncio = anunciar(&t, &qname, &nuevo_qname, &desde)?;
    taller.escribir(&manifiesto.path, anuncio);

    Ok((nuevo_qname, rastro))
}

// ── `ore package move` ──────────────────────────────────────────────────────

/// **Mover un documento a otro paquete.**
///
/// Es [`planificar`] una vez y aplicar. Lo que hace y lo que no —incluido por
/// qué **no toca `exports`**— está en la cabecera de [`dividir`], que comparte
/// con él todo salvo el número de documentos.
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
    let (dir_origen, dir_destino) = match sitios(&pkg, &miembros, doc, destino) {
        Ok(x) => x,
        Err(c) => return c,
    };
    if pkg
        .docs
        .iter()
        .any(|d| d.qname().as_deref() == Some(&format!("{destino}.{}", nombre_de(doc))))
    {
        eprintln!(
            "error: `{destino}` ya tiene un documento llamado `{}`",
            nombre_de(doc)
        );
        eprintln!("  Mover este encima perdería el que hay, y eso no lo decide este mando.");
        return ExitCode::from(65);
    }

    let mut taller = Taller::default();
    let (nuevo, rastro) = match planificar(
        &pkg,
        &miembros,
        &mut taller,
        doc,
        &dir_origen,
        &dir_destino,
        destino,
        since,
        &[],
    ) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("error: {e}");
            eprintln!("  Nada se ha movido: o se hacen las cuatro cosas o ninguna.");
            return ExitCode::from(70); // EX_SOFTWARE
        }
    };
    if let Err(e) = taller.aplicar() {
        eprintln!("error: {e}");
        return ExitCode::from(73); // EX_CANTCREAT
    }

    println!("  ✓ {qname} → {nuevo}");
    contar(&rastro, destino, std::slice::from_ref(&nuevo));
    if since.is_none() {
        aviso_de_version(&pkg, &dir_origen);
    }
    diagnosticos(raiz)
}

// ── `ore package split` ─────────────────────────────────────────────────────

/// **Partir un paquete**, que son dos preguntas y no una.
///
/// # Sin `--to` no mueve nada
///
/// Enumera las **componentes**: los grupos de documentos que se nombran entre
/// sí. Es el mismo reparto que `drift-detect` —enseñar y parar es un acto, mover
/// es otro, y fallan por separado—.
///
/// # Y la respuesta tiene dos mitades, medidas
///
/// Sobre un paquete **recién descubierto** la clausura es exacta y sale gratis:
/// 30 documentos en **10 componentes de exactamente 3** —`Table` + `View` +
/// `Entity` por objeto—, y mover una componente entera deja **cero** referencias
/// cruzando. Ahí esto calcula algo que una persona no calcula bien.
///
/// Sobre uno **modelado** no hay corte gratis: los tres paquetes de
/// `acme-retail` son **una sola componente** y su corte más barato cuesta
/// exactamente un cruce. Eso no lo convierte en un mal corte —puede ser justo el
/// límite que se quería trazar— pero cambia lo que este mando hace: **decir el
/// precio, no buscarlo**.
///
/// # La arista se cuenta sin dirección
///
/// Y costó verlo: mover un documento no rompe solo a quien lo nombra, también
/// hace cruzar **lo que él nombra**. Para el corte da igual el sentido.
///
/// # Lo que no decide
///
/// El **dueño** del paquete nuevo —es un acto de gobierno, y por eso
/// `package new` es un verbo aparte—, si el cruce es **aceptable**, y `exports`:
/// es *«esto lo expongo a propósito»*, así que se dice la línea y no se escribe.
pub fn dividir(
    raiz: &Path,
    paquete: &str,
    con: &[String],
    destino: Option<&str>,
    since: Option<&str>,
) -> ExitCode {
    let pkg = ore_core::validate::cargar_paquete(raiz).0;
    let miembros = ore_core::link::miembros(&pkg);
    let Some(manifiesto) = pkg
        .docs
        .iter()
        .filter(|d| d.kind == ore_core::document::Kind::Package)
        .find(|d| d.meta("name").and_then(|n| n.as_str()) == Some(paquete))
    else {
        eprintln!("error: no hay ningún paquete que se llame `{paquete}`");
        return ExitCode::from(65);
    };
    let Some(dir_origen) = manifiesto.path.parent().map(Path::to_path_buf) else {
        eprintln!("error: no se pudo situar `{paquete}`");
        return ExitCode::from(70);
    };

    let dentro: Vec<&Loaded> = pkg
        .docs
        .iter()
        .filter(|d| ore_core::pertenencia::DEL_PAQUETE.contains(&d.kind))
        .filter(|d| ore_core::link::miembro_de(&miembros, &d.path) == Some(&dir_origen))
        .collect();
    let grafo = grafo_de(&pkg, &dentro);

    let Some(destino) = destino else {
        // Sin destino: se enseña y se para.
        let comps = componentes(&dentro, &grafo);
        println!(
            "{paquete} · {} documento(s), {} componente(s)",
            dentro.len(),
            comps.len()
        );
        for (i, c) in comps.iter().enumerate() {
            println!();
            println!("  ── {} · {} documento(s)", i + 1, c.len());
            for q in c {
                println!("     {q}");
            }
        }
        println!();
        if comps.len() > 1 {
            println!("  · cada componente sale entera SIN una sola referencia cruzando.");
        } else {
            println!("  · una sola componente: no hay corte gratis. Elige uno con `--con`");
            println!("    y este mando dirá lo que cuesta antes de moverlo.");
        }
        println!("  ore package split {paquete} --con <qname> --to <paquete>");
        return ExitCode::SUCCESS;
    };

    if con.is_empty() {
        eprintln!("error: con `--to` hace falta `--con <qname>`, al menos uno");
        eprintln!("  Cuál es el corte no lo decide este mando: partir un paquete por la");
        eprintln!("  mitad sin que nadie lo haya dicho sería inventar un límite.");
        return ExitCode::from(64); // EX_USAGE
    }

    // Los que se mueven, resueltos y comprobados ANTES de tocar nada.
    let mut mueven: Vec<&Loaded> = Vec::new();
    for q in con {
        let Some(d) = dentro.iter().find(|d| d.qname().as_deref() == Some(q)) else {
            eprintln!("error: `{q}` no es un documento gobernado de `{paquete}`");
            return ExitCode::from(65);
        };
        mueven.push(d);
    }
    let nombres: Vec<String> = mueven.iter().filter_map(|d| d.qname()).collect();

    let dir_destino = match sitios(&pkg, &miembros, mueven[0], destino) {
        Ok((_, d)) => d,
        Err(c) => return c,
    };

    // **Lo que arrastra**, que es el aviso que importa: si el conjunto pedido no
    // es una componente entera, se dice qué falta para que el corte salga a cero.
    let cierre = clausura(&nombres, &grafo);
    let falta: Vec<&String> = cierre.iter().filter(|q| !nombres.contains(q)).collect();

    let mut taller = Taller::default();
    let mut rastro = Rastro::default();
    let mut nuevos = Vec::new();
    for d in &mueven {
        match planificar(
            &pkg,
            &miembros,
            &mut taller,
            d,
            &dir_origen,
            &dir_destino,
            destino,
            since,
            &nombres,
        ) {
            Ok((n, r)) => {
                nuevos.push(n);
                rastro.reapuntados.extend(r.reapuntados);
                for c in r.cruzan {
                    if !rastro.cruzan.contains(&c) {
                        rastro.cruzan.push(c);
                    }
                }
            }
            Err(e) => {
                eprintln!("error: {e}");
                eprintln!("  Nada se ha movido: o van los {} o ninguno.", mueven.len());
                return ExitCode::from(70);
            }
        }
    }
    if let Err(e) = taller.aplicar() {
        eprintln!("error: {e}");
        return ExitCode::from(73);
    }

    println!("  ✓ {} documento(s) → `{destino}`", nuevos.len());
    for n in &nuevos {
        println!("     {n}");
    }
    contar(&rastro, destino, &nuevos);
    if !falta.is_empty() {
        println!();
        println!(
            "  · el corte no es una componente entera. Con {} más saldría a cero:",
            falta.len()
        );
        for f in &falta {
            println!("      {f}");
        }
    }
    if since.is_none() {
        aviso_de_version(&pkg, &dir_origen);
    }
    diagnosticos(raiz)
}

// ── `ore package merge` ─────────────────────────────────────────────────────

/// **Fundir un paquete en otro**, dejando una lápida.
///
/// # El paquete no desaparece, y eso es lo que hace que funcione
///
/// El razonamiento que casi bloquea esto era estructural y sonaba bien: los tres
/// alcances de `moved` anuncian **dentro de un artefacto que sobrevive**, y un
/// paquete que desaparece se lleva su manifiesto, así que no habría dónde poner
/// el anuncio. Hacía falta un cuarto alcance.
///
/// **No hacía falta.** Un paquete no tiene que desaparecer: se queda como
/// **lápida** —`status: retired`, cero documentos, y un `moved` por cada uno de
/// los que se fueron— y `moved.to` ya cruza de paquete. Se midió con tres
/// experimentos y su control:
///
/// | cómo se deja el origen | qué dice `ore diff` |
/// |---|---|
/// | lápida | `changes: []` · **compatible** · minor |
/// | sin el anuncio | `OOS5007` · **breaking** · major |
/// | el manifiesto borrado | `OOS5007` + `OOS5021` |
///
/// Y el estado tampoco hubo que inventarlo: `01-package` §2.3 adopta el enum de
/// ODCS **verbatim**, y `retired` es uno de los cinco.
///
/// # Las colisiones se niegan, y no se resuelven
///
/// Si los dos paquetes tienen un documento con el mismo nombre, la fusión **no
/// es mecánica**: es una decisión por colisión, y este mando no la toma. La
/// misma elección que `move` al no sobrescribir — perder un documento no lo
/// decide una herramienta.
///
/// # Y lo que la lápida no cuenta
///
/// `diff` compara **dos versiones del mismo paquete**, así que la lápida le
/// vale; a quien importe el origen desde fuera no le decía nada. Eso lo cierra
/// `OOS2031` —depender de un paquete retirado—, que sale de la misma medida.
pub fn fundir(raiz: &Path, origen: &str, destino: &str, since: Option<&str>) -> ExitCode {
    let pkg = ore_core::validate::cargar_paquete(raiz).0;
    let miembros = ore_core::link::miembros(&pkg);

    let manifiesto = |n: &str| {
        pkg.docs
            .iter()
            .filter(|d| d.kind == ore_core::document::Kind::Package)
            .find(|d| d.meta("name").and_then(|x| x.as_str()) == Some(n))
    };
    let (Some(m_origen), Some(m_destino)) = (manifiesto(origen), manifiesto(destino)) else {
        eprintln!("error: hacen falta los dos paquetes, y alguno no está");
        eprintln!("  ore package new <nombre> --owner <handle>");
        return ExitCode::from(65);
    };
    if m_origen.path == m_destino.path {
        eprintln!("error: `{origen}` y `{destino}` son el mismo paquete");
        return ExitCode::from(65);
    }
    let (Some(dir_origen), Some(dir_destino)) = (m_origen.path.parent(), m_destino.path.parent())
    else {
        eprintln!("error: no se pudo situar alguno de los dos");
        return ExitCode::from(70);
    };
    if !usable_como_namespace(destino) {
        eprintln!("error: `{destino}` no puede ser un espacio de nombres — `OOS2030`");
        return ExitCode::from(65);
    }

    let mueven: Vec<&Loaded> = pkg
        .docs
        .iter()
        .filter(|d| ore_core::pertenencia::DEL_PAQUETE.contains(&d.kind))
        .filter(|d| ore_core::link::miembro_de(&miembros, &d.path) == Some(dir_origen))
        .collect();
    if mueven.is_empty() {
        eprintln!("error: `{origen}` no tiene contenido gobernado que fundir");
        eprintln!("  Si ya está vacío, lo que queda es retirarlo: `status: retired`.");
        return ExitCode::from(65);
    }

    // Las colisiones, ANTES de tocar nada. No se resuelven: se dicen.
    let alli: Vec<String> = pkg
        .docs
        .iter()
        .filter(|d| ore_core::link::miembro_de(&miembros, &d.path) == Some(dir_destino))
        .map(nombre_de)
        .collect();
    let choques: Vec<String> = mueven
        .iter()
        .map(|d| nombre_de(d))
        .filter(|n| alli.contains(n))
        .collect();
    if !choques.is_empty() {
        eprintln!(
            "error: {} nombre(s) existen en los dos paquetes",
            choques.len()
        );
        for c in &choques {
            eprintln!("    {c}");
        }
        eprintln!("  Fundir encima perdería uno de los dos, y cuál se pierde no lo decide");
        eprintln!("  una herramienta. Renómbralo con `moved`, o muévelo aparte.");
        return ExitCode::from(65);
    }

    // El `status` de la lápida se localiza ahora, porque sin él no hay fusión
    // que valga: un paquete vacío y `active` es peor que no haber empezado.
    let Some(estado) = m_origen.meta("status") else {
        eprintln!("error: `{origen}` no declara `status`, y la lápida lo necesita");
        return ExitCode::from(65);
    };
    let pos_estado = estado.pos();

    let nombres: Vec<String> = mueven.iter().filter_map(|d| d.qname()).collect();
    let mut taller = Taller::default();
    let mut rastro = Rastro::default();
    let mut nuevos = Vec::new();
    for d in &mueven {
        match planificar(
            &pkg,
            &miembros,
            &mut taller,
            d,
            dir_origen,
            dir_destino,
            destino,
            since,
            &nombres,
        ) {
            Ok((n, r)) => {
                nuevos.push(n);
                rastro.reapuntados.extend(r.reapuntados);
                for c in r.cruzan {
                    if !rastro.cruzan.contains(&c) {
                        rastro.cruzan.push(c);
                    }
                }
            }
            Err(e) => {
                eprintln!("error: {e}");
                eprintln!("  Nada se ha movido: o va el paquete entero o ninguno.");
                return ExitCode::from(70);
            }
        }
    }

    // Y la lápida. Va DESPUÉS de los movimientos porque cada uno anadió su
    // `moved` al mismo manifiesto: se lee del taller, no del disco.
    let t = match taller.leer(&m_origen.path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(70);
        }
    };
    let acaba_en_salto = t.ends_with('\n');
    let mut lineas: Vec<String> = t.lines().map(String::from).collect();
    let Some((i, l)) = sustituir_en(&lineas, pos_estado, "retired") else {
        eprintln!("error: no se pudo retirar `{origen}`");
        eprintln!("  Nada se ha movido: sin la lápida, los nombres viejos se rompen.");
        return ExitCode::from(70);
    };
    lineas[i] = l;
    let mut lapida = lineas.join("\n");
    if acaba_en_salto {
        lapida.push('\n');
    }
    taller.escribir(&m_origen.path, lapida);

    if let Err(e) = taller.aplicar() {
        eprintln!("error: {e}");
        return ExitCode::from(73);
    }

    println!(
        "  ✓ `{origen}` → `{destino}` · {} documento(s)",
        nuevos.len()
    );
    for n in &nuevos {
        println!("     {n}");
    }
    println!();
    println!("  ✓ `{origen}` queda como LÁPIDA: `status: retired`, sin documentos,");
    println!("    y con un `moved` por cada uno. Su nombre no se rompe.");
    contar(&rastro, destino, &nuevos);
    if since.is_none() {
        aviso_de_version(&pkg, dir_origen);
    }
    println!();
    println!("  · quien dependa de `{origen}` lo verá: `OOS2031`.");
    diagnosticos(raiz)
}

// ── El grafo del paquete ────────────────────────────────────────────────────

/// Quién nombra a quién, **sin dirección**.
///
/// Sin dirección porque para el corte da igual el sentido: una arista que cruza
/// el límite es una arista que cruza, la escriba quien la escriba.
fn grafo_de(pkg: &Package, dentro: &[&Loaded]) -> BTreeMap<String, BTreeSet<String>> {
    let suyos: BTreeSet<String> = dentro.iter().filter_map(|d| d.qname()).collect();
    let mut g: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for d in &pkg.docs {
        let Some(mio) = d.qname() else { continue };
        for r in ore_core::exporta::referencias(d) {
            if !suyos.contains(&r.destino) || !suyos.contains(&mio) || r.destino == mio {
                continue;
            }
            g.entry(mio.clone()).or_default().insert(r.destino.clone());
            g.entry(r.destino).or_default().insert(mio.clone());
        }
    }
    for q in &suyos {
        g.entry(q.clone()).or_default();
    }
    g
}

fn componentes(dentro: &[&Loaded], g: &BTreeMap<String, BTreeSet<String>>) -> Vec<Vec<String>> {
    let mut visto: BTreeSet<String> = BTreeSet::new();
    let mut out = Vec::new();
    for d in dentro {
        let Some(q) = d.qname() else { continue };
        if visto.contains(&q) {
            continue;
        }
        let comp = clausura(&[q], g);
        visto.extend(comp.iter().cloned());
        out.push(comp);
    }
    out.sort_by_key(|c| std::cmp::Reverse(c.len()));
    out
}

/// Todo lo que se alcanza desde `semillas`. Es la clausura del corte.
fn clausura(semillas: &[String], g: &BTreeMap<String, BTreeSet<String>>) -> Vec<String> {
    let mut visto: BTreeSet<String> = BTreeSet::new();
    let mut pila: Vec<String> = semillas.to_vec();
    while let Some(x) = pila.pop() {
        if !visto.insert(x.clone()) {
            continue;
        }
        if let Some(vs) = g.get(&x) {
            pila.extend(vs.iter().cloned());
        }
    }
    visto.into_iter().collect()
}

// ── Lo compartido ───────────────────────────────────────────────────────────

fn nombre_de(d: &Loaded) -> String {
    d.meta("name")
        .and_then(|n| n.as_str())
        .unwrap_or_default()
        .to_string()
}

/// El directorio del paquete de origen y el del destino, con sus negativas.
fn sitios(
    pkg: &Package,
    miembros: &[PathBuf],
    doc: &Loaded,
    destino: &str,
) -> Result<(PathBuf, PathBuf), ExitCode> {
    if !ore_core::pertenencia::DEL_PAQUETE.contains(&doc.kind) {
        eprintln!(
            "error: un `{}` no es de un paquete: su nombre es el de un vocabulario compartido",
            doc.kind.as_str()
        );
        eprintln!("  Tiene que significar lo mismo desde todos los paquetes, así que moverlo");
        eprintln!("  entre ellos no querría decir nada. Vive en la raíz del workspace.");
        return Err(ExitCode::from(65));
    }
    if !usable_como_namespace(destino) {
        eprintln!("error: `{destino}` no puede ser un espacio de nombres");
        eprintln!("  Un paquete así puede contener vocabulario compartido, no contenido");
        eprintln!("  gobernado — `OOS2030`.");
        return Err(ExitCode::from(65));
    }
    let Some(m) = pkg
        .docs
        .iter()
        .filter(|d| d.kind == ore_core::document::Kind::Package)
        .find(|d| d.meta("name").and_then(|n| n.as_str()) == Some(destino))
    else {
        eprintln!("error: no hay ningún paquete que se llame `{destino}`");
        eprintln!("  ore package new {destino} --owner <handle>");
        return Err(ExitCode::from(65));
    };
    let (Some(dir_destino), Some(dir_origen)) = (
        m.path.parent(),
        ore_core::link::miembro_de(miembros, &doc.path),
    ) else {
        eprintln!("error: no se pudo situar el documento o el paquete destino");
        return Err(ExitCode::from(70));
    };
    if dir_origen == dir_destino {
        eprintln!(
            "error: `{}` ya está en `{destino}`",
            doc.qname().unwrap_or_default()
        );
        return Err(ExitCode::from(65));
    }
    Ok((dir_origen.to_path_buf(), dir_destino.to_path_buf()))
}

fn contar(rastro: &Rastro, destino: &str, nuevos: &[String]) {
    if !rastro.reapuntados.is_empty() {
        println!();
        println!(
            "  {} referencia(s) reapuntada(s):",
            rastro.reapuntados.len()
        );
        for r in &rastro.reapuntados {
            println!("    {r}");
        }
    }
    if rastro.cruzan.is_empty() {
        println!();
        println!("  · ni una sola referencia cruza: el corte sale a cero.");
        return;
    }
    println!();
    println!(
        "  · ahora {} referencia(s) cruzan a `{destino}`:",
        rastro.cruzan.len()
    );
    for c in &rastro.cruzan {
        println!("      {c}");
    }
    println!("    `exports` es «esto lo expongo a propósito», así que este mando no lo");
    println!("    toca. En el manifiesto de `{destino}`:");
    println!("      spec: {{ …, exports: [{}] }}", nuevos.join(", "));
}

fn aviso_de_version(pkg: &Package, dir_origen: &Path) {
    let v = pkg
        .docs
        .iter()
        .filter(|d| d.kind == ore_core::document::Kind::Package)
        .find(|d| d.path.parent() == Some(dir_origen))
        .and_then(|d| d.meta("version"))
        .and_then(|n| n.as_str())
        .unwrap_or("0.1.0")
        .to_string();
    println!();
    println!("  · `since: {v}` es la versión que el paquete declara HOY");
    println!("    `ore diff` calcula el salto que este cambio exige. Si publicas con");
    println!("    uno mayor, ajústala: `--since` la fija.");
}

fn diagnosticos(raiz: &Path) -> ExitCode {
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
