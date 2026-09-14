-- 028 · EL MATERIAL SE FUE A LA CELDA, Y AQUÍ NO QUEDA NADA
--
-- ══════════════════════════════════════════════════════════════════════════
-- La 0024-⑤, medida antes (`medida-el-cofre-y-su-almacen.py`): el cofre corría
-- EN el inquilino y guardaba el cifrado en ESTA base, central. Desde el
-- 2026-09-14 el material vive en el Secret Manager de la celda, bajo el prefijo
-- del inquilino y con su llave como CMEK; el plano de control se queda con el
-- METADATO —`cofre.secreto`, `iam.concesion`— que es lo que Redpanda y
-- WarpStream se quedan también.
--
-- ── ⛔⛔ Y NO SE BORRA LO QUE NO SE HA MUDADO ───────────────────────────────
--
--   `ore-cofre mudar` corre EN cada inquilino con su cuenta —la única que puede
--   abrir su llave y escribir bajo su prefijo— y borra cada fila al llevarla.
--   Esta migración se NIEGA mientras quede una: un `drop` con filas dentro
--   sería destruir secretos que nadie ha copiado, y el mensaje dice de qué
--   organizaciones son para que se sepa dónde correr la mudanza.
--
-- ⚠️ Por eso esta migración puede quedar en rojo un rato, y es lo correcto: el
--   runner reintenta en cada reconciliación, y pasa sola cuando la mudanza
--   haya corrido en todos. Un rojo que dice qué falta vale más que un verde
--   que perdió filas.
-- ══════════════════════════════════════════════════════════════════════════

do $$
declare
  quedan text;
begin
  if to_regclass('cofre.material') is null then
    return;  -- ya se fue
  end if;
  select string_agg(distinct o.nombre, ', ' order by o.nombre) into quedan
    from cofre.material m
    join cofre.secreto s on s.id = m.secreto
    join iam.organizacion o on o.id = s.organizacion;
  if quedan is not null then
    raise exception using
      message = format('cofre.material aun guarda material de: %s. Corre `ore-cofre mudar` en cada una (malla/45-la-mudanza-del-cofre.yaml) antes de borrar.', quedan);
  end if;
  -- Material huérfano (sin secreto o sin organización) no debería existir; si
  -- lo hubiera, tampoco se borra en silencio.
  if exists (select 1 from cofre.material) then
    raise exception 'cofre.material tiene filas que no cuelgan de ningun secreto vivo: mirarlas antes de borrar';
  end if;
end $$;

drop view if exists cofre.vigente;
drop table if exists cofre.material;

comment on table cofre.secreto is
  'El METADATO de un secreto: nombre, clase, quien lo emitio, cuando. El material '
  'vive en el Secret Manager de la celda como t-<inquilino>-cofre-<nombre>. 0024-5.';
