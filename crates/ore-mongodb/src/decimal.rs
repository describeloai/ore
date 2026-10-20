//! **`Decimal128` de BSON a un entero escalado**, sin pasar por `f64`.
//!
//! BSON guarda un decimal de 34 cifras con exponente libre. Arrow lo guarda como
//! un `i128` y una escala fija. Entre los dos no hay redondeo: o el valor cabe
//! exacto en `Decimal<p, s>` o es un error que lo dice (0032 §4, y
//! `docs/federation.md` §1.3: «un valor que no cabe en su tipo es un error que
//! nombra columna y fila, no un nulo»). El mismo criterio que `BIGNUMERIC` en
//! BigQuery (0042): lo que no cabe, texto exacto.
//!
//! Se lee de la forma textual de `bson::Decimal128` (`"12345.678"`, `"1.23E+5"`,
//! `"NaN"`…), que es la que el propio crate canoniza.

use mongodb::bson::Decimal128;

/// Un decimal finito: `±digitos × 10^exp`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dec {
    pub negativo: bool,
    /// Sin ceros a la izquierda; `"0"` para el cero.
    pub digitos: String,
    pub exp: i32,
}

/// Lo que un `Decimal128` puede ser y no es un número.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Especial {
    NaN,
    Infinito,
}

pub fn de(d: &Decimal128) -> Result<Dec, Especial> {
    analizar(&d.to_string())
}

/// Analiza la forma textual. Separado de [`de`] para probarlo sin BSON.
pub fn analizar(texto: &str) -> Result<Dec, Especial> {
    let t = texto.trim();
    let (negativo, t) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let bajo = t.to_ascii_lowercase();
    if bajo.starts_with("nan") || bajo.starts_with("snan") {
        return Err(Especial::NaN);
    }
    if bajo.starts_with("inf") {
        return Err(Especial::Infinito);
    }
    let (mantisa, exp10) = match t.find(['E', 'e']) {
        Some(i) => (&t[..i], t[i + 1..].parse::<i32>().unwrap_or(0)),
        None => (t, 0),
    };
    let (ent, frac) = mantisa.split_once('.').unwrap_or((mantisa, ""));
    let mut digitos = format!("{ent}{frac}");
    let exp = exp10 - frac.len() as i32;
    let sin_ceros = digitos.trim_start_matches('0').to_string();
    digitos = if sin_ceros.is_empty() {
        "0".into()
    } else {
        sin_ceros
    };
    Ok(Dec {
        negativo,
        digitos,
        exp,
    })
}

impl Dec {
    /// La escala mínima que representa el valor exacto (sin ceros de relleno a
    /// la derecha) y cuántas cifras tiene su parte entera.
    pub fn forma(&self) -> (u32, u32) {
        if self.digitos == "0" {
            return (0, 1);
        }
        let ceros = self.digitos.len() - self.digitos.trim_end_matches('0').len();
        let n = (self.digitos.len() - ceros) as i32;
        let exp = self.exp + ceros as i32;
        // valor = (n cifras) × 10^exp
        let escala = (-exp).max(0) as u32;
        let enteras = (n + exp).max(1) as u32;
        (escala, enteras)
    }

    /// El valor como `i128` con `escala` decimales y `precision` cifras como
    /// mucho. `Err` dice por qué no cabe.
    pub fn a_i128(&self, escala: u8, precision: u8) -> Result<i128, String> {
        let k = self.exp + i32::from(escala);
        let mut digitos = self.digitos.clone();
        if k >= 0 {
            if digitos != "0" {
                digitos.push_str(&"0".repeat(k as usize));
            }
        } else {
            let quitar = (-k) as usize;
            if quitar > digitos.len()
                || !digitos[digitos.len() - quitar..].bytes().all(|b| b == b'0')
            {
                return Err(format!(
                    "{} tiene más decimales de los que caben en la escala {escala}",
                    self.texto()
                ));
            }
            digitos.truncate(digitos.len() - quitar);
            if digitos.is_empty() {
                digitos.push('0');
            }
        }
        let digitos = digitos.trim_start_matches('0');
        let digitos = if digitos.is_empty() { "0" } else { digitos };
        if digitos.len() > usize::from(precision) {
            return Err(format!(
                "{} no cabe en Decimal({precision}, {escala})",
                self.texto()
            ));
        }
        let n: i128 = digitos
            .parse()
            .map_err(|_| format!("{} no cabe en Decimal({precision}, {escala})", self.texto()))?;
        Ok(if self.negativo { -n } else { n })
    }

