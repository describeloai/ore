-- ore-catalogo · esquema v2 (ADR 0079 E1a·5c·1): la puerta de los punteros sobre el catalogo.
-- Escribir con compare-and-set, la historia de una ref, la version y el ciclo de las ramas.
--
-- Por RUTA, como los ficheros del arbol (`datasets/<b>/<s>/<n>.json`, el de antes `datasets/<p>_<n>.json`):
-- quien llama resuelve el nombre en sus rutas (la de su sitio primero) y el catalogo se comporta como
-- un directorio de punteros. Asi la implementacion del catalogo es la de los ficheros por construccion,
-- y el andamio (que replica por ruta) no cambia.

CREATE INDEX commits_ref_id ON commits (ref, id);

-- El nombre en el lago por la metadata_location: lo que hay entre `/ore/v2/` y su `/metadata/`
-- (`ore_core::punteros::dataset_de_ubicacion`).
CREATE FUNCTION dataset_de_ubicacion(p_ml text) RETURNS text LANGUAGE sql IMMUTABLE AS $$
  SELECT nullif(substring(p_ml from '/ore/v2/(.*)/metadata/'), '')
$$;

-- El puntero que una ref ve en una ruta (la regla `manda`): lo propio de la rama (una lapida es
-- «no hay»), y si no tiene nada propio, lo de main al dia. `de`: `aqui` | `main`.
CREATE FUNCTION efectivo(p_ref text, p_activo text) RETURNS TABLE (de text, ml text, puntero jsonb)
LANGUAGE plpgsql STABLE AS $$
DECLARE r estado%ROWTYPE;
BEGIN
  IF p_ref <> 'main' THEN
    -- Una rama que no está no ve nada (en git, no se puede leer).
    IF NOT EXISTS (SELECT 1 FROM refs WHERE nombre = p_ref) THEN RETURN; END IF;
    SELECT * INTO r FROM estado WHERE ref = p_ref AND activo = p_activo;
    IF FOUND THEN
      IF NOT r.borrado THEN RETURN QUERY SELECT 'aqui'::text, r.metadata_location, r.puntero; END IF;
      RETURN;
    END IF;
  END IF;
  RETURN QUERY SELECT CASE WHEN p_ref = 'main' THEN 'aqui' ELSE 'main' END, e.metadata_location, e.puntero
    FROM estado e WHERE e.ref = 'main' AND e.activo = p_activo;
END $$;

-- Deja `p_activo` en la ref como `p_puntero` (null = que no este), con su commit.
CREATE FUNCTION poner_en(p_ref text, p_activo text, p_puntero jsonb, p_commit bigint) RETURNS void
LANGUAGE plpgsql AS $$
BEGIN
  IF p_puntero IS NOT NULL THEN
    INSERT INTO estado (ref, activo, metadata_location, puntero, escrito_por, borrado, commit_id)
    VALUES (p_ref, p_activo, p_puntero->>'metadata_location', p_puntero, p_puntero->>'escrito_por', false, p_commit)
    ON CONFLICT (ref, activo) DO UPDATE
      SET metadata_location = EXCLUDED.metadata_location, puntero = EXCLUDED.puntero,
          escrito_por = EXCLUDED.escrito_por, borrado = false, commit_id = EXCLUDED.commit_id,
          git_blob = NULL;
  ELSIF p_ref = 'main' THEN
    DELETE FROM estado WHERE ref = 'main' AND activo = p_activo;
  ELSIF EXISTS (SELECT 1 FROM estado WHERE ref = 'main' AND activo = p_activo) THEN
    -- Lo que main tiene y la rama quita: una lapida.
    INSERT INTO estado (ref, activo, borrado, commit_id) VALUES (p_ref, p_activo, true, p_commit)
    ON CONFLICT (ref, activo) DO UPDATE
      SET metadata_location = NULL, puntero = NULL, escrito_por = NULL, borrado = true,
          commit_id = EXCLUDED.commit_id, git_blob = NULL;
  ELSE
    DELETE FROM estado WHERE ref = p_ref AND activo = p_activo;
  END IF;
END $$;

