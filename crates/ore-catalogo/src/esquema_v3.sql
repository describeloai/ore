-- ore-catalogo · esquema v3 (ADR 0079 E1a·5c·5c·1): las escrituras de definicion. Los cerrojos de
-- una fusion, el plan de fusion (las reglas de hoy, `git::fusionar_puntero`, con el puntero ENTERO) y
-- su aplicacion ligada al commit de git (`idempotencia = git:<sha>`), tambien tardia.
--
-- Por RUTA, como la v2: quien llama resuelve los nombres en sus rutas (tambien lo elegido).

-- Lo que devolvio aplicar un plan: la segunda vez se devuelve lo mismo (y el reconciliador lo lee).
ALTER TABLE commits ADD COLUMN resultado jsonb;

-- **Los cerrojos de una fusion en curso**: una ruta en una ref, de un dueno, hasta que caduca. Uno
-- caducado no cuenta (y el siguiente `cerrar` lo pisa): una caida no deja nada cerrado para siempre.
CREATE TABLE cerrojos (
  ref     text        NOT NULL,
  activo  text        NOT NULL,
  dueno   text        NOT NULL,
  caduca  timestamptz NOT NULL,
  PRIMARY KEY (ref, activo)
);
CREATE INDEX ON cerrojos (dueno);

-- Los cerrojos de transaccion de `escribir` (v2), en SU orden: por ruta y, en cada ruta, las ramas
-- antes que main (`escribir` en una rama toma la suya y luego, compartido, el de main). Asi `cerrar`
-- y `aplicar_plan` esperan a quien escribe ahora, y quien llega despues ve lo que dejaron.
CREATE FUNCTION tomar_rutas(p_refs text[], p_rutas text[]) RETURNS void LANGUAGE plpgsql AS $$
DECLARE q text; r text;
BEGIN
  FOR q IN SELECT DISTINCT x FROM unnest(p_rutas) x ORDER BY 1 LOOP
    FOR r IN SELECT y FROM (SELECT DISTINCT y FROM unnest(p_refs) y) u ORDER BY (y = 'main'), y LOOP
      PERFORM pg_advisory_xact_lock(hashtext(r || '|' || q));
    END LOOP;
  END LOOP;
END $$;

-- **Cerrar** `p_rutas` en cada una de `p_refs` para `p_dueno`, durante `p_plazo`. El mismo dueno
-- renueva. Si otro dueno vivo tiene alguna: SQLSTATE OR429 (nada se cierra), con la ruta y su dueno
-- en DETAIL.
CREATE FUNCTION cerrar(p_refs text[], p_rutas text[], p_dueno text, p_plazo interval) RETURNS void
LANGUAGE plpgsql AS $$
DECLARE o cerrojos%ROWTYPE;
BEGIN
  PERFORM tomar_rutas(p_refs, p_rutas);
  SELECT * INTO o FROM cerrojos
   WHERE ref = ANY (p_refs) AND activo = ANY (p_rutas) AND dueno <> p_dueno AND caduca > clock_timestamp()
   ORDER BY activo, ref LIMIT 1;
  IF FOUND THEN
    RAISE EXCEPTION USING ERRCODE = 'OR429', MESSAGE = 'otra fusion tiene cerrada parte del alcance',
      DETAIL = jsonb_build_object('ref', o.ref, 'ruta', o.activo, 'dueno', o.dueno)::text;
  END IF;
  INSERT INTO cerrojos (ref, activo, dueno, caduca)
  SELECT r, q, p_dueno, clock_timestamp() + p_plazo
    FROM (SELECT DISTINCT r FROM unnest(p_refs) r) rs, (SELECT DISTINCT q FROM unnest(p_rutas) q) qs
  ON CONFLICT (ref, activo) DO UPDATE SET dueno = EXCLUDED.dueno, caduca = EXCLUDED.caduca;
END $$;

-- **Soltar** todo lo de un dueno. Devuelve cuantos solto.
CREATE FUNCTION soltar(p_dueno text) RETURNS int LANGUAGE sql AS $$
  WITH s AS (DELETE FROM cerrojos WHERE dueno = p_dueno RETURNING 1) SELECT count(*)::int FROM s
