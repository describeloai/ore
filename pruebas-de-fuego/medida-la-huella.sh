#!/usr/bin/env bash
# 0047 · M6 · La huella de hoy.
#
# Cuánto tiene `iam.huella`, a qué ritmo crece, qué operaciones hay, y si sus columnas y sus
# índices aguantarían la actividad del plano de datos. DE LECTURA, y SÓLO AGREGADOS: no imprime
# personas, ni `sobre`, ni `detalle` —de `detalle` sólo sus claves—.
#
#     bash pruebas-de-fuego/medida-la-huella.sh
#
# Entra por `kubectl exec` en el Postgres de `identidad` con el usuario del propio contenedor
# (sus variables, que no salen de él). Necesita el contexto del cluster `ore-mesh`.
set -euo pipefail

NS=identidad
POD=idp-db-0

kubectl exec -i -n "$NS" "$POD" -- sh -c 'psql -U "$POSTGRES_USER" -d iam -v ON_ERROR_STOP=1 -P pager=off' <<'SQL'
\echo '== 1 · el tamaño'
select count(*) as filas,
       min(cuando)::date as desde,
       max(cuando)::date as hasta,
       pg_size_pretty(pg_total_relation_size('iam.huella')) as ocupa
  from iam.huella;

\echo '== 2 · el ritmo: filas por día, los últimos 30'
select cuando::date as dia, count(*) as filas
  from iam.huella
 where cuando > now() - interval '30 days'
 group by 1 order by 1;

\echo '== 3 · las operaciones'
select operacion, count(*) as filas,
       min(cuando)::date as primera, max(cuando)::date as ultima
  from iam.huella group by 1 order by 2 desc;

\echo '== 4 · quién: cuántas personas y cuántas veces actúa un agente'
select count(distinct quien) as personas,
       count(*) filter (where agente is not null) as con_agente,
       count(*) filter (where agente is null) as sin_agente
  from iam.huella;

\echo '== 5 · sobre qué: el prefijo de `sobre` (hasta la primera barra), sin el nombre'
select coalesce(nullif(split_part(sobre, '/', 1), sobre), '(sin barra)') as prefijo,
       count(*) as filas
  from iam.huella group by 1 order by 2 desc limit 20;

\echo '== 6 · el detalle: sus claves, y cuántas filas llevan cada una'
select k as clave, count(*) as filas
  from iam.huella, lateral jsonb_object_keys(case when jsonb_typeof(detalle) = 'object'
                                                  then detalle else '{}'::jsonb end) k
 group by 1 order by 2 desc limit 30;

\echo '== 7 · las columnas'
select column_name, data_type, is_nullable
  from information_schema.columns
 where table_schema = 'iam' and table_name = 'huella' order by ordinal_position;

\echo '== 8 · los índices'
select indexname, indexdef from pg_indexes where schemaname = 'iam' and tablename = 'huella';

\echo '== 9 · quién puede escribirla y leerla'
select grantee, string_agg(privilege_type, ', ' order by privilege_type) as puede
  from information_schema.role_table_grants
 where table_schema = 'iam' and table_name = 'huella'
 group by 1 order by 1;

\echo '== 10 · el resto del esquema, por tamaño (para comparar)'
select c.relname as tabla, c.reltuples::bigint as filas_estimadas,
       pg_size_pretty(pg_total_relation_size(c.oid)) as ocupa
  from pg_class c join pg_namespace n on n.oid = c.relnamespace
 where n.nspname = 'iam' and c.relkind = 'r'
 order by pg_total_relation_size(c.oid) desc limit 12;
SQL
