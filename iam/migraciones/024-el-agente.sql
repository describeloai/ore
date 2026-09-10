-- 024 · EL AGENTE — un sujeto que no es nadie, y uno POR INQUILINO
--
-- ══════════════════════════════════════════════════════════════════════════
-- ⛔⛔ LO QUE LO FORZÓ, MEDIDO EN VIVO
--
--   El primer Job de catálogo que llegó a pedir de verdad —con su credencial ya
--   guardada en el cofre— murió así:
--
--       ### pidiendo `fuente-postgresql_20260910_1305` al custodio
--       ✗ el custodio contesto 422
--       {"error":"quien pide no es una persona conocida aqui"}
--
--   Y eso NO era un fallo: el custodio traduce el `sub` del testigo a una fila
--   de `iam.persona` antes de mirar nada más, y un Job no es una persona.
--
-- ── ⭐⭐ Y ESTO NO ES MODELAR NADA NUEVO ─────────────────────────────────
--
--   Tres migraciones ya lo nombran, dos le hacen sitio, y ninguna lo implementó:
--
--     007  `sujeto` es TEXTO y no clave ajena a `persona`, «porque aquí también
--          entra un agente —un Job de ORE actuando por alguien—, y eso es
--          RFC 8693»
--     008  `iam.huella` tiene columna `agente` desde su creación: «`quien` es la
--          persona; `agente` es lo que actuó por ella»
--     014  y aquí el hueco está DICHO: una potestad «necesita un concepto de
--          agente que aquí no existe»
--
--   ⇒ Lo que faltaba no era la idea. Era la tabla.
--
-- ══════════════════════════════════════════════════════════════════════════
-- ⭐⭐ POR QUÉ LLEVA `organizacion`, QUE ES LA DECISIÓN DE ESTA MIGRACIÓN
-- ══════════════════════════════════════════════════════════════════════════
--
-- `iam.persona` NO la lleva: una persona es la misma en todas las
-- organizaciones a las que pertenece, y la `021` lo decidió así.
--
-- Un agente no. Medido el 2026-09-10: `idp-agente` es **el mismo cliente
-- `ore-agente` en `t-demo` y en `t-prueba`** —dos namespaces, un secreto, un
-- `sub`—. Sin esta columna, el sujeto sería uno solo y concederle `usar` en una
-- organización lo pondría a un `grant` de distancia de todas.
--
-- ⇒ Con ella, el MISMO `sub` del IdP es un sujeto DISTINTO en cada inquilino.
--   La concesión de `demo` nombra `age_…` de demo, y la de `prueba` otro. Es la
--   misma figura que la KEK por organización y que `ore-driver-<inquilino>`:
--   **compartir la credencial no puede significar compartir la autoridad.**
--
-- ⚠️ Y LO QUE ESTO NO ARREGLA, dicho para que no se confunda con lo que sí:
--   la credencial sigue siendo UNA. Quien la robe puede pedir un testigo y ese
--   testigo resuelve al agente de cualquier inquilino donde exista fila. Lo que
--   esta tabla acota es la AUTORIDAD —hay que conceder por inquilino, y se ve
--   quién actuó— no el alcance de un robo.
--
--   ⇒ Eso se cierra con un cliente de IdP por inquilino, y ese día lo único que
--     cambia es el `sub` de estas filas. La forma ya es la correcta.
-- ══════════════════════════════════════════════════════════════════════════

create table if not exists iam.agente (
  id            text        primary key,
  -- Atado al emisor, igual que `iam.persona`: un `sub` sin su emisor no
  -- identifica a nadie — dos realms pueden emitir el mismo.
  emisor        text        not null,
  sub           text        not null,
  organizacion  text        not null references iam.organizacion(id),
  -- Para leerlo en una huella sin tener que ir a buscarlo. No decide nada.
  nombre        text,
  creado_en     timestamptz not null default now(),
  -- ⛔ La unicidad incluye la organización: es lo que permite que el mismo
  --   `(emisor, sub)` sea un sujeto por inquilino y no uno global.
  unique (emisor, sub, organizacion)
);

comment on table iam.agente is
  'Un sujeto que no es una persona: un Job actuando dentro de UNA organizacion. El mismo `sub` del IdP da un agente distinto por inquilino, y por eso la unicidad lleva la organizacion dentro.';

comment on column iam.agente.organizacion is
  'Lo que hace que compartir la credencial no signifique compartir la autoridad. `iam.persona` no lo lleva a proposito: una persona es la misma en todas las organizaciones a las que pertenece (021).';

-- ── Quién lo lee y quién lo escribe ──────────────────────────────────────
--
-- ⭐ El custodio SÓLO LEE, igual que con `iam.persona`: necesita traducir un
--   `sub` a un sujeto para preguntar por su concesión, y nada más. Registrar un
--   agente es un acto de operador, no algo que un custodio haga al vuelo.
--
-- ⛔ Y ni `update` ni `delete` para nadie de la aplicación. Un agente no se
--   edita: se registra o se deja de conceder. Cambiar a qué `sub` apunta una
--   fila ya concedida sería mover la autoridad sin que se note en ninguna
--   concesión — el mismo rodeo que la `021` cierra por arriba.
grant select on iam.agente to ore_cofre;
grant select, insert on iam.agente to ore_iam;