$$;

-- **Escribir** (v2) contesta OR429 si una ruta que tocaria esta cerrada en esa ref por un cerrojo
-- vivo (de cualquier dueno: lo de la fusion lo escribe `aplicar_plan`, no `escribir`). DETAIL:
-- {"indice": el primer cambio cerrado, "ruta", "dueno"}. Se mira tras la idempotencia (una
-- repeticion de algo ya escrito devuelve su commit) y con los cerrojos de transaccion de la v2
-- tomados: `cerrar` espera a quien escribe ahora, y quien escribe despues ve el cerrojo.
ALTER FUNCTION escribir(text, text, text, text, jsonb) RENAME TO escribir_sin_cerrojos;

CREATE FUNCTION escribir(p_ref text, p_sujeto text, p_operacion text, p_idem text, p_cambios jsonb)
RETURNS bigint LANGUAGE plpgsql AS $$
DECLARE v_id bigint; i int; q text; o cerrojos%ROWTYPE;
BEGIN
  IF p_idem IS NOT NULL THEN
    PERFORM pg_advisory_xact_lock(hashtext('idem|' || p_idem));
    SELECT id INTO v_id FROM commits WHERE idempotencia = p_idem;
    IF FOUND THEN RETURN v_id; END IF;
  END IF;
  -- Los de la v2, en su mismo orden (la v2 los vuelve a tomar: son reentrantes).
  FOR q IN SELECT DISTINCT x FROM jsonb_array_elements(p_cambios) cc,
                                  jsonb_array_elements_text(cc->'rutas') x ORDER BY 1 LOOP
    PERFORM pg_advisory_xact_lock(hashtext(p_ref || '|' || q));
    IF p_ref <> 'main' THEN PERFORM pg_advisory_xact_lock_shared(hashtext('main|' || q)); END IF;
  END LOOP;
  FOR i IN 0 .. jsonb_array_length(p_cambios) - 1 LOOP
    SELECT * INTO o FROM cerrojos
     WHERE ref = p_ref AND caduca > clock_timestamp()
       AND activo IN (SELECT jsonb_array_elements_text(p_cambios->i->'rutas'))
     ORDER BY activo LIMIT 1;
    IF FOUND THEN
      RAISE EXCEPTION USING ERRCODE = 'OR429', MESSAGE = 'el puntero esta en una fusion en curso: reintenta',
        DETAIL = jsonb_build_object('indice', i, 'ruta', o.activo, 'dueno', o.dueno)::text;
    END IF;
  END LOOP;
  RETURN escribir_sin_cerrojos(p_ref, p_sujeto, p_operacion, p_idem, p_cambios);
END $$;

-- El puntero de main en una ruta en el commit `p_hasta` (el punto de salida de una rama): la ultima
-- fila de su historia hasta ahi. Una fila de borrado, o ninguna, es «no estaba» (null).
CREATE FUNCTION main_puntero_en(p_activo text, p_hasta bigint) RETURNS jsonb LANGUAGE sql STABLE AS $$
  SELECT puntero FROM historia WHERE ref = 'main' AND activo = p_activo AND commit_id <= p_hasta
   ORDER BY commit_id DESC LIMIT 1
$$;

-- **El plan de fusion** de `p_rama` en main, por ruta, sin escribir nada. Es `git::fusionar_puntero`
-- de hoy, comparando el puntero ENTERO (igualdad de jsonb, como el blob de git):
--   b = main en el punto de salida de la rama; m = main hoy; r = lo propio de la rama (null si es
--   una lapida).
--   | la rama no tiene nada propio, o lo propio es igual a b | promocion / main |
--   | m = b, o m = r                                          | promocion / rama (borrados y nacidos) |
--   | los dos, r con receta (sin la clave escrito_por)        | reconstruir / main |
--   | los dos, sin receta (una lapida nunca tiene receta)     | conflicto / lo elegido, o «sin elegir» |
-- `p_rutas`: el alcance (null = todo lo propio de la rama). `p_elegidos`: {"<ruta>": "rama"|"main"}
-- POR RUTA: quien llama pasa el nombre a sus rutas (la de su sitio y la de antes, si quiere).
-- Devuelve {"estado": "plan"|"sin elegir", "rama", "pasos": [{activo, caso, resultado, rama, main,
-- propio}], "sin_elegir": [{activo, rama, main}]}. `rama` y `main` son los punteros que se vieron
-- (null = no hay); `propio`, si la rama tenia fila propia (lapida incluida). Con algo sin elegir no
-- se aplica nada (el 409 de hoy).
CREATE FUNCTION plan_fusion(p_rama text, p_rutas text[], p_elegidos jsonb) RETURNS jsonb
LANGUAGE plpgsql STABLE AS $$
DECLARE
  v_punto bigint; q text; e estado%ROWTYPE; propio boolean; b jsonb; m jsonb; r jsonb;
  caso text; res text; pasos jsonb := '[]'; sin jsonb := '[]';
