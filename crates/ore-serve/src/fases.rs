//! **Dónde se va el tiempo de una petición** (L1, 2026-10-14).
//!
//! Medido en victor: hubo un atasco de unos 6 minutos (20:36–20:42) en el que
//! `/fuentes`, `/assets`, `/arbol`, `loadTable` y el `over()` de los puestos se
//! pararon a la vez con la CPU parada (45m de 500m) y salieron en cascada; y
//! cada `GET /puestos/{id}/datos/{x}` tardaba ~15 s. El código dice dónde
//! podría ser —el candado del espejo retenido mientras `git fetch` habla con la
//! forja sin plazo; el índice de activos rehecho con un `git log` por activo; los
//! subprocesos `ore`— pero no cuánto. Esto lo cuenta, petición a petición.
//!
//! Un hilo por conexión (`ore-entrada`), así que lo de una petición se acumula
//! en el hilo que la atiende: [`empezar`] al llegar, y [`resumen`] va al final
//! de su línea de acceso. Lo que un hilo hijo haga no se cuenta (no lo hay en
//! las rutas medidas). Y un `git` de más de [`LENTO`] se dice EN EL MOMENTO
//! (`lento · git …`): un cuelgue deja rastro aunque la petición no acabe nunca.

use std::cell::RefCell;
use std::time::Duration;

/// A partir de aquí un `git` se dice en cuanto termina, con sus argumentos.
pub const LENTO: Duration = Duration::from_secs(5);

#[derive(Default)]
struct Fases {
    /// Las veces que se esperó al candado del espejo, y cuánto en total.
    candado: (u32, Duration),
    /// Los `git` que habla con la forja (`fetch`, `clone`, `ls-remote`, `push`).
    red: (u32, Duration),
    /// Los demás `git`, en local (`log`, `rev-parse`, `worktree`, `status`…).
    local: (u32, Duration),
    /// Los subprocesos `ore`.
    ore: (u32, Duration),
    /// El índice de activos: de la caché (`true`) o rehecho, y cuánto.
    indice: Vec<(bool, Duration)>,
}

thread_local! {
    static F: RefCell<Fases> = RefCell::new(Fases::default());
}

/// Una petición empieza en este hilo: lo de la anterior (misma conexión) fuera.
pub fn empezar() {
    F.with(|f| *f.borrow_mut() = Fases::default());
}

/// Se esperó `d` al candado del espejo.
pub fn candado(d: Duration) {
    F.with(|f| {
        let mut f = f.borrow_mut();
        f.candado.0 += 1;
        f.candado.1 += d;
    });
}

/// Un `git` con estos argumentos tardó `d`.
pub fn git(args: &[&str], d: Duration) {
    let red = matches!(
        args.first().copied(),
        Some("fetch" | "clone" | "ls-remote" | "push" | "pull")
    );
    F.with(|f| {
        let mut f = f.borrow_mut();
        let c = if red { &mut f.red } else { &mut f.local };
        c.0 += 1;
        c.1 += d;
    });
    if d >= LENTO {
        eprintln!(
            "lento · git {} · {} ms",
            args.iter().take(4).copied().collect::<Vec<_>>().join(" "),
            d.as_millis()
        );
    }
}

/// Un subproceso `ore` tardó `d`.
pub fn ore(d: Duration) {
    F.with(|f| {
        let mut f = f.borrow_mut();
        f.ore.0 += 1;
        f.ore.1 += d;
    });
}

/// El índice de activos: de la caché o rehecho, en `d`.
pub fn indice(de_cache: bool, d: Duration) {
    F.with(|f| f.borrow_mut().indice.push((de_cache, d)));
}

/// Lo de esta petición, en una línea; `None` si no hizo nada de lo medido.
pub fn resumen() -> Option<String> {
    F.with(|f| {
        let f = f.borrow();
        let mut p = Vec::new();
        let par =
            |nombre: &str, (n, d): (u32, Duration)| format!("{nombre} {n}×{} ms", d.as_millis());
        if f.candado.0 > 0 {
            p.push(par("candado", f.candado));
        }
        if f.red.0 > 0 {
            p.push(par("git-red", f.red));
        }
        if f.local.0 > 0 {
            p.push(par("git-local", f.local));
        }
        if f.ore.0 > 0 {
            p.push(par("ore", f.ore));
        }
        for (de_cache, d) in &f.indice {
            p.push(format!(
                "índice {} {} ms",
                if *de_cache { "caché" } else { "rehecho" },
                d.as_millis()
            ));
        }
        (!p.is_empty()).then(|| p.join(" · "))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lo_de_una_peticion_va_en_una_linea_y_no_pasa_a_la_siguiente() {
        empezar();
        assert_eq!(resumen(), None);
        candado(Duration::from_millis(1200));
        candado(Duration::from_millis(3));
        git(&["fetch", "--prune", "origin"], Duration::from_millis(80));
        git(&["log", "-1", "--", "x"], Duration::from_millis(9));
        git(&["log", "-1", "--", "y"], Duration::from_millis(11));
        ore(Duration::from_millis(3100));
        indice(false, Duration::from_millis(9000));
        assert_eq!(
            resumen().as_deref(),
            Some(
                "candado 2×1203 ms · git-red 1×80 ms · git-local 2×20 ms · ore 1×3100 ms · índice rehecho 9000 ms"
            )
        );
        empezar();
        assert_eq!(resumen(), None);
    }
}
