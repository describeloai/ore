"""ORE 0068 C3 · El previo de Q10–Q12: dónde se va el tiempo, antes de arreglar nada.

Lo ejecuta `medir.py` tras cargar las tablas (`PREVIO=lentas.py bash medir.sh …`), con su sesión `s`, su
`linea()` y su `vivos`. Una línea `C3P|…` por medida y, cada 10 s, la etapa de Spark que corre
(`C3P|etapa|…`: nombre, tareas hechas, activas y totales): la UI de Spark está apagada en la imagen.

  E1 · `zone`: cuántas, sus vértices (media, máximo, reparto) y las más grandes por nombre;
  E2 · el join que Q10 y Q11 tienen dentro (`trip` × `zone`, `ST_Within` del punto de recogida), contado;
  E3 · el mismo, con `zone` subdividida a ≤ N vértices (`ST_SubDivideExplode`), contado sin repetir;
  E4 · Q12 (KNN, `trip` × `building`), con su línea de tiempo.
Cada una con su límite (`LIMITE_S`), como medir.py.
"""
import threading
import time

PIEZAS = int(os.environ.get("PIEZAS", "256"))  # noqa: F821 — os, s, linea, LIMITE vienen de medir.py
SOLO_E = set(filter(None, os.environ.get("EXPERIMENTOS", "").split(",")))  # noqa: F821
st = s.sparkContext._jsc.sc().statusTracker()  # noqa: F821


def etapas(alto):
    """Cada 10 s, las etapas activas, hasta que `alto` se encienda."""
    visto = {}
    while not alto.is_set():
        try:
            for i in st.getActiveStageIds():
                si = st.getStageInfo(i)
                if si.isEmpty():
                    continue
                e = si.get()
                d = (e.numCompletedTasks(), e.numActiveTasks(), e.numTasks())
                if visto.get(i) != d:
                    visto[i] = d
                    linea("etapa", t="%.0f" % (time.time() - T0), id=i,  # noqa: F821
                          nombre=e.name().split(" at ")[0].replace(" ", "_")[:40],
                          hechas=d[0], activas=d[1], total=d[2], ejecutores=vivos["n"])  # noqa: F821
        except Exception:  # noqa: BLE001 — al cerrar
            pass
        alto.wait(10)


def medir(nombre, sql):
    if SOLO_E and nombre.split("-")[0] not in SOLO_E:
        return
    alto, r, t = threading.Event(), {}, time.time()
    threading.Thread(target=etapas, args=(alto,), daemon=True).start()
    df = s.sql(sql)  # noqa: F821
    p = df._jdf.queryExecution().executedPlan().toString()
    join = ",".join(j for j in ("BroadcastIndexJoin", "RangeJoin", "DistanceJoin", "KNN", "BroadcastNestedLoopJoin",
                                "SortMergeJoin", "BroadcastHashJoin") if j in p) or "-"
    grupo = "c3p-" + nombre

    def ir():
        s.sparkContext.setJobGroup(grupo, grupo, True)  # noqa: F821
        try:
            r["f"] = [tuple(x) for x in df.collect()]
        except Exception as e:  # noqa: BLE001
            r["e"] = [x for x in str(e).splitlines() if "Exception" in x or "Error" in x][-1:] or [str(e)[:200]]

    h = threading.Thread(target=ir, daemon=True)
    h.start()
    h.join(LIMITE)  # noqa: F821
    alto.set()
    if h.is_alive():
        s.sparkContext.cancelJobGroup(grupo)  # noqa: F821
        linea(nombre, estado="limite", s=LIMITE, join=join)  # noqa: F821
        h.join(120)
        return
    if "e" in r:
        linea(nombre, estado="error", s="%.1f" % (time.time() - t), join=join,  # noqa: F821
              error=r["e"][0][:240].replace(" ", "_"))
        return
    linea(nombre, estado="bien", s="%.1f" % (time.time() - t), join=join, filas=len(r["f"]))  # noqa: F821
    for f in r["f"][:25]:
        print("C3P|%s|fila|%r" % (nombre, f), flush=True)


ZONA = "ST_GeomFromWKB(z_boundary)"
medir("E1-zone", """
  select count(*) zonas, avg(v) media_vertices, max(v) max_vertices, sum(v) vertices,
         sum(if(v > 10000, 1, 0)) mas_de_10k, sum(if(v > 1000, 1, 0)) mas_de_1k,
         sum(if(v > %d, 1, 0)) mas_de_piezas
  from (select ST_NPoints(%s) v from zone)""" % (PIEZAS, ZONA))
medir("E1-mayores", """
  select z_zonekey, z_name, ST_NPoints(%s) v, round(ST_Area(%s), 3) area
  from zone order by v desc limit 15""" % (ZONA, ZONA))
medir("E2-join", """
  select count(*) pares from trip t, zone z
  where ST_Within(ST_GeomFromWKB(t.t_pickuploc), %s)""" % ZONA)
medir("E3-subdividido", """
  with piezas as (select z_zonekey, ST_SubDivideExplode(%s, %d) g from zone)
  select count(*) pares from (
    select distinct t.t_tripkey, p.z_zonekey from trip t, piezas p
    where ST_Within(ST_GeomFromWKB(t.t_pickuploc), p.g))""" % (ZONA, PIEZAS))
medir("E4-q12", consultas["q12"])  # noqa: F821
