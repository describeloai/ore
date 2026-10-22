"""ORE 0072 E2·0 · Previo E (T11b): cómo se reparten los 600 M de viajes de SpatialBench sf100 (el banco,
`trip.parquet`). Emite «E2S|…»: total, caja, la rejilla de 1° más poblada y cuántas celdas H3 hay por resolución."""
import json, os, sys, time
sys.path.insert(0, "/opt/ore")
os.environ.setdefault("ORE_SERVE", "http://ore-serve.%s.svc.cluster.local:8080" % os.environ.get("NS", "t-victor"))
import ore  # noqa: E402

BANCO = "https://storage.googleapis.com/project-8853a180-450d-47be-b83-banco-geo"
TRIP = BANCO + "/spatialbench/sf100/trip.parquet"
def di(que, **kw): print("E2S|" + que + "|" + json.dumps(kw, ensure_ascii=False, default=str), flush=True)
con = ore._duckdb()
for e in ("spatial", "h3", "httpfs"): con.execute("load " + e)
con.execute("set memory_limit = '40GB'"); con.execute("set threads = 8")
con.execute("create or replace secret banco (type http, bearer_token '%s', scope '%s')" % (ore._token_de_google(), BANCO))
t0 = time.time()
con.execute(f"create table p as select ST_X(g)::float x, ST_Y(g)::float y from (select ST_GeomFromWKB(t_pickuploc) g from read_parquet('{TRIP}'))")
di("leido", s=round(time.time() - t0), filas=con.execute("select count(*) from p").fetchone()[0])
di("caja", r=con.execute("select min(x), max(x), min(y), max(y), avg(x), avg(y) from p").fetchone())
di("rejilla_1g", top=[list(r) for r in con.execute("select floor(x)::int lo, floor(y)::int la, count(*) n from p group by all order by n desc limit 40").fetchall()])
di("celdas_1g", n=con.execute("select count(*) from (select distinct floor(x), floor(y) from p)").fetchone()[0])
di("continentes", r=[list(r) for r in con.execute("""select case when x < -30 then 'americas' when x < 60 then 'europa_africa' else 'asia_oceania' end z,
     case when y > 0 then 'norte' else 'sur' end h, count(*) from p group by all order by 3 desc""").fetchall()])
for res in (3, 5, 7):
    t0 = time.time()
    di("h3", res=res, celdas=con.execute(f"select count(distinct h3_latlng_to_cell(y, x, {res})) from p").fetchone()[0], s=round(time.time() - t0))
di("fin")