BEGIN
  IF p_rama = 'main' THEN
    RAISE EXCEPTION USING ERRCODE = 'OR422', MESSAGE = 'main no se fusiona en main';
  END IF;
  SELECT punto_salida INTO v_punto FROM refs WHERE nombre = p_rama;
  IF NOT FOUND THEN
    RAISE EXCEPTION USING ERRCODE = 'OR404', MESSAGE = format('la rama `%s` no esta en el catalogo', p_rama);
  END IF;
  FOR q IN SELECT DISTINCT x FROM unnest(coalesce(p_rutas,
             ARRAY(SELECT activo FROM estado WHERE ref = p_rama))) x ORDER BY 1 LOOP
    SELECT * INTO e FROM estado WHERE ref = p_rama AND activo = q;
    propio := FOUND;
    r := CASE WHEN propio AND NOT e.borrado THEN e.puntero END;
    b := main_puntero_en(q, v_punto);
    SELECT puntero INTO m FROM estado WHERE ref = 'main' AND activo = q;
    IF NOT FOUND THEN m := NULL; END IF;
    IF NOT propio OR r IS NOT DISTINCT FROM b THEN
      caso := 'promocion'; res := 'main';                -- no lo toco: queda el de main
    ELSIF m IS NOT DISTINCT FROM b OR m IS NOT DISTINCT FROM r THEN
      caso := 'promocion'; res := 'rama';                -- solo lo toco la rama
    ELSIF r IS NOT NULL AND NOT (r ? 'escrito_por') THEN
      caso := 'reconstruir'; res := 'main';              -- los dos, con receta: se rehace en main
    ELSE
      caso := 'conflicto';
      res := CASE p_elegidos->>q WHEN 'rama' THEN 'rama' WHEN 'main' THEN 'main' ELSE 'sin elegir' END;
      IF res = 'sin elegir' THEN
        sin := sin || jsonb_build_object('activo', q, 'rama', r, 'main', m);
      END IF;
    END IF;
    pasos := pasos || jsonb_build_object('activo', q, 'caso', caso, 'resultado', res,
                                         'rama', r, 'main', m, 'propio', propio);
  END LOOP;
  RETURN jsonb_build_object('estado', CASE WHEN jsonb_array_length(sin) > 0 THEN 'sin elegir' ELSE 'plan' END,
                            'rama', p_rama, 'pasos', pasos, 'sin_elegir', sin);
END $$;

-- La rama trae main: lo propio que ya es igual a main deja de ser propio (y una lapida de lo que main
-- ya no tiene), y su punto de salida pasa a main de hoy.
CREATE FUNCTION traer_main_limpiando(p_rama text) RETURNS void LANGUAGE sql AS $$
  DELETE FROM estado r
   WHERE r.ref = p_rama
     AND ((NOT r.borrado AND r.puntero IS NOT DISTINCT FROM
             (SELECT m.puntero FROM estado m WHERE m.ref = 'main' AND m.activo = r.activo))
          OR (r.borrado AND NOT EXISTS (SELECT 1 FROM estado m WHERE m.ref = 'main' AND m.activo = r.activo)));
  SELECT traer_main(p_rama);
$$;

-- **Aplicar un plan** (de `plan_fusion`, el que viajo en el commit de git `p_sha`), en una
-- transaccion e idempotente por `git:<sha>`: la segunda vez devuelve lo de la primera, con
-- "repetida": true, sin tocar nada.
--   - Cada paso con resultado `rama` pone en main el puntero del plan (o lo quita si la rama lo
--     borro), con historia y outbox. Lo que ya esta igual no se toca.
--   - Si main no esta como se planeo (`main` del paso): sin `p_tardia` es un fallo del protocolo
--     (OR500: el cerrojo debia impedirlo). Con `p_tardia` (el cerrojo caduco antes de aplicar) gana
--     lo de main y el paso se informa en `pendientes`, como `aplicar_tardia` de E1a·3.
--   - La rama se pone al dia: lo del plan que sigue como se planeo deja de ser suyo (lo promovido y
--     lo que se queda en main: reconstruir, o main elegido), menos lo pendiente; luego trae main
--     (`traer_main_limpiando`). Sin `p_tardia`, una rama movida con el cerrojo puesto es OR500.
-- Siempre deja un commit (aunque no mueva nada): `aplicado(sha)` lo encuentra.
-- Devuelve {"commit", "aplicados": [rutas], "pendientes": [{activo, se_queda, no_entra}], "tardia"}.
CREATE FUNCTION aplicar_plan(p_rama text, p_sha text, p_sujeto text, p_plan jsonb, p_tardia boolean)
RETURNS jsonb LANGUAGE plpgsql AS $$
DECLARE
  v_id bigint; v_res jsonb; p jsonb; q text; m jsonb; prev_ml text; e estado%ROWTYPE; rv jsonb;
  aplicados jsonb := '[]'; pend jsonb := '[]'; pend_a text[] := '{}';
BEGIN
  PERFORM pg_advisory_xact_lock(hashtext('idem|git:' || p_sha));
  SELECT id, resultado INTO v_id, v_res FROM commits WHERE idempotencia = 'git:' || p_sha;
  IF FOUND THEN RETURN coalesce(v_res, jsonb_build_object('commit', v_id)) || '{"repetida": true}'; END IF;
  IF p_plan->>'estado' IS DISTINCT FROM 'plan' THEN
    RAISE EXCEPTION USING ERRCODE = 'OR422', MESSAGE = 'un plan con algo sin elegir no se aplica';
  END IF;
  IF p_plan->>'rama' IS DISTINCT FROM p_rama THEN
    RAISE EXCEPTION USING ERRCODE = 'OR422',
      MESSAGE = format('el plan es de `%s`, no de `%s`', p_plan->>'rama', p_rama);
  END IF;
  IF NOT EXISTS (SELECT 1 FROM refs WHERE nombre = p_rama) THEN
    RAISE EXCEPTION USING ERRCODE = 'OR404', MESSAGE = format('la rama `%s` no esta en el catalogo', p_rama);
  END IF;
  PERFORM tomar_rutas(ARRAY[p_rama, 'main'],
                      ARRAY(SELECT x->>'activo' FROM jsonb_array_elements(p_plan->'pasos') x));
  INSERT INTO commits (ref, sujeto, operacion, idempotencia)
  VALUES ('main', p_sujeto, 'fusionar ' || p_rama, 'git:' || p_sha) RETURNING id INTO v_id;
  FOR p IN SELECT * FROM jsonb_array_elements(p_plan->'pasos') LOOP
    CONTINUE WHEN p->>'resultado' <> 'rama';
    q := p->>'activo';
    SELECT puntero, metadata_location INTO m, prev_ml FROM estado WHERE ref = 'main' AND activo = q;
    IF NOT FOUND THEN m := NULL; prev_ml := NULL; END IF;
    CONTINUE WHEN m IS NOT DISTINCT FROM nullif(p->'rama', 'null'::jsonb);   -- ya esta
    IF m IS DISTINCT FROM nullif(p->'main', 'null'::jsonb) THEN
      IF NOT p_tardia THEN
        RAISE EXCEPTION USING ERRCODE = 'OR500', MESSAGE = format('main movio `%s` con el cerrojo puesto', q);
      END IF;
      pend := pend || jsonb_build_object('activo', q, 'se_queda', m, 'no_entra', p->'rama');
      pend_a := pend_a || q;
      CONTINUE;
    END IF;
    PERFORM poner_en('main', q, nullif(p->'rama', 'null'::jsonb), v_id);
    INSERT INTO historia VALUES (v_id, 'main', q, prev_ml, p->'rama'->>'metadata_location',
                                 p->'rama'->>'escrito_por', 'fusionar', nullif(p->'rama', 'null'::jsonb));
    INSERT INTO outbox (commit_id, ref, activo, metadata_location)
    VALUES (v_id, 'main', q, p->'rama'->>'metadata_location');
    aplicados := aplicados || to_jsonb(q);
  END LOOP;
  -- La rama: lo del plan que sigue como se planeo deja de ser suyo (lo pendiente se queda en ella).
  FOR p IN SELECT * FROM jsonb_array_elements(p_plan->'pasos') LOOP
    q := p->>'activo';
    CONTINUE WHEN q = ANY (pend_a) OR p->>'resultado' NOT IN ('rama', 'main');
    SELECT * INTO e FROM estado WHERE ref = p_rama AND activo = q;
    CONTINUE WHEN NOT FOUND;
    rv := CASE WHEN e.borrado THEN NULL ELSE e.puntero END;
    IF rv IS NOT DISTINCT FROM nullif(p->'rama', 'null'::jsonb) THEN
      DELETE FROM estado WHERE ref = p_rama AND activo = q;
    ELSIF NOT p_tardia THEN
      RAISE EXCEPTION USING ERRCODE = 'OR500', MESSAGE = format('la rama movio `%s` con el cerrojo puesto', q);
    END IF;
  END LOOP;
  PERFORM traer_main_limpiando(p_rama);
  v_res := jsonb_build_object('commit', v_id, 'aplicados', aplicados, 'pendientes', pend, 'tardia', p_tardia);
  UPDATE commits SET resultado = v_res WHERE id = v_id;
  RETURN v_res;
