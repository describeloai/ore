# 0048 · ORE IdP

**Estado:** aceptado y **en vivo** (2026-09-30). ORE es dueño de su IdP: lo corre, lo declara,
lo concilia y lo mide. `rubix` exige AAL2 en las tres puertas, y `rubix-interno` en las dos
suyas. Deudas 1 a 5 saldadas el 2026-10-02; lo que queda, al final.

## Qué es

**ORE IdP es quién eres.** El único sitio de la casa que ve una contraseña o una passkey, y el
que firma los tokens que todo lo demás verifica. No decide a qué perteneces ni qué puedes: eso
es de `ore-iam` (pertenencia y potestades) y lo pregunta cada módulo por `ore-acceso` (0047).

```
  persona / agente ──► ORE IdP ──token (iss, sub, aud)──► ore-serve, ore-iam, consola
                       quién eres                         ¿perteneces? ¿puedes? → ore-iam
```

| pieza | dónde | qué |
|---|---|---|
| **el servidor** | `malla/60-idp.yaml`, ns `identidad` | Keycloak 26.0.7, imagen cocida (`ore/idp`), una instancia, su Postgres en el clúster |
| **el emisor** | `https://login.paladio.io/realms/rubix` | la única entrada pública (`63-entrada-del-idp.yaml`). La consola de administración no sale: `port-forward` |
| **el operador** | `malla/59-*.yaml` | el de Keycloak y sus CRD, exportados de lo que corre; `prune: disabled` (borrar el CRD borra el IdP) |
| **la definición** | `identidad/realm.mjs` | flujos, passkeys, factores mínimos, correo, clientes de la consola |
| **lo de ORE y el manifiesto** | `identidad/ore.mjs` → `malla/61-realms.yaml` | audiencias `ore-serve` y `modelos`, el ámbito `basic` (el `sub`), el registro con su organización, la consola en local. **`realmsDeOre()` es el único realm deseado** |
| **el reconciliador** | `identidad/aplicar.mjs` | deja el realm vivo como `realmsDeOre()` dice, y también **resta**. `--plan` sólo hace GET y dice todo lo que cambiaría |
| **la guarda** | `identidad/medir.mjs`, `pruebas-de-fuego/el-segundo-factor.sh` (CI) | mide **recorriendo los flujos**, no leyendo un campo |
| **las copias** | `malla/62-copias-del-idp.yaml` | volcado con fecha fuera del clúster: el disco no cubre un `DROP` ni una migración de Keycloak que sale mal |
| **el aprovisionador** | `malla/68-el-admin-del-aprovisionador.sh` | dos identidades: una administra clientes (crea `ore-agente-<celda>`), la otra se presenta ante `ore-iam`. Separadas a propósito |
| **las llaves** | `malla/50-jwks.yaml` | el JWKS lo trae un Job; `ore-serve` no sale a buscarlo (0020) |

### Los realms

| realm | para quién | cómo se entra |
|---|---|---|
| **`rubix`** | las personas de los clientes y los agentes de cada celda | registro abierto (con su organización, 035). Entrada, reposición y registro con **dos factores**: passkey, o TOTP de recuperación. El registro enrola la passkey antes de abrir sesión |
| **`rubix-interno`** | nosotros (un empleado no es miembro de ninguna organización de cliente) | sin registro y sin organizaciones. Hoy no lo consume nadie: se separó antes de que hubiera dos poblaciones |

### Quién recibe un token

- **Una persona**, por `rubix-consola`: cliente público con PKCE y `ore-serve` en el `aud`.
- **Un agente**, por `ore-agente-<celda>`: cuenta de servicio con `rubix_tipo=agente`,
  `rubix_celda=<celda>` y tokens de 5 minutos. La crea el aprovisionador (paso ⑦) y la registra
  en `iam.agente`.
- **`ore-serve`** es una audiencia y no inicia sesión de nadie: existe para poder decir que un
  token es *para nosotros*.

### Lo que no cambia, a propósito

El realm `rubix`, el emisor, los flujos `browser-rubix`/`reposicion-rubix`, los clientes y los
claims `rubix_tipo`/`rubix_celda`/`organization`. Cambiar cualquiera cambia el `iss` o los
tokens, y pide una migración como la `034`. **Los nombres `rubix` son una identidad publicada,
no una marca.**

## De dónde viene

El IdP ya corría en ORE desde la mudanza (0020), pero su definición seguía en la plataforma
(`C:\Rubix\deploy\identidad`), escribiendo para el clúster viejo. El operador no estaba en ningún
repositorio (`kubectl apply` a mano el 2026-09-08), y `malla/gen-realm.py` hacía de puente.

Lo destapó la deuda del segundo factor (relajado el 2026-08-26, plazo 2026-09-30). El
reconciliador de la plataforma conciliaba un realm **sin lo de ORE**: ejecutado, habría encendido
`verifyEmail` sin correo y quitado `localhost` de la consola.

