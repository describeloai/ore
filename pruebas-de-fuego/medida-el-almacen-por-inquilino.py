# -*- coding: utf-8 -*-
"""Medida: ¿puede el cofre de UN inquilino escribir en el Secret Manager del
proyecto compartido sin poder tocar lo de otro? Es lo que decide si la 0024-5
—el material en la celda— se puede hacer en `compartido` con un proyecto para
todos, o exige un proyecto por inquilino.

Lo que se prueba, desde DENTRO del pod del cofre de `demo` (Workload Identity
`ore-cofre-demo`), tras darle `secretmanager.admin` CON CONDICION de prefijo:

    resource.name.startsWith("projects/<numero>/secrets/t-demo-cofre-")

  1. crear      t-demo-cofre-medida        debe PODER
  2. crear      t-prueba-cofre-medida      debe NO poder   ← el create con condicion
  3. leer       t-prueba-forja-token       debe NO poder
  4. listar     el proyecto                debe NO poder   (la lista no tiene prefijo)
  5. añadir version y leerla en el suyo    debe PODER, y con CMEK de su llave

⛔ Lo que la PRIMERA pasada destapo, y son dos actos de plataforma (una vez):
   · el Secret Manager no tenia identidad de servicio en el proyecto
       gcloud beta services identity create --service=secretmanager.googleapis.com
   · y esa identidad tiene que poder cifrar con la llave de CADA organizacion
       gcloud kms keys add-iam-policy-binding <clave> --keyring=ore --location=europe-west1
         --member=serviceAccount:service-<numero>@gcp-sa-secretmanager.iam.gserviceaccount.com
         --role=roles/cloudkms.cryptoKeyEncrypterDecrypter
   Sin lo segundo el create con CMEK falla con FAILED_PRECONDITION. El aprovisionador
   lo hace en su paso ②. Y `--kms-key-name` no vale: solo admite replica automatica,
   que exige llave GLOBAL; con llave regional la CMEK va por `--replication-policy-file`.

⚠️ Deja puesta la vinculacion condicionada: es exactamente la que el
   aprovisionador va a hacer en su paso ③, y quitarla al final seria medir en
   un mundo que no es el de despues. Lo que si borra es el secreto de prueba.

    PYTHONIOENCODING=utf-8 python pruebas-de-fuego/medida-el-almacen-por-inquilino.py
"""
import os
import subprocess
import sys

PROYECTO = "project-8853a180-450d-47be-b83"
NUMERO = "339497864493"
LUGAR = "europe-west1"
INQ = "demo"
OTRO = "prueba"
SA = "ore-cofre-%s@%s.iam.gserviceaccount.com" % (INQ, PROYECTO)
PREFIJO = "projects/%s/secrets/t-%s-cofre-" % (NUMERO, INQ)
CMEK = "projects/%s/locations/%s/keyRings/ore/cryptoKeys/%s" % (PROYECTO, LUGAR, INQ)


def fuera(*args):
    r = subprocess.run(list(args), capture_output=True, text=True, encoding="utf-8", errors="replace",
                       timeout=120, shell=(args[0] == "gcloud"))
    return r.returncode, (r.stdout + r.stderr).strip()


def dentro(orden, antes=""):
    """gcloud DENTRO del pod del cofre: con SU identidad, no la mia."""
    env = dict(os.environ, MSYS_NO_PATHCONV="1")
    r = subprocess.run(["kubectl", "-n", "t-" + INQ, "exec", "deploy/ore-cofre", "-c", "cofre", "--",
                        "sh", "-c", antes + "HOME=/tmp CLOUDSDK_CONFIG=/tmp/.gcloud /google-cloud-sdk/bin/gcloud " + orden],
                       capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=120, env=env)
    return r.returncode, (r.stdout + r.stderr).strip().splitlines()[0] if (r.stdout + r.stderr).strip() else ""


def titulo(t):
    print()
    print(t)
    print("-" * len(t))


fallos = 0


def espera(nombre, codigo, salida, debe_poder):
    global fallos
    pudo = codigo == 0
    ok = pudo == debe_poder
    if not ok:
        fallos += 1
    print("   %s %-44s %s" % ("✓" if ok else "✗", nombre, ("pudo" if pudo else "no pudo") + ("" if ok else "  ← MAL")))
    if not ok or not pudo:
        print("       %s" % salida[:140])