END $$;

-- **Aplicar una definicion** (retirar, renombrar) que git ya confirmo en `p_sha`: `escribir` con
-- `idempotencia = git:<sha>` (los mismos cambios de la v2, y su 429 si otro tiene la ruta cerrada).
-- `p_dueno`: lo que la propia definicion cerro (renombrar cierra su schema mientras graba): se
-- suelta en la MISMA transaccion, sin un instante en que otro escriba entre soltar y aplicar.
CREATE FUNCTION aplicar_definicion(p_ref text, p_sha text, p_sujeto text, p_operacion text, p_cambios jsonb,
                                   p_dueno text)
RETURNS bigint LANGUAGE plpgsql AS $$
BEGIN
  IF p_dueno IS NOT NULL THEN PERFORM soltar(p_dueno); END IF;
  RETURN escribir(p_ref, p_sujeto, p_operacion, 'git:' || p_sha, p_cambios);
END $$;

-- **Lo propio de una rama** (·5c·5c·3: `/ramas/{r}/cambios` y la huella de una propuesta), por ruta:
-- `antes` = main en su punto de salida, `despues` = lo de la rama (null si lo borro), `hoy` = main
-- hoy. Lo que la rama no toco no sale: no es un cambio suyo.
CREATE FUNCTION propios(p_rama text) RETURNS TABLE (activo text, antes jsonb, despues jsonb, hoy jsonb, borrado boolean)
LANGUAGE sql STABLE AS $$
  SELECT e.activo, main_puntero_en(e.activo, r.punto_salida),
         CASE WHEN e.borrado THEN NULL ELSE e.puntero END,
         (SELECT m.puntero FROM estado m WHERE m.ref = 'main' AND m.activo = e.activo), e.borrado
    FROM estado e JOIN refs r ON r.nombre = e.ref
   WHERE e.ref = p_rama AND p_rama <> 'main'
   ORDER BY e.activo
$$;

-- **El reconciliador**: ¿esta ya aplicado lo del commit de git `p_sha`?
CREATE FUNCTION aplicado(p_sha text) RETURNS boolean LANGUAGE sql STABLE AS $$
  SELECT EXISTS (SELECT 1 FROM commits WHERE idempotencia = 'git:' || p_sha)
$$;
