# 0053 F7·4 · copiar desde el origen, en vivo (test6)

## 1 · Celda **SQL** (el guion): una base estándar y dos copias desde el origen

```sql
create standard database if not exists f7_copias;

-- copiar una tabla filtrada: un RANGO, que el `where` de un dataset nunca pudo decir
create or replace dataset f7_copias.default.pedidos_altos as
select id, cliente_id, total, fecha
from s3_demo.nueva_carpeta.pedidos
where total > 500;

-- copiar un cálculo entre dos orígenes: BigQuery × S3, agregado
create materialized view f7_copias.default.ventas_por_pais as
select c.pais, count(*) as pedidos, sum(p.total) as total
from bigquery_20260927_1428.ventas.clientes c
join s3_demo.nueva_carpeta.pedidos p on p.cliente_id = c.id
group by c.pais;
```

Debe decir, por la primera copia, `f7_copias.pedidos_altos · a copy from the origin: the view
f7_copias.pedidos_altos_consulta holds the query…`, y por la segunda, su vista y su copia
`f7_copias.ventas_por_pais_copia`.

## 2 · Celda **Python**: el Job de la copia, y leer lo copiado

```python
import time, ore

# 2a · que el Job copie ya (sin esperar a la pasada): rehacer la copia de la base
c, r = ore.session.pedir("POST", "/paquetes/f7_copias/copia/rehacer", {})
print("rehacer:", c, r)

# 2b · esperar a que las copias estén (el Job lee los dos orígenes y calcula)
for _ in range(60):
    try:
        a = ore.sql("select count(*) as n, min(total) as minimo from f7_copias.pedidos_altos")
        b = ore.sql("select * from f7_copias.ventas_por_pais_copia order by pedidos desc")
        break
    except Exception as e:
        print("…", str(e)[:120]); time.sleep(10)
print(a); print(b)

# 2c · lo copiado NO es una lectura en vivo: no pasa por /federation/read ni tiene su tope
print("leído en vivo:", ore._leido_en_vivo(a))   # → []

# 2d · guardar a mano lo leído en vivo se niega (F7·1)
vivo = ore.sql("select id, total from s3_demo.nueva_carpeta.pedidos where total > 500")
try:
    ore.write("f7_copias.a_mano", vivo)
except PermissionError as e:
    print("write() de lo leído en vivo:", str(e)[:140])
```