**Decidido:** ORE es dueño de la definición, el reconciliador, la imagen, el tema y el operador.
La plataforma es un cliente más del IdP (`RUBIX_OIDC_EMISOR`, `RUBIX_OIDC_AUDIENCIA` y su
verificación de tokens se quedan allí).

## Cómo se hizo

- **I1 · el generador y el reconciliador.** `realm.mjs`, traído tal cual. `ore.mjs`, portado de
  `gen-realm.py` (que se fue). **Medido: `rubix` y `rubix-interno` salen byte a byte iguales.**
  `rubix-dev` desapareció: no existe en vivo (034) y el reconciliador lo habría creado.
- **I2 · el operador y la imagen.** El operador y sus CRD, exportados de lo que corre:
  `kubectl diff --server-side` no muestra cambios salvo la anotación. La imagen es el target
  `idp` del `Dockerfile`, con las cinco opciones de build y el tema del correo; Cloud Build
  publica `idp:main` e `idp:<sha>`.
- **I3 · la guarda.** `medir.mjs` comprueba dos factores y uno resistente a phishing en la
  entrada, que lo declarado coincida con lo medido, la reposición sin rebajar, ninguna credencial
  de correo y ningún retorno en claro a otra máquina. Muerde: con `EXIGIR_SEGUNDO_FACTOR = false`
  sale con 1. En CI se exige además que `61-realms.yaml` sea **exactamente** lo que emite
  `ore.mjs`.

### En vivo (2026-09-30)

Primero `aplicar.mjs --plan`; después lo aplicó en `rubix` una persona, con el admin de arranque.

| | antes | después |
|---|---|---|
| la entrada | **1 factor** (declaraba `aal=AAL2`) | 2: passkey o TOTP |
| la reposición | 1 | 2 |
| **el registro** | **abría sesión con 1** | enrola la passkey antes de entrar |

- **La tercera puerta la encontró la prueba, no la guarda.** El registro no pasa por el flujo de
  entrada. Se cerró con `webauthn-register` como acción por defecto, y `medir.mjs` ganó la
  regla ⑥. Comprobado: el registro pide la passkey, una cuenta sin factor lo pide al entrar, y la
  consola entra con la passkey.
- **`rubix-consola-lector` sale de `realmsDeOre()`.** Nadie lo usa y su secreto vivía en el
  proyecto viejo. En vivo sigue existiendo, sin papeles.

## Lo que se decidió no hacer

- **Subir Keycloak a 26.2 para tener tokens por celda (RFC 8693): no.** Al medirlo (0047 M5)
  apareció un hueco mayor, que ese cambio no cerraba: registro abierto, celdas alcanzables desde
  internet y `ore-serve` sin comprobar pertenencia. Cualquier cuenta habría podido entrar en
  cualquier celda. Lo cierra **0047 A9′** sin tocar el IdP: la celda pregunta a `ore-iam`
  (`organizacion:leer`) si quien llega es de su organización. Si `ore-iam` no contesta, hay
  10 minutos de gracia para quien ya pasó; quien no, recibe 503. **La pertenencia la sabe
  `ore-iam`, no Keycloak.** Con eso, el token por celda queda reducido a un caso menor (una celda
  comprometida que reenvía un token a otra de sus propias organizaciones) y se aplaza.
- **Renombrar `rubix` → `ore`:** no (ver *Lo que no cambia*).

## Saldado (2026-10-02)

**Deuda 1 · retirar es nombrar.** Dejar de declarar un cliente no lo retiraba: el reconciliador
resta flujos, papeles, ámbitos y mapeadores, pero no clientes. Al medirlo salió uno peor que
`ore-agente`: **`iam-agente`**, el cliente de `98-los-cuatro-verbos.yaml`, era en `iam` una
*persona*, fundadora y única `ORGADMIN` de `prueba`. Desde A9′, un secreto de
`client_credentials`, sin segundo factor, administraba una organización.

- `RETIRADOS` en `ore.mjs`. `aplicar.mjs --plan` hace el **censo** de todo cliente vivo (de
  fábrica, declarado, gestionado fuera, retirado, desconocido); aplicar borra los retirados y lo
  dice. La prueba de fuego exige que ningún retirado vuelva al manifiesto.
- `prueba` era desechable y se retiró entera:
  - En `iam`, la 047: celda, agente, miembro, organización y `cofre_prueba`, con huella.
  - En GCP, el ⓪ del aprovisionador: secretos `t-prueba-*`, la copia, siete cuentas, el DNS y la
    organización `t-prueba` de la forja.
  - En el clúster, `t-prueba`.
- En vivo:
  - Borrados `iam-agente`, `ore-agente`, `ore-agente-prueba`, `ore-agente-prueba-dos` y
    `rubix-consola-lector`, y el Secret `identidad/iam-agente`.
  - El censo de después: sólo los de fábrica, los declarados, `ore-agente-demo`,
    `ore-agente-victor` y `ore-aprovisionador`. Ningún desconocido.
  - `demo` y `victor`, `/salud` 200, y sus agentes siguen entrando.
