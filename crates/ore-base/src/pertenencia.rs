//! **Las reglas de los nombres** (CI C4: vocabulario): qué puede ser un `namespace`, qué
//! bases están reservadas y qué es un *handle* (`team:<h>`, `user:<h>`) y cómo se deriva de un
//! usuario. La pertenencia de un documento a su paquete (`OOS2030`) vive en
//! `ore_core::pertenencia`, que re-exporta esto.

/// Si un nombre puede ser un `namespace`.
///
/// **Los dos vocabularios no coinciden, y eso lo destapó construir el paso 2.**
/// El esquema publicado deja que un paquete se llame `oos.dev` o `mi-paquete`
/// —`packageName` admite puntos, guiones y barras, porque el nombre de un
/// paquete es también la coordenada con la que otro lo importa— y un
/// `namespace` es un `identifier`, que no admite ninguno de los tres.
///
/// Así que hay nombres de paquete para los que esta regla es **insatisfacible**,
/// y hay uno en el corpus: el paquete `oos.dev`. No se puede arreglar
/// aflojándola —el `namespace` es lo que es— así que se dice: un paquete cuyo
/// nombre no sea un identificador **no puede contener contenido gobernado**, y
/// el diagnóstico lo explica en vez de pedir lo imposible.
/// Lo que un nombre de paquete tiene que ser para ser un espacio de nombres:
/// una letra y luego letras, dígitos y `_`. Público porque quien crea un
/// paquete (`discover --name`, el alta de `ore-serve`) tiene que decirlo ANTES
/// de escribir uno entero que no compile.
pub fn puede_ser_namespace(n: &str) -> bool {
    let mut c = n.chars();
    c.next().is_some_and(|p| p.is_ascii_alphabetic())
        && c.all(|x| x.is_ascii_alphanumeric() || x == '_')
}

/// docs/ejecuciones.md §8.1 ② · **Las bases que nadie crea ni escribe**:
/// `system` es la de las tablas del sistema (`system.default.executions`),
/// que escribe `ore-serve` con sus punteros en el lago, fuera del árbol.
pub const BASES_RESERVADAS: &[&str] = &["system"];

/// Si `n` es una de [`BASES_RESERVADAS`] (sin mirar mayúsculas: el SQL no las mira).
pub fn es_base_reservada(n: &str) -> bool {
    BASES_RESERVADAS.iter().any(|r| r.eq_ignore_ascii_case(n))
}

/// Lo que un `owner` tiene que ser (`OOS2009`): `team:<handle>` o
/// `user:<handle>`, y el handle en minúsculas, dígitos y guiones. Público por
/// lo mismo que `puede_ser_namespace`: quien lo escribe (`discover --owner`, el
/// alta de `ore-serve` con su organización) tiene que saberlo ANTES.
pub fn es_handle(s: &str) -> bool {
    let Some((tipo, h)) = s.split_once(':') else {
        return false;
    };
    matches!(tipo, "team" | "user")
        && h.starts_with(|c: char| c.is_ascii_lowercase())
        && h.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Lo más largo que se deja un handle que se DERIVA (`handle_de`). Uno escrito
/// a mano no tiene techo: `es_handle` no lo pide y no se le añade aquí.
pub const HANDLE_MAXIMO: usize = 39;

/// **Un handle a partir de un nombre de usuario** (o de lo que haga de él): en
/// minúsculas, las vocales con tilde y la `ñ` a su letra, todo lo demás que no
/// sea letra o dígito a `-`, sin guiones repetidos ni en los bordes, y como mucho
/// [`HANDLE_MAXIMO`]. Si el usuario es un correo, cuenta lo de antes de la `@`.
/// Si empieza por dígito, `u-` delante: un handle empieza por letra (`OOS2009`).
///
/// Es la regla con la que `ore-iam` da a cada persona su `user:<handle>` la
/// primera vez que la ve (la 048) y con la que `ore-serve` deriva uno cuando no
/// hay `ore-iam` a quien preguntar. `None` si no queda nada.
///
/// ⚠️ Derivar no es asignar: dos usuarios distintos pueden dar el mismo
///   (`ana.g` y `ana_g`), y quien asigna desempata.
pub fn handle_de(usuario: &str) -> Option<String> {
    let local = usuario.split('@').next().unwrap_or(usuario);
    let mut h = String::new();
    for c in local.to_lowercase().chars() {
        let c = match c {
            'á' | 'à' | 'ä' | 'â' | 'ã' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' | 'õ' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            c if c.is_ascii_lowercase() || c.is_ascii_digit() => c,
            _ => '-',
        };
        if c == '-' && (h.is_empty() || h.ends_with('-')) {
            continue;
        }
        h.push(c);
    }
    let mut h = h.trim_end_matches('-').to_string();
    if h.starts_with(|c: char| c.is_ascii_digit()) {
        h = format!("u-{h}");
    }
    h.truncate(HANDLE_MAXIMO);
    let h = h.trim_end_matches('-').to_string();
    es_handle(&format!("user:{h}")).then_some(h)
}
