"""ORE 0068 C2·3 · EL MOTOR HEAVYWEIGHT DEL SDK.

En un repositorio Heavyweight el build corre en `puesto-spark` (el puesto de Python más Spark 4.1 y
Sedona 1.9.1) con `ORE_MOTOR=heavyweight`, y los mismos nombres hacen lo mismo con otro motor
(decidido 2026-10-18): `over()` da un DataFrame de Spark, `write()` escribe uno, `sql()` es Spark SQL.

⭐ SPARK ESCRIBE SUS FICHEROS; LO DE ORE LO PONE ESTO. Spark hace el commit por el catálogo `/v1` de
ORE (el servidor juzga ahí las promesas, venga de quien venga), y el arnés pone en la escritura lo
que en Lightweight pone `ore-store`:
  · la geometría: cada columna de Sedona sale como WKB (`binary`), y la tabla dice su CRS y su tipo
    en `ore.geo.<col>`, sacados de los datos (un SRID por columna, o no se escribe);
  · la procedencia, en el resumen del snapshot (`ore.procedencia`), como `write()`;
  · el techo: un transform sólo escribe lo que declara.
Y al leer, lo contrario: una columna con `ore.geo.<col>` llega como geometría de Sedona con su SRID.

Medido antes de escribirlo (`pruebas-de-fuego/geo/c2/arnes-spark/`): `createOrReplace`, `overwrite`,
`append`, `tableProperty` y `snapshot-property` van por `/v1`; Sedona no toma un `binary` como
geometría sin `ST_GeomFromWKB`.
"""
import json
import os

from . import DEFAULT, _Result, _corto, _partes, _procedencia, session

TOKEN_DEL_POD = "/var/run/ore/pod/token"
_spark = None
# Las bases cuyo catálogo ya está dicho a Spark (`ore_<base>`, con su `warehouse`).
_bases = set()


def activo():
    """¿Corre este proceso un build Heavyweight? Lo dice el arnés (`ORE_MOTOR`)."""
    return os.environ.get("ORE_MOTOR") == "heavyweight"


def _memoria_del_driver():
    """El heap de la JVM, desde lo que mide el pod (`ORE_MEMORIA_MB`, que la plantilla iguala a
    `limits.memory`). Medido (pieza 0): la JVM ocupa el heap + ~0,8 GiB, y Python ~130 MB. Se dejan
    1,5 GiB fuera y del resto el 80 % es heap: 4 GiB → 2 GiB; 48 GiB → ~37 GiB."""
    mb = int(os.environ.get("ORE_MEMORIA_MB", "0") or 0)
    if mb <= 0:
        return "2g"
    return "%dm" % max(1024, int((mb - 1536) * 0.8))


def _identidad():
    """Las propiedades del catálogo que dicen quién habla: el puesto, y quien lo abrió.
    En el clúster, el token del pod leído en CADA petición (`IdentidadDelPuesto`); en el
    laboratorio, la identidad de cabecera (`ORE_SUJETO`), fija."""
    props = {}
    if session.id:
        props["header.x-ore-puesto"] = session.id
    sujeto = os.environ.get("ORE_SUJETO")
    if sujeto:
        props["header.x-ore-sujeto"] = sujeto
        return props
    fichero = os.environ.get("ORE_TOKEN_DEL_POD", TOKEN_DEL_POD)
    if not os.path.exists(fichero):
        raise RuntimeError("Heavyweight: this job has no pod identity (%s): Spark cannot talk to the catalog" % fichero)
    props.update({"rest.auth.type": "ore.spark.IdentidadDelPuesto",
                  "ore.identidad.fichero": fichero, "ore.identidad.cabecera": "x-ore-pod"})
    return props


def _catalogo(nombre, warehouse=None):
    """Las propiedades de un catálogo de Spark sobre el `/v1` de ORE."""
    props = {"": "org.apache.iceberg.spark.SparkCatalog", "type": "rest", "uri": session.servidor,
             "header.X-Iceberg-Access-Delegation": "vended-credentials",
             # S3 (R2 y el laboratorio) por ruta; para GCS no cuenta. El FileIO lo elige el esquema.
             "s3.path-style-access": "true", "client.region": "auto"}
    if warehouse:
        props["warehouse"] = warehouse
    props.update(_identidad())
    return {("spark.sql.catalog.%s.%s" % (nombre, k)).rstrip("."): v for k, v in props.items()}


def sesion():
    """La sesión de Spark del build: una, con el catálogo `ore` por defecto (un SQL que nombra
    `base.tabla` la encuentra) y el heap que el pod deja. Local (`local[*]`, un pod) salvo en un
    Build elástico (`ORE_SPARK_ELASTICO`, C3·c), donde este pod es el driver y los ejecutores los
    pide él a Kubernetes según la carga."""
    global _spark
    if _spark is None:
        from pyspark.sql import SparkSession

        elastico = _elastico()
        b = (SparkSession.builder.master(elastico.pop("master") if elastico else "local[*]")
             .appName(os.environ.get("PUESTO") or "ore")
             .config("spark.driver.memory", _memoria_del_driver())
             .config("spark.sql.defaultCatalog", "ore"))
        for k, v in list(_catalogo("ore").items()) + list((elastico or {}).items()):
            b = b.config(k, v)
        _spark = b.getOrCreate()
    return _spark


