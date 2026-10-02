// ═══════════════════════════════════════════════════════════════════
// ✏️ 2026-09-30 · ESTE FICHERO VIVE EN ORE (ADR 0048, ORE IdP).
//
//   Vino de la plataforma (`C:\Rubix\deploy\identidad\realm.mjs`, db57ac0) tal
//   cual, con dos cambios: la regex de organización va dentro y el CLI se fue (lo
//   emite `identidad/ore.mjs`). Todo lo demás —los flujos, las passkeys, los
//   factores mínimos, el correo, los clientes— es el mismo texto, y sus razones con él.
// ═══════════════════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════════════════
// `M·23·0` · LA PLANTILLA DE REALM — un realm POR CELDA, y ninguno escrito a mano
//
// ── ⭐⭐⭐ POR QUÉ ES UN GENERADOR Y NO UN JSON DE EJEMPLO ──────────
//
//   La premisa ⓑ de [`63`](../../docs/canon/63-el-plano-de-identidad.md) dice **un realm
//   por celda**. Eso son N ficheros de ~200 líneas cada uno, y un fichero copiado es un
//   fichero que diverge: al tercer cliente, uno tendrá PKCE y otro no, y **eso no dará
//   error nunca**.
//
//   ⇒ La plantilla es CÓDIGO, la celda es el parámetro, y las guardas se escriben UNA vez.
//     Es la misma disciplina que `modelo/generador/`: el contrato se GENERA de la fuente.
//
// ── ⛔ Y LO QUE ESTE FICHERO NO PONE, a propósito ───────────────────
//
//   Ni un permiso. Ni un rol de recurso. Ni una política de `Authorization Services`. Los
//   roles que aquí aparecen son **de ORGANIZACIÓN** —lo que el IdP sabe decir— y no tienen
//   nada que ver con `lector`/`dueno`, que son papeles **sobre recursos** y viven en
//   `rubix.concesion`. Mezclarlos es la línea que `63` §2 dice que no se cruza.
//
//   ⚠️ Y por eso aquí NO hay grupos con permisos dentro: un grupo es un *userset*, y un
//     userset reabre `G·II` ([`64`](../../docs/canon/64-la-plataforma-gestionada.md) §3). El
//     día que haya grupos, MATERIALIZAN concesiones — hechos, no reglas — y eso es `M·24`.
//
//   uso:  node deploy/identidad/realm.mjs t_01J9ZK… [t_…]     → salida/<celda>.json
// ═══════════════════════════════════════════════════════════════════



/**
 * ⚠️ El patrón vive también en `modelo/puerta/sujeto.mjs` y en tres tablas. Aquí se repite
 *    en vez de importarse porque `deploy/` no depende de `modelo/` —la dirección de esta
 *    casa es una sola—, y lo que cierra el círculo es `check-idp` ①: coge el realm generado
 *    y comprueba que **el núcleo lo acepta como celda**. Se cierra ejecutando, no compartiendo.
 */
// ⭐⭐ DE LA HOJA, no deletreado — y esto se descubrió el 2026-08-27 al aplicar `73`.
//
//   Aquí vivía `/^t_[0-9A-HJKMNP-TV-Z]{26}$/`: la SEXTA copia de la gramática, y la que el
//   censo de `73` no vio **porque sólo miró `modelo/`**. Su propio aviso decía «no salió de
//   modelo/», y ésta es la prueba: el corte habría dejado el generador de realms exigiendo
//   el prefijo muerto, con el síntoma «no vale como alias de organización» en el despliegue
//   y no en las pruebas.
//
// ⚠️ Se importa de `modelo/identidad/autoridad.mjs` y no de otro sitio porque esa hoja NO
//    tiene un solo import: traerla no arrastra `shapes/` ni `facetas/` detrás.
// ✏️ ORE (0048): la regex, aquí dentro. En la plataforma se importaba de
//   `modelo/identidad/autoridad.mjs` (una hoja sin imports): `o_` y un ULID de Crockford.
const ORGANIZACION = /^o_[0-9A-HJKMNP-TV-Z]{26}$/;

// ✏️ ORE (0048): la cabecera y el CLI que emitían `salida/*.json` y el manifiesto del
//   clúster viejo (namespace `rubix`, CR `rubix-idp`) se fueron. Este módulo sólo DICE los
//   realms; los emite `identidad/ore.mjs`, con lo que ORE añade, a `malla/61-realms.yaml`.

/** ⭐ `P·0·c`: DOS realms, no N. Uno para nosotros, uno para TODOS los clientes. */
export const REALM_INTERNO = 'rubix-interno';
export const REALM_SAAS = 'rubix';

/** El claim de organización — el nombre que emite el ámbito estándar. */
export const CLAIM_ORG = 'organization';

/** El claim que dice de qué clase es el principal. ⭐ Lo LEE `modelo/puerta/sujeto.mjs`. */
export const CLAIM_TIPO = 'rubix_tipo';
/** Y a quién se emite: es el `aud` que `M·23b` exige verificar. */
export const AUDIENCIA = 'rubix-api';

// ═══════════════════════════════════════════════════════════════════
// `P·2` · LA ENTRADA — AAL2, y la vara es de fuera
// ═══════════════════════════════════════════════════════════════════

/**
 * ⭐⭐⭐ EL FLUJO DE ENTRADA, ESCRITO ENTERO — y por qué no se retoca el de fábrica
 *
 * ── ⛔⛔ QUÉ ESTABA MAL, y no daba error ────────────────────────────
 *
 *   El flujo `browser` de fábrica lleva un subflujo *Browser - Conditional OTP* con
 *   requisito **CONDITIONAL**: pide el segundo factor **sólo si el usuario ya lo tiene
 *   configurado**. Eso es *ofrecer* MFA.
 *
 *   ⭐ **AAL2 no la ofrece: la EXIGE.** NIST SP 800-63B-4 pide *«posesión y control de DOS
 *     factores distintos»*. Un condicional deja que cada persona decida su propio nivel de
 *     garantía — y el realm sigue diciendo que tiene MFA, porque la tiene configurada.
 *
 * ── ⭐⭐ LA FORMA, y cada requisito es la decisión ───────────────────
 *
 *   ```
 *   browser-rubix                        (top level)
 *     ├─ auth-cookie                     ALTERNATIVE   sesión ya establecida
 *     ├─ identity-provider-redirector    ALTERNATIVE   ⭐ el hueco por donde entrará `P·3`
 *     └─ …-formularios                   ALTERNATIVE
 *          ├─ auth-username-password-form  REQUIRED    ① algo que SABES
 *          └─ …-segundo-factor             REQUIRED    ⛔ REQUIRED, no CONDITIONAL
 *               ├─ webauthn-authenticator   ALTERNATIVE  ② algo que TIENES — passkey
 *               └─ auth-otp-form            ALTERNATIVE  ② TOTP, de RECUPERACIÓN
 *   ```
 *
 *   ⚠️ `identity-provider-redirector` se queda ALTERNATIVE a propósito: es la puerta del
 *      corredor de `P·3`. Cuando un cliente traiga su IdP, la garantía la declara ÉL — y
 *      esa frontera hay que dejarla escrita ahora, no descubrirla luego.
 *
 * ── ⭐ PASSKEY PRIMERO, TOTP DE RECUPERACIÓN — el atajo ⓒ, cobrado ──
 *
 *   CISA nombra resistentes a phishing **FIDO2/WebAuthn y PIV/CAC**, y excluye SMS, voz,
 *   enlaces por correo y el *push* de aprobar/denegar. ⇒ el orden de las dos ALTERNATIVE no
 *   es alfabético: **WebAuthn va primero** y TOTP existe para no dejar a nadie fuera.
 *
 *   ⛔ Y por eso TOTP entra **ya como recuperación** y no como el factor principal que luego
 *     habría que retirar: esa migración está escrita en la práctica de 2026, y **saltarla es
 *     más barato que hacerla**.
 *
 * ⚠️ Y el nombre lleva `-rubix`: tocar el flujo `browser` de fábrica lo marca como
 *    modificado y hace que cada actualización de Keycloak sea una negociación. Uno propio no.
 */
export const FLUJO_ENTRADA = 'browser-rubix';
const FORMULARIOS = `${FLUJO_ENTRADA}-formularios`;
const SEGUNDO_FACTOR = `${FLUJO_ENTRADA}-segundo-factor`;

