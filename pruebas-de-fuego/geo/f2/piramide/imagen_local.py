"""ORE 0072 F2·3·2·3 · la imagen `ore-piramide:local` de punta a punta: lee una instantánea del lago de victor y
sube su pirámide al bucket del banco, con el token de quien lo lanza (de `gcloud`, por la entrada estándar: no
se imprime ni va en `argv`).

  python pruebas-de-fuego/geo/f2/piramide/imagen_local.py gs://…/metadata/….metadata.json nombre [suelo]
"""
import json
import subprocess
import sys

ml, nombre = sys.argv[1], sys.argv[2]
suelo = sys.argv[3] if len(sys.argv) > 3 else "14"
tok = subprocess.run("gcloud auth print-access-token", shell=True, capture_output=True, text=True, check=True).stdout.strip()
pet = {"fuente": {"metadata_location": ml, "geometria": "geom", "id": "id", "crs": "EPSG:4326",
                  "credencial": {"gcs.oauth2.token": tok}},
       "capa": nombre, "suelo": suelo,
       "destino": {"ubicacion": "gs://project-8853a180-450d-47be-b83-banco-geo/piramide/imagen-%s.pmtiles" % nombre,
                   "credencial": {"gcs.oauth2.token": tok}},
       "limites": {"segundos": "1800"}, "trabajo": "/tmp/p"}
p = subprocess.run(["docker", "run", "--rm", "-i", "ore-piramide:local"], input=json.dumps(pet).encode(),
                   capture_output=True)
print(p.stdout.decode("utf-8", "replace"))
print(p.stderr.decode("utf-8", "replace")[-800:])
print("salida", p.returncode)
