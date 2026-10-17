"""G2·4·0 (b): la tabla de ORE (v3, `geom` binary) promovida a mano a `geometry` solo en metadatos,
como en G2·0: un esquema nuevo con el mismo id de campo, y un metadata.json siguiente."""
import copy, glob, json, os, sys, uuid
raiz = sys.argv[1]
md = sorted(glob.glob(raiz + "/metadata/*.metadata.json"))[-1]
m = json.load(open(md))
crs = json.loads(m["properties"]["ore.geo.geom"]).get("crs", "OGC:CRS84")
s = copy.deepcopy([x for x in m["schemas"] if x["schema-id"] == m["current-schema-id"]][0])
s["schema-id"] = max(x["schema-id"] for x in m["schemas"]) + 1
for f in s["fields"]:
    if f["name"] == "geom":
        f["type"] = "geometry" if crs == "OGC:CRS84" else "geometry(%s)" % crs
m["schemas"].append(s)
m["current-schema-id"] = s["schema-id"]
m["metadata-log"] = m.get("metadata-log", []) + [{"timestamp-ms": m["last-updated-ms"], "metadata-file": md}]
n = os.path.join(raiz, "metadata", "00001-%s.metadata.json" % uuid.uuid4())
json.dump(m, open(n, "w"), indent=1)
print("promovida:", n, "crs", crs, "props", {k: v for k, v in m["properties"].items() if k.startswith("ore.geo")})