/**
 * ⭐⭐⭐ EL FLUJO DE REPOSICIÓN — la puerta de atrás, y hasta hoy sin medir
 *
 * ⛔⛔ EL AGUJERO QUE ESTO CIERRA, dicho antes que nada:
 *
 *   `medir-entrada.mjs` recorría **`realm.browserFlow`** y nada más. La recuperación de
 *   contraseña usaba el flujo de FÁBRICA (`reset credentials`), que ni siquiera está en el
 *   artefacto ⇒ **ninguna guarda de este repositorio miraba esa puerta**.
 *
 *   Y lo que hay detrás importa: con el correo como único paso, **el buzón pasa a ser un
 *   factor equivalente a la contraseña**. Hoy no es una rebaja porque la entrada está
 *   relajada a uno; el **2026-09-30**, cuando `EXIGIR_SEGUNDO_FACTOR` vuelva a `true`, la
 *   puerta principal pediría DOS y ésta seguiría pidiendo UNO.
 *
 * ⚠️ NIST SP 800-63B-4 §6.1.2.3 es explícito: la recuperación **no puede rebajar el AAL**.
 *
 * ⭐ Por eso se GENERA en vez de heredar el de fábrica: lo que no está en el artefacto no se
 *   puede medir, y lo que no se mide acaba siendo por donde se entra.
 */
export const FLUJO_REPOSICION = 'reposicion-rubix';
const REPOSICION_SEGUNDO = `${FLUJO_REPOSICION}-segundo-factor`;

/**
 * ⏭️⛔ LA MFA, RELAJADA A PROPÓSITO — y con fecha de muerte
 *
 * ── QUÉ SE HA HECHO Y POR QUÉ ───────────────────────────────────────
 *
 *   `P·2` dejó el segundo factor en **REQUIRED**, que es lo que AAL2 exige. El 2026-08-26
 *   se baja a **CONDITIONAL** para levantar el primer login del prototipo: con REQUIRED, el
 *   primer usuario tiene que enrolar passkey o TOTP antes de poder entrar, y eso es
 *   fricción en el minuto en que lo que hace falta es **ver una sesión funcionando**.
 *
 * ⛔⛔ Y CONDITIONAL SIGNIFICA EXACTAMENTE ESTO: la entrada vuelve a ser de **UN factor**.
 *   No es «MFA opcional»: es que el realm **ya no opera a AAL2**, aunque sus atributos lo
 *   sigan diciendo. Por eso `check-entrada` se pone **ROJO** mientras esto valga `false` —
 *   una deuda que no se ve no es una deuda: es un olvido con buena letra.
 *
 * 🏁 SE RESTAURA cuando haya un usuario de verdad con su segundo factor enrolado — o el
 *   2026-09-30, lo que llegue antes. Cambiar esta constante a `true` y reaplicar.
 *
 * ⚠️ Y no se toca el FLUJO: sigue teniendo su rama de segundo factor con passkey y TOTP.
 *    Lo único que cambia es el requisito, que es lo que se puede devolver con una palabra.
 */
export const EXIGIR_SEGUNDO_FACTOR = true;
export const MFA_RELAJADA_HASTA = '2026-09-30';

/**
 * ⭐⭐⭐ LOS ENTORNOS — y por qué `localhost` NO puede estar en producción
 *
 * ── ⛔⛔ LO QUE HABÍA, Y ERA UN HUECO DE VERDAD ────────────────────
 *
 *   Hasta el 2026-08-27 había UNA lista con las dos cosas dentro:
 *
 *       RETORNOS = ['http://localhost:3000/auth/callback', 'https://app.paladio.io/…']
 *
 *   ⇒ el realm de PRODUCCIÓN entregaba códigos de autorización a `localhost`. Cualquiera
 *     que consiguiera que alguien completara un flujo con ese `redirect_uri` recibía el
 *     código **en un servicio corriendo en la máquina de esa persona**.
 *
 *   ⚠️ Y `check-entrada` ② lo daba por bueno, porque comprueba **comodines** y
 *      `http://localhost:3000/auth/callback` no lleva ninguno. Pasaba en verde.
 *
 * ── ⭐⭐ QUÉ HACE LA INDUSTRIA, que es de donde sale esta forma ────
 *
 *     Auth0     un TENANT por entorno            Okta      ORGS separadas
 *     Clerk     *Development instance* y *Production instance*, y la de producción
 *               **EXIGE un dominio real**: no te deja meter `localhost`
 *     Cognito   un *user pool* por entorno
 *
 *   ⇒ El invariante es el mismo en los cuatro: **el almacén de identidad se duplica por
 *     entorno, y `localhost` existe SÓLO en el de desarrollo.** Keycloak no trae el
 *     concepto, así que se implementa con lo que sí trae: un realm por entorno.
 *
 * ── ⛔ Y POR QUÉ LA IDENTIDAD SE SEPARA **PRIMERO** ──────────────────
 *
 *   Todo lo demás en Rubix se **replaya desde `rubix.outbox`**: el grafo, el catálogo, las
 *   proyecciones. El IdP no — `014-sujeto.sql` ya lo dejó escrito: es **AUTORIDAD, no
 *   proyección**. ⇒ un error de desarrollo en cualquier otro plano se deshace; borrar gente
 *   o recrear flujos en el IdP, no. Y `aplicar-entrada.mjs` recrea flujos en cada iteración.
 *
 * ⚠️ Lo que esto NO es: un segundo entorno. Sigue habiendo un clúster, una instancia de
 *    Keycloak y una base. Lo separado es el plano que no se puede reconstruir.
 */
export const ENTORNOS = Object.freeze({
  produccion: Object.freeze({
    sufijo: '',
    nombre: 'Rubix',
    // ⛔ NI UN `http://`. Es la regla entera, y `check-entrada` ② la exige.
    retornos: Object.freeze(['https://app.paladio.io/auth/callback']),
    salidas: Object.freeze(['https://app.paladio.io/']),
    origenes: Object.freeze(['https://app.paladio.io']),
  }),
  desarrollo: Object.freeze({
    sufijo: '-dev',
    // ⭐ El nombre se ve en la página de login y en el correo: quien entre aquí **sabe que
    //   no es producción** sin tener que mirar la URL.
    nombre: 'Rubix (desarrollo)',
    retornos: Object.freeze(['http://localhost:3000/auth/callback']),
    salidas: Object.freeze(['http://localhost:3000/']),
    origenes: Object.freeze(['http://localhost:3000']),
  }),
});

/** ⛔ Falla si el entorno no existe. Un `?? ENTORNOS.produccion` habría hecho que una
 *  errata en el nombre generase silenciosamente el realm de PRODUCCIÓN — que es justo la
 *  clase de fallo que esta separación existe para impedir. */
export function entornoDe(nombre) {
  const e = ENTORNOS[nombre];
  if (!e) throw new Error(`entorno desconocido: ${nombre} (hay: ${Object.keys(ENTORNOS).join(', ')})`);
  return e;
}


/** Un paso del flujo. ⚠️ `autheticatorFlow` va con la errata: es el nombre real del campo
 *  en la representación de Keycloak, y escribirlo bien lo haría desaparecer en silencio. */
const paso = (quien, requisito, prioridad, esFlujo = false) => ({
  ...(esFlujo ? { flowAlias: quien } : { authenticator: quien }),
  requirement: requisito,
  priority: prioridad,
  autheticatorFlow: esFlujo,
  userSetupAllowed: false,
});

const flujo = (alias, descripcion, ejecuciones, topLevel = false) => ({
  alias,
  description: descripcion,
  providerId: 'basic-flow',
  topLevel,
  builtIn: false,
  authenticationExecutions: ejecuciones,
});

/** Los tres flujos que sostienen la exigencia. Van juntos: partirlos rompe el alias. */
export function flujosDeEntrada() {
  return [
    flujo(FLUJO_ENTRADA, 'AAL2 · NIST SP 800-63B-4 — dos factores, sin condicional', [
      paso('auth-cookie', 'ALTERNATIVE', 10),
      paso('identity-provider-redirector', 'ALTERNATIVE', 20),
      paso(FORMULARIOS, 'ALTERNATIVE', 30, true),
    ], true),
    flujo(FORMULARIOS, 'contraseña y SEGUNDO FACTOR', [
      paso('auth-username-password-form', 'REQUIRED', 10),
      // ⛔ Aquí muerde `EXIGIR_SEGUNDO_FACTOR`. REQUIRED = AAL2. CONDITIONAL = un factor.
      //   La rama de abajo NO cambia: sigue ofreciendo passkey y TOTP. Lo que cambia es si
      //   se puede pasar de largo — y eso es toda la diferencia entre ofrecer y exigir.
      paso(SEGUNDO_FACTOR, EXIGIR_SEGUNDO_FACTOR ? 'REQUIRED' : 'CONDITIONAL', 20, true),
    ]),
    flujo(SEGUNDO_FACTOR, 'passkey primero, TOTP de recuperación', [
      paso('webauthn-authenticator', 'ALTERNATIVE', 10),
      paso('auth-otp-form', 'ALTERNATIVE', 20),
    ]),
  ];
}

