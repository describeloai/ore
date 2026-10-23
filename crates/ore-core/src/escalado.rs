//! **El escalado de Query por coste admitido** (Query Q5·1, `docs/query.md` §3.2).
//!
//! Cuántos trabajadores de `ore-consulta` pedir **ahora**, con lo que ya se sabe:
//! lo que cada trabajador vivo dice que le cabe, los que están en camino y lo que
//! espera en la cola (la memoria y los hilos que la admisión le dio a cada
//! consulta). No por fallos ni por la cola que crece: por lo admitido (la idea de
//! *Intelligent Workload Management* de Databricks, con la estimación de §3.4 en
//! vez de un modelo aprendido).
//!
//! La regla:
//! 1. Lo encolado se reparte, en su orden, en lo que cabe (lo libre de los vivos y
//!    los que están en camino, enteros); lo que no cabe abre un trabajador nuevo.
//! 2. **Mientras hay actividad, queda siempre un trabajador entero libre** (un G):
//!    la siguiente consulta grande no espera a que se encienda uno (§3.1, sin
//!    «preparando cómputo»). Sin actividad, nada: el suelo caliente basta.
//! 3. Nunca se pide por encima del techo de la celda, contando lo que ya está en
//!    camino: pedir dos veces lo mismo no lo trae antes.
//!
//! Devolver no es cosa de esta regla: un trabajador de quita y pon se va solo
//! cuando lleva un rato sin consultas (`ORE_CONSULTA_OCIOSO_S`), y el del suelo
//! no se va.

use crate::admision::Umbrales;

/// Lo libre de un trabajador, o lo que una consulta pide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hueco {
    pub mb: u64,
    pub hilos: u64,
}

/// Lo que se sabe en este momento.
#[derive(Debug, Clone, Default)]
pub struct Foto {
    /// Lo libre de cada trabajador vivo (lo que dijo al pedir, menos lo que tomó).
    pub vivos: Vec<Hueco>,
    /// Trabajadores pedidos que todavía no piden: llegan enteros.
    pub en_camino: u64,
    /// Lo que espera, en el orden de la cola.
    pub encoladas: Vec<Hueco>,
    /// Hubo consultas hace poco (la ventana la decide quien llama).
    pub activa: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub pedir: u64,
    pub razon: String,
}

/// Un trabajador entero: lo que la admisión reparte de él (G).
pub fn entero(u: &Umbrales) -> Hueco {
    Hueco {
        mb: u.trabajador_mb * u.reparto_pct / 100,
        hilos: u.g_hilos,
    }
}

fn cabe(h: &Hueco, c: &Hueco) -> bool {
    h.mb >= c.mb && h.hilos >= c.hilos
}

