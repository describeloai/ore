-- Query Q0 · Lo que un cliente escribe en DuckDB y no está en TPC: el SQL «amigable» y las funciones de
-- cada día, sobre las tablas de TPC-H. Una consulta por bloque, separadas por `;` al final de línea.
-- (nombre) en el comentario de cada una.

-- (exclude)
select * exclude (l_comment, l_shipinstruct) from lineitem order by l_orderkey, l_linenumber limit 5;
-- (replace)
select * replace (round(l_extendedprice) as l_extendedprice) from lineitem order by l_orderkey, l_linenumber limit 5;
-- (group_by_all)
select l_returnflag, l_linestatus, sum(l_quantity) q from lineitem group by all order by all;
-- (order_by_all)
select n_regionkey, count(*) n from nation group by n_regionkey order by all;
-- (cast_dos_puntos)
select o_orderkey::varchar k, o_totalprice::int p from orders order by o_orderkey limit 5;
-- (from_primero)
from nation select n_name, n_regionkey order by n_name limit 5;
-- (qualify)
select o_custkey, o_orderkey, o_totalprice from orders qualify row_number() over (partition by o_custkey order by o_totalprice desc) = 1 order by o_custkey limit 10;
-- (ilike)
select count(*) from part where p_name ilike '%green%';
-- (filtro_en_agregado)
select count(*) filter (where l_returnflag = 'R') r, count(*) total from lineitem;
-- (date_trunc)
select date_trunc('month', o_orderdate) m, count(*) from orders group by 1 order by 1 limit 5;
-- (strftime)
select strftime(o_orderdate, '%Y-%m') m, count(*) from orders group by 1 order by 1 limit 5;
-- (extract)
select extract(year from o_orderdate) y, sum(o_totalprice) from orders group by 1 order by 1;
-- (intervalo)
select count(*) from orders where o_orderdate >= date '1995-01-01' - interval 30 day;
-- (date_diff)
select avg(date_diff('day', o_orderdate, l_shipdate)) from orders join lineitem on l_orderkey = o_orderkey;
-- (list_agg)
select n_regionkey, list(n_name order by n_name) nombres from nation group by 1 order by 1;
-- (string_agg)
select n_regionkey, string_agg(n_name, ',' order by n_name) nombres from nation group by 1 order by 1;
-- (unnest)
select unnest([1, 2, 3]) x;
-- (literal_lista)
select [1, 2, 3] l, len([1, 2, 3]) n;
-- (struct)
select {'a': 1, 'b': 'x'} s;
-- (struct_campo)
select s.a from (select {'a': n_nationkey, 'b': n_name} s from nation) order by 1 limit 3;
-- (lambda)
select list_transform([1, 2, 3], x -> x * 2) l;
-- (lista_filtro)
select list_filter([1, 2, 3, 4], x -> x % 2 = 0) l;
-- (corte_de_texto)
select n_name[1:3] p from nation order by n_name limit 3;
-- (regexp)
select count(*) from customer where regexp_matches(c_phone, '^1[0-9]-');
-- (regexp_extract)
select regexp_extract(c_phone, '^([0-9]+)-', 1) p, count(*) from customer group by 1 order by 1 limit 3;
-- (split)
select string_split(c_phone, '-')[1] p from customer order by c_custkey limit 3;
-- (concat_op)
select n_name || '-' || n_regionkey x from nation order by n_nationkey limit 3;
-- (coalesce_ifnull)
select ifnull(null, 1) a, coalesce(null, 2) b, nullif(1, 1) c;
-- (case_when)
select case when o_totalprice > 200000 then 'grande' else 'normal' end t, count(*) from orders group by 1 order by 1;
-- (percentil)
select quantile_cont(o_totalprice, 0.5) mediana from orders;
-- (mediana)
select median(o_totalprice) from orders;
-- (aprox_distintos)
select approx_count_distinct(o_custkey) from orders;
-- (arg_max)
select arg_max(o_orderkey, o_totalprice) from orders;
-- (ventana_rango)
select o_orderdate, sum(o_totalprice) over (order by o_orderdate, o_orderkey rows between 6 preceding and current row) s from orders order by o_orderdate, o_orderkey limit 5;
-- (ventana_lag)
select o_orderkey, o_totalprice - lag(o_totalprice) over (order by o_orderkey) d from orders order by o_orderkey limit 5;
-- (cte_recursiva)
with recursive r(n) as (select 1 union all select n + 1 from r where n < 10) select sum(n) from r;
-- (generate_series)
select count(*) from generate_series(1, 100);
-- (range)
select count(*) from range(100);
-- (pivot)
pivot (select l_returnflag, l_linestatus, l_quantity from lineitem) on l_linestatus using sum(l_quantity) group by l_returnflag order by l_returnflag;
-- (unpivot)
unpivot (select 1 id, 2 a, 3 b) on a, b into name k value v order by k;
-- (asof)
select count(*) from orders o asof join lineitem l on l.l_orderkey = o.o_orderkey and l.l_shipdate >= o.o_orderdate;
-- (union_by_name)
select 1 a, 2 b union all by name select 3 b, 4 a order by a;
-- (columns_star)
select max(columns('l_.*price')) from lineitem;
-- (sample)
select count(*) > 0 from lineitem using sample 10%;
-- (read_parquet)
select count(*) from read_parquet('tpch/nation.parquet');
-- (describe)
describe lineitem;
-- (summarize)
summarize nation;
-- (json)
select json_extract('{"a": {"b": 5}}', '$.a.b') v;
-- (json_flecha)
select '{"a": {"b": 5}}'::json->'a'->>'b' v;
-- (epoch)
select epoch_ms(0) t;
-- (hash)
select hash('x') = hash('x') h;
-- (md5)
select md5('ore') h;
-- (greatest)
select greatest(1, 5, 3) g, least(1, 5, 3) l;
-- (round_dec)
select round(avg(l_discount), 4) from lineitem;
-- (bool_and)
select bool_and(l_quantity > 0), bool_or(l_quantity > 49) from lineitem;
-- (in_subconsulta)
select count(*) from customer where c_custkey in (select o_custkey from orders where o_totalprice > 400000);
-- (exists)
select count(*) from customer c where exists (select 1 from orders o where o.o_custkey = c.c_custkey);
-- (left_anti)
select count(*) from customer anti join orders on o_custkey = c_custkey;
-- (semi)
select count(*) from customer semi join orders on o_custkey = c_custkey;
-- (st_punto)
select ST_AsText(ST_Point(1, 2)) p;
-- (st_distancia)
select round(ST_Distance(ST_Point(0, 0), ST_Point(3, 4)), 3) d;
-- (st_dentro)
select ST_Contains(ST_GeomFromText('POLYGON((0 0, 0 2, 2 2, 2 0, 0 0))'), ST_Point(1, 1)) c;
-- (st_area)
select round(ST_Area(ST_GeomFromText('POLYGON((0 0, 0 2, 2 2, 2 0, 0 0))')), 3) a;
-- (h3)
select h3_latlng_to_cell(40.4, -3.7, 8) c;