# ── C3·c · El Build elástico ──────────────────────────────────────────────────────────────────────
#
# Lo medido en victor (C3·c0, `pruebas-de-fuego/geo/c3/c0/`): el driver (este pod, en el pool
# `pesado-driver`) pide los ejecutores a Kubernetes con su cuenta (`driver-pesado`); ORE pone el suelo
# (1) y el techo (`ORE_SPARK_ELASTICO`: lo que dejan el pool y la cuota) y Spark decide por la carga.
# Cada ejecutor va a la cola de su celda (`cola-pesada`): Kueue le garantiza uno y le presta lo que la
# otra celda no usa (D1, medido: sin Kueue, el segundo Build pasaba ~100 s sin ejecutores).

EJECUTOR_CORES = 7
# 34g de heap + 4g de overhead + 4g de Python = 42 GiB por ejecutor: cabe en un n2-highmem-8.
EJECUTOR_HEAP, EJECUTOR_OVERHEAD, EJECUTOR_PYTHON = "34g", "4g", "4g"
EJECUTOR_DISCO = "250Gi"  # CE·6: de los 281 GiB asignables de la SSD local de un nodo pesado
PLANTILLA_DEL_EJECUTOR = "/tmp/ore-ejecutor.yaml"
CUENTA = "/var/run/secrets/kubernetes.io/serviceaccount"


def _plantilla_del_ejecutor():
    """El pod de un ejecutor, como lo exige la política de la celda: la cuenta `ejecutor` sin token,
    sin privilegios, en el pool pesado y con la entrada de la imagen. Spark pone encima la imagen,
    los recursos, `spark-role: executor` y el dueño (este pod)."""
    return "\n".join([
        "apiVersion: v1",
        "kind: Pod",
        "metadata:",
        "  labels: {ore.dev/spark: ejecutor}",
        "spec:",
        "  serviceAccountName: ejecutor",
        "  automountServiceAccountToken: false",
        "  securityContext: {runAsNonRoot: true, runAsUser: 65532, runAsGroup: 65532,"
        " seccompProfile: {type: RuntimeDefault}}",
        "  nodeSelector: {ore.dev/pool: pesado}",
        "  tolerations: [{key: ore.dev/pesado, operator: Equal, value: \"true\", effect: NoSchedule}]",
        "  containers:",
        "    - name: ejecutor",
        "      command: [\"bash\", \"/opt/ore/spark/ejecutor.sh\"]",
        "      workingDir: /tmp",
        "      env: [{name: HOME, value: /tmp}]",
        "      securityContext: {allowPrivilegeEscalation: false, capabilities: {drop: [ALL]}}",
        # CE·6 · Su disco, en la SSD local del pool: el *shuffle* va a un `emptyDir`. Spark añade la
        # CPU y la memoria a estos recursos, no los sustituye.
        "      resources: {requests: {ephemeral-storage: %s}, limits: {ephemeral-storage: %s}}"
        % (EJECUTOR_DISCO, EJECUTOR_DISCO),
        ""])


