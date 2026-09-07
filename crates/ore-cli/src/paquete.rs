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

use std::path::Path;
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

#[cfg(test)]
mod tests {
    use super::*;

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