/**
 * ⭐⭐⭐ LA PRESENTACIÓN DEL CORREO — y la caducidad, que es lo que más se nota
 *
 * ── ⛔⛔ LOS 5 MINUTOS DE FÁBRICA SON UNA TRAMPA ────────────────────
 *
 *   Medido contra el realm vivo el 2026-08-26: `actionTokenGeneratedByUserLifespan = 300`.
 *   Cinco minutos **para un enlace que viaja por correo**.
 *
 *   ⇒ Entre que el relay lo entrega, Gmail lo clasifica y la persona lo ve, cinco minutos
 *     se van. Y lo que se encuentra entonces no es «caducado» con una explicación útil: es
 *     una página de error. Así que vuelve a pedirlo — y vuelve a tardar.
 *
 * ⭐ 30 minutos: bastante para que llegue y se lea, poco para que un buzón comprometido
 *   sirva de mucho. Y **sólo para `reset-credentials`**: la clave lleva el sufijo de la
 *   acción, así que la verificación de correo y las acciones de administrador siguen en 5.
 *
 * ⚠️ No se toca `actionTokenGeneratedByUserLifespan` a secas: eso las aflojaría todas, y
 *    ninguna de las otras viaja por un canal tan lento.
 *
 * ── ⭐ Y EL IDIOMA NO NECESITA TEMA PROPIO ───────────────────────────
 *
 *   `internationalizationEnabled` + `defaultLocale` bastan: Keycloak trae los idiomas en el
 *   tema `base`. Lo que SÍ necesita tema propio es la MARCA — y por eso `emailTheme` apunta
 *   a `rubix`, que se monta desde un ConfigMap (ver `deploy/base/kustomization.yaml`).
 *
 * ⛔ `loginTheme` se queda SIN tocar, y no es olvido: un tema de login lleva CSS, fuentes e
 *   imágenes, y eso no cabe en un ConfigMap. Entra el día que se recueza la imagen.
 */
export function presentacionDelCorreo() {
  return {
    emailTheme: 'rubix',
    internationalizationEnabled: true,
    defaultLocale: 'es',
    supportedLocales: ['es', 'en'],
  };
}

/** ⭐ Los atributos de caducidad que acompañan al correo. Van aparte porque en Keycloak
 *  viven en `attributes` y no en campos propios — y ahí se mezclan con la declaración de
 *  garantía, así que conviene poder fusionarlos sin pisar nada. */
export function caducidadDelEnlace() {
  return {
    // 30 minutos, y SÓLO para la reposición de credenciales.
    'actionTokenGeneratedByUserLifespan.reset-credentials': '1800',
  };
}

/**
 * ⭐⭐⭐ EL CORREO DE SALIDA — y la mejor credencial es la que no existe
 *
 * ⛔⛔ LA AVERÍA QUE ESTO CIERRA: hasta el 2026-08-26 el realm tenía `resetPasswordAllowed`
 *   y `verifyEmail` en `true` con `smtpServer` **VACÍO**. ⇒ la página de login llevaba días
 *   enseñando *«Forgot Password?»* y quien lo pulsara se quedaba esperando un correo que
 *   nadie iba a enviar. Y no fue teoría: ese mismo día `VERIFY_EMAIL` dejó a una persona
 *   fuera de su propia cuenta por esta misma causa.
 *
 * ── ⭐⭐⭐ POR QUÉ EL RELAY, Y NO UN App Password ─────────────────
 *
 *   Se llegó aquí por descarte, y los dos descartes son medidas:
 *
 *     ⓐ el `routing` de Gmail —donde vive el relay— **no tiene superficie de API**: ni
 *        `gcloud`, ni GAM, ni un service account con delegación de dominio. Se dio por
 *        bloqueado y se eligió el App Password.
 *     ⓑ y el App Password tampoco salió: la página contesta *«not available for your
 *        account»*, que en Workspace es lo normal — Google los desactiva por defecto.
 *
 *   ⇒ Las dos ramas acababan en la consola de administración. Y si hay que entrar igual,
 *     se entra a hacer lo bueno.
 *
 * ── ⭐⭐ LO QUE GANA, Y NO ES POCO ─────────────────────────────────
 *
 *     credencial   ⛔ **NINGUNA** — autentica por IP de origen. Nada que guardar, nada que
 *                  rotar, nada que se pueda filtrar: la mejor credencial es la que no existe.
 *     remitente    `no-reply@paladio.io`, y **sin crear el buzón**: el relay admite cualquier
 *                  dirección del dominio. ⭐ Esto BORRA la deuda de que un correo automático
 *                  saliera a nombre de una persona.
 *     cupo         10 000/día en vez de 2 000, y ligado al DOMINIO y no a nadie.
 *
 * ── ⚠️ Y LO QUE CUESTA, dicho ─────────────────────────────────────
 *
 *   Con `Only addresses in my domains` + IP, **cualquier cosa que salga por esa IP puede
 *   enviar como `@paladio.io`**. Un App Password habría sido un permiso más estrecho, y
 *   conviene no fingir lo contrario.
 *
 *   ⇒ Lo que lo hace sostenible es que la IP esté **fijada a propósito**. ✏️ En ORE (0048,
 *     deuda 3, 2026-10-02) es `salida-a-origenes` (34.156.87.237, `MANUAL_ONLY`), el NAT por
 *     el que sale TODO el clúster desde que `sistema-spot` es privado. La de antes,
 *     `paladio-publica` (207.175.59.130), era del clúster viejo y se fue con él: el correo
 *     dejó de salir y nadie relacionó las dos cosas, que es la avería que este párrafo
 *     avisaba. ⇒ `identidad/sonda-correo.sh` lo mide, y decide `HAY_CORREO` (`ore.mjs`).
 *
 * ⛔ Y este objeto sigue sin llevar `password` — ahora porque no existe ninguna.
 *   `check-entrada` ⑥ lo comprueba igual: una guarda no se relaja porque hoy no haya nada
 *   que proteger; se relaja el día que alguien vuelve a poner algo y ya nadie mira.
 */
export function correoDeSalida(nombreVisible = 'Rubix') {
  return {
    // ⚠️ `smtp-relay.gmail.com`, NO `smtp.gmail.com`. Son servicios distintos: el segundo
    //    exige SMTP AUTH de una cuenta concreta; éste autoriza por IP y envía a nombre del
    //    dominio. Confundirlos da un error de autenticación que no menciona el host.
    host: 'smtp-relay.gmail.com',
    // 587 + STARTTLS, no 465. El 465 es SMTPS implícito; 587 es el puerto de envío estándar
    // (RFC 6409) y el único que abre `allow-egress-correo-idp`.
    port: '587',
    starttls: 'true',
    ssl: 'false',
    // ⭐⭐ AQUÍ ESTÁ TODO: sin autenticación SMTP ⇒ no hay `user` ni `password` que poner,
    //   ni secreto que crear, ni rotación que recordar. Lo autoriza la IP de salida.
    auth: 'false',
    // ⭐ Un remitente que NO es una persona — y el relay lo permite sin que el buzón exista.
    from: 'no-reply@paladio.io',
    // ⛔⛔ EL NOMBRE DEL REMITENTE LO PONE EL REALM, y no es cosmética. Desde que hay un
    //   realm por entorno (`72`), producción y desarrollo mandan correos de recuperación
    //   **desde la misma dirección y con el mismo asunto**. En una bandeja de entrada son
    //   indistinguibles — y son enlaces que cambian la contraseña de cuentas DISTINTAS.
    //
    //   ⇒ `Rubix` frente a `Rubix (desarrollo)` en la columna de remitente, que es donde de
    //     verdad se mira. Y sale del `displayName` del realm: una sola fuente, así que el
    //     día que un realm se renombre el correo le sigue sin que nadie se acuerde.
    fromDisplayName: nombreVisible,
    // ⚠️ Pero las respuestas tienen que llegar a alguien: `no-reply` no tiene buzón, así que
    //    responder a un correo de recuperación rebotaría en silencio.
    replyTo: 'victor@paladio.io',
    replyToDisplayName: nombreVisible,
  };
}