def _elastico():
    """La configuración del Build elástico, o `None` si este pod corre solo. `master` va aparte."""
    techo = int(os.environ.get("ORE_SPARK_ELASTICO", "0") or 0)
    if techo <= 0:
        return None
    with open(PLANTILLA_DEL_EJECUTOR, "w") as f:
        f.write(_plantilla_del_ejecutor())
    ns = open(CUENTA + "/namespace").read().strip()
    pod = os.environ["ORE_POD_NOMBRE"]
    return {
        "master": "k8s://https://kubernetes.default.svc:443",
        "spark.kubernetes.namespace": ns,
        "spark.kubernetes.driver.pod.name": pod,
        "spark.kubernetes.authenticate.caCertFile": CUENTA + "/ca.crt",
        "spark.kubernetes.authenticate.oauthTokenFile": CUENTA + "/token",
        "spark.driver.host": os.environ["ORE_POD_IP"],
        "spark.driver.port": "7078",
        "spark.driver.blockManager.port": "7079",
        "spark.blockManager.port": "7079",
        "spark.kubernetes.container.image": os.environ["ORE_SPARK_IMAGEN"],
        "spark.kubernetes.container.image.pullPolicy": "IfNotPresent",
        "spark.kubernetes.executor.podTemplateFile": PLANTILLA_DEL_EJECUTOR,
        "spark.kubernetes.executor.podTemplateContainerName": "ejecutor",
        "spark.kubernetes.executor.podNamePrefix": "ej-" + pod[-24:].strip("-"),
        "spark.kubernetes.executor.request.cores": str(EJECUTOR_CORES),
        "spark.kubernetes.executor.label.kueue.x-k8s.io/queue-name": "cola-pesada",
        "spark.kubernetes.executor.label.kueue.x-k8s.io/managed": "true",
        "spark.kubernetes.executor.annotation.kueue.x-k8s.io/pod-suspending-parent": "spark",
        "spark.kubernetes.decommission.script": "/opt/ore/spark/decom.sh",
        "spark.executor.cores": str(EJECUTOR_CORES),
        "spark.executor.memory": EJECUTOR_HEAP,
        "spark.executor.memoryOverhead": EJECUTOR_OVERHEAD,
        "spark.executor.pyspark.memory": EJECUTOR_PYTHON,
        "spark.pyspark.python": "python3",
        "spark.authenticate": "true",
        "spark.dynamicAllocation.enabled": "true",
        "spark.dynamicAllocation.shuffleTracking.enabled": "true",
        "spark.dynamicAllocation.shuffleTracking.timeout": "120s",
        "spark.dynamicAllocation.minExecutors": "1",
        "spark.dynamicAllocation.initialExecutors": "1",
        "spark.dynamicAllocation.maxExecutors": str(techo),
        "spark.dynamicAllocation.executorIdleTimeout": "60s",
        "spark.dynamicAllocation.schedulerBacklogTimeout": "1s",
        # c0: la sesión esperaba 30 s a que llegaran ejecutores; las tareas ya esperan a los suyos.
        "spark.scheduler.minRegisteredResourcesRatio": "0",
        "spark.decommission.enabled": "true",
        "spark.storage.decommission.enabled": "true",
        "spark.storage.decommission.shuffleBlocks.enabled": "true",
        "spark.storage.decommission.rddBlocks.enabled": "true",
        "spark.sql.shuffle.partitions": str(64 * techo),
        "spark.driver.maxResultSize": "4g",
    }


def _tabla(nombre):
    """El nombre de Spark de un nombre del árbol: `ore_<base>.<schema>.<tabla>`, en el catálogo de
    su base (el `prefix` de `/v1`, como Unity: 0038 P4). Lo dice a Spark la primera vez."""
    base, ns, t = _partes(_corto(nombre))
    cat = "ore_" + base
    if base not in _bases:
        s = sesion()
        for k, v in _catalogo(cat, warehouse=base).items():
            s.conf.set(k, v)
        _bases.add(base)
    return "`%s`.`%s`.`%s`" % (cat, ns, t)


def _metadatos(nombre):
    """`(código, metadata)` de la tabla por `/v1` (`loadTable`): su esquema vigente y sus propiedades."""
    base, ns, t = _partes(_corto(nombre))
    c, r = session.pedir("GET", "/v1/%s/namespaces/%s/tables/%s" % (base, ns, t))
    return c, (((r or {}).get("metadata") or {}) if c == 200 else (r or {}))


def _campos(md):
    """Los campos del esquema vigente de una tabla: `[{name, type, …}]`."""
    esquema = next((s for s in md.get("schemas", []) if s.get("schema-id") == md.get("current-schema-id")), None)
    return (esquema or md.get("schema") or {}).get("fields", [])


def _geo_de(nombre, md=None):
    """`{columna: {"crs": …, "type": …}}`: lo que la tabla dice de sus columnas geo (`ore.geo.*`)."""
    if md is None:
        c, md = _metadatos(nombre)
        if c != 200:
            return {}
    props = md.get("properties") or {}
    geo = {}
    for k, v in props.items():
        if k.startswith("ore.geo."):
            try:
                geo[k[len("ore.geo."):]] = json.loads(v)
            except ValueError:
                pass
    return geo


def _srid(crs):
    """`EPSG:25830` → 25830; `OGC:CRS84` → 4326 (Sedona no conoce `OGC:CRS84`, y su 4326 ya es
    lon/lat: medido en C2·2). Otro, `None`."""
    c = (crs or "").upper()
    if c == "OGC:CRS84":
        return 4326
    if c.startswith("EPSG:") and c[5:].isdigit():
        return int(c[5:])
    return None


# ORE 0068 C3·a · Lo que se difunde en un join espacial: un dataset geo con pocas filas Y pocos bytes.
#   Spark sólo difunde por los bytes de los ficheros (`autoBroadcastJoinThreshold`, 10 MB): 8 000
#   polígonos detallados pesan 37 MB, no se difundían y el join tardaba 1,6× más (previo de C3).
#   Medido en victor (20 M puntos, heap 38 g): difundir gana hasta ~100 k polígonos (24,9 s frente a
#   30,6 s de `RangeJoin`); con 300 k ya pierde (37,4 frente a 30,5 s), con 1 M por 3× y casi sin
#   memoria (41 de 48 GB), y 5 M no caben. El catálogo da las filas; el techo de bytes guarda de
#   pocas filas enormes.
DIFUNDIR_FILAS = 150_000
DIFUNDIR_BYTES = 48 << 20
# ORE 0068 C4·3b · En el build de un enriquecimiento (`ORE_ENRICH=1`, lo pone su celda) las capas son de
#   referencia: el tope que manda es el de filas, y el de bytes sube. Medido en victor (10 M de puntos, un
#   ejecutor): 8 000 polígonos de ~500 vértices (~64 MB) fuera del tope caían en `RangeJoin` de 13 tareas;
#   difundidas, la consulta pasó de ~870 a 592 s. Los ejecutores tienen 34 GiB de heap.
DIFUNDIR_BYTES_ENRICH = 512 << 20


