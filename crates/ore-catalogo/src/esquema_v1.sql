-- ore-catalogo · esquema v1 (ADR 0079 E1a·4): el estado vivo de los punteros, sus ramas y su
-- historia, en el esquema del inquilino (`search_path`). Lo que escribe con CAS y fusiona (E1a·2–3)
-- entra en la v2, con el corte (E1a·5).

CREATE TABLE esquema_version (
  version  int PRIMARY KEY,
  aplicado timestamptz NOT NULL DEFAULT now()
);

-- Un commit del catalogo. Los replicados de git llevan su sha: replicar es idempotente.
CREATE TABLE commits (
  id            bigserial PRIMARY KEY,
  ref           text        NOT NULL,
  cuando        timestamptz NOT NULL DEFAULT clock_timestamp(),
  sujeto        text,
  operacion     text,
  idempotencia  text UNIQUE,
  git_sha       text UNIQUE,
  git_autor     text,
  git_cuando    timestamptz
);

-- Una rama: de que sale y en que commit del catalogo esta su punto de salida.
-- git_sha y git_main: con que punta de la rama y de main se calculo (el andamio).
CREATE TABLE refs (
  nombre        text PRIMARY KEY,
  base          text,
  punto_salida  bigint,
  protegida     boolean NOT NULL DEFAULT false,
  git_sha       text,
  git_main      text
);
INSERT INTO refs (nombre, base, punto_salida) VALUES ('main', NULL, 0);

-- El estado: en main, una fila por puntero vivo. En una rama, SOLO lo propio: lo que cambio o creo
-- (borrado = false) y lo que borro (borrado = true, una lapida). Lo demas se lee de main al dia.
-- El activo es la ruta del puntero en el arbol (`datasets/<b>/<s>/<n>.json`, `copias/...`).
CREATE TABLE estado (
  ref               text    NOT NULL,
  activo            text    NOT NULL,
  metadata_location text,
  puntero           jsonb,                 -- el puntero entero; null en una lapida
  escrito_por       text,                  -- null = con receta (lo mantiene una definicion)
  borrado           boolean NOT NULL DEFAULT false,
  commit_id         bigint,
  git_blob          text,                  -- el andamio: de que blob se copio
  PRIMARY KEY (ref, activo)
) WITH (fillfactor = 70);

-- La historia: cada movimiento de un puntero.
CREATE TABLE historia (
  commit_id    bigint  NOT NULL,
  ref          text    NOT NULL,
  activo       text    NOT NULL,
  de           text,
  a            text,                       -- null = borrado
  escrito_por  text,
  operacion    text,
  puntero      jsonb
);
CREATE INDEX ON historia (ref, activo, commit_id);

-- El evento sale de la transaccion (ADR 0079 §5): una fila por puntero movido, que lee la
-- decodificacion logica (E1a·1b) por la publicacion del esquema (`eventos_t_<org>`, FOR TABLES IN
-- SCHEMA), que crea el alta: crear una publicacion no es cosa del papel del inquilino.
CREATE TABLE outbox (
  id                bigserial PRIMARY KEY,
  commit_id         bigint      NOT NULL,
  ref               text        NOT NULL,
  activo            text        NOT NULL,
  metadata_location text,
  cuando            timestamptz NOT NULL DEFAULT clock_timestamp()
);

-- El andamio: cada commit de primer padre de main, y el commit del catalogo vigente en el. Da el
-- punto de salida de una rama a partir de su merge-base. Se va con el andamio.
CREATE TABLE andamio_mapa (
  git_sha   text   PRIMARY KEY,
  commit_id bigint NOT NULL
);

-- Lo que main tenia de un activo en un commit dado (para el punto de salida de una rama).
CREATE FUNCTION main_en(p_activo text, p_hasta bigint) RETURNS text LANGUAGE sql STABLE AS $$
  SELECT a FROM historia WHERE ref = 'main' AND activo = p_activo AND commit_id <= p_hasta
   ORDER BY commit_id DESC LIMIT 1
$$;