/**
 * ⭐⭐⭐ EL FLUJO DE REPOSICIÓN — y en qué se aparta del de fábrica, con el motivo
 *
 * ⛔① SIN `reset-otp`. El de fábrica lo lleva, y lo que hace es **borrar el segundo
 *    factor**. Aquí el segundo factor se *valida* (`auth-otp-form` / passkey) o no hay.
 *
 * ⛔② Y EL ORDEN CAMBIA: el segundo factor va **ANTES** de `reset-password`, no después.
 *    El de fábrica pone la contraseña nueva y pregunta el OTP a continuación — así que
 *    quien sólo controle el buzón ya ha cambiado la contraseña antes de que nadie le pida
 *    el segundo factor. ⇒ se prueba primero y se cambia después.
 *
 * ⭐ Y la exigencia **no se escribe dos veces**: la rama de segundo factor usa el MISMO
 *   `EXIGIR_SEGUNDO_FACTOR` que la entrada. Por construcción, la puerta de atrás pide lo
 *   mismo que la de delante — y el día que alguien toque una sola, la guarda lo dice.
 */
export function flujoDeReposicion() {
  return [
    flujo(FLUJO_REPOSICION, 'Reposición de credenciales — sin rebajar el AAL de la entrada', [
      paso('reset-credentials-choose-user', 'REQUIRED', 10),
      paso('reset-credential-email', 'REQUIRED', 20),
      paso(REPOSICION_SEGUNDO, EXIGIR_SEGUNDO_FACTOR ? 'REQUIRED' : 'CONDITIONAL', 30, true),
      paso('reset-password', 'REQUIRED', 40),
    ], true),
    flujo(REPOSICION_SEGUNDO, 'segundo factor VALIDADO — nunca borrado', [
      paso('webauthn-authenticator', 'ALTERNATIVE', 10),
      paso('auth-otp-form', 'ALTERNATIVE', 20),
    ]),
  ];
}

/**
 * ⭐⭐ LA POLÍTICA DE PASSKEYS — y el campo que de verdad decide el nivel
 *
 *   `webAuthnPolicyUserVerificationRequirement: 'required'` obliga al autenticador a
 *   **verificar a la persona** (PIN o biometría) antes de firmar. Sin él, una llave robada
 *   es un factor completo: con él, la llave sola no vale.
 *
 * ⚠️ `authenticatorAttachment` queda sin especificar A PROPÓSITO: fijarlo a `platform`
 *    excluiría las llaves físicas, y a `cross-platform` excluiría el móvil de todo el mundo.
 *    ⛔ Una política de MFA que deja a la mitad de la gente sin poder cumplirla se acaba
 *    apagando entera.
 */
export function politicaDePasskeys(nombre) {
  return {
    webAuthnPolicyRpEntityName: nombre,
    webAuthnPolicyRpId: '',
    webAuthnPolicySignatureAlgorithms: ['ES256', 'RS256'],
    webAuthnPolicyUserVerificationRequirement: 'required',
    webAuthnPolicyAttestationConveyancePreference: 'not specified',
    webAuthnPolicyAuthenticatorAttachment: 'not specified',
    webAuthnPolicyRequireResidentKey: 'not specified',
    webAuthnPolicyCreateTimeout: 0,
    // ⛔ Que la misma llave no se registre dos veces: dos credenciales del mismo trasto
    //   parecen dos factores en el listado y son uno.
    webAuthnPolicyAvoidSameAuthenticatorRegister: true,
    webAuthnPolicyAcceptableAaguids: [],
  };
}

/**
 * ⭐⭐⭐ LA DECLARACIÓN, EN EL ARTEFACTO — no en un documento
 *
 *   *«Se declara»* no es escribirlo en un `.md`: es que el realm lo lleve encima y se pueda
 *   leer sin preguntarle a nadie. Estos atributos viajan con el realm y sobreviven al export.
 *
 * ── ⚠️ Y FAL1, no FAL2, aunque duela ────────────────────────────────
 *
 *   NIST SP 800-63C-4 es explícito: **FAL2 exige que la aserción vaya CIFRADA a la clave
 *   pública del RP**. Nosotros la firmamos (RS256) y verificamos `aud` y PKCE — eso es
 *   exactamente el perfil de FAL1.
 *
 *   ⛔ Declarar FAL2 porque *«tenemos PKCE y todo bien montado»* sería inventarse la vara.
 *     El valor de una declaración está entero en que sea **comprobable por un tercero**.
 *
 *   ⇒ El hueco tiene nombre: **cifrar el ID Token al RP**. Y FAL3 tiene otro: ligar la
 *     aserción a una clave que el suscriptor demuestre poseer (DPoP o mTLS).
 */
export function declaracionDeGarantia() {
  return {
    'rubix.aal': 'AAL2',
    'rubix.aal.norma': 'NIST SP 800-63B-4',
    'rubix.fal': 'FAL1',
    'rubix.fal.norma': 'NIST SP 800-63C-4',
    'rubix.fal.falta-para-fal2': 'cifrar la aserción a la clave pública del RP',
  };
}

/**
 * Los autenticadores que **verifican una credencial**. Todo lo demás mueve a la persona por
 * el flujo sin comprobar nada de ella.
 *
 * ⚠️ Lista CERRADA a propósito: si Keycloak trae uno nuevo y alguien lo mete en el flujo,
 *    `factoresMinimos` lo contará como **cero** y la guarda saldrá roja. ⭐ Ese es el
 *    comportamiento correcto — un autenticador desconocido no se presume factor.
 */
export const CREDENCIALES = Object.freeze([
  'auth-username-password-form',
  'auth-otp-form',
  'webauthn-authenticator',
  'webauthn-authenticator-passwordless',
  'auth-spnego',
  'direct-grant-validate-password',
  'direct-grant-validate-otp',
  'recovery-authn-code-form',
]);

/**
 * ⛔ Los dos pasos que abren un camino SIN autenticar, y que no son un agujero:
 *
 *     auth-cookie                    la sesión YA estaba establecida — se autenticó antes
 *     identity-provider-redirector   delega en OTRO IdP, que declara SU propia garantía
 *     organization                   ⭐ igual que el anterior, por el IdP de la Organization
 *
 * ⭐ Van nombrados uno a uno, no por categoría. Si alguien añade un CUARTO, cuenta como
 *   camino de cero factores y la guarda cae — que es exactamente lo que queremos que pase.
 *
 * ⚠️ `organization` entró el 2026-08-26 al medir el flujo de FÁBRICA vivo: con Organizations
 *    encendido, Keycloak monta una rama *Organization Identity-First Login* al mismo nivel
 *    que `forms`, y sin nombrarla el flujo de fábrica medía **0** en vez de **1**. ⇒ el
 *    «antes» y el «después» de `P·2` no habrían sido comparables, que es como se cuelan las
 *    mejoras que no lo son.
 */
/**
 * ⭐⭐ LO QUE CUENTA COMO FACTOR **EN UNA REPOSICIÓN**, que no es lo mismo que al entrar
 *
 *   En el flujo de entrada, la contraseña prueba quién eres. En el de reposición **no hay
 *   contraseña que probar** — es justo la que se ha perdido—, así que la prueba es otra:
 *
 *     reset-credential-email   demuestra CONTROL DEL BUZÓN     → sí es un factor
 *     auth-otp-form            *valida* un OTP                → sí
 *     webauthn-authenticator   passkey de segundo factor       → sí
 *
 * ⛔ Los dos que PARECEN pasos y no prueban nada:
 *
 *     reset-credentials-choose-user   dice quién DICE ser. No lo demuestra.
 *     reset-password                  es el RESULTADO, no la prueba
 *
 * ⛔⛔⛔ Y `reset-otp`, QUE ES LO CONTRARIO DE UN FACTOR — leído del propio servidor:
 *
 *     «Removes existing OTP configurations (if chosen) and sets the 'Configure OTP'
 *      required action»
 *
 *   ⇒ **no verifica el segundo factor: lo BORRA.** El subflujo *Reset - Conditional OTP*
 *     de fábrica no comprueba nada; es la puerta por la que se le quita a alguien su MFA.
 *
 *   ⚠️ Contarlo habría dado el número MÁS ALTO justo al flujo que **destruye** seguridad.
 *     Se descubrió preguntando al servidor por la descripción del proveedor, no leyendo el
 *     nombre — y el nombre decía exactamente lo contrario de lo que hace.
 */
export const CREDENCIALES_DE_REPOSICION = Object.freeze([
  'reset-credential-email',
  'auth-otp-form',
  'webauthn-authenticator',
  'webauthn-authenticator-passwordless',
  'recovery-authn-code-form',
]);