def _tamano(md):
    """`(filas, bytes)` del snapshot vigente de una tabla, de su resumen; `(None, None)` si no lo dice."""
    actual = md.get("current-snapshot-id")
    snap = next((x for x in md.get("snapshots") or [] if x.get("snapshot-id") == actual), None)
    resumen = (snap or {}).get("summary") or {}
    try:
        return int(resumen["total-records"]), int(resumen["total-files-size"])
    except (KeyError, TypeError, ValueError):
        return None, None


def _se_difunde(filas, bytes_):
    """¿Se difunde un dataset geo de `filas` y `bytes_`? Sin saberlo, no: lo que hace Spark solo."""
    if filas is None or bytes_ is None:
        return False
    tope_f = int(os.environ.get("ORE_DIFUNDIR_FILAS") or DIFUNDIR_FILAS)
    por_defecto = DIFUNDIR_BYTES_ENRICH if os.environ.get("ORE_ENRICH") == "1" else DIFUNDIR_BYTES
    tope_b = int(os.environ.get("ORE_DIFUNDIR_BYTES") or por_defecto)
    return filas <= tope_f and bytes_ <= tope_b


def over(view):
    """Un dataset del lago como DataFrame de Spark, con sus columnas geo como geometría de Sedona
    con su SRID (de `ore.geo.<col>`). Uno geo pequeño (`_se_difunde`) va marcado para difundirse: en
    un join espacial, Sedona lo indexa entero en cada ejecutor (`BroadcastIndexJoin`). La marca
    sigue en SQL, porque las vistas de `sql()` salen de aquí (2b)."""
    from pyspark.sql import functions as F

    from . import _leidas

    df = sesion().table(_tabla(view))
    # Lo leído va a la procedencia de lo que se escriba (fuera de un transform), como en Lightweight.
    if _corto(view) not in _leidas:
        _leidas.append(_corto(view))
    c, md = _metadatos(view)
    if c != 200:
        return df
    geo = _geo_de(view, md)
    if not geo:
        return df
    cols = []
    for c in df.columns:
        g = geo.get(c)
        if g is None:
            cols.append(F.col(c))
            continue
        e = "ST_GeomFromWKB(`%s`)" % c
        s = _srid(g.get("crs"))
        if s is not None:
            e = "ST_SetSRID(%s, %d)" % (e, s)
        cols.append(F.expr(e).alias(c))
    df = df.select(*cols)
    return F.broadcast(df) if _se_difunde(*_tamano(md)) else df


def sql(query):
    """Spark SQL sobre los datasets del árbol, como `over()` los da (2b): ore-serve dice la consulta
    con cada nombre del árbol que lee cambiado por una vista temporal (`POST /puestos/{id}/sql`
    con `para: "spark"`), cada vista se registra con `over()` —la geometría, convertida y con su
    SRID— y se corre la consulta. Sin puesto (o sin un `a.b` en el texto), Spark tal cual, con el
    catálogo `ore` por defecto."""
    s = sesion()
    if not session.id or "." not in query:
        return s.sql(query)
    c, r = session.pedir("POST", "/puestos/%s/sql" % session.id, {"texto": query, "para": "spark"})
    if c != 200:
        raise RuntimeError("sql(): %s" % ((r or {}).get("error") if isinstance(r, dict) else r))
    p = r["spark"]
    for vista, nombre in p["vistas"]:
        over(nombre).createOrReplaceTempView(vista.strip("`"))
    return s.sql(p["consulta"])


def por_posicion(df, nombre, posiciones):
    """`insert into p.t select …` en Spark: una columna sin nombre toma la de la tabla en su
    posición, como `_por_posicion` en Lightweight (decidido 2026-09-24)."""
    nombre = _corto(nombre)
    c, md = _metadatos(nombre)
    primera = posiciones[0] + 1
    if c == 404:
        raise RuntimeError("insert into %s: the table does not exist yet, and column %d of the select has no name "
                           "to take: give it one (`… as name`)" % (nombre, primera))
    if c != 200:
        raise RuntimeError("insert into %s: ore-serve answered %s" % (nombre, c))
    de_la_tabla = [f["name"] for f in _campos(md)]
    columnas = list(df.columns)
    for p in posiciones:
        if p >= len(de_la_tabla):
            raise RuntimeError("insert into %s: column %d of the select has no name and the table only has %d: "
                               "give it one (`… as name`)" % (nombre, p + 1, len(de_la_tabla)))
        columnas[p] = de_la_tabla[p]
    return df.toDF(*columnas)