/// **La regla.** `techo`: los trabajadores que la celda puede tener a la vez,
/// contando el suelo.
pub fn escalar(f: &Foto, u: &Umbrales, techo: u64) -> Decision {
    let todo = entero(u);
    let mut huecos: Vec<Hueco> = f.vivos.clone();
    huecos.extend((0..f.en_camino).map(|_| todo));
    let mut nuevos = 0u64;
    for c in &f.encoladas {
        // Lo que pide más que un trabajador entero (no debería: la admisión no
        // da más), cuenta como uno.
        let c = Hueco {
            mb: c.mb.min(todo.mb),
            hilos: c.hilos.min(todo.hilos),
        };
        match huecos.iter_mut().find(|h| cabe(h, &c)) {
            Some(h) => {
                h.mb -= c.mb;
                h.hilos -= c.hilos;
            }
            None => {
                nuevos += 1;
                huecos.push(Hueco {
                    mb: todo.mb - c.mb,
                    hilos: todo.hilos - c.hilos,
                });
            }
        }
    }
    let por_la_cola = nuevos;
    let hay_trabajo = f.activa || !f.encoladas.is_empty();
    let margen = hay_trabajo && !huecos.iter().any(|h| cabe(h, &todo));
    if margen {
        nuevos += 1;
    }
    let hay = f.vivos.len() as u64 + f.en_camino;
    let cabe_bajo_el_techo = techo.saturating_sub(hay);
    let pedir = nuevos.min(cabe_bajo_el_techo);
    let razon = if nuevos == 0 {
        if hay_trabajo {
            "cabe lo encolado y queda un trabajador entero libre".to_string()
        } else {
            "sin actividad: basta el suelo".to_string()
        }
    } else {
        let mut r = Vec::new();
        if por_la_cola > 0 {
            r.push(format!(
                "lo encolado no cabe en lo que hay: {por_la_cola} más"
            ));
        }
        if margen {
            r.push("para que quede un trabajador entero libre: 1 más".to_string());
        }
        let mut r = r.join("; ");
        if pedir < nuevos {
            r.push_str(&format!(
                "; el techo de la celda es {techo} y hay {hay}: se piden {pedir}"
            ));
        }
        r
    };
    Decision { pedir, razon }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn u() -> Umbrales {
        Umbrales::default()
    }
    fn g() -> Hueco {
        entero(&u())
    }
    fn l() -> Hueco {
        Hueco {
            mb: u().l_memoria_mb,
            hilos: u().l_hilos,
        }
    }
    fn libre(mb: u64, hilos: u64) -> Hueco {
        Hueco { mb, hilos }
    }

    #[test]
    fn sin_actividad_basta_el_suelo() {
        let f = Foto {
            vivos: vec![g()],
            ..Default::default()
        };
        assert_eq!(escalar(&f, &u(), 4).pedir, 0);
        // Ni aunque el suelo esté ocupado: sin nadie preguntando, no se enciende nada.
        let f = Foto {
            vivos: vec![libre(0, 0)],
            ..Default::default()
        };
        assert_eq!(escalar(&f, &u(), 4).pedir, 0);
    }

    #[test]
    fn con_actividad_queda_siempre_un_trabajador_entero_libre() {
        // El suelo libre: ya está el hueco.
        let f = Foto {
            vivos: vec![g()],
            activa: true,
            ..Default::default()
        };
        assert_eq!(escalar(&f, &u(), 4).pedir, 0);
        // El suelo corriendo una G: se pide otro antes de que haga falta.
        let f = Foto {
            vivos: vec![libre(0, 0)],
            activa: true,
            ..Default::default()
        };
        let d = escalar(&f, &u(), 4);
        assert_eq!(d.pedir, 1, "{}", d.razon);
        assert!(d.razon.contains("entero libre"), "{}", d.razon);
    }

    #[test]
    fn lo_que_esta_en_camino_cuenta_y_no_se_pide_dos_veces() {
        let f = Foto {
            vivos: vec![libre(0, 0)],
            en_camino: 1,
            activa: true,
            ..Default::default()
        };
        assert_eq!(escalar(&f, &u(), 4).pedir, 0);
        // Y una G encolada va al que llega; el margen pide uno más.
        let f = Foto {
            encoladas: vec![g()],
            ..f
        };
        assert_eq!(escalar(&f, &u(), 4).pedir, 1);
    }

    #[test]
    fn una_g_que_no_cabe_abre_uno_y_el_margen_otro() {
        // El suelo con unas L (le quedan 30 GB y 4 hilos): la G no cabe.
        let f = Foto {
            vivos: vec![libre(30_000, 4)],
            encoladas: vec![g()],
            activa: true,
            ..Default::default()
        };
        let d = escalar(&f, &u(), 4);
        assert_eq!(d.pedir, 2, "{}", d.razon);
        assert!(
            d.razon.contains("1 más") && d.razon.contains("entero libre"),
            "{}",
            d.razon
        );
    }

    #[test]
    fn las_l_se_reparten_en_lo_libre_y_por_hilos() {
        // Cuatro L caben en el suelo libre (8 hilos / 2), y queda sin hueco entero:
        // el margen pide uno.
        let f = Foto {
            vivos: vec![g()],
            encoladas: vec![l(); 4],
            ..Default::default()
        };
        assert_eq!(escalar(&f, &u(), 4).pedir, 1);
        // Treinta L: cuatro por trabajador; el suelo y siete más, y el margen.
        let f = Foto {
            vivos: vec![g()],
            encoladas: vec![l(); 30],
            ..Default::default()
        };
        assert_eq!(escalar(&f, &u(), 100).pedir, 8);
    }

    #[test]
    fn el_techo_manda_y_se_dice() {
        let f = Foto {
            vivos: vec![libre(0, 0)],
            encoladas: vec![g(), g(), g()],
            ..Default::default()
        };
        let d = escalar(&f, &u(), 2);
        assert_eq!(d.pedir, 1, "{}", d.razon);
        assert!(d.razon.contains("techo"), "{}", d.razon);
        // Con el techo lleno, nada.
        let f = Foto { en_camino: 1, ..f };
        assert_eq!(escalar(&f, &u(), 2).pedir, 0);
    }

    #[test]
    fn el_entero_es_lo_que_la_admision_reparte() {
        assert_eq!(
            entero(&u()),
            Hueco {
                mb: 39_321,
                hilos: 8
            }
        );
    }
}
