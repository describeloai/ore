-- 030 · RECORTAR (1 de 2): la celda es la unidad; la organización deja de exigir lo suyo
--
-- ══════════════════════════════════════════════════════════════════════════
-- Es la mitad «recortar» de la R1 de la 0025. La 029 ensanchó —duplicó `arbol` y
-- `entrada` en la celda—, la E2 cambió cada lector a su propio commit, y la
-- guarda (`medida-la-celda-tiene-nombre.py`) llegó a cero lectores salvo dos
-- que esta migración retira: el `insert` de `fundar` (que escribía las dos
-- verdades a propósito) y el `grant` por columnas de la 023.
--
-- ── ⛔ Se NIEGA si las dos verdades no coinciden ──────────────────────────
--
--   Recortar una columna que dice algo distinto de su copia sería perder un
--   dato. `iam.discrepancias` tiene que estar vacía; si no, esta migración lo
--   dice y no toca nada. Es la misma disciplina que la 028 con el material.
--
-- ── Lo que además cambia de forma, y por qué aquí ───────────────────────
--
--   · `celda_una_por_organizacion` fuera: es LA línea que prohibía la segunda
--     serverless. En su lugar, `(organizacion, nombre)` único y `nombre` único
--     GLOBAL — el nombre de una celda es un namespace, un host y una cuenta de
--     Google, y ninguno de los tres admite dos.
--   · `cofre.secreto` único por `(celda, nombre)` y `celda not null`: un secreto
--     es de una celda (0025-④). El disparador que la ponía por defecto se va:
--     el cofre la escribe desde la E2.
--   · `iam.discrepancias` se queda hasta la 031: las columnas siguen ahí, y se
--     cotejan solo cuando la organizacion aun las lleva (null = ya no).
--
-- 📎 `docs/decisions/0025-la-celda-tiene-nombre.md`, E3
-- ══════════════════════════════════════════════════════════════════════════

do $$
declare cuantas int; lista text;
begin
  select count(*), string_agg(organizacion || ': ' || que, '; ') into cuantas, lista from iam.discrepancias;
  if cuantas > 0 then
    raise exception using message = format('iam.discrepancias tiene %s filas y no se recorta nada: %s', cuantas, lista);
  end if;
  if exists (select 1 from cofre.secreto where celda is null) then
    raise exception 'hay secretos sin celda: no se puede exigir celda not null';
  end if;
end $$;

-- ① la organización DEJA de exigir lo que es de la celda, y todavía no lo pierde
--
-- ⚠️ En dos migraciones, no en una: el `fundar` que hoy corre escribe
--   `organizacion.arbol/entrada` (las dos verdades); el que viene ya no. Si esta
--   migración borrara las columnas, el de hoy fallaría al fundar hasta que la
--   imagen nueva llegara; si el de mañana llegara antes que la migración,
--   fallaría por el `not null`. Se quita el `not null` aquí, el código deja de
--   escribir, y la 031 borra. Es la R1 aplicada a un ESCRITOR.
alter table iam.organizacion alter column arbol   drop not null;
alter table iam.organizacion alter column entrada drop not null;

-- ② N celdas por organización; el nombre de la celda, único en el mundo
drop index if exists iam.celda_una_por_organizacion;
create unique index if not exists celda_nombre_unico on iam.celda (nombre);
create unique index if not exists celda_por_organizacion_y_nombre on iam.celda (organizacion, nombre);
comment on table iam.celda is
  'Donde vive un arbol. Una organizacion tiene N; cada una su nombre (= namespace), '
  'su cluster, su arbol, su entrada y su puerta. 0025.';

-- ③ un secreto es de una celda
drop trigger if exists secreto_celda_por_defecto on cofre.secreto;
drop function if exists cofre.celda_por_defecto();
alter table cofre.secreto alter column celda set not null;
alter table cofre.secreto drop constraint if exists secreto_organizacion_nombre_key;
create unique index if not exists secreto_por_celda_y_nombre on cofre.secreto (celda, nombre);

-- ④ la doble verdad, mientras dure: una organización que ya no lleva arbol/entrada
--    (null, escrita por el `fundar` nuevo) no discrepa de nadie.
create or replace view iam.discrepancias as
  select o.nombre as organizacion, 'arbol: ' || o.arbol || ' vs ' || c.arbol as que
    from iam.organizacion o join iam.celda c on c.organizacion = o.id and c.nombre = o.nombre
   where o.arbol is not null and o.arbol is distinct from c.arbol
  union all
  select o.nombre, 'entrada: ' || o.entrada || ' vs ' || c.entrada
    from iam.organizacion o join iam.celda c on c.organizacion = o.id and c.nombre = o.nombre
   where o.entrada is not null and o.entrada is distinct from c.entrada;