#: El tipo de Spark de un primitivo de Iceberg (lo que `insert` convierte al tipo de su columna).
_SPARK_DE_ICEBERG = {"boolean": "boolean", "int": "int", "long": "bigint", "float": "float", "double": "double",
                     "date": "date", "timestamp": "timestamp_ntz", "timestamptz": "timestamp", "string": "string",
                     "binary": "binary"}


def _tipo_spark(t):
    if not isinstance(t, str):
        return None
    if t.startswith("decimal("):
        return t
    return _SPARK_DE_ICEBERG.get(t)


def como_la_tabla(df, nombre):
    """`insert into b.s.d …` en Spark: cada columna, al tipo de la suya en el dataset, como
    `_como_la_tabla`. Una geometría no (la tabla la guarda en WKB; `write()` la convierte); si el
    dataset aún no existe, sus tipos son los de lo que llega."""
    from pyspark.sql import functions as F

    c, md = _metadatos(nombre)
    if c != 200:
        return df
    tipos = {f["name"]: _tipo_spark(f.get("type")) for f in _campos(md)}
    geo = {k[len("ore.geo."):] for k in (md.get("properties") or {}) if k.startswith("ore.geo.")}
    cols = []
    for f in df.schema.fields:
        destino = tipos.get(f.name)
        if destino and f.name not in geo and not _es_geometria(f.dataType) and f.dataType.simpleString() != destino:
            cols.append(F.col("`%s`" % f.name).cast(destino).alias(f.name))
        else:
            cols.append(F.col("`%s`" % f.name))
    return df.select(*cols)


def _es_geometria(tipo):
    return type(tipo).__name__ in ("GeometryType", "GeometryUDT") or getattr(tipo, "typeName", lambda: "")() == "geometry"


def _contrato_geo(df, columnas, de_la_tabla=None):
    """De los datos, el CRS y el tipo de cada columna geo, en UNA pasada: un SRID por columna (0 o
    varios, no se escribe: lo mismo que niegan Python, Node y Java) y su subtipo (`Any` si hay
    varios). Una columna sin un solo valor (un `append` vacío) no dice nada: vale lo que ya dice
    la tabla (`de_la_tabla`, su `ore.geo.*`), si lo dice."""
    from pyspark.sql import functions as F

    aggs = []
    for c in columnas:
        aggs += [F.expr("collect_set(ST_SRID(`%s`))" % c).alias("s_" + c),
                 F.expr("collect_set(ST_GeometryType(`%s`))" % c).alias("t_" + c)]
    fila = df.agg(*aggs).first()
    contrato = {}
    for c in columnas:
        srids = [s for s in fila["s_" + c] if s is not None]
        if not srids and not fila["t_" + c] and (de_la_tabla or {}).get(c, {}).get("crs"):
            g = de_la_tabla[c]
            contrato[c] = {"crs": g["crs"], "type": g.get("type") or "Geometry<Any, %s>" % g["crs"]}
            continue
        if len(srids) != 1 or not srids[0]:
            raise ValueError("write(): `%s` has %s: a geometry column is written with ONE CRS "
                             "(ST_SetSRID / ST_Transform first)" % (c, "no CRS" if not srids or not srids[0]
                                                                     else "several CRS %s" % sorted(srids)))
        tipos = sorted({t[3:] if t.startswith("ST_") else t for t in fila["t_" + c] if t})
        sub = tipos[0] if len(tipos) == 1 else "Any"
        crs = "EPSG:%d" % srids[0]
        contrato[c] = {"crs": crs, "type": "Geometry<%s, %s>" % (sub, crs)}
    return contrato


#: ORE 0072 F1·0 · El tamaño del grupo de filas de una tabla geo escrita desde Spark: ~4 MB son
#: ~100 000 puntos por grupo, lo que escribe `ore-store` (medido: la tesela de Madrid sobre 20 M
#: de puntos lee 2 de 210 grupos; con los 128 MB de serie, 5 de 16 y 6,2 M de filas).
GRUPO_GEO_BYTES = str(4 * 1024 * 1024)


def _hilbert_udf(x0, y0, x1, y1):
    """La clave Hilbert de `ore-store` (`lago_geo::ordenar_hilbert`): el centro de la caja de cada
    valor en una rejilla de 2¹⁶ × 2¹⁶ sobre la extensión `(x0, y0, x1, y1)` de los centros; las
    nulas, al final."""
    import numpy as np
    import pandas as pd
    from pyspark.sql.functions import pandas_udf

    @pandas_udf("long")
    def clave(cx, cy):
        n = 1 << 16
        nula = cx.isna().to_numpy() | cy.isna().to_numpy()

        def celda(v, lo, hi):
            if not hi > lo:
                return np.zeros(len(v), dtype=np.int64)
            return np.clip(np.round((np.nan_to_num(v.to_numpy(dtype="float64")) - lo) / (hi - lo) * (n - 1)), 0, n - 1).astype(np.int64)

        x, y = celda(cx, x0, x1), celda(cy, y0, y1)
        d = np.zeros(len(x), dtype=np.int64)
        s = n // 2
        while s > 0:
            rx = ((x & s) > 0).astype(np.int64)
            ry = ((y & s) > 0).astype(np.int64)
            d += s * s * ((3 * rx) ^ ry)
            gira = ry == 0
            voltea = gira & (rx == 1)
            x = np.where(voltea, n - 1 - x, x)
            y = np.where(voltea, n - 1 - y, y)
            x, y = np.where(gira, y, x), np.where(gira, x, y)
            s //= 2
        return pd.Series(np.where(nula, np.int64(1) << 62, d))

    return clave


