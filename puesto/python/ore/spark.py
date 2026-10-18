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
    """La sesión de Spark del build: una, local (C2: un nodo; C3 la reparte), con el catálogo `ore`
    por defecto (un SQL que nombra `base.tabla` la encuentra) y el heap que el pod deja."""
    global _spark
    if _spark is None:
        from pyspark.sql import SparkSession

        b = (SparkSession.builder.master("local[*]").appName(os.environ.get("PUESTO") or "ore")
             .config("spark.driver.memory", _memoria_del_driver())
             .config("spark.sql.defaultCatalog", "ore"))
        for k, v in _catalogo("ore").items():
            b = b.config(k, v)
        _spark = b.getOrCreate()
    return _spark


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


def _geo_de(nombre):
    """`{columna: {"crs": …, "type": …}}`: lo que la tabla dice de sus columnas geo (`ore.geo.*`)."""
    base, ns, t = _partes(_corto(nombre))
    c, r = session.pedir("GET", "/v1/%s/namespaces/%s/tables/%s" % (base, ns, t))
    if c != 200:
        return {}
    props = ((r or {}).get("metadata") or {}).get("properties") or {}
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


def over(view):
    """Un dataset del lago como DataFrame de Spark, con sus columnas geo como geometría de Sedona
    con su SRID (de `ore.geo.<col>`)."""
    from pyspark.sql import functions as F

    from . import _leidas

    df = sesion().table(_tabla(view))
    # Lo leído va a la procedencia de lo que se escriba (fuera de un transform), como en Lightweight.
    if _corto(view) not in _leidas:
        _leidas.append(_corto(view))
    geo = _geo_de(view)
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
    return df.select(*cols)


def sql(query):
    """Spark SQL, con el catálogo `ore` por defecto. ⚠️ Las columnas geo de lo que se lee llegan
    como WKB (`binary`): `over()` las convierte, una sentencia no (C2·3 pieza 2b)."""
    return sesion().sql(query)


def _es_geometria(tipo):
    return type(tipo).__name__ in ("GeometryType", "GeometryUDT") or getattr(tipo, "typeName", lambda: "")() == "geometry"


def _contrato_geo(df, columnas):
    """De los datos, el CRS y el tipo de cada columna geo, en UNA pasada: un SRID por columna (0 o
    varios, no se escribe: lo mismo que niegan Python, Node y Java) y su subtipo (`Any` si hay
    varios)."""
    from pyspark.sql import functions as F

    aggs = []
    for c in columnas:
        aggs += [F.expr("collect_set(ST_SRID(`%s`))" % c).alias("s_" + c),
                 F.expr("collect_set(ST_GeometryType(`%s`))" % c).alias("t_" + c)]
    fila = df.agg(*aggs).first()
    contrato = {}
    for c in columnas:
        srids = [s for s in fila["s_" + c] if s is not None]
        if len(srids) != 1 or not srids[0]:
            raise ValueError("write(): `%s` has %s: a geometry column is written with ONE CRS "
                             "(ST_SetSRID / ST_Transform first)" % (c, "no CRS" if not srids or not srids[0]
                                                                     else "several CRS %s" % sorted(srids)))
        tipos = sorted({t[3:] if t.startswith("ST_") else t for t in fila["t_" + c] if t})
        sub = tipos[0] if len(tipos) == 1 else "Any"
        crs = "EPSG:%d" % srids[0]
        contrato[c] = {"crs": crs, "type": "Geometry<%s, %s>" % (sub, crs)}
    return contrato


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
    contrato = _contrato_geo(df, geos) if geos else {}
    salida = df.select(*[F.expr("ST_AsBinary(`%s`)" % c).alias(c) if c in contrato else F.col(c) for c in df.columns])
    destino = _tabla(nombre)
    procedencia = json.dumps(_procedencia(nombre, None), sort_keys=True)
    w = salida.writeTo(destino).using("iceberg").option("snapshot-property.ore.procedencia", procedencia)
    for c, g in contrato.items():
        w = w.tableProperty("ore.geo." + c, json.dumps(g, separators=(",", ":")))
    existe = sesion().catalog.tableExists(destino.replace("`", ""))
    if mode == "overwrite" or not existe:
        w.createOrReplace()
    else:
        w.append()
        # `append` no pone propiedades de tabla: las de geo, aparte (igual que las que ya tenía).
        for c, g in contrato.items():
            sesion().sql("alter table %s set tblproperties ('ore.geo.%s' = '%s')"
                         % (destino, c, json.dumps(g, separators=(",", ":")).replace("'", "\\'")))
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


__all__ = ["activo", "sesion", "over", "sql", "write", "DEFAULT"]