-- **Escribir**: todos los cambios o ninguno, en una transaccion. Cada cambio es
--   {"rutas": [la de su sitio, la de antes...], "esperado": null | "nada" | {"ml": s},
--    "que": "retirar" | {"sustituir": {...}} | {"poner": {"campos": {...}, "quitar": [k...]}}}
-- Lo de hoy es lo de la primera ruta que exista; se escribe en la primera y se retiran las demas.
-- Un `esperado` que no se cumple: SQLSTATE OR409, con el indice del cambio y lo que hay en DETAIL.
-- La misma clave de idempotencia dos veces: el commit de la primera, sin tocar nada.
CREATE FUNCTION escribir(p_ref text, p_sujeto text, p_operacion text, p_idem text, p_cambios jsonb)
RETURNS bigint LANGUAGE plpgsql AS $$
DECLARE
  v_id bigint; c jsonb; i int; j int; rutas text[]; q text;
  prev_ml text; prev_p jsonb; hallado boolean; e jsonb; nuevo jsonb; ml text; d text;
BEGIN
  IF p_ref <> 'main' AND NOT EXISTS (SELECT 1 FROM refs WHERE nombre = p_ref) THEN
    RAISE EXCEPTION USING ERRCODE = 'OR404', MESSAGE = format('la rama `%s` no esta en el catalogo', p_ref);
  END IF;
  IF p_idem IS NOT NULL THEN
    PERFORM pg_advisory_xact_lock(hashtext('idem|' || p_idem));
    SELECT id INTO v_id FROM commits WHERE idempotencia = p_idem;
    IF FOUND THEN RETURN v_id; END IF;
  END IF;
  -- Los cerrojos de las rutas que se tocan, en orden (sin abrazos mortales); en una rama, tambien
  -- compartido el de main: lo heredado no cambia mientras se compara con el.
  FOR q IN SELECT DISTINCT x FROM jsonb_array_elements(p_cambios) cc,
                                  jsonb_array_elements_text(cc->'rutas') x ORDER BY 1 LOOP
    PERFORM pg_advisory_xact_lock(hashtext(p_ref || '|' || q));
    IF p_ref <> 'main' THEN PERFORM pg_advisory_xact_lock_shared(hashtext('main|' || q)); END IF;
  END LOOP;
  -- Primero se comprueba todo.
  FOR i IN 0 .. jsonb_array_length(p_cambios) - 1 LOOP
    c := p_cambios->i; e := c->'esperado';
    SELECT array_agg(x) INTO rutas FROM jsonb_array_elements_text(c->'rutas') x;
    hallado := false; prev_ml := NULL;
    FOREACH q IN ARRAY rutas LOOP
      SELECT f.ml INTO prev_ml FROM efectivo(p_ref, q) f;
      IF FOUND THEN hallado := true; EXIT; END IF;
    END LOOP;
    IF (e = '"nada"'::jsonb AND hallado)
       OR (jsonb_typeof(e) = 'object' AND (prev_ml IS DISTINCT FROM e->>'ml' OR NOT hallado)) THEN
      RAISE EXCEPTION USING ERRCODE = 'OR409', MESSAGE = 'el puntero cambio',
        DETAIL = jsonb_build_object('indice', i, 'hay', prev_ml)::text;
    END IF;
  END LOOP;
  INSERT INTO commits (ref, sujeto, operacion, idempotencia) VALUES (p_ref, p_sujeto, p_operacion, p_idem)
  RETURNING id INTO v_id;
  -- Luego se escribe todo.
  FOR i IN 0 .. jsonb_array_length(p_cambios) - 1 LOOP
    c := p_cambios->i;
    SELECT array_agg(x) INTO rutas FROM jsonb_array_elements_text(c->'rutas') x;
    hallado := false; prev_ml := NULL; prev_p := NULL;
    FOREACH q IN ARRAY rutas LOOP
      SELECT f.ml, f.puntero INTO prev_ml, prev_p FROM efectivo(p_ref, q) f;
      IF FOUND THEN hallado := true; EXIT; END IF;
    END LOOP;
    IF c->'que' = '"retirar"'::jsonb THEN
      nuevo := NULL;
    ELSIF c->'que' ? 'sustituir' THEN
      nuevo := c->'que'->'sustituir';
    ELSE
      nuevo := coalesce(prev_p, '{}'::jsonb);
      nuevo := nuevo - ARRAY(SELECT jsonb_array_elements_text(coalesce(c->'que'->'poner'->'quitar', '[]')));
      nuevo := nuevo || coalesce(c->'que'->'poner'->'campos', '{}'::jsonb);
    END IF;
    -- Su `dataset`, el de su metadata_location si se sabe: lo que reclama.
    IF nuevo IS NOT NULL THEN
      ml := nuevo->>'metadata_location'; d := dataset_de_ubicacion(ml);
      IF d IS NOT NULL THEN nuevo := nuevo || jsonb_build_object('dataset', d); END IF;
    END IF;
    FOR j IN 1 .. array_length(rutas, 1) LOOP
      q := rutas[j];
      IF j = 1 AND (nuevo IS NOT NULL OR hallado) THEN
        PERFORM poner_en(p_ref, q, nuevo, v_id);
        INSERT INTO historia VALUES (v_id, p_ref, q, prev_ml, nuevo->>'metadata_location',
                                     nuevo->>'escrito_por', p_operacion, nuevo);
        INSERT INTO outbox (commit_id, ref, activo, metadata_location)
        VALUES (v_id, p_ref, q, nuevo->>'metadata_location');
      ELSIF j > 1 AND EXISTS (SELECT 1 FROM efectivo(p_ref, q)) THEN
        -- La de antes, fuera.
        PERFORM poner_en(p_ref, q, NULL, v_id);
        INSERT INTO historia VALUES (v_id, p_ref, q, NULL, NULL, NULL, p_operacion, NULL);
        INSERT INTO outbox (commit_id, ref, activo, metadata_location) VALUES (v_id, p_ref, q, NULL);
      END IF;
    END LOOP;
  END LOOP;
  RETURN v_id;