def _con_cobertura(df, geos):
    """ORE 0072 F1·1/F1·2 · Lo que `ore-store` pone en una tabla geo, puesto aquí: la cobertura
    `<col>_bbox` {xmin, ymin, xmax, ymax} de cada columna de `geos` (al final, nula si el valor es
    nulo o vacío) y las filas en orden Hilbert por la primera. `df` lleva las geometrías de Sedona."""
    from pyspark.sql import functions as F

    caja = lambda c: F.when(F.col("`%s`" % c).isNull() | F.expr("ST_IsEmpty(`%s`)" % c), F.lit(None)).otherwise(
        F.struct(*[F.expr("ST_%s(`%s`)" % (f, c)).alias(n) for f, n in
                   (("XMin", "xmin"), ("YMin", "ymin"), ("XMax", "xmax"), ("YMax", "ymax"))]))
    for c in geos:
        df = df.withColumn(c + "_bbox", caja(c))
    if not geos:
        return df
    b = "`%s_bbox`" % geos[0]
    df = df.withColumn("__ore_cx", F.expr("(%s.xmin + %s.xmax) / 2" % (b, b))) \
           .withColumn("__ore_cy", F.expr("(%s.ymin + %s.ymax) / 2" % (b, b)))
    ext = df.agg(F.min("__ore_cx"), F.min("__ore_cy"), F.max("__ore_cx"), F.max("__ore_cy")).first()
    if ext[0] is None:
        return df.drop("__ore_cx", "__ore_cy")
    clave = _hilbert_udf(ext[0], ext[1], ext[2], ext[3])
    return (df.withColumn("__ore_h", clave("__ore_cx", "__ore_cy")).orderBy("__ore_h")
              .drop("__ore_h", "__ore_cx", "__ore_cy"))


def write(nombre, df, mode="overwrite"):
    """Escribe un DataFrame de Spark como el dataset `nombre`, por `/v1`, con lo de ORE puesto."""
    from pyspark.sql import DataFrame
    from pyspark.sql import functions as F

    if not isinstance(df, DataFrame):
        raise TypeError("write(): in a Heavyweight repository `data` is a Spark DataFrame (from `over()` or `sql()`), "
                        "not %s" % type(df).__name__)
    if mode not in ("overwrite", "append"):
        raise NotImplementedError("write(mode=%r): a Heavyweight repository writes `overwrite` or `append` (C2·3)" % mode)
    nombre = _corto(nombre, "write(): the name")
    geos = [f.name for f in df.schema.fields if _es_geometria(f.dataType)]
    destino = _tabla(nombre)
    existe = sesion().catalog.tableExists(destino.replace("`", ""))
    contrato = _contrato_geo(df, geos, _geo_de(nombre) if existe and mode == "append" else None) if geos else {}
    # ORE 0072 F1·2: la cobertura y el orden Hilbert, como `ore-store`. Una tabla tiene `<col>_bbox`
    # sólo si TODOS sus ficheros la llevan (quien pode por ella descartaría las filas sin ella): al
    # anexar a una que no la tiene, no se pone; llega con la próxima sobrescritura.
    if contrato:
        if mode == "overwrite" or not existe:
            cubiertas = list(contrato)
        else:
            # lo que tiene la tabla, del catálogo por el que se escribe (el mismo que hará el `append`)
            tiene = set(sesion().table(destino.replace("`", "")).columns)
            cubiertas = [c for c in contrato if c + "_bbox" in tiene]
        for c in cubiertas:
            contrato[c]["covering"] = c + "_bbox"
        df = _con_cobertura(df, cubiertas)
    salida = df.select(*[F.expr("ST_AsBinary(`%s`)" % c).alias(c) if c in contrato else F.col("`%s`" % c)
                         for c in df.columns])
    procedencia = json.dumps(_procedencia(nombre, None), sort_keys=True)
    # `ore.modo`, la palabra que pone `ore-store`: ORE dice con ella si la escritura reemplazó lo
    # que había (`derivedFrom`, el puntero). Sin ella, un `append` contaría como reemplazo.
    w = (salida.writeTo(destino).using("iceberg")
         .option("snapshot-property.ore.procedencia", procedencia)
         .option("snapshot-property.ore.modo", {"overwrite": "sobrescribir", "append": "anexar"}[mode]))
    # Las claves en orden, como las escribe `ore-store` (`covering`, `crs`, `type`).
    for c, g in contrato.items():
        w = w.tableProperty("ore.geo." + c, json.dumps(g, separators=(",", ":"), sort_keys=True))
    if contrato:
        w = w.tableProperty("write.parquet.row-group-size-bytes", GRUPO_GEO_BYTES)
    if mode == "overwrite" or not existe:
        w.createOrReplace()
    else:
        w.append()
        # `append` no pone propiedades de tabla: las de geo, aparte (igual que las que ya tenía).
        for c, g in contrato.items():
            sesion().sql("alter table %s set tblproperties ('ore.geo.%s' = '%s')"
                         % (destino, c, json.dumps(g, separators=(",", ":"), sort_keys=True).replace("'", "\\'")))
        if contrato:
            sesion().sql("alter table %s set tblproperties ('write.parquet.row-group-size-bytes' = '%s')"
                         % (destino, GRUPO_GEO_BYTES))
    # Lo que se escribió, del snapshot vigente que da el catálogo (sin volver a contar las filas).
    # ⚠️ No por la tabla de metadatos de Spark (`….snapshots`): Spark la pide como un namespace de
    #   dos niveles, `/v1` contesta 400 y sólo con un 404 la buscaría como tabla de metadatos.
    base, ns, t = _partes(nombre)
    c, r = session.pedir("GET", "/v1/%s/namespaces/%s/tables/%s" % (base, ns, t))
    m = ((r or {}).get("metadata") or {}) if c == 200 else {}
    actual = m.get("current-snapshot-id")
    snap = next((x for x in m.get("snapshots") or [] if x.get("snapshot-id") == actual), {})
    resumen = snap.get("summary") or {}
    filas = int(resumen.get("total-records") or 0)
    return _Result({"table": nombre, "rows": filas, "snapshot": str(actual or ""),
                    "metadata_location": (r or {}).get("metadata-location", "") if c == 200 else "",
                    "operation": resumen.get("operation", ""), "repeated": False, "mode": mode,
                    "added": int(resumen.get("added-records") or 0), "before": 0, "warnings": [],
                    "engine": "spark", "geo": contrato})


