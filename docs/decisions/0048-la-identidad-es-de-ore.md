# 0048 · La identidad es de ORE

**Estado:** aceptado · I1–I3 hechos en el código (2026-09-30); el realm vivo, por conciliar.

## El contexto, medido

El IdP (Keycloak 26.0.7) corre en el clúster de ORE desde la mudanza (0020): su base, su
entrada pública (`login.paladio.io`), sus copias (`malla/60`, `62`, `63`), el admin del
aprovisionador (`68`) y los agentes de cada celda (paso ⑦). Pero **su definición no**:

| pieza | estaba en | para qué clúster |
|---|---|---|
| los realms: flujos, passkeys, factores, correo, clientes (`realm.mjs`, 1.254 líneas) | la plataforma, `C:\Rubix\deploy\identidad` | el viejo: ns `rubix`, CR `rubix-idp` |
| el reconciliador vivo (`aplicar-entrada.mjs`) | la plataforma | el viejo: `rubix-idp-service`, secreto `rubix-idp-admin` |
| la imagen cocida del IdP | la plataforma, `idp/Dockerfile` | — (ORE usaba su etiqueta) |
| el tema del correo | la plataforma, un ConfigMap del viejo | en ORE, en ningún sitio |
| el operador de Keycloak | **ningún repositorio**: `kubectl apply` a mano el 2026-09-08 | el nuevo |
| el puente | ORE, `malla/gen-realm.py`: copiaba `salida/*.json` y añadía lo de ORE | — |

Lo destapó la deuda del segundo factor (relajado el 2026-08-26, plazo 2026-09-30): para
saldarla no había forma limpia de tocar el realm vivo. El script que lo haría buscaba el IdP
en otro clúster, y **conciliaba un realm sin lo de ORE**: ejecutado, habría encendido
`verifyEmail` (sin correo: quien se registra se queda esperando) y quitado `localhost` de la
consola.

## La decisión

**ORE es dueño de su identidad.** La definición, el reconciliador, la imagen, el tema y el
operador viven en ORE. La plataforma pasa a ser **un cliente más** del IdP (sus variables
`RUBIX_OIDC_EMISOR` y `RUBIX_OIDC_AUDIENCIA` y su verificación de tokens se quedan allí).

**Un solo realm deseado.** `identidad/ore.mjs` → `realmsDeOre()` es lo que el manifiesto
importa en un realm nuevo **y** lo que `identidad/aplicar.mjs` concilia en uno vivo.

**Lo que no cambia**, a propósito: el realm `rubix`, el emisor
`https://login.paladio.io/realms/rubix`, los flujos `browser-rubix`/`reposicion-rubix`, los
clientes y los claims `rubix_tipo`/`rubix_celda`. Cambiar cualquiera cambia el `iss` o los
tokens, y pide una migración como la `034`.

## Lo hecho

**I1 · el generador y el reconciliador** (`identidad/`)
- `realm.mjs`: la definición, traída **tal cual** (la regex de organización, dentro; el CLI,
  fuera). `ore.mjs`: lo que ORE añade, portado de `gen-realm.py` (que se va), y el manifiesto.
  **Medido: `rubix` y `rubix-interno` salen byte a byte iguales** que con `gen-realm.py`.
- `aplicar.mjs`: el reconciliador contra el IdP de ORE y con `realmsDeOre()`; el admin, sin
  valor por defecto (el de arranque del operador, `identidad/idp-initial-admin`).
- **`rubix-dev` ya no sale**: no existe en vivo (se renombró a `rubix`, 034), `ore-iam`
  rechaza su emisor, y el reconciliador lo **crearía** al no encontrarlo.

**I2 · lo demás del IdP**
- El operador y sus CRD, **exportados de lo que corre** (`malla/59-*.yaml`), con
  `prune: disabled`: borrar un CRD borra todos sus CR, y aquí eso es el IdP. Medido con
  `kubectl diff --server-side`: nada cambia salvo esa anotación.
- La imagen: target `idp` del `Dockerfile` (Keycloak cocido, las cinco opciones de build) con
  el tema del correo dentro (`emailTheme: 'rubix'`); Cloud Build la construye (`idp:main`,
  `idp:<sha>`). **La que corre la fija `malla/60-idp.yaml`** y cambiarla reinicia el login.

**I3 · las guardas**
- `identidad/medir.mjs` mide **recorriendo los flujos** (no leyendo un campo): dos factores y
  uno resistente a phishing en la entrada, lo declarado igual a lo medido, la reposición sin
  rebajar, sin credencial de correo, sin retornos en claro a otra máquina. Medido que muerde:
  con `EXIGIR_SEGUNDO_FACTOR = false`, sale con 1.
- `pruebas-de-fuego/el-segundo-factor.sh` (en CI) corre esa medida, exige las dos puertas en
  `REQUIRED` y que `61-realms.yaml` sea **exactamente** lo que emite `ore.mjs`.

## Lo que queda, y de quién

- **El realm vivo.** El artefacto dice AAL2; el realm, hasta que se concilie, no.
  `aplicar.mjs --verificar` y después sin él (la contraseña del admin por tubería: lo
  ejecuta una persona). Probar con un usuario sin segundo factor que Keycloak **pide
  registrarlo** y no bloquea.
- **La imagen nueva del IdP** (con el tema): pasar `60-idp.yaml` a una `idp:<sha>` cuando se
  decida el reinicio del login.
- **El correo.** El artefacto declara `smtp-relay.gmail.com` (por IP, sin credencial) y el
  registro apaga `verifyEmail` «porque no hay correo»: una de las dos cosas está vieja.
- **La plataforma.** Sus comprobaciones de identidad (`check-entrada.sh`, `check-idp.sh`,
  `medir-entrada.mjs`) importan su `realm.mjs`: retirarlas o apuntarlas aquí es suyo.
- Subir Keycloak a 26.2 (0047 M5, tokens por celda): operador, CRD e imagen a la vez.
