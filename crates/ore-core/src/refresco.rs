//! ⭐ **El refresco que cumple *Fresh within*** (ADR 0063 S8).
//!
//! Hasta S8, una copia sólo se rehacía al decidirla, al cambiar la lista de
//! copias o a mano: con `freshness: 1h` declarada, Health decía la verdad
//! (`freshness-exceeded`) y nadie la arreglaba. Esto decide **qué copias toca
//! refrescar ahora**, de lo que el índice de `/assets` ya trae de cada una —su
//! `freshness`, la edad que Health calculó y el último intento—; `ore-serve`
//! encola la pasada (D-S8-1: él decide; sin nada que vencer, no hay pod).
//!
//! Sólo entran **las copias que declaran `freshness`**: el cliente lo activa al
//! declararlo, y lo apaga al quitarlo. Una colección no tiene `freshness` en la
//! spec, y lo escrito lo refresca quien lo escribe.
//!
//! - **Cuándo** (D-S8-2): al [`UMBRAL`] de su `freshness` desde lo último que se
//!   sabe de ella —la última comprobación de que el origen no cambió, o la hora
//!   de sus datos—: con `1h`, a los 48 minutos. Lo que no se sabe de cuándo es
//!   (sin hora, o nunca copiada) toca ya.
//! - **Tras un fallo** (D-S8-3): espera creciente, la [`cadencia`](toca) por
//!   2, 4, 8…, y como mucho una vez por periodo de `freshness`. Cuenta como fallo
//!   lo que no renueva la frescura: `error`, `retenida` (una promesa la paró) y
//!   `sin-comprobar` (dijo «ya está» sin medida: sin señal de cambio, una pasada
//!   normal no puede respaldarla).
//! - **Una sola vez a la vez**: lo encolado no se encola otra vez hasta que la
//!   pasada deje su intento, o pase [`EN_VUELO`].
//!
//! Puro y sin reloj: la hora llega de fuera, como en [`crate::salud`].

use crate::parse::Node;

/// La fracción de `freshness` a la que una copia toca: 8/10.
pub const UMBRAL: (i64, i64) = (8, 10);

/// Cuánto se espera a que una pasada encolada deje su intento antes de darla
/// por perdida: su plazo (`activeDeadlineSeconds: 3600`) y el arranque.
pub const EN_VUELO: i64 = 75 * 60;

/// Una copia que declara `freshness`, como la ve el índice.
#[derive(Debug, Clone, PartialEq)]
pub struct Copia {
    /// `paquete.schema.nombre` en su forma corta: la de `VISTAS` y el puntero.
    pub vista: String,
    /// `freshness`, en segundos.
    pub limite: i64,
    /// La edad que Health calculó (`ageSeconds`, o `lagSeconds` sin
    /// comprobación posterior); `None` si no se sabe.
    pub edad: Option<i64>,
    /// El último intento: cuándo, y si renovó la frescura.
    pub intento: Option<(i64, bool)>,
}

/// Lo que el vigilante recuerda de cada copia entre una vuelta y la siguiente.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Memoria {
    /// Cuándo la encoló por última vez, si aún no ha visto su intento.
    pub encolada: Option<i64>,
    /// Los intentos seguidos que no renovaron la frescura.
    pub fallos: u32,
}

/// Lo que un intento dice de la frescura: `publicada` y `sin-cambios` la
/// renuevan; lo demás no.
fn renueva(resultado: &str) -> bool {
    matches!(resultado, "publicada" | "sin-cambios")
}