-- Todo lo que una ref ve: lo propio, y lo de main que no tapa (la regla `manda`).
CREATE FUNCTION vista(p_ref text)
RETURNS TABLE (activo text, de text, ml text, puntero jsonb, git_blob text) LANGUAGE sql STABLE AS $$
  SELECT e.activo, 'rama', e.metadata_location, e.puntero, e.git_blob FROM estado e
   WHERE p_ref <> 'main' AND e.ref = p_ref AND NOT e.borrado
  UNION ALL
  SELECT m.activo, 'main', m.metadata_location, m.puntero, m.git_blob FROM estado m
   WHERE m.ref = 'main'
     AND (p_ref = 'main' OR NOT EXISTS (SELECT 1 FROM estado r WHERE r.ref = p_ref AND r.activo = m.activo))
$$;

-- Replicar un commit de main: sus punteros cambiados (texto null = borrado), en una transaccion,
-- con historia y outbox. Idempotente por el sha de git.
CREATE FUNCTION replicar_main(p_sha text, p_autor text, p_cuando timestamptz,
                              p_activos text[], p_blobs text[], p_textos text[]) RETURNS bigint
LANGUAGE plpgsql AS $$
DECLARE v_id bigint; i int; v_p jsonb; v_antes text;
BEGIN
  SELECT id INTO v_id FROM commits WHERE git_sha = p_sha;
  IF FOUND THEN RETURN v_id; END IF;
  INSERT INTO commits (ref, sujeto, operacion, git_sha, git_autor, git_cuando)
  VALUES ('main', p_autor, 'replicar', p_sha, p_autor, p_cuando) RETURNING id INTO v_id;
  FOR i IN 1 .. coalesce(array_length(p_activos, 1), 0) LOOP
    SELECT metadata_location INTO v_antes FROM estado WHERE ref = 'main' AND activo = p_activos[i];
    IF p_textos[i] IS NULL THEN
      v_p := NULL;
      DELETE FROM estado WHERE ref = 'main' AND activo = p_activos[i];
    ELSE
      v_p := p_textos[i]::jsonb;
      INSERT INTO estado (ref, activo, metadata_location, puntero, escrito_por, borrado, commit_id, git_blob)
      VALUES ('main', p_activos[i], v_p->>'metadata_location', v_p, v_p->>'escrito_por', false, v_id, p_blobs[i])
      ON CONFLICT (ref, activo) DO UPDATE
        SET metadata_location = EXCLUDED.metadata_location, puntero = EXCLUDED.puntero,
            escrito_por = EXCLUDED.escrito_por, borrado = false, commit_id = EXCLUDED.commit_id,
            git_blob = EXCLUDED.git_blob;
    END IF;
    INSERT INTO historia VALUES (v_id, 'main', p_activos[i], v_antes, v_p->>'metadata_location',
                                 v_p->>'escrito_por', 'replicar', v_p);
    INSERT INTO outbox (commit_id, ref, activo, metadata_location)
    VALUES (v_id, 'main', p_activos[i], v_p->>'metadata_location');
  END LOOP;
  RETURN v_id;
END $$;

-- Replicar una rama: lo propio entero (texto null = lapida) sustituye a lo que tenia, en una
-- transaccion; el punto de salida es el commit del catalogo de su merge-base con main.
CREATE FUNCTION replicar_rama(p_ref text, p_sha text, p_main text, p_punto bigint,
                              p_activos text[], p_blobs text[], p_textos text[]) RETURNS void
LANGUAGE plpgsql AS $$
BEGIN
  IF p_ref = 'main' THEN RAISE EXCEPTION 'main no se replica como rama'; END IF;
  INSERT INTO refs (nombre, base, punto_salida, git_sha, git_main) VALUES (p_ref, 'main', p_punto, p_sha, p_main)
  ON CONFLICT (nombre) DO UPDATE
    SET punto_salida = EXCLUDED.punto_salida, git_sha = EXCLUDED.git_sha, git_main = EXCLUDED.git_main;
  DELETE FROM estado WHERE ref = p_ref;
  INSERT INTO estado (ref, activo, metadata_location, puntero, escrito_por, borrado, git_blob)
  SELECT p_ref, u.a, (u.t::jsonb)->>'metadata_location', u.t::jsonb, (u.t::jsonb)->>'escrito_por',
         u.t IS NULL, u.b
    FROM unnest(p_activos, p_blobs, p_textos) AS u(a, b, t);
END $$;

-- Borrar una rama: deja de reclamar lo suyo.
CREATE FUNCTION borrar_rama(p_nombre text) RETURNS void LANGUAGE sql AS $$
  DELETE FROM estado WHERE ref = p_nombre;
  DELETE FROM refs WHERE nombre = p_nombre;
$$;