/**
 * ⛔⛔ PASOS QUE **QUITAN** UN FACTOR. No valen 0: son negativos.
 *
 *   Un flujo de reposición que los ejecute deja la cuenta con MENOS garantía de la que
 *   tenía al empezar — y lo hace en el momento en que la persona no puede autenticarse,
 *   que es justo cuando menos se le puede pedir que se dé cuenta.
 *
 * ⭐ Van nombrados uno a uno, como `PASOS_QUE_NO_AUTENTICAN`. Si aparece un cuarto, la
 *   guarda no lo verá — y por eso la guarda también mira los que NO conoce.
 */
export const PASOS_QUE_QUITAN_FACTOR = Object.freeze(['reset-otp']);

export const PASOS_QUE_NO_AUTENTICAN = Object.freeze([
  'auth-cookie', 'identity-provider-redirector', 'organization',
]);

/**
 * ⭐⭐⭐ CUÁNTOS FACTORES EXIGE, DE VERDAD — recorriendo el grafo, no leyendo un campo
 *
 * ── ⛔⛔ POR QUÉ ESTO NO ES UN `grep` ────────────────────────────────
 *
 *   Un realm puede tener `otpPolicyType: 'totp'`, una política de contraseñas impecable y
 *   MFA **que no se exige nunca**, porque el subflujo que la pide es CONDITIONAL. Los tres
 *   campos salen verdes por separado. Lo único que dice la verdad es **el camino más corto
 *   de la raíz al éxito**, y eso hay que recorrerlo.
 *
 * ── LA SEMÁNTICA DE KEYCLOAK, que es la que decide la cuenta ────────
 *
 *     REQUIRED      tienen que pasar TODOS      ⇒ se SUMAN
 *     ALTERNATIVE   basta con UNO               ⇒ se toma el MÍNIMO
 *     CONDITIONAL   puede no ejecutarse         ⇒ ⛔ cuenta CERO
 *     DISABLED      no se ejecuta               ⇒ cero
 *
 * ⚠️ CONDITIONAL contando cero es el corazón del asunto: es exactamente lo que hace el flujo
 *    `browser` de fábrica, y por eso «tenemos MFA configurada» y «exigimos MFA» son dos
 *    frases distintas.
 *
 * @param {object} realm el realm generado
 * @returns {number} el número de credenciales del camino más BARATO hasta entrar
 */
export function factoresMinimos(realm, opciones = {}) {
  // ⭐ Parametrizable, y con los MISMOS defectos de siempre: `factoresMinimos(realm)` sigue
  //   midiendo la entrada. Lo que se añade es poder apuntar el mismo medidor a OTRA puerta.
  //   ⛔ Dos medidores distintos para dos puertas serían dos varas, y la comparación entre
  //     ellas —que es lo Único que de verdad importa— dejaría de significar nada.
  const credenciales = opciones.credenciales ?? CREDENCIALES;
  const flujos = new Map((realm.authenticationFlows ?? []).map((f) => [f.alias, f]));
  const raiz = opciones.flujo ?? realm.browserFlow;
  if (!raiz || !flujos.has(raiz)) return 0;

  const contar = (alias, visto = new Set()) => {
    // ⛔ Un ciclo no se cuenta como camino barato: se descarta con infinito.
    if (visto.has(alias)) return Infinity;
    const f = flujos.get(alias);
    if (!f) return Infinity;
    const dentro = new Set(visto).add(alias);

    let requeridos = 0;
    let corre = false;
    const alternativas = [];
    for (const e of f.authenticationExecutions ?? []) {
      const quien = e.autheticatorFlow ? e.flowAlias : e.authenticator;
      const coste = e.autheticatorFlow
        ? contar(quien, dentro)
        : (credenciales.includes(quien) ? 1 : 0);

      switch (e.requirement) {
        case 'REQUIRED': requeridos += coste; corre = true; break;
        case 'ALTERNATIVE': alternativas.push(coste); corre = true; break;
        // ⛔ Puede saltarse ⇒ el camino barato lo esquiva y no paga nada.
        case 'CONDITIONAL': break;
        default: break;
      }
    }

    // ⛔⛔ UN SUBFLUJO QUE NO EJECUTA NADA NO ES UN CAMINO DE CERO FACTORES: ES QUE NO HAY
    //   CAMINO. Y la diferencia no es teórica — la destapó el flujo de FÁBRICA vivo el
    //   2026-08-26: con Organizations encendido, Keycloak monta una rama *Organization* al
    //   mismo nivel que `forms` cuyo único hijo es CONDITIONAL. Contándola como 0, el flujo
    //   de fábrica medía **0 factores**; y lo que hace de verdad es no aplicar y dejar pasar
    //   al siguiente ALTERNATIVE.
    //
    // ⚠️ Y por eso se distingue SUBFLUJO de HOJA: una hoja **siempre se ejecuta**, así que
    //    una hoja desconocida en posición ALTERNATIVE sí es una puerta trasera de coste 0.
    //    Un subflujo vacío, no. Colapsar los dos casos rompería uno de los dos negativos.
    if (!corre) return Infinity;

    // Si hay REQUIRED, las ALTERNATIVE no abren camino propio: se ejecutan igualmente.
    if (requeridos > 0 || alternativas.length === 0) return requeridos;
    return Math.min(...alternativas);
  };

  // ⭐ Los pasos que NO autentican se apartan ANTES de medir: `auth-cookie` daría siempre
  //   cero y taparía cualquier exigencia que hubiera debajo.
  const copia = structuredClone(realm);
  const flujosCopia = new Map(copia.authenticationFlows.map((f) => [f.alias, f]));
  for (const f of flujosCopia.values()) {
    f.authenticationExecutions = (f.authenticationExecutions ?? [])
      .filter((e) => e.autheticatorFlow || !PASOS_QUE_NO_AUTENTICAN.includes(e.authenticator));
  }
  flujos.clear();
  for (const [k, v] of flujosCopia) flujos.set(k, v);

  const n = contar(raiz);
  return Number.isFinite(n) ? n : 0;
}

/** El mapper que estampa el tipo. ⛔ `hardcoded-claim`: un valor FIJO por cliente, no un
 *  atributo del usuario — un atributo lo puede editar quien administre el directorio, y
 *  entonces una persona podría declararse agente. */
const mapperTipo = (valor) => ({
  name: `rubix-tipo-${valor}`,
  protocol: 'openid-connect',
  protocolMapper: 'oidc-hardcoded-claim-mapper',
  consentRequired: false,
  config: {
    'claim.name': CLAIM_TIPO,
    'claim.value': valor,
    'jsonType.label': 'String',
    'access.token.claim': 'true',
    'id.token.claim': 'false',
    'userinfo.token.claim': 'false',
    // ⛔ Y NO al `introspection`: cuanto menos superficie repita el claim, menos sitios
    //   donde pueda decir otra cosa.
    'access.tokenResponse.claim': 'false',
  },
});

/** El mapper de audiencia. ⭐ Sin él, `aud` no nos nombra y `M·23b` rechaza TODO — que es
 *  el comportamiento correcto, y por eso este mapper es obligatorio en cada cliente. */
const mapperAudiencia = () => ({
  name: 'rubix-audiencia',
  protocol: 'openid-connect',
  protocolMapper: 'oidc-audience-mapper',
  consentRequired: false,
  config: {
    'included.client.audience': AUDIENCIA,
    'access.token.claim': 'true',
    'id.token.claim': 'false',
  },
});

/**
 * ⭐ Un cliente de AGENTE: credenciales de cliente, sin persona detrás… **de momento**.
 *
 * ⚠️ Y conviene saber lo que HOY no vale: un token de `client_credentials` a secas lo
 *    rechaza `identidadDeClaims`, porque un agente solo no tiene a quién no exceder. Este
 *    cliente existe para que el agente tenga IDENTIDAD PROPIA —que es lo que `M·23c` va a
 *    intercambiar por un token con `sub` + `act`—, no para entrar por su cuenta.
 */
