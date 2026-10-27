//! De YAML a un árbol con posiciones y estilo de escalar.
//!
//! Se construye sobre la capa de **eventos** de `saphyr-parser`, no sobre su
//! API de árbol, y la razón está en `docs/decisions/0001-parser-de-yaml.md`:
//!
//! > No queremos un deserializador. Queremos un front-end de compilador.
//!
//! Lo que un deserializador tira es exactamente lo que necesitamos. El **estilo**
//! de un escalar decide si `OOS6003` se dispara —`68400.50` sin comillas es una
//! pérdida de precisión, entrecomillado es correcto— y la **posición** decide si
//! el error sirve para algo.

use crate::diag::Pos;
use saphyr_parser::{Event, Parser, ScalarStyle, Span, SpannedEventReceiver};

/// Cómo venía escrito un escalar en el origen.
///
/// No es cosmética: `Plain` frente a entrecomillado es la diferencia entre un
/// número y una cadena, y de ahí depende `OOS6003`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// Sin comillas. Sujeto a resolución implícita de tipo.
    Plain,
    /// `'…'` o `"…"`. Siempre una cadena, sin ambigüedad.
    Quoted,
    /// `|` o `>`. Siempre una cadena.
    Block,
}

#[derive(Debug, Clone)]
pub enum Node {
    /// El texto **crudo**, tal como venía. Nunca se convierte a número: convertir
    /// es perder, y `68400.50` no tiene representación exacta en binario.
    Scalar {
        raw: String,
        style: Style,
        pos: Pos,
    },
    Mapping {
        entries: Vec<(Node, Node)>,
        pos: Pos,
    },
    Sequence {
        items: Vec<Node>,
        pos: Pos,
    },
}

impl Node {
    pub fn pos(&self) -> Pos {
        match self {
            Node::Scalar { pos, .. } | Node::Mapping { pos, .. } | Node::Sequence { pos, .. } => {
                *pos
            }
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Node::Scalar { raw, .. } => Some(raw),
            _ => None,
        }
    }

    /// Busca una clave en un mapa. Devuelve la clave y el valor: la posición de
    /// la **clave** es la que quiere ver quien lee el error.
    pub fn get(&self, key: &str) -> Option<(&Node, &Node)> {
        match self {
            Node::Mapping { entries, .. } => entries
                .iter()
                .find(|(k, _)| k.as_str() == Some(key))
                .map(|(k, v)| (k, v)),
            _ => None,
        }
    }

    pub fn entries(&self) -> &[(Node, Node)] {
        match self {
            Node::Mapping { entries, .. } => entries,
            _ => &[],
        }
    }

    pub fn items(&self) -> &[Node] {
        match self {
            Node::Sequence { items, .. } => items,
            _ => &[],
        }
    }
}

/// Qué falló al analizar, con dónde.
#[derive(Debug)]
pub struct ParseError {
    pub message: String,
    pub pos: Pos,
}

// ── Construcción del árbol ──────────────────────────────────────────────────
//
// La capa de eventos no construye el mapa: lo construimos aquí. Es más trabajo
// y compra dos cosas que una librería no daría — detectar claves duplicadas con
// LAS DOS posiciones, y aplicar la política de anclas de la especificación en
// lugar de heredar la de otro.

enum Frame {
    Map {
        entries: Vec<(Node, Node)>,
        pending_key: Option<Node>,
        pos: Pos,
    },
    Seq {
        items: Vec<Node>,
        pos: Pos,
    },
}

struct Builder {
    stack: Vec<Frame>,
    /// Un fichero es un FLUJO y un flujo tiene cero o más documentos
    /// (`90-canonical-form` §5.3). Con la pila vacía, cada valor que llega es la
    /// raíz de uno nuevo.
    roots: Vec<Node>,
    error: Option<ParseError>,
    depth: usize,
}

const MAX_DEPTH: usize = 128;

fn pos_of(span: Span) -> Pos {
    Pos {
        line: span.start.line(),
        col: span.start.col() + 1,
    }
}

impl Builder {
    fn push_value(&mut self, node: Node) {
        if self.error.is_some() {
            return;
        }
        match self.stack.last_mut() {
            None => self.roots.push(node),
            Some(Frame::Seq { items, .. }) => items.push(node),
            Some(Frame::Map {
                entries,
                pending_key,
                ..
            }) => match pending_key.take() {
                None => *pending_key = Some(node),
                Some(key) => {
                    // Claves duplicadas: dos verdades en un documento. Se rechaza
                    // señalando ambas, que es lo que el parser no podría hacer
                    // por nosotros.
                    if let Some(name) = key.as_str()
                        && let Some((prev, _)) =
                            entries.iter().find(|(k, _)| k.as_str() == Some(name))
                    {
                        self.error = Some(ParseError {
                            message: format!(
                                "clave `{name}` declarada dos veces; la primera en la línea {}",
                                prev.pos().line
                            ),
                            pos: key.pos(),
                        });
                        return;
                    }
                    entries.push((key, node));
                }
            },
        }
    }
}