/// **Las copias con `freshness` del índice de `/assets`** (con la salud ya
/// anotada). Lo demás —sin `freshness`, sin `from`, otro `kind`— no entra.
pub fn del_indice(indice: &Node) -> Vec<Copia> {
    let texto = |n: &Node, k: &str| {
        n.get(k)
            .and_then(|(_, v)| v.as_str())
            .filter(|s| !s.is_empty() && *s != "null")
            .map(String::from)
    };
    let entero = |n: Option<&Node>, k: &str| {
        n.and_then(|n| n.get(k))
            .and_then(|(_, v)| v.as_str())
            .and_then(|s| s.parse::<i64>().ok())
    };
    let mut out: Vec<Copia> = indice
        .get("items")
        .map(|(_, i)| i.entries())
        .unwrap_or(&[])
        .iter()
        .filter_map(|(_, it)| {
            if texto(it, "kind").as_deref() != Some("Dataset") {
                return None;
            }
            let define = it.get("define").map(|(_, d)| d)?;
            texto(define, "from")?;
            let limite = texto(define, "freshness")
                .as_deref()
                .and_then(crate::frescura::duracion)
                .filter(|l| *l > 0)?;
            let vista = crate::normalize::corto(
                &texto(it, "paquete")?,
                &texto(it, "schema").unwrap_or_else(|| "default".into()),
                &texto(it, "name")?,
            );
            let fresca = it
                .get("salud")
                .and_then(|(_, s)| s.get("freshness"))
                .map(|(_, f)| f);
            let edad = entero(fresca, "ageSeconds").or(entero(fresca, "lagSeconds"));
            let intento = it
                .get("puntero")
                .and_then(|(_, p)| p.get("intento"))
                .and_then(|(_, i)| {
                    let cuando = texto(i, "cuando")
                        .as_deref()
                        .and_then(crate::frescura::epoca)?;
                    Some((cuando, renueva(&texto(i, "resultado").unwrap_or_default())))
                });
            Some(Copia {
                vista,
                limite,
                edad,
                intento,
            })
        })
        .collect();
    out.sort_by(|a, b| a.vista.cmp(&b.vista));
    out
}

/// Pone al día lo que se recuerda con lo que el índice dice ahora: si la pasada
/// encolada ya dejó su intento, cuenta si renovó o no, y deja de estar en vuelo.
/// Un fallo que no venía de una pasada encolada aquí (una a mano) cuenta uno.
pub fn observar(m: &mut Memoria, c: &Copia) {
    let Some((cuando, bien)) = c.intento else {
        return;
    };
    match m.encolada {
        Some(t) if cuando > t => {
            m.encolada = None;
            m.fallos = if bien { 0 } else { m.fallos.saturating_add(1) };
        }
        Some(_) => {}
        None if bien => m.fallos = 0,
        None => m.fallos = m.fallos.max(1),
    }
}

