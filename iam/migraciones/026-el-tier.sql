-- 026 · EL TIER — qué promete cada sitio donde puede correr un inquilino
--
-- ══════════════════════════════════════════════════════════════════════════
-- ⭐⭐ POR QUÉ ES UNA TABLA Y NO UN `check` EN `celda.tier`
--
--   La `025` dejó `tier` como un texto con `check (tier in (...))`. Vale para
--   sostener el alfabeto y no vale para lo que la consola tiene que DECIR de
--   cada uno: qué promete y qué cuota lleva. «Serverless: sin nodos que
--   gestionar, 10 vCPU, 36 GiB, 50 jobs» tiene que salir de algún sitio, y
--   ese sitio no puede ser una constante en la consola — sería la segunda
--   descripción de una cuota que ya está escrita en `malla/11-el-inquilino.yaml`,
--   y dos descripciones divergen el día que alguien toca una.
--
--   ⇒ Se escribe AQUÍ, en el plano de control, que es de quien es la promesa.
--     Y `pruebas-de-fuego/medida-el-tier-y-la-cuota.py` comprueba que estos
--     números son los de la plantilla, byte a byte. Si alguien sube la cuota
--     en el malla y no aquí, la medida se pone roja — que es lo que no puede
--     hacer un comentario.
--
-- ── ⚠️ LA CUOTA ES `null` PARA DEDICADO Y BYOC, A PROPÓSITO ────────────────
--
--   Un clúster propio no tiene cuota de plataforma: tiene lo que tenga. Poner
--   un número sería inventarlo. El día que un dedicado se aprovisione con un
--   tamaño, el tamaño irá en SU celda, no en el tier.
--
-- ══════════════════════════════════════════════════════════════════════════

create table if not exists iam.tier (
  nombre   text primary key,
  -- Cómo se llama de cara al cliente. Las palabras de Redpanda: es lo que un
  -- cliente que viene de allí espera leer, y «Serverless» para el compartido
  -- no miente — es una promesa de operación, no una arquitectura (0024 ①).
  titulo   text not null,
  -- La promesa, en una frase. Se pinta tal cual.
  promesa  text not null,
  -- La cuota del namespace, como la escribe `ResourceQuota`: texto de
  -- Kubernetes ("10", "36Gi"), no números convertidos. Convertir es una
  -- segunda oportunidad de divergir.
  cuota_cpu     text,
  cuota_memoria text,
  cuota_jobs    text
);

insert into iam.tier (nombre, titulo, promesa, cuota_cpu, cuota_memoria, cuota_jobs) values
  ('compartido', 'Serverless',
   'Nothing to manage: a namespace on a shared node, isolated by quota and network policy. Catalog jobs scale from zero.',
   '10', '36Gi', '50'),
  ('dedicado', 'Dedicated',
   'A cluster of your own, operated by us. Physical isolation and private networking. Sized per tenant.',
   null, null, null),
  ('byoc', 'BYOC',
   'Your cloud account, our control plane. An agent in your cluster pulls its manifests; nothing of ours reaches in.',
   null, null, null)
on conflict (nombre) do update
  set titulo = excluded.titulo, promesa = excluded.promesa,
      cuota_cpu = excluded.cuota_cpu, cuota_memoria = excluded.cuota_memoria, cuota_jobs = excluded.cuota_jobs;

-- ⭐ Y la celda pasa a apuntar al tier. El `check` de la `025` se queda: es
--   redundante con la clave ajena y no estorba, y quitarlo sería una línea
--   de más en una migración que ya hace bastante.
alter table iam.celda
  drop constraint if exists celda_tier_fkey,
  add constraint celda_tier_fkey foreign key (tier) references iam.tier(nombre);
