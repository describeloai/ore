-- 043 · EL NOMBRE DE UN SECRETO LO OCUPA EL VIVO, NO EL RETIRADO
--
-- La `030` hizo único `(celda, nombre)` en `cofre.secreto` antes de que hubiera
-- baja. La `037` trajo la baja —la fila se queda, con `retirado_en`, porque
-- «tiene que seguir contando que existió»— y no tocó el índice. Así que un nombre
-- retirado quedaba ocupado PARA SIEMPRE.
--
-- Medido el 2026-09-29: siete secretos retirados (tres en demo, cuatro en
-- victor), y con ellos siete nombres que nadie podía volver a usar. El síntoma
-- era en la consola: dar de baja un origen y volver a darlo de alta con el mismo
-- nombre fallaba en el custodio con el texto de Postgres («duplicate key …
-- `secreto_por_celda_y_nombre`»), y `ore-serve` lo pintaba como un 502 que
-- además decía, en falso, que la fuente había quedado declarada.
--
-- ⭐ Único entre los VIVOS. Todo lo que busca un secreto por nombre para usarlo o
--   retirarlo ya filtra `retirado_en is null` (`resolver`, `retirar`, `mudar`,
--   `retirar-huerfanos`); `listar` enseña los dos, cada uno con su marca. Y las
--   concesiones del retirado quedaron revocadas con él: las del nuevo nacen aparte.
drop index if exists cofre.secreto_por_celda_y_nombre;
create unique index if not exists secreto_vivo_por_celda_y_nombre
  on cofre.secreto (celda, nombre)
  where retirado_en is null;