    /// El texto canónico del valor, sin notación científica.
    pub fn texto(&self) -> String {
        let signo = if self.negativo && self.digitos != "0" {
            "-"
        } else {
            ""
        };
        if self.exp >= 0 {
            let ceros = if self.digitos == "0" {
                0
            } else {
                self.exp as usize
            };
            return format!("{signo}{}{}", self.digitos, "0".repeat(ceros));
        }
        let dec = (-self.exp) as usize;
        let mut d = self.digitos.clone();
        if d.len() <= dec {
            d = format!("{}{d}", "0".repeat(dec - d.len() + 1));
        }
        let (e, f) = d.split_at(d.len() - dec);
        format!("{signo}{e}.{f}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(t: &str) -> Dec {
        analizar(t).unwrap()
    }

    #[test]
    fn lee_las_formas_que_emite_bson() {
        assert_eq!(
            d("12345.678"),
            Dec {
                negativo: false,
                digitos: "12345678".into(),
                exp: -3
            }
        );
        assert_eq!(
            d("-0.005"),
            Dec {
                negativo: true,
                digitos: "5".into(),
                exp: -3
            }
        );
        assert_eq!(
            d("1.23E+5"),
            Dec {
                negativo: false,
                digitos: "123".into(),
                exp: 3
            }
        );
        assert_eq!(
            d("1E-7"),
            Dec {
                negativo: false,
                digitos: "1".into(),
                exp: -7
            }
        );
        assert_eq!(
            d("0"),
            Dec {
                negativo: false,
                digitos: "0".into(),
                exp: 0
            }
        );
        assert_eq!(d("0E+3").digitos, "0");
        assert_eq!(analizar("NaN"), Err(Especial::NaN));
        assert_eq!(analizar("-Infinity"), Err(Especial::Infinito));
    }

    /// Los valores de `ore_prueba.tipos` (X0).
    #[test]
    fn la_forma_dice_la_escala_y_las_cifras_enteras() {
        assert_eq!(d("12345678901234567890123456789.123").forma(), (3, 29));
        assert_eq!(d("1234567890123456789012345678901234").forma(), (0, 34));
        assert_eq!(d("0.005").forma(), (3, 1));
        assert_eq!(d("10.50").forma(), (1, 2));
        assert_eq!(d("100").forma(), (0, 3));
        assert_eq!(d("1E+3").forma(), (0, 4));
    }

    #[test]
    fn cabe_exacto_o_se_dice() {
        // 29 enteras y 3 decimales caben en (38, 3): son 32 cifras.
        let n = d("12345678901234567890123456789.123")
            .a_i128(3, 38)
            .unwrap();
        assert_eq!(n, 12345678901234567890123456789123);
        // Una escala mayor añade ceros, sin cambiar el valor.
        assert_eq!(d("0.005").a_i128(5, 38).unwrap(), 500);
        assert_eq!(d("-0.005").a_i128(3, 38).unwrap(), -5);
        assert_eq!(d("0").a_i128(2, 38).unwrap(), 0);
        // 34 enteras con escala 5 = 39 cifras: no caben en 38.
        let e = d("1234567890123456789012345678901234")
            .a_i128(5, 38)
            .unwrap_err();
        assert!(e.contains("no cabe en Decimal(38, 5)"), "{e}");
        // 0.005 no cabe en escala 2 sin perder el 5: no se redondea.
        let e = d("0.005").a_i128(2, 38).unwrap_err();
        assert!(e.contains("más decimales"), "{e}");
        // Pero 0.50 sí cabe en escala 1: el cero de relleno se puede quitar.
        assert_eq!(d("0.50").a_i128(1, 38).unwrap(), 5);
    }

    #[test]
    fn el_texto_no_usa_notacion_cientifica() {
        assert_eq!(d("1.23E+5").texto(), "123000");
        assert_eq!(d("1E-7").texto(), "0.0000001");
        assert_eq!(d("-12345.678").texto(), "-12345.678");
        assert_eq!(d("0").texto(), "0");
    }
}
