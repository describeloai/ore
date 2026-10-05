-- 050 · EL PUESTO LO ABRE ALGUIEN (0053 F4·3, la huella desde un puesto)
--
-- Lo que pasa en un puesto no llega a la huella. La celda habla con el token de
-- su pod (`x-ore-pod`), no con el de la persona, y `ore-iam` sólo anota un
-- evento con `Ore-Sujeto` o con la `decision` viva que lo dejó pasar (`hizo`):
-- una celda no puede atribuirle a nadie lo que `ore-iam` no autorizó. Medido el
-- 2026-10-05: las cuatro lecturas en vivo de test6 dieron 400 y fueron a
-- `muertos/`.
--
-- ⭐ El arreglo no toca esa regla: le da lo que pide. Al abrir el puesto la
--   persona está en la consola con su token; `ore-serve` pregunta entonces
--   `puesto:abrir`, guarda la decisión con el puesto, y cada evento que sale de
--   él la nombra. Quien queda en la huella es quien `ore-iam` vio abrirlo.
--
-- ── A quién ─────────────────────────────────────────────────────────────────
--
-- A todo el que pertenece, por defecto: hoy cualquiera de la organización abre
-- su puesto, y esto no le quita eso a nadie — sólo lo deja dicho. Va en
-- `iam.por_defecto`, como leer la organización: pertenecer ya lo da.
--
-- ⚠️ La decisión vive 24 h (`RETENCION`). Un puesto abierto más tiempo la
--   renueva cuando la persona vuelve a pedirlo desde la consola (`POST /puestos`
--   devuelve el que hay y pregunta otra vez).
insert into iam.potestad (nombre, que_hace, ejercida) values
  ('puesto:abrir', 'abrir un puesto de trabajo. Lo que haga despues queda a nombre de quien lo abrio', true)
on conflict (nombre) do nothing;

create or replace view iam.por_defecto as
  select nombre as potestad from iam.potestad
   where nombre in ('organizacion:leer', 'miembro:listar', 'rol:listar', 'puesto:abrir');
