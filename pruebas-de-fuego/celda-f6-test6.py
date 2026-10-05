# 0053 F6·3 · SQL en vivo desde el puesto (test6): ore.sql() y ore.explain().
import time, warnings
import ore

warnings.simplefilter("always")


def prueba(nombre, q, **kw):
    t = time.time()
    try:
        with warnings.catch_warnings(record=True) as w:
            d = ore.sql(q, **kw)
        avisos = [str(x.message)[:110] for x in w]
        print(f"✓ {nombre}: {len(d)} filas · {time.time() - t:.2f}s" + (f" · ⚠ {avisos}" if avisos else ""))
        return d
    except Exception as e:
        print(f"✗ {nombre}: {type(e).__name__}: {str(e)[:200]}")


# 1 · cada origen, por el nombre de su tabla
prueba("Neon, un usuario", "SELECT id, campaign_id FROM postgresql_20260921_2055.public.ai_insights WHERE user_id = 'user_2z6AxnKAsi72sT810URRFk8MZ3m'")
prueba("BigQuery, un país", "SELECT id, email FROM bigquery_20260927_1428.ventas.clientes WHERE pais = 'ES'")
prueba("S3, primeros 20", "SELECT * FROM s3_demo.nueva_carpeta.pedidos LIMIT 20")

# 2 · las bases foráneas de hoy, en vivo
prueba("foránea bq_foreign", "SELECT email, pais FROM bq_foreign.ventas.clientes WHERE pais = 'ES'")
prueba("foránea foreign_test (from/fields)", "SELECT summary_text FROM foreign_test.ai_insights LIMIT 3")

# 3 · juntas: dos orígenes, y un origen con una copia del lago
prueba("BigQuery × S3", """
  SELECT c.email, p.total FROM bigquery_20260927_1428.ventas.clientes c
  JOIN s3_demo.nueva_carpeta.pedidos p ON p.cliente_id = c.id
  WHERE c.pais = 'ES'""")
prueba("Neon × copia del lago (bq.ventas.pedidos)", """
  SELECT count(*) AS n FROM postgresql_20260921_2055.public.ai_insights a
  CROSS JOIN (SELECT count(*) FROM bq.ventas.pedidos) l""")

# 4 · el tope: la sintética entera se corta (aviso), y con strict=True es error
prueba("sintética entera (corte)", "SELECT id FROM bigquery_20260927_1428.ventas.ore_e2e_sintetica")
prueba("sintética entera, strict", "SELECT id FROM bigquery_20260927_1428.ventas.ore_e2e_sintetica", strict=True)

# 5 · lo que se le pide a cada origen
ore.explain("""
  SELECT c.email, p.total FROM bigquery_20260927_1428.ventas.clientes c
  JOIN s3_demo.nueva_carpeta.pedidos p ON p.cliente_id = c.id
  WHERE c.pais = 'ES' AND p.total > 100""")
