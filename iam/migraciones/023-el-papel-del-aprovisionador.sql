-- 023 · EL PAPEL DEL APROVISIONADOR — cuatro columnas, y ni una más
--
-- ══════════════════════════════════════════════════════════════════════════
-- ⛔⛔ LO QUE ESTO SUSTITUYE, Y ERA PEOR DE LO QUE EL DISEÑO PROHIBÍA
--
--   `aprovisionar-inquilino.sh` abre con esta frase:
--
--       «⭐⭐ El aprovisionador NO tiene credenciales de clúster.»
--
--   Y lee la fila del inquilino así:
--
--       kubectl exec -n identidad idp-db-0 -- psql -U keycloak -d iam -tAc …
--
--   ⇒ `pods/exec` sobre la base de identidad no es «leer una fila»: es un
--     `psql` como superusuario dentro del pod. Medido con ese MISMO acceso:
--     `cofre.material`, `iam.persona` e `iam.concesion` enteras.
--
--   ⚠️ Y la `0022` rechazó que el aprovisionador creara los `Secret` porque
--     «quien puede crear el de un inquilino puede leer el de todos». Esto es
--     **estrictamente más que eso**: no lee los secretos de un namespace, lee
--     el censo. La propiedad no estaba rota en el diseño — estaba rota en
--     cómo se corría.
--
-- ── ⭐ Y por eso el arreglo es un papel y no una promesa ──────────────────
--
--   Es la misma figura que la `020`: allí se repartió `iam` y `cofre` en dos
--   papeles para que la separación fuera un `grant` y no una regla que alguien
--   recuerda. Éste es el tercero, y el más estrecho de los tres.
--
-- ══════════════════════════════════════════════════════════════════════════

-- `create role` no admite `if not exists`, y los papeles son del SERVIDOR y no
-- de la base: si otra base lo creó, aquí ya está.
do $$
begin
  if not exists (select 1 from pg_roles where rolname = 'ore_aprovisionador') then
    create role ore_aprovisionador nologin;
  end if;
end $$;

comment on role ore_aprovisionador is
  'Quien CONVERGE hacia la fila. Lee como se llaman las cosas de una organizacion y nada mas: no sabe quien es nadie ni que hay cifrado.';

-- ── ⭐⭐ POR COLUMNAS, Y AHÍ ESTÁ TODO EL TRABAJO ─────────────────────────
--
-- `grant select on iam.organizacion` habría bastado para que el guion
-- funcionara, y habría dado de más: `creada_por` es una clave foránea a
-- `iam.persona` —el dueño de cada organización— y `estado` dice quién está
-- suspendido. Nada de eso hace falta para crear una clave y un repositorio.
--
-- ⇒ Postgres sabe conceder por columna, así que se concede por columna. Lo que
--   el aprovisionador necesita es **cómo se llaman las cosas**:
--
--     nombre    para componer los nombres de las cuentas y del namespace
--     arbol     `<propietario>/<repositorio>` en la forja        (`017`)
--     kek       `<llavero>/<clave>` en el KMS                    (`019`)
--     entrada   el host de su puerta                             (`022`)
--
-- ⛔ Y `id` NO está. El aprovisionador no escribe en ninguna tabla que lo
--   referencie, así que un identificador interno sólo le serviría para
--   correlacionar filas que no tiene por qué ver.
grant usage on schema iam to ore_aprovisionador;
grant select (nombre, arbol, kek, entrada)
  on iam.organizacion to ore_aprovisionador;

-- ══════════════════════════════════════════════════════════════════════════
-- ⚠️ LO QUE ESTE PAPEL NO PUEDE, Y CONVIENE QUE ESTÉ ESCRITO
-- ══════════════════════════════════════════════════════════════════════════
--
-- ⛔ No puede ESCRIBIR nada. Ni siquiera en `iam.organizacion`: converger hacia
--   la fila significa que la fila manda, y quien converge no la corrige. Fundar
--   sigue siendo un acto de operador con un dueño detrás.
--
-- ⛔ No alcanza `cofre` — no hay `usage` sobre ese esquema, así que ni el
--   material ni los nombres de sus tablas existen para él.
--
-- ⛔ Y no deja huella, porque no tiene `insert` sobre `iam.huella`. Es
--   deliberado y hay que decirlo: lo que el aprovisionador hace queda escrito
--   en un commit y en la nube, no en el libro de `iam`. Anotar aquí exigiría
--   darle escritura sobre una tabla del censo — y una lectura de cuatro
--   columnas no vale un permiso de escritura.
--
-- ── ⚠️ Y lo que esto todavía NO arregla ──────────────────────────────────
--
-- El papel existe; el guion sigue usando `kubectl exec`. Esto es la mitad que
-- se puede escribir en SQL — la otra es que el aprovisionador corra como un
-- Job, con su propia credencial de base y sin RBAC ninguno. Hasta entonces,
-- este papel es una promesa cumplida a medias: está el `grant`, falta quien lo
-- use.