/**
 * ⭐⭐⭐ EL MAPPER DE ORGANIZACIÓN — y sus dos ajustes NO son preferencias.
 *
 * ── ⛔⛔ `multivalued: true`, y esto es lo importante ──
 *
 *   Medido sobre el código fuente de 26.0.7 (`OrganizationMembershipMapper.resolveValue`):
 *
 *       simple        →  organizations.get(0).getAlias()      ⛔ UNA ARBITRARIA, sin aviso
 *       multivaluado  →  { "<alias>": { … }, … }              ✅ TODAS
 *
 *   Con el defecto —simple— una persona que pertenezca a dos organizaciones recibe **una de
 *   las dos sin criterio**, y con ella vería el inquilino equivocado. No falla y no grita.
 *
 *   ⇒ Multivaluado, para que el núcleo pueda VER que hay dos y rechazarlas. La guarda vive
 *     en `organizacionDeClaims`; esto es lo que le da algo que mirar.
 *
 * ⚠️ Y la otra mitad: el mapper hace `return` si la petición **no trae el ámbito de
 *    organización**. Por eso el ámbito va como DEFECTO en el cliente, no como opcional: si
 *    dependiera de que el cliente lo pida, un cliente distraído obtendría un token sin
 *    organización — y sin organización el núcleo no deja entrar, que es lo correcto, pero el
 *    síntoma sería «no entra nadie» en vez de «falta un ámbito».
 */
/**
 * ✔✔ 2026-08-28 · ESTO YA NO SE AÑADE AL CLIENTE. Se APLICA sobre el mapper que el
 *   ámbito integrado `organization` ya trae.
 *
 * ⛔⛔ EL FALLO QUE LO MOTIVA, medido con `generate-example-access-token`:
 *
 *     [ "o_01J…", { "o_01J…": { "id": "d56c3b36…" } } ]
 *        ↑ el INTEGRADO      ↑ el NUESTRO
 *
 *   Keycloak trae su propio mapper dentro del ámbito `organization` —multivaluado pero SIN
 *   `addOrganizationId`— y añadir el nuestro al cliente daba **DOS mappers escribiendo el
 *   mismo claim**. Keycloak no se queja: los concatena en un array, y el núcleo lo rechaza
 *   con «se esperaba un objeto por alias».
 *
 * ⭐⭐ Y el ámbito NO se puede quitar: este mapper hace `return` si la petición no lo trae,
 *   así que sin ámbito no hay claim. ⇒ la salida no era quitar el ámbito ni el nuestro: era
 *   dejar de tener dos. Se configura el que ya existe.
 *
 * ⚠️ Lo destapó el primer token de NAVEGADOR que pasó por `organizacionDeClaims`. Las
 *   pruebas fabrican los claims a mano, así que una duplicación del emisor no aparece ahí
 *   ni puede aparecer — queda dicho porque es una clase de fallo que este repositorio no
 *   sabe cazar todavía.
 */
export const mapperOrganizacion = () => ({
  name: 'organizacion',
  protocol: 'openid-connect',
  protocolMapper: 'oidc-organization-membership-mapper',
  config: {
    'claim.name': CLAIM_ORG,
    'jsonType.label': 'JSON',
    multivalued: 'true',
    addOrganizationId: 'true',
    'id.token.claim': 'true',
    'access.token.claim': 'true',
    'introspection.token.claim': 'true',
  },
});

/**
 * ⛔⛔ EL DOMINIO SE DERIVA DEL ALIAS, Y NO ES EL ALIAS. Medido, en dos intentos:
 *
 *     ModelValidationException: You must provide at least one domain
 *     ModelValidationException: The specified domain is invalid: t_01j9zk….rubix.interno
 *
 *   El alias admite `_` —`t_<ULID>` lo lleva— y **una etiqueta DNS no**. El servidor además
 *   pasa el dominio a minúsculas antes de validarlo, así que el ULID en mayúsculas tampoco
 *   sobrevive tal cual. ⇒ se traduce: `_` → `-`, y todo a minúsculas.
 *
 * ⚠️ Y sigue siendo un MARCADOR bajo un sufijo nuestro, no enrutable. El dominio de verdad
 *    —el del correo del cliente— llega con el corredor en `P·3`.
 */
const dominioDe = (alias) => `${alias.replace(/_/g, '-').toLowerCase()}.rubix.interno`;

/**
 * Una organización del realm SaaS. ⭐ El **alias** ES `t_<ULID>`: es el valor que acaba en el
 * claim y el mismo que viaja dentro de cada IRI de sujeto. Si divergieran, el sujeto acabaría
 * en otro inquilino y ninguna comprobación de campo por separado lo vería.
 */
export function organizacionDe(alias, { nombre = alias, dominios = [dominioDe(alias)] } = {}) {
  if (!ORGANIZACION.test(alias ?? '')) {
    throw new Error(`«${alias}» no vale como alias de organización: tiene que ser o_<ULID>, `
                  + 'porque es el valor que el núcleo compara contra el IRI del sujeto');
  }
  return {
    alias,
    name: nombre,
    enabled: true,
    // ⛔⛔ AL MENOS UN DOMINIO, Y NO ES OPCIONAL — medido, no leído.
    //
    //   Aquí decía «se deja vacío a propósito hasta `P·3`, es cosa del corredor». Era FALSO,
    //   y lo dijo el servidor al importar:
    //
    //       ModelValidationException: You must provide at least one domain
    //
    //   ⇒ Una organización sin dominio **no existe** para el emisor: el dominio es lo que
    //     usa el «identity-first login» para encaminar a cada persona a su IdP, así que sin
    //     él la organización no tiene por dónde entrar.
    //
    // ⚠️ Y el que ponemos por defecto es un MARCADOR, no un dominio del cliente: `<alias>`
    //    bajo un sufijo nuestro, no enrutable. El día que el cliente traiga el suyo —`P·3`,
    //    el corredor— se sustituye. Se dice para que nadie lo lea como si fuera real.
    domains: dominios.map((d) => ({ name: d, verified: false })),
  };
}

/**
 * ⭐⭐⭐ LA CUENTA DE SERVICIO DE **SÓLO LECTURA** — `74` ③ / [`71`](../../docs/decisiones/71-la-consola.md) §4·bis
 *
 * Es la credencial con la que la CONSOLA le pregunta al IdP por su propia gente. Y su valor
 * está entero en lo que **no** puede hacer.
 *
 * ── ⛔⛔ POR QUÉ NO SE USA EL TOKEN DE QUIEN MIRA, que era lo elegante ──
 *
 *   Es lo que hace la propia consola de Keycloak, y lo que hace AWS. Y no nos vale **por una
 *   versión**: administrar organizaciones sin `manage-realm` llega en Keycloak **26.7.0** y
 *   corremos **26.0.7**. Dárselo a un administrador de cliente sería entregarle el realm
 *   entero — su gente, sus flujos de autenticación, todo.
 *
 *   ⇒ Así que se hace lo que hace el mercado cuando NO eres el IdP (Auth0 lo dice con estas
 *     palabras sobre su Management API): **una credencial de servicio en un backend**, nunca
 *     el token del navegador.
 *
 * ── ⭐ Y POR QUÉ SÓLO LECTURA, hoy ──────────────────────────────────
 *
 *   Porque la credencial que ESCRIBE todavía no tiene que existir. Partirlas no es ceremonia:
 *   el día que se filtre una, lo que se filtra es **lo que esa cuenta podía hacer**. Una sola
 *   cuenta para todo convierte cualquier fuga en la peor fuga.
 */
export const CLIENTE_LECTOR = 'rubix-consola-lector';

/**
 * ⛔⛔ LOS CUATRO PAPELES, Y NI UNO MÁS — y esta lista es una LISTA BLANCA, no un mínimo.
 *
 *   `aplicar-entrada.mjs` no sólo los añade: **quita todo lo que no esté aquí**. La
 *   diferencia importa — «tiene al menos estos» convierte «sólo lectura» en una esperanza;
 *   «tiene exactamente estos» lo convierte en un hecho comprobable.
 *
 * ⚠️ `query-users` es el que sorprende, y hace falta: sin él, `view-users` deja LEER un
 *    usuario que ya conoces pero no BUSCAR — y listar los miembros de una organización es
 *    una búsqueda. Sin él la consola no vería a nadie y el síntoma sería una lista vacía,
 *    no un 403.
 */
export const PAPELES_DE_LECTURA = Object.freeze([
  'view-users', 'view-realm', 'view-clients', 'query-users',
]);

/** ⭐ Confidencial y SIN un solo flujo de navegador: esta cuenta no es de nadie que mire. */
export const clienteLector = () => ({
  clientId: CLIENTE_LECTOR,
  name: 'Rubix · consola (lectura)',
  description: 'Cuenta de servicio de SÓLO LECTURA con la que la consola consulta el IdP.',
  enabled: true,
  protocol: 'openid-connect',
  publicClient: false,
  serviceAccountsEnabled: true,
  // ⛔ Los tres apagados a propósito. Un `standardFlowEnabled` aquí permitiría iniciar sesión
  //   COMO esta cuenta desde un navegador, y `directAccessGrants` permitiría cambiarla por
  //   usuario y contraseña. Ninguna de las dos tiene sentido para un backend, y las dos son
  //   puertas que nadie vigila porque nadie recuerda que están abiertas.
  standardFlowEnabled: false,
  directAccessGrantsEnabled: false,
  implicitFlowEnabled: false,
  // ⭐ Sin `redirectUris` ni `webOrigins`: no vuelve a ningún sitio porque nunca sale.
  attributes: { 'access.token.lifespan': '300' },
});