def cabeza(df, limite):
    """Las primeras `limite` filas de un DataFrame de Spark, en Arrow, para `table()`: la geometría,
    en WKB en hexadecimal, como viaja en texto (OOS v1alpha32 01 §5)."""
    from pyspark.sql import functions as F

    geos = {f.name for f in df.schema.fields if _es_geometria(f.dataType)}
    return df.select(*[F.expr("lower(hex(ST_AsBinary(`%s`)))" % c).alias(c) if c in geos else F.col("`%s`" % c)
                       for c in df.columns]).limit(limite).toArrow()


def ensayar_escritura(df, mode, limite):
    """ORE 0068 C2·3 pieza 5 · `write()` en un Preview Heavyweight: lo que escribiría, sin escribir.
    Lo mismo que comprueba `write()` (un DataFrame de Spark, el modo, un CRS por columna geo) y,
    para la consola, la forma del Preview de Lightweight (`table()`): las primeras `limite` filas
    —la geometría, en WKB en hexadecimal, como viaja en texto (OOS v1alpha32 01 §5)—, el total, y
    el tipo con que cada columna nacería en el lago (la geometría, `binary` con su `ore.geo`).
    `(vista, contrato_geo)`."""
    from pyspark.sql import DataFrame
    from pyspark.sql import functions as F

    from . import _tipo_iceberg, table

    if not isinstance(df, DataFrame):
        raise TypeError("write(): in a Heavyweight repository `data` is a Spark DataFrame (from `over()` or `sql()`), "
                        "not %s" % type(df).__name__)
    if mode not in ("overwrite", "append"):
        raise NotImplementedError("write(mode=%r): a Heavyweight repository writes `overwrite` or `append` (C2·3)" % mode)
    geos = [f.name for f in df.schema.fields if _es_geometria(f.dataType)]
    contrato = _contrato_geo(df, geos) if geos else {}
    total = df.count()
    if total == 0:
        raise ValueError("write(): the table has no rows")
    cabeza = df.select(*[F.expr("lower(hex(ST_AsBinary(`%s`)))" % c).alias(c) if c in contrato else F.col("`%s`" % c)
                         for c in df.columns]).limit(limite).toArrow()
    vista = table(cabeza, limite)
    vista["total"] = total
    ids = iter(range(len(cabeza.schema) + 1, 10**7))
    for c, f in zip(vista["columnas"], cabeza.schema):
        if f.name in contrato:
            c.update({"type": contrato[f.name]["type"], "iceberg": "binary", "geo": contrato[f.name]})
        else:
            c["iceberg"] = _tipo_iceberg(f.name, f.type, ids)
    return vista, contrato