END $$;

-- **La historia de una ruta en una ref**, de la mas nueva a la mas vieja: las versiones que dejaron
-- puntero. En main, la de main. En una rama que tiene algo propio en esas rutas, lo suyo y lo de main
-- hasta su punto de salida (lo que `git log` de la rama ve); si no tiene nada propio, lo hereda de
-- main al dia, y su historia es la de main.
CREATE FUNCTION historia_de(p_ref text, p_rutas text[], p_hasta int)
RETURNS TABLE (id text, cuando text, quien text, puntero jsonb) LANGUAGE sql STABLE AS $$
  WITH propia AS (
    SELECT p_ref <> 'main' AND EXISTS (SELECT 1 FROM estado WHERE ref = p_ref AND activo = ANY (p_rutas)) AS si
  ), punto AS (
    SELECT coalesce((SELECT punto_salida FROM refs WHERE nombre = p_ref), 0) AS p
  )
  SELECT coalesce(c.git_sha, 'c' || c.id),
         to_char(coalesce(c.git_cuando, c.cuando) AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"Z"'),
         coalesce(c.git_autor, c.sujeto), h.puntero
    FROM historia h JOIN commits c ON c.id = h.commit_id, propia, punto
   WHERE h.activo = ANY (p_rutas) AND h.puntero IS NOT NULL
     AND ((NOT propia.si AND h.ref = 'main')
          OR (propia.si AND (h.ref = p_ref OR (h.ref = 'main' AND h.commit_id <= punto.p))))
   ORDER BY h.commit_id DESC
   LIMIT p_hasta
$$;

-- **La version de lo que ve una ref**: cambia si y solo si cambia algo suyo o de main. Con `c`
-- delante: nunca se toma por un sha de git.
CREATE FUNCTION version_de(p_ref text) RETURNS text LANGUAGE sql STABLE AS $$
  SELECT 'c' || coalesce(max(id), 0) FROM commits WHERE ref IN (p_ref, 'main')
$$;

-- **Las ramas.** Nacer: sale de main de hoy (o hereda lo propio de otra rama). Traer main: su punto
-- de salida pasa a main de hoy. Borrar: `borrar_rama` (v1).
CREATE FUNCTION crear_rama(p_nombre text, p_desde text) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
  IF p_nombre = 'main' THEN RAISE EXCEPTION USING ERRCODE = 'OR422', MESSAGE = 'main no se crea'; END IF;
  INSERT INTO refs (nombre, base, punto_salida)
  VALUES (p_nombre, 'main', (SELECT coalesce(max(id), 0) FROM commits WHERE ref = 'main'))
  ON CONFLICT (nombre) DO NOTHING;
  IF FOUND AND p_desde <> 'main' THEN
    INSERT INTO estado (ref, activo, metadata_location, puntero, escrito_por, borrado, commit_id)
    SELECT p_nombre, activo, metadata_location, puntero, escrito_por, borrado, commit_id
      FROM estado WHERE ref = p_desde;
  END IF;
END $$;

CREATE FUNCTION traer_main(p_nombre text) RETURNS void LANGUAGE sql AS $$
  UPDATE refs SET punto_salida = (SELECT coalesce(max(id), 0) FROM commits WHERE ref = 'main')
   WHERE nombre = p_nombre AND nombre <> 'main'
$$;