impl SpannedEventReceiver<'_> for Builder {
    fn on_event(&mut self, ev: Event<'_>, span: Span) {
        if self.error.is_some() {
            return;
        }
        let pos = pos_of(span);
        match ev {
            Event::Scalar(val, style, _anchor, _tag) => {
                let style = match style {
                    ScalarStyle::Plain => Style::Plain,
                    ScalarStyle::SingleQuoted | ScalarStyle::DoubleQuoted => Style::Quoted,
                    _ => Style::Block,
                };
                self.push_value(Node::Scalar {
                    raw: val.into_owned(),
                    style,
                    pos,
                });
            }
            Event::MappingStart(..) => {
                self.depth += 1;
                if self.depth > MAX_DEPTH {
                    self.error = Some(ParseError {
                        message: "anidamiento excesivo".into(),
                        pos,
                    });
                    return;
                }
                self.stack.push(Frame::Map {
                    entries: Vec::new(),
                    pending_key: None,
                    pos,
                });
            }
            Event::SequenceStart(..) => {
                self.depth += 1;
                if self.depth > MAX_DEPTH {
                    self.error = Some(ParseError {
                        message: "anidamiento excesivo".into(),
                        pos,
                    });
                    return;
                }
                self.stack.push(Frame::Seq {
                    items: Vec::new(),
                    pos,
                });
            }
            Event::MappingEnd | Event::SequenceEnd => {
                self.depth = self.depth.saturating_sub(1);
                let node = match self.stack.pop() {
                    Some(Frame::Map { entries, pos, .. }) => Node::Mapping { entries, pos },
                    Some(Frame::Seq { items, pos }) => Node::Sequence { items, pos },
                    None => return,
                };
                self.push_value(node);
            }
            // Un alias es una ambigüedad que la forma canónica no debe heredar
            // (90-canonical-form §4). Se rechaza en el análisis, donde hay
            // posición para decirlo.
            Event::Alias(_) => {
                self.error = Some(ParseError {
                    message: "alias YAML (`*`) no admitido: la forma canónica no puede \
                              heredar la ambigüedad de un ancla"
                        .into(),
                    pos,
                });
            }
            _ => {}
        }
    }
}

/// Analiza un FLUJO YAML y devuelve la raíz de cada documento.
///
/// `90-canonical-form` §5.3: un motor conforme DEBE leer todos los documentos de
/// un fichero. Hasta que esa frase se escribió, aquí se pasaba `multi = false` y
/// el analizador rompía el bucle tras el primero — un `Binding` puesto detrás de
/// un `---` no existía, y nada lo decía.
pub fn parse_stream(text: &str) -> Result<Vec<Node>, ParseError> {
    let mut b = Builder {
        stack: Vec::new(),
        roots: Vec::new(),
        error: None,
        depth: 0,
    };
    if let Err(e) = Parser::new_from_str(text).load(&mut b, true) {
        let m = e.marker();
        return Err(ParseError {
            message: e.info().to_string(),
            pos: Pos {
                line: m.line(),
                col: m.col() + 1,
            },
        });
    }
    if let Some(e) = b.error {
        return Err(e);
    }
    Ok(b.roots)
}

/// El primer documento del flujo. Para los sitios donde se espera uno solo: el
/// manifiesto que `source add` edita, un contrato ajeno que se importa.
pub fn parse(text: &str) -> Result<Node, ParseError> {
    parse_stream(text)?.into_iter().next().ok_or(ParseError {
        message: "documento vacío".into(),
        pos: Pos { line: 1, col: 1 },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conserva_el_texto_crudo_y_el_estilo() {
        let n = parse("plano: 68400.50\ncomillas: \"68400.50\"\n").unwrap();
        let (_, plano) = n.get("plano").unwrap();
        let (_, comillas) = n.get("comillas").unwrap();
        // El mismo texto; lo que los distingue es el estilo, y de eso depende OOS6003.
        assert_eq!(plano.as_str(), Some("68400.50"));
        assert_eq!(comillas.as_str(), Some("68400.50"));
        assert!(matches!(
            plano,
            Node::Scalar {
                style: Style::Plain,
                ..
            }
        ));
        assert!(matches!(
            comillas,
            Node::Scalar {
                style: Style::Quoted,
                ..
            }
        ));
    }

    /// El *problema de Noruega*: en YAML 1.1 esto sería `false`.
    #[test]
    fn no_resuelve_tipos_implicitos() {
        let n = parse("consent: no\ncountry: NO\n").unwrap();
        assert_eq!(n.get("consent").unwrap().1.as_str(), Some("no"));
        assert_eq!(n.get("country").unwrap().1.as_str(), Some("NO"));
    }

    #[test]
    fn rechaza_claves_duplicadas_senalando_ambas() {
        let e = parse("name: primera\nname: segunda\n").unwrap_err();
        assert!(e.message.contains("dos veces"), "{}", e.message);
        assert!(e.message.contains("línea 1"), "{}", e.message);
        assert_eq!(e.pos.line, 2);
    }

    #[test]
    fn rechaza_alias() {
        let e = parse("base: &b hola\nuno: *b\n").unwrap_err();
        assert!(e.message.contains("alias"), "{}", e.message);
    }

    #[test]
    fn registra_posiciones() {
        let n = parse("a: 1\nb: 2\nc: 3\n").unwrap();
        assert_eq!(n.get("c").unwrap().0.pos().line, 3);
    }
}
