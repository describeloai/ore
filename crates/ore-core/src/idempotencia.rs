//! **La `Idempotency-Key`** de la spec REST de Iceberg: un UUID por operación
//! lógica, que el cliente reutiliza **sólo** en los reintentos de esa
//! operación. Quien la recibe (ore-serve, en la cabecera) y quien la coteja
//! (`ore datasets --commit --idempotencia`) comparten aquí qué forma tiene,
//! para que lo que llega de la red no viaje a la línea de órdenes sin mirarlo.
//! El diseño: ADR 0031, «La idempotencia, revisada».

/// Una `Idempotency-Key` con la forma de la spec: un UUID en texto, 36
/// caracteres (`8-4-4-4-12` hexadecimales). La spec pide UUIDv7, pero
/// cualquier UUID identifica una operación igual de bien: no se exige la versión.
pub fn bien_formada(c: &str) -> bool {
    c.len() == 36
        && c.char_indices().all(|(i, ch)| match i {
            8 | 13 | 18 | 23 => ch == '-',
            _ => ch.is_ascii_hexdigit(),
        })
}

#[cfg(test)]
mod tests {
    use super::bien_formada as ok;

    #[test]
    fn una_idempotency_key_es_un_uuid() {
        assert!(ok("0192f0c4-7b2a-7c3e-9d41-5a6b7c8d9e0f"));
        assert!(ok("0192F0C4-7B2A-7C3E-9D41-5A6B7C8D9E0F"));
        assert!(!ok("op-1"));
        assert!(!ok("0192f0c47b2a7c3e9d415a6b7c8d9e0f"));
        assert!(!ok("0192f0c4-7b2a-7c3e-9d41-5a6b7c8d9e0g"));
        assert!(!ok("0192f0c4-7b2a-7c3e-9d41-5a6b7c8d9e0f0"));
        // lo que no puede llegar a la línea de órdenes
        assert!(!ok("--sujeto=persona:ana-00000000000000000"));
        assert!(!ok(""));
    }
}