/// **¿Toca refrescarla ahora?** `Ok(por qué)` si toca; `Err(por qué no)` si no.
/// `cadencia` es cada cuánto mira el vigilante (segundos): la base de la
/// espera tras un fallo.
pub fn toca(c: &Copia, m: &Memoria, ahora: i64, cadencia: i64) -> Result<String, String> {
    if let Some(t) = m.encolada
        && ahora - t < EN_VUELO
        && c.intento.is_none_or(|(cuando, _)| cuando <= t)
    {
        return Err(format!("queued {} s ago, waiting for its pass", ahora - t));
    }
    let umbral = c.limite * UMBRAL.0 / UMBRAL.1;
    let por_que = match c.edad {
        None => "when its data is from is not known".to_string(),
        Some(e) if e >= umbral => format!(
            "{} s old, {} % of its freshness ({} s)",
            e,
            e * 100 / c.limite.max(1),
            c.limite
        ),
        Some(e) => {
            return Err(format!(
                "{} s old: due at {} s ({} % of {} s)",
                e,
                umbral,
                UMBRAL.0 * 100 / UMBRAL.1,
                c.limite
            ));
        }
    };
    if let Some((cuando, false)) = c.intento {
        let fallos = m.fallos.clamp(1, 20);
        let espera = (cadencia.max(1) << fallos).min(c.limite);
        let desde = ahora - cuando;
        if desde < espera {
            return Err(format!(
                "its last pass did not renew it ({fallos} in a row): next try in {} s",
                espera - desde
            ));
        }
        return Ok(format!(
            "{por_que}; retrying after {fallos} pass(es) that did not renew it"
        ));
    }
    Ok(por_que)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    const HORA: i64 = 3600;

    fn copia(edad: Option<i64>, intento: Option<(i64, bool)>) -> Copia {
        Copia {
            vista: "vara_pg.ore_vara.pedidos".into(),
            limite: HORA,
            edad,
            intento,
        }
    }

    #[test]
    fn solo_entran_las_copias_que_declaran_freshness() {
        let indice = crate::parse::parse(
            r#"{"items":{
              "dataset:vara_pg.ore_vara.pedidos":{"kind":"Dataset","paquete":"vara_pg","schema":"ore_vara","name":"pedidos",
                "define":{"from":"table:pg.public.pedidos","freshness":"1h"},
                "puntero":{"intento":{"cuando":"2026-10-16T07:00:00Z","resultado":"sin-cambios"}},
                "salud":{"freshness":{"ageSeconds":3000,"lagSeconds":90000}}},
              "dataset:vara_pg.clientes":{"kind":"Dataset","paquete":"vara_pg","schema":"default","name":"clientes",
                "define":{"from":"table:pg.public.clientes","freshness":"15m"},
                "salud":{"freshness":{"ageSeconds":null,"lagSeconds":null}}},
              "dataset:vara_pg.sin":{"kind":"Dataset","paquete":"vara_pg","schema":"default","name":"sin",
                "define":{"from":"table:pg.public.sin"}},
              "dataset:sandbox.escrito":{"kind":"Dataset","paquete":"sandbox","schema":"default","name":"escrito",
                "define":{"columns":3,"freshness":"1h"}},
              "collection:m.i":{"kind":"MediaCollection","define":{"freshness":"1h"}}
            }}"#,
        )
        .unwrap();
        let c = del_indice(&indice);
        assert_eq!(c.len(), 2, "{c:?}");
        assert_eq!(c[0].vista, "vara_pg.clientes");
        assert_eq!((c[0].limite, c[0].edad, c[0].intento), (900, None, None));
        assert_eq!(c[1].vista, "vara_pg.ore_vara.pedidos");
        assert_eq!(c[1].edad, Some(3000), "la edad de Health, no el retraso");
        assert_eq!(c[1].intento, Some((1_792_134_000, true)));
    }

    #[test]
    fn toca_al_ochenta_por_ciento_y_lo_que_no_se_sabe_ya() {
        let m = Memoria::default();
        assert!(toca(&copia(Some(2000), None), &m, 0, 60).is_err());
        assert!(
            toca(&copia(Some(2880), None), &m, 0, 60).is_ok(),
            "48 min de 1h"
        );
        assert!(toca(&copia(None, None), &m, 0, 60).is_ok());
    }

    #[test]
    fn lo_encolado_no_se_encola_otra_vez_hasta_su_intento() {
        let c = copia(Some(3000), Some((100, true)));
        let m = Memoria {
            encolada: Some(1000),
            fallos: 0,
        };
        assert!(toca(&c, &m, 1060, 60).is_err(), "en vuelo");
        // Ya dejó su intento: si sigue vencida, toca otra vez.
        let c = copia(Some(3000), Some((1050, true)));
        assert!(toca(&c, &m, 1060, 60).is_ok());
        // Perdida: pasado el plazo, se encola otra vez.
        let c = copia(Some(3000), Some((100, true)));
        assert!(toca(&c, &m, 1000 + EN_VUELO, 60).is_ok());
    }

    #[test]
    fn tras_un_fallo_espera_creciente_y_como_mucho_un_periodo() {
        let mut m = Memoria {
            encolada: Some(1000),
            fallos: 0,
        };
        let c = copia(Some(3000), Some((1100, false)));
        observar(&mut m, &c);
        assert_eq!(
            m,
            Memoria {
                encolada: None,
                fallos: 1
            }
        );
        assert!(toca(&c, &m, 1100 + 119, 60).is_err(), "1 fallo: 2 min");
        assert!(toca(&c, &m, 1100 + 120, 60).is_ok());
        m.fallos = 3;
        assert!(toca(&c, &m, 1100 + 479, 60).is_err(), "3 fallos: 8 min");
        assert!(toca(&c, &m, 1100 + 480, 60).is_ok());
        m.fallos = 12;
        assert!(
            toca(&c, &m, 1100 + HORA, 60).is_ok(),
            "nunca más de un periodo"
        );
        // Una pasada que renueva pone la cuenta a cero.
        let mut m = Memoria {
            encolada: Some(2000),
            fallos: 4,
        };
        observar(&mut m, &copia(Some(10), Some((2100, true))));
        assert_eq!(m, Memoria::default());
        // Un fallo de una pasada a mano cuenta uno.
        let mut m = Memoria::default();
        observar(&mut m, &copia(Some(3000), Some((50, false))));
        assert_eq!(m.fallos, 1);
    }
}