- **Dos cosas que la poda enseñó:**
  - La Kustomization `malla` va con `prune: false` (15-…), así que sacar `prueba` de `13-…` no la
    borra. Se borran a mano sus dos Kustomization (esas sí podan su inventario, namespace
    incluido), sus dos GitRepository y la cuenta de la cola.
  - El namespace se quedó en `Terminating` por un NEG de GKE con finalizador, sin ningún backend
    que lo usara. Borrar el NEG en GCP lo soltó.
- **El orden importa:** la poda va *después* de la 047. Con `prueba` activa y sin enganche a
  mano, el convergedor le habría renderizado uno.

**Deuda 4 · la identidad vieja, fuera de la plataforma** (`C:\Rubix` `a5afb08`). El clúster viejo ya
no existe, y su reconciliador era una trampa: escuchaba por defecto en `127.0.0.1:18080`, el
mismo puerto que el túnel de ORE al IdP vivo, y conciliaba sin lo de ORE. Fuera
`deploy/identidad`, `deploy/base/identidad`, `idp/`, `cloudbuild-idp.yaml` y cinco checks; queda
la lápida `MUDADO-A-ORE.md`.

**Deuda 5 · `rubix-interno` conciliado.** Su entrada exigía 1 factor; ahora 2, como `rubix`.

**Deuda 2 · la imagen de ORE, con su tema.** Corría `idp:26.0.7-1`, la cocida en la plataforma:
en el pod, `themes/` sólo tenía el README, y el realm pide `emailTheme: 'rubix'`.

- **Medido antes, las dos imágenes en local:**
  - `kc.sh show-config` persistido, idéntico: db=postgres, health, metrics, optimized.
  - El mismo usuario y el mismo entrypoint.
  - En el sistema de ficheros sólo cambian los jars generados (no reproducibles) y entra
    `themes/rubix/email`.
- `60-idp.yaml` la fija **por digest** (`idp@sha256:39bc18b8…`, que es `idp:main` desde el 30-sep),
  no por `main`: reiniciar el login se decide con un commit allí.
- Antes del cambio, una copia de la base (`keycloak-20261002T073535Z.dump`).
- **En vivo:**
  - El login, cortado ~1 min (09:37–09:38).
  - `--verificar`: AAL2 en los dos realms.
  - `victor` acepta tokens nuevos de agente y de persona.

**Deuda 3 · hay correo.** Estaba peor de lo que decía: `rubix` ofrecía «¿Olvidaste tu
contraseña?» y el relay **rechazaba**: `550`, la IP no era la registrada en Workspace
(`207.175.59.130`, del clúster viejo). Y la causa era de red: `sistema-spot` tenía IP pública
efímera y no salía por el NAT, lo que también desmentía la IP fija que se anuncia a los clientes.

- **C0**: `HAY_CORREO` (`ore.mjs`), un solo interruptor para la reposición y la verificación.
  Apagado mientras no hubo correo.
- **C1**: `sistema-spot`, nodos privados, y el NAT con puertos dinámicos. Todo el clúster sale
  por `34.156.87.237`.
  - ⛔ **Costó ~13 min de caída**: el nodo nuevo no cabía en la cuota de 12 vCPU (ver
    `malla/README.md`).
- **C2** (la persona): `34.156.87.237` registrada en el relay de Workspace, y la vieja fuera.
- **C3**: `aplicar.mjs` gobierna `smtpServer`. Producción firmaba «Rubix (desarrollo)».
- **C4**: `identidad/sonda-correo.sh` en verde (`MAIL FROM` y `RCPT TO` 250, sin `DATA`).
- **C5**: una reposición real, pedida desde la página de login. **Llegó bien.**
- **C6**: `HAY_CORREO = true`: reposición y `verifyEmail` encendidos en `rubix`. Se acaba la
  contradicción entre el SMTP declarado y `verifyEmail` apagado.

## Deuda y pistas

| # | deuda | por qué importa | pista |
|---|---|---|---|
| 6 | **Usuarios de prueba del registro** (los de la tercera puerta) | cuentas reales en el realm de producción | listarlos con el admin y borrarlos; comprobar que no quedan en `iam.pertenencia` |
| 7 | **El que concilia usa el admin de arranque** (`identidad/idp-initial-admin`) | es una credencial compartida y omnipotente, sin passkey | un admin nominal en `master` con passkey, y el de arranque, deshabilitado |
| 8 | **`MFA_RELAJADA_HASTA = '2026-09-30'`** sigue en `realm.mjs` | ya no tiene efecto (`EXIGIR_SEGUNDO_FACTOR = true`), pero confunde | quitarlo, o convertirlo en historia en el comentario |
| 9 | **Una sola instancia** | el IdP caído significa que nadie entra; las celdas aguantan con tokens vivos y la gracia de A9′ | medir antes qué pide una segunda instancia (la caché distribuida de Keycloak, la base compartida) y cuánto cuesta |
| 10 | **El token por celda** (aplazado, no olvidado) | sólo cubre el reenvío entre celdas de las propias organizaciones | si hace falta: 26.2 o superior, con operador, CRD e imagen subidos **a la vez** (I2 dejó los tres declarados) |
