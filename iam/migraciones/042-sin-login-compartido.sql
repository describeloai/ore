-- 042 · SIN LOGIN COMPARTIDO: SE RETIRA `cofre_app` (0047 A7a.6)
--
-- `cofre_app` era EL login de los tres custodios, y con el cada uno leia el
-- censo y los secretos de todos (0047 M4). Desde la A7a cada custodio entra con
-- el papel de su celda (040), la malla trae el suyo (A7a.3) y la seguridad por
-- fila no deja a nadie ver otra organizacion (041). `cofre_app` quedo sin celda:
-- no veia nada.
--
-- ⭐ Y no se retira «porque ya paso un dia». Se retira porque se midieron, el
--   2026-09-28, los eventos que tenian que salir bien sin el (0047 § A7a.6):
--   cada custodio reinicia, trae su secreto y conecta (E1); listar y retirar
--   con su login, y nada de otra organizacion (E2); los verbos de operador
--   (E3); una pasada entera de la convergencia (E4); nadie mas lo usaba —se le
--   quito el login y se desactivo su secreto, y durante una pasada nada se
--   quejo— (E5); y la vuelta atras sin el —rotar la clave de una celda y que su
--   custodio vuelva— (E6).
--
-- `if exists`: en las bases de prueba nunca existio.
do $$
begin
  if exists (select 1 from pg_roles where rolname = 'cofre_app') then
    revoke ore_cofre from cofre_app;
    drop role cofre_app;
  end if;
end $$;