class _Puente:
    """Lo que el Java de un build pide (`ore.Puente`, pieza 4): los mismos `over`, `sql` y `write` de
    este SDK, con el techo y todo. Los `Dataset<Row>` van por referencia (`_jdf`). Lo que lanza se
    guarda, para decirlo como es: py4j lo daría a Java envuelto en un `Py4JException`."""

    def __init__(self):
        self.error = None

    def _guardar(self, f):
        try:
            return f()
        except Exception as e:  # noqa: BLE001
            self.error = e
            raise

    def over(self, nombre):
        from . import over as _over

        return self._guardar(lambda: _over(nombre)._jdf)

    def sql(self, consulta):
        from . import sql as _sql

        return self._guardar(lambda: _sql(consulta)._jdf)

    def write(self, nombre, datos, modo):
        from pyspark.sql import DataFrame

        from . import write as _write

        return self._guardar(lambda: json.dumps(dict(_write(nombre, DataFrame(datos, sesion()), mode=modo)), default=str))

    class Java:
        implements = ["ore.Puente"]


#: Lo que dice py4j cuando lo que falló fue Python, llamado desde Java.
_DESDE_PYTHON = "An exception was raised by the Python Proxy"


def java(especificacion):
    """ORE 0068 C2·3 pieza 4 · **El build de un transform de Java en Spark.** El código Java corre
    en la JVM de Spark de este proceso: `ore.ArnesDeSpark` lo compila (javac, en el driver), carga
    su clase con D15 armado y halla el `@Transform`; aquí se le pone el techo —el `transform()` de
    siempre, con lo que la anotación declara— y se llama. Su `over()`, `sql()` y `write()`
    (`ore.Ore`, los mismos nombres con `Dataset<Row>`) vuelven a este SDK por `_Puente`: lo de ORE
    está en un solo sitio. Cada fallo va al informe con su tipo, su fichero y su línea, como en
    Lightweight. `especificacion`: `{fichero, metodo, fuentes}` y, en un Preview (pieza 5),
    `output`: la salida que el código del editor declara; entonces `write()` no escribe y lo que
    escribiría va al informe (`preview`), como en el arnés del Preview de Python."""
    from . import _ensayo

    salida = especificacion.get("output")
    if salida is None:
        return _java(especificacion, None)
    _ensayo(salida, especificacion["metodo"])
    try:
        _java(especificacion, salida)
    finally:
        visto = _ensayo(None)
    return visto


def _java(especificacion, preview):
    from py4j.protocol import Py4JJavaError
    from pyspark.java_gateway import ensure_callback_server_started

    from . import _informe_del_build, _para_el_informe, transform

    s = sesion()
    ensure_callback_server_started(s.sparkContext._gateway)
    puente = _Puente()
    jvm = s._jvm
    jvm.ore.Ore.puente(puente)
    fichero, metodo = especificacion["fichero"], especificacion["metodo"]

    def falla(tipo, mensaje, linea, en):
        _para_el_informe({"error": {"tipo": tipo, "fichero": en, "linea": linea}})
        return RuntimeError("%s (%s, line %d)" % (mensaje, en, linea) if linea else mensaje)

    def de_java(e):
        j = e.java_exception
        if j.getClass().getName() != "ore.ArnesDeSpark$Falla":
            return falla("runtime", "%s: %s" % (j.getClass().getSimpleName(), j.getMessage()), None, fichero)
        linea = j.linea()
        mensaje = j.getMessage()
        # Lo que falló en Python (el techo, una geometría sin CRS…) se dice como Python lo dijo,
        # con la línea del Java que lo pidió.
        if puente.error is not None and _DESDE_PYTHON in (mensaje or ""):
            mensaje = "%s: %s" % (type(puente.error).__name__, puente.error)
        return falla(j.tipo(), mensaje, linea if linea >= 0 else None, j.fichero())

    try:
        prep = jvm.ore.ArnesDeSpark.preparar(json.dumps({k: especificacion[k] for k in ("fichero", "metodo", "fuentes")}))
    except Py4JJavaError as e:
        raise de_java(e) from None

    def llamar():
        puente.error = None
        return prep.llamar()

    llamar.__name__ = metodo
    corre = transform(list(prep.inputs()), prep.output())(llamar)
    try:
        hecho = corre()
    except Py4JJavaError as e:
        raise de_java(e) from None
    if preview is None:
        _informe_del_build(corre.output, fichero, metodo, json.loads(hecho) if hecho else None)
        return
    from . import _ensayando

    visto = (_ensayando or {}).get("visto")
    if visto is None:
        raise falla("not-written", "`%s` returned without calling `write()`: there is nothing to preview" % metodo,
                    None, fichero)
    print("%s · preview of %s:%s · %d rows · nothing was written" % (visto["output"], fichero, metodo, visto["total"]))
    _para_el_informe({"preview": visto})


__all__ = ["activo", "sesion", "over", "sql", "write", "por_posicion", "como_la_tabla", "java", "DEFAULT"]
