import json, pyproj
from pyproj import CRS
codigos = ["OGC:CRS84"] + [f"EPSG:{c}" for c in (
    [4326, 4258, 4230, 4979, 4937, 4269, 4267, 4171, 3857, 3395, 3035, 3034, 2154, 27700, 31370, 3006, 5514,
     2180, 3763, 2056, 28992, 3067, 3812, 5070, 3310, 2263, 4083, 4082, 32662]
    + list(range(25828, 25836)) + list(range(23028, 23032)) + list(range(26910, 26920))
    + list(range(32601, 32661)) + list(range(32701, 32761)))]
tabla = {"_fuente": f"pyproj {pyproj.__version__}, PROJ {pyproj.proj_version_str}, base EPSG de PROJ"}
for c in codigos:
    tabla[c] = CRS.from_user_input(c).to_json_dict()
s = json.dumps(tabla, separators=(",", ":"), ensure_ascii=False, sort_keys=True)
open("/salida/crs_projjson.json", "w", encoding="utf-8", newline="\n").write(s + "\n")
print(len(codigos), "CRS,", len(s) // 1024, "KB,", tabla["_fuente"])