export const clienteAgente = (id) => ({
  clientId: id,
  name: `Agente ${id}`,
  enabled: true,
  protocol: 'openid-connect',
  publicClient: false,
  serviceAccountsEnabled: true,
  standardFlowEnabled: false,
  directAccessGrantsEnabled: false,
  implicitFlowEnabled: false,
  protocolMappers: [mapperTipo('agente'), mapperAudiencia()],
});

/**
 * ⭐⭐⭐ EL REALM DE UNA CELDA.
 *
 * ⭐⭐⭐ `P·0·c` · UN realm para TODOS los clientes, con una ORGANIZACIÓN por cliente dentro.
 *
 * ── ⛔ Por qué dejó de ser un realm por cliente ──
 *
 *   La práctica de operadores acota realm-por-inquilino a **5–20**: más allá, *«estás
 *   gestionando una plataforma IAM en vez de construyendo tu producto»*. Y los cuatro grandes
 *   ponen la identidad en la CUENTA, no en la partición.
 *
 * ── ⚠️ Y lo que se PIERDE, escrito donde se comete ──
 *
 *   Con un realm por organización, un token de otra **no se podía construir**: lo firmaba otra
 *   clave. Ahora lo firma la misma y la pertenencia viaja en un claim ⇒ la garantía baja de
 *   **imposible de construir** a **imposible de falsificar sin la clave del emisor**.
 *
 *   ⇒ No es lo mismo, y por eso `organizacionDeClaims` tiene los negativos que tiene.
 *
 * @param {{organizaciones?: string[], agentes?: string[]}} [op]
 */
export function realmSaaS({ organizaciones = [], agentes = [], entorno = 'produccion' } = {}) {
  // ⛔ El defecto es `produccion` a propósito, y es la decisión dura: quien olvide el
  //   parámetro genera el realm real, no uno de juguete. Al revés —con `desarrollo` por
  //   defecto— un despliegue distraído habría sustituido producción por un realm de pruebas
  //   sin que nadie lo pidiera, y eso sí que no se deshace.
  const ENT = entornoDe(entorno);
  for (const o of organizaciones) organizacionDe(o);
  return {
    // ⭐ `rubix` ó `rubix-dev` — el mismo generador, dos realms. Que salgan del MISMO código
    //   es lo que impide la deriva entre entornos, que es la avería clásica de tener dos.
    realm: `${REALM_SAAS}${ENT.sufijo}`,
    // ⛔⛔ `id` Y `realm` SON DOS CAMPOS DISTINTOS, y parametrizar sólo uno cuesta caro:
    //   al crear `rubix-dev` con `realm: 'rubix-dev'` e `id: 'rubix'`, Postgres contestó
    //   `duplicate key … Key (id)=(rubix) already exists` y la Admin API lo devolvió como un
    //   escueto **409 «Conflict detected. See logs for details»** — que no menciona el `id`
    //   ni el realm. Hubo que leer el log del pod para saber qué chocaba.
    id: `${REALM_SAAS}${ENT.sufijo}`,
    enabled: true,
    // ⭐ Se ve en la página de login Y en el correo: quien entre en desarrollo lo sabe sin
    //   mirar la URL. Un realm de pruebas que se presenta igual que el real es una trampa.
    displayName: ENT.nombre,

    // ⭐⭐ El eje de tenencia, NATIVO. Y va por realm, no por servidor: el realm interno lo
    //   lleva apagado, porque nosotros no somos un cliente de nosotros mismos.
    organizationsEnabled: true,
    organizations: organizaciones.map((o) => organizacionDe(o)),

    // ── LO QUE SE APAGA, y cada línea tiene su motivo ────────────────
    //
    // ⛔ El registro anónimo de clientes es **DCR**, y `M·23d` lo da por DEPRECADO en la
    //   spec de MCP: construir sobre DCR «porque es lo que sale en los tutoriales» es
    //   empezar con deuda el primer día. Un cliente nuevo es una DECISIÓN.
    // ✅✅ 2026-08-28 · PASA A `true`, Y ES SEGURO PORQUE LA PERTENENCIA CAMBIÓ DE MANO.
    //
    //   Aquí decía `false` con este motivo: *«Nadie se da de alta solo en el directorio de un
    //   cliente»*. Era correcto **mientras el claim `organization` decidiera la pertenencia**:
    //   registrarse hubiera sido colarse en el directorio de alguien.
    //
    //   `76` ANEXO la mudó a `rubix.invitacion`. ⇒ registrarse en el realm **ya no concede
    //   nada**: sin un vale vivo no se acuña sujeto, y sin sujeto no hay identidad en Rubix.
    //   Un desconocido con cuenta es exactamente igual de forastero que sin ella.
    //
    // ⛔ Y lo que NO cambia: `verifyEmail: true` sigue abajo, y es lo que hace que quien se
    //   registre llegue con el correo verificado — la condición exacta que exige admitir.
    //
    // ⚠️ Lo que SÍ se paga: cualquiera puede crear una cuenta en este realm. Sin invitación
    //    no ve nada, pero **ocupa una fila en el IdP**. El día que eso moleste, la respuesta no
    //    es volver a `false` —sería cerrar la puerta a los invitados— sino podar las cuentas
    //    sin sujeto, que es una operación que hoy no existe. Queda dicho.
    registrationAllowed: true,
    resetPasswordAllowed: true,
    rememberMe: false,
    verifyEmail: true,
    loginWithEmailAllowed: true,
    duplicateEmailsAllowed: false,

    // ── LO QUE OBLIGA EL MODO DIRECTORIO (`63` §1·bis ⓒ) ────────────
    //
    // ⚠️ Custodiamos las credenciales de los empleados de todos los clientes. Eso no es una
    //    casilla: es bloqueo por intentos, política de contraseñas y MFA desde el día uno.
    bruteForceProtected: true,
    permanentLockout: false,
    maxFailureWaitSeconds: 900,
    failureFactor: 5,
    passwordPolicy: 'length(12) and notUsername(undefined) and passwordHistory(3) and forceExpiredPasswordChange(365)',
    otpPolicyType: 'totp',
    otpPolicyAlgorithm: 'HmacSHA256',

    // ── ⭐⭐⭐ `P·2` · AAL2 — DOS FACTORES, SIN CONDICIONAL ──────────
    //
    //   Lo que había: contraseña + TOTP **si el usuario lo tenía puesto**. Eso es ofrecer
    //   MFA, y AAL2 la EXIGE. Lo que hay ahora: el segundo factor es REQUIRED en el flujo.
    //
    // ⛔ `browserFlow` es lo que hace que el flujo de arriba SIRVA: declararlo sin
    //   apuntarlo aquí dejaría los tres flujos escritos en el realm y sin usar — verde en
    //   cualquier `grep`, y la entrada seguiría siendo de un factor.
    // ⭐ El correo de salida — sin credencial. Sin esto, `resetPasswordAllowed` de abajo es
    //   una promesa que el realm no puede cumplir, y `check-entrada` ⑥ lo pone rojo.
    smtpServer: correoDeSalida(ENT.nombre),
    // ⭐ Marca e idioma del correo — y `emailTheme` es lo único que exige un tema propio.
    ...presentacionDelCorreo(),
    authenticationFlows: [...flujosDeEntrada(), ...flujoDeReposicion()],
    browserFlow: FLUJO_ENTRADA,
    // ⭐⭐ Y LA PUERTA DE ATRÁS, DECLARADA. Hasta el 2026-08-26 esto heredó el flujo de
    //   FÁBRICA — que no está en el artefacto, así que **ninguna guarda lo medía**.
    resetCredentialsFlow: FLUJO_REPOSICION,
    ...politicaDePasskeys('Rubix'),
    attributes: { ...declaracionDeGarantia(), ...caducidadDelEnlace() },

    // ── LA VIDA DE LOS TOKENS ───────────────────────────────────────
    //
    // ⭐⭐ Y AQUÍ ESTÁ EL PRECIO DE LA REVOCACIÓN, dicho con un número: deshabilitar a
    //   alguien en el IdP **no mata su token en curso**. Lo mata cuando caduca. Cinco
    //   minutos es la ventana que el control negativo ② de `check-idp` tiene que respetar —
    //   si midiera antes, diría que la revocación no muerde y estaría midiendo el reloj.
    //
    // ⛔ Bajarla más no es gratis: cada refresco es una petición a la pieza más crítica de
    //   la malla. 300 s es el punto declarado, no el que venía de fábrica.
    accessTokenLifespan: 300,
    ssoSessionIdleTimeout: 1800,
    ssoSessionMaxLifespan: 36000,
    revokeRefreshToken: true,
    refreshTokenMaxReuse: 0,

    // ⛔ Sólo firma asimétrica. `modelo/` no verifica HMAC, así que un realm que emitiera
    //   HS256 produciría tokens que nadie puede validar — y el síntoma sería «no entra
    //   nadie», que manda a mirar al sitio equivocado.
    defaultSignatureAlgorithm: 'RS256',

    clients: [
      // ── ① EL SERVIDOR DE RECURSOS ───────────────────────────────
      //
      // ⭐ No inicia sesión ni pide tokens: existe para SER la audiencia. Es el `resource
      //   server` de OAuth 2.1, y su `clientId` es lo que `RUBIX_OIDC_AUDIENCIA` compara.
      {
        clientId: AUDIENCIA,
        name: 'Rubix API',
        enabled: true,
        protocol: 'openid-connect',
        publicClient: false,
        standardFlowEnabled: false,
        directAccessGrantsEnabled: false,
        serviceAccountsEnabled: false,
        implicitFlowEnabled: false,
      },
      // ── ② LA PERSONA ────────────────────────────────────────────
      //
      // ⛔ Cliente PÚBLICO con PKCE S256 **obligatorio**, y sin ningún otro flujo. OAuth 2.1
      //   retira `implicit` y `password`; dejarlos «por compatibilidad» es dejar la puerta
      //   por la que se cuela quien no quiere hacer PKCE.
      {
        clientId: 'rubix-consola',
        name: 'Rubix · consola',
        enabled: true,
        protocol: 'openid-connect',
        publicClient: true,
        standardFlowEnabled: true,
        directAccessGrantsEnabled: false,
        implicitFlowEnabled: false,
        serviceAccountsEnabled: false,
        // ⭐ 2026-08-26 · LAS URIs DE RETORNO, POR FIN PUESTAS — y exactas.
        //
        //   Hasta hoy este cliente no tenía ninguna, así que **rechazaba toda petición de
        //   autorización**: no es que apuntara a un sitio equivocado, es que no apuntaba a
                //   ninguno. Las entradas salen de `ENTORNOS`, arriba, con su porqué.
        //
        // ⛔ Y siguen sin comodines. El aviso que había aquí no se retira: se CUMPLE.
        redirectUris: [...ENT.retornos],
        webOrigins: [...ENT.origenes],
        attributes: {
          'pkce.code.challenge.method': 'S256',
          'post.logout.redirect.uris': ENT.salidas.join('##'),
        },
        // ⛔⛔ POR DEFECTO, NO OPCIONAL. El mapper de organización hace `return` si la
        //   petición no trae este ámbito ⇒ como opcional, un cliente distraído recibiría un
        //   token SIN organización. El núcleo lo rechazaría —correcto— pero el síntoma sería
        //   «no entra nadie», que manda a mirar al sitio equivocado.
        //
        // ✔✔ 2026-08-28 · ENTRAN `basic` Y `acr`, Y FALTABAN POR ESCRIBIR ESTA LISTA
        //
        //   ⛔⛔ En Keycloak 24+ el claim `sub` **lo pone el ámbito `basic`**, no el núcleo del
        //     emisor. Declarar `defaultClientScopes` SUSTITUYE la lista por defecto del
        //     servidor, así que omitir `basic` deja el token SIN `sub` — y sin `sub` no hay
        //     identidad: `identidadDeClaims` falla con «el token no trae `sub`».
        //
        //   ⚠️ Y estuvo TAPADO: `organizacionDeClaims` corre ANTES que la comprobación de
        //      `sub`, así que mientras el claim de organización vino mal, este fallo ni se
        //      llegó a ver. Dos averías apiladas en el mismo token.
        //
        //   ⭐ `acr` entra por coherencia con los otros clientes del realm y porque este
        //     realm declara AAL2: sin él, el nivel de autenticación no viaja en el token.
        //
        //   ⭐⭐ La lección, que vale más que la línea: **una lista por defecto que se
        //     sobrescribe hay que escribirla ENTERA**. Lo que se omite no se hereda — se
        //     pierde, y lo que se pierde aquí es el claim que dice quién eres.
        defaultClientScopes: ['basic', 'acr', 'organization', 'profile', 'email', 'roles', 'web-origins'],
        // ⛔⛔ AQUÍ YA NO VA `mapperOrganizacion()`, y quitarlo fue un ARREGLO — ver abajo.
        protocolMappers: [mapperTipo('persona'), mapperAudiencia()],
      },
      // ── ③ LA CONSOLA, LEYENDO ───────────────────────────────────
      //
      // ⚠️ Va en el realm SaaS y no en el interno, y no es indiferente: los papeles de
      //    `realm-management` son **de un realm concreto**. Una cuenta en `rubix-interno`
      //    no puede administrar `rubix`, por muchos papeles que tenga.
      clienteLector(),
      ...agentes.map(clienteAgente),
    ],
  };
}

