create or replace dataset ventas.cotejo as
with t as (select *, monotonically_increasing_id() __ore_k from ventas.clientes),
p1 as (select t.__ore_k, l.`nombre` __ore_c_1 from t join ventas.municipios l on ST_Within(t.`ubicacion`, l.`geom`)),
c1 as (select __ore_k, min(__ore_c_1) `municipio` from p1 group by 1),
p2 as (select t.__ore_k, l.`ventas` __ore_c_1 from t join ventas.competidores l on ST_DWithin(t.`ubicacion`, l.`geom`, 500)),
c2 as (select __ore_k, count(*) `competidores_500m`, sum(__ore_c_1) `ventas_500m` from p2 group by 1),
p3 as (select t.__ore_k, l.`nombre` __ore_c_1, ST_Distance(t.`ubicacion`, l.`geom`) __ore_d from t join ventas.competidores l on ST_DWithin(t.`ubicacion`, l.`geom`, 5000)),
c3 as (select __ore_k, min_by(__ore_c_1, __ore_d) `competidor_cercano`, min(__ore_d) `dist_competidor` from p3 group by 1),
p4 as (select *, sum(__ore_a) over (partition by __ore_k, __ore_c_3) __ore_w_3 from (select t.__ore_k, l.`poblacion` __ore_c_1, l.`renta_media` __ore_c_2, l.`uso` __ore_c_3, l.`id` __ore_c_4, ST_Area(ST_Intersection(t.__ore_b, l.`geom`)) __ore_a, ST_Area(l.`geom`) __ore_al from (select __ore_k, ST_Buffer(`ubicacion`, 500) __ore_b from t) t join ventas.secciones l on ST_Intersects(t.__ore_b, l.`geom`)) q),
c4 as (select __ore_k, sum(__ore_c_1 * __ore_a / nullif(__ore_al, 0)) `poblacion_500m`, sum(__ore_c_2 * __ore_a) / nullif(sum(__ore_a), 0) `renta_500m`, min_by(__ore_c_3, named_struct('w', -__ore_w_3, 'v', __ore_c_3)) `uso_500m`, min_by(__ore_c_4, named_struct('w', -__ore_a, 'v', __ore_c_4)) `seccion` from p4 group by 1)
select t.`cliente_id`, t.`ubicacion`, c1.`municipio`, coalesce(c2.`competidores_500m`, 0) `competidores_500m`, c2.`ventas_500m`, c3.`competidor_cercano`, c3.`dist_competidor`, c4.`poblacion_500m`, c4.`renta_500m`, c4.`uso_500m`, c4.`seccion`
from t left join c1 using (__ore_k) left join c2 using (__ore_k) left join c3 using (__ore_k) left join c4 using (__ore_k)