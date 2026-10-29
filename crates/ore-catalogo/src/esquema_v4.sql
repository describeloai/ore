-- ore-catalogo · esquema v4 (ADR 0079 E1a·5e): la cola del documentador.
--
-- Una escritura mixta (nace una tabla, cambian sus columnas o su linaje) escribe el puntero aquí y
-- el `Dataset` escrito por máquina en git después. Antes de intentar git, ore-serve encola el
-- documento; si git va bien, lo quita; si no, se queda y el documentador lo lleva a git en su
-- pasada. `antes` es el fichero que había en git al calcularlo (null: no existía): el documentador
-- no pisa uno que alguien cambió entretanto.

CREATE TABLE documentos (
  ref     text        NOT NULL,
  ruta    text        NOT NULL,
  texto   text        NOT NULL,
  antes   text,
  n       bigserial   NOT NULL,
  cuando  timestamptz NOT NULL DEFAULT clock_timestamp(),
  PRIMARY KEY (ref, ruta)
);

-- **Encolar** los documentos de una escritura en `p_ref`: `[{ruta, texto, antes}]`. Uno que ya
-- estaba en la cola para esa ruta se pisa (manda el último) y toma un `n` nuevo. Devuelve
-- `[{ruta, n}]`, lo que hay que pasar a `quitar_documentos`.
CREATE FUNCTION encolar_documentos(p_ref text, p_docs jsonb) RETURNS jsonb LANGUAGE sql AS $$
  WITH e AS (
    INSERT INTO documentos (ref, ruta, texto, antes)
    SELECT p_ref, d->>'ruta', d->>'texto', d->>'antes' FROM jsonb_array_elements(p_docs) d
    ON CONFLICT (ref, ruta) DO UPDATE
      SET texto = EXCLUDED.texto, antes = EXCLUDED.antes,
          n = nextval(pg_get_serial_sequence('documentos', 'n')), cuando = clock_timestamp()
    RETURNING ruta, n
  )
  SELECT coalesce(jsonb_agg(jsonb_build_object('ruta', ruta, 'n', n) ORDER BY ruta), '[]') FROM e
$$;

-- **Quitar** de la cola lo que ya está en git: `[{ruta, n}]`. Sólo si sigue siendo ese `n`: uno
-- más nuevo que llegó entretanto se queda. Devuelve cuántos quitó.
CREATE FUNCTION quitar_documentos(p_ref text, p_hechos jsonb) RETURNS int LANGUAGE sql AS $$
  WITH q AS (
    DELETE FROM documentos c
     USING jsonb_array_elements(p_hechos) h
     WHERE c.ref = p_ref AND c.ruta = h->>'ruta' AND c.n = (h->>'n')::bigint
    RETURNING 1
  )
  SELECT count(*)::int FROM q
$$;