/**
 * ⭐ EL REALM INTERNO — nosotros. Y va aparte por la razón por la que existe la separación:
 * un empleado de la plataforma **no es miembro de ninguna organización de cliente**, y
 * mezclarlo en el realm SaaS sería exactamente el eje que este hito vino a poner en su sitio.
 *
 * ⛔ `organizationsEnabled: false`: no somos un cliente de nosotros mismos.
 * ⚠️ Y hoy **no lo consume nadie**. Se crea porque la separación se hace ANTES de que haya
 *    dos poblaciones, no después — moverlas luego es migrar personas.
 */
export function realmInterno() {
  return {
    realm: REALM_INTERNO,
    id: REALM_INTERNO,
    enabled: true,
    displayName: 'Rubix · interno',
    organizationsEnabled: false,
    registrationAllowed: false,
    bruteForceProtected: true,
    permanentLockout: false,
    maxFailureWaitSeconds: 900,
    failureFactor: 5,
    passwordPolicy: 'length(12) and notUsername(undefined) and passwordHistory(3) and forceExpiredPasswordChange(365)',
    otpPolicyType: 'totp',
    otpPolicyAlgorithm: 'HmacSHA256',

    // ⛔⛔ `P·2` · Y AQUÍ **CON MÁS MOTIVO**, no por simetría.
    //
    //   Éste es el realm de quien ADMINISTRA la plataforma. Un realm interno sin MFA es el
    //   camino corto a todos los clientes a la vez — y es justo el que se olvida, porque
    //   *«hoy no lo consume nadie»*. ⭐ El trinquete ③ mira LOS DOS y no pregunta cuál importa.
    // ⭐ El correo de salida — sin credencial. Sin esto, `resetPasswordAllowed` de abajo es
    //   una promesa que el realm no puede cumplir, y `check-entrada` ⑥ lo pone rojo.
    smtpServer: correoDeSalida('Rubix · interno'),
    // ⭐ Marca e idioma del correo — y `emailTheme` es lo único que exige un tema propio.
    ...presentacionDelCorreo(),
    authenticationFlows: [...flujosDeEntrada(), ...flujoDeReposicion()],
    browserFlow: FLUJO_ENTRADA,
    // ⭐⭐ Y LA PUERTA DE ATRÁS, DECLARADA. Hasta el 2026-08-26 esto heredó el flujo de
    //   FÁBRICA — que no está en el artefacto, así que **ninguna guarda lo medía**.
    resetCredentialsFlow: FLUJO_REPOSICION,
    ...politicaDePasskeys('Rubix · interno'),
    attributes: { ...declaracionDeGarantia(), ...caducidadDelEnlace() },

    // ⭐ Más estricto que el SaaS a propósito: quien administra la plataforma tiene más
    //   alcance que cualquier usuario de un cliente, así que su sesión dura menos.
    accessTokenLifespan: 300,
    ssoSessionIdleTimeout: 900,
    ssoSessionMaxLifespan: 28800,
    revokeRefreshToken: true,
    refreshTokenMaxReuse: 0,
    defaultSignatureAlgorithm: 'RS256',
    clients: [],
  };
}
