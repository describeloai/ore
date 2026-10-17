"""ORE 0067 G2·2 · `spatial` y `h3` en un puesto de victor sin salida a internet: se copian del bucket del
banco (`duckdb/v1.5.6/linux_amd64/`) con la cuenta del puesto. `cargar(c)` deja una conexión con las dos.
Hasta G2·3, que las mete en la imagen. Solo, emite «EXT|…»."""
import json, os, urllib.request

BUCKET = "project-8853a180-450d-47be-b83-banco-geo"
DIR = "/tmp/ext"


def token():
    r = urllib.request.Request("http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token",
                               headers={"Metadata-Flavor": "Google"})
    return json.load(urllib.request.urlopen(r))["access_token"]


def cargar(c):
    import duckdb  # noqa
    destino = f"{DIR}/v1.5.6/linux_amd64"
    os.makedirs(destino, exist_ok=True)
    for ext in ("spatial", "h3"):
        f = f"{destino}/{ext}.duckdb_extension"
        if not os.path.exists(f):
            r = urllib.request.Request(
                f"https://storage.googleapis.com/storage/v1/b/{BUCKET}/o/duckdb%2Fv1.5.6%2Flinux_amd64%2F{ext}.duckdb_extension?alt=media",
                headers={"authorization": f"Bearer {token()}"})
            open(f, "wb").write(urllib.request.urlopen(r).read())
    # `httpfs` y las demás vienen en la imagen, en su directorio: se cargan antes de cambiarlo.
    c.execute("set extension_directory = '/opt/ore/duckdb'")
    c.execute("load httpfs")
    c.execute(f"set extension_directory = '{DIR}'")
    c.execute("set allow_extensions_metadata_mismatch = true")
    c.execute("load spatial")
    c.execute("load h3")
    return c


if __name__ == "__main__":
    import duckdb, time
    t0 = time.time()
    try:
        c = cargar(duckdb.connect())
        print("EXT|carga|bien|", f"{time.time() - t0:.1f} s", c.execute("select ST_AsText(ST_Point(1, 2)), h3_latlng_to_cell(40.4, -3.7, 8)").fetchone())
    except Exception as e:
        print("EXT|carga|falla|", str(e).splitlines()[0][:300])
    print("## ok")