# ── ① la vinculacion condicionada, desde fuera ───────────────────────────────
titulo("① LA VINCULACION: admin del almacen, SOLO bajo su prefijo")
print("   %s" % SA)
print("   roles/secretmanager.admin  si  resource.name.startsWith(\"%s\")" % PREFIJO)
c, s = fuera("gcloud", "projects", "add-iam-policy-binding", PROYECTO,
             "--member=serviceAccount:" + SA, "--role=roles/secretmanager.admin",
             "--condition=expression=resource.name.startsWith(\"%s\"),title=cofre-%s,description=el cofre de %s solo bajo su prefijo" % (PREFIJO, INQ, INQ),
             "--format=none")
print("   %s" % ("puesta" if c == 0 else "FALLO: " + s[:200]))
if c:
    sys.exit(2)
# La condicion tarda en propagarse; se reintenta el primer paso.
import time

# ── ② lo que puede y lo que no, desde dentro ─────────────────────────────────
titulo("② DESDE DENTRO DEL POD DEL COFRE DE `%s`" % INQ)
mio = "t-%s-cofre-medida" % INQ
ajeno = "t-%s-cofre-medida" % OTRO
for intento in range(6):
    # ⚠️ CMEK con replica en UNA region va por fichero de politica: `--kms-key-name`
    #   solo vale con replicacion automatica, que exige una llave GLOBAL — y la
    #   nuestra es de `europe-west1` a proposito.
    politica = '{"userManaged":{"replicas":[{"location":"%s","customerManagedEncryption":{"kmsKeyName":"%s"}}]}}' % (LUGAR, CMEK)
    c, s = dentro("secrets create %s --replication-policy-file=/tmp/rep.json --labels=proyecto=ore,inquilino=%s 2>&1"
                  % (mio, INQ), antes="printf '%%s' '%s' > /tmp/rep.json; " % politica)
    if c == 0 or "already exists" in s:
        break
    time.sleep(10)
espera("crear el suyo (%s) con CMEK de su llave" % mio, 0 if (c == 0 or "already exists" in s) else c, s, True)

c, s = dentro("secrets create %s --replication-policy=user-managed --locations=%s 2>&1" % (ajeno, LUGAR))
espera("crear uno con prefijo de OTRO (%s)" % ajeno, c, s, False)
if c == 0:
    # Si pudo, hay que borrarlo: es un secreto ajeno creado por quien no debia.
    fuera("gcloud", "secrets", "delete", ajeno, "--quiet")

c, s = dentro("secrets versions access latest --secret=t-%s-forja-token 2>&1" % OTRO)
espera("leer el testigo de la forja de OTRO", c, s, False)

c, s = dentro("secrets list --format='value(name)' 2>&1")
espera("listar el proyecto entero", c, s, False)

c, s = dentro("secrets versions add %s --data-file=- 2>&1" % mio, antes="printf hola | ")
espera("añadir una version al suyo", c, s, True)

c, s = dentro("secrets versions access latest --secret=%s 2>&1" % mio)
espera("leer la ultima version del suyo", c, s, True)

c, s = dentro("secrets versions describe latest --secret=%s --format='value(name)' 2>&1" % mio)
espera("saber QUE version es (describe latest)", c, s, True)
print("       %s" % s)

c, s = dentro("secrets describe %s --format='value(replication.userManaged.replicas[0].customerManagedEncryption.kmsKeyName)' 2>&1" % mio)
espera("el secreto esta cifrado con SU CMEK", 0 if s == CMEK else 1, s, True)

# ── ③ limpiar lo de prueba, no la vinculacion ────────────────────────────────
titulo("③ LIMPIEZA")
c, s = fuera("gcloud", "secrets", "delete", mio, "--quiet")
print("   %s borrado: %s" % (mio, "si" if c == 0 else s[:120]))
print("   la vinculacion condicionada se QUEDA: es la del aprovisionador ③")

print()
if fallos:
    print("  => %d comprobaciones MAL. Con esto la 0024-5 en `compartido` NO se sostiene con un proyecto." % fallos)
    sys.exit(1)
print("  => El almacen se parte por PREFIJO con una condicion IAM, incluido el create:")
print("     un proyecto para el compartido vale, y cada cofre solo ve `t-<n>-cofre-*`.")
