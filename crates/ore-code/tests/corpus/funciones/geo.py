# ORE 0067 G2·3·6: una firma con geometría (OOS v1alpha32).
from ore import function
from ore.tipos import Geography, Geometry


@function
def area_de(parcela: Geometry["MultiPolygon", "EPSG:25830"]) -> float:
    return parcela.area


@function
def centro(zona: Geometry) -> Geometry["Point"]:
    return zona.centroid


@function
def tramos(ruta: Geography["LineString"], cortes: list[Geometry["Point", "EPSG:4326"]]) -> int:
    return len(cortes)
