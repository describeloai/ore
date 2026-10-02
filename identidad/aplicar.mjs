// ═══════════════════════════════════════════════════════════════════
// ✏️ 2026-09-30 · EN ORE (ADR 0048). Vino de la plataforma (`aplicar-entrada.mjs`)
//   con tres cambios: concilia `realmsDeOre()` (lo que el manifiesto dice, con lo de
//   ORE), habla con el IdP de ORE, y el admin no tiene valor por defecto.
//
//   uso (Git Bash; la contraseña NUNCA por argumento):
//     kubectl port-forward -n identidad pod/idp-0 18080:8080
//     export ORE_IDP_ADMIN="$(kubectl -n identidad get secret idp-initial-admin -o jsonpath='{.data.username}' | base64 -d)"
//     kubectl -n identidad get secret idp-initial-admin -o jsonpath='{.data.password}' | base64 -d \
//       | node identidad/aplicar.mjs [--plan | --verificar] [--realm=rubix]
// ═══════════════════════════════════════════════════════════════════
//
// `P·2·b` · LA POLÍTICA DE ENTRADA, APLICADA AL SERVIDOR VIVO
//
// ── ⛔⛔ POR QUÉ EXISTE ESTE FICHERO ─────────────────────────────────
//
//   `KeycloakRealmImport` **se salta los realms que ya existen** — y se declara `Done: True`
//   igualmente. Medido el 2026-08-26:
//
//       Realm 'rubix' already exists. Import skipped     ← el log del Job
//       status.conditions[Done] = True                    ← el CR, a la vez
//
//   ⇒ Una política escrita en `realm.mjs` **no llega al servidor**. Y el CRD de 26.0.7 no
//     tiene override: sólo `keycloakCRName`, `placeholders`, `realm` y `resources`.
//
// ── ⭐⭐⭐ Y ESTO NO ES UN PARCHE: ES EL EMBRIÓN DEL RECONCILIADOR ───
//
//   `realms.yaml` ya decía que faltaba *«un reconciliador contra la Admin API, y entra con
//   `P·4`»*. Esto es su primer trozo, acotado a una cosa: **el flujo de entrada**.
//
//   ⚠️ Y por eso es IDEMPOTENTE y no incremental: borra los flujos que gestiona y los vuelve
//      a poner. Reconciliar es *«deja esto así»*, no *«añade esto». Un reconciliador que
//      acumula produce, al tercer pase, un flujo con seis ramas que nadie escribió.
//
// ── ⛔ LA CONTRASEÑA NO SE PASA POR ARGUMENTO ───────────────────────
//
//   Entra por **stdin**. Un argumento se ve en `ps`, se queda en el historial del shell y
//   viaja en los logs de quien lo invoque. Aquí llega por una tubería y muere con el proceso.
//
//   (El uso de la plataforma —`rubix-idp-admin`, `-n rubix svc/rubix-idp-service`— era del
//   clúster viejo. El de ORE está en la cabecera de arriba.)
//
// 📎 de la plataforma: docs/iam/03-el-programa.md `P·2·b` · aquí: identidad/realm.mjs, identidad/ore.mjs
// ═══════════════════════════════════════════════════════════════════

import { realmsDeOre, accionesDe, RETIRADOS, GESTIONADOS_FUERA, DE_FABRICA } from './ore.mjs';
import {
  FLUJO_ENTRADA, factoresMinimos,
  FLUJO_REPOSICION, CREDENCIALES_DE_REPOSICION,
  politicaDePasskeys, declaracionDeGarantia,
  CLIENTE_LECTOR, PAPELES_DE_LECTURA, clienteLector,
} from './realm.mjs';

const IDP = process.env.ORE_IDP ?? process.env.RUBIX_IDP ?? 'http://127.0.0.1:18080';
// ✏️ ORE (0048): sin valor por defecto. El admin del IdP de ORE es el de arranque del
//   operador (`identidad/idp-initial-admin`, `username`), y adivinarlo era entrar como otro.
const USUARIO = process.env.ORE_IDP_ADMIN ?? process.env.RUBIX_IDP_ADMIN;
if (!USUARIO) {
  console.error('falta ORE_IDP_ADMIN: el usuario admin (kubectl -n identidad get secret idp-initial-admin -o jsonpath={.data.username} | base64 -d)');
  process.exit(2);
}
const SOLO_VERIFICAR = process.argv.includes('--verificar');
// ⭐ ORE (0048): `--plan` dice TODO lo que aplicar cambiaría —flujos, ajustes, passkeys y
//   atributos, el cliente público, el lector, los mapeadores de organización— sólo con
//   GET, y sale. `--verificar` sólo mide los factores; contra producción no se aplica sin
//   haber leído el plan: el reconciliador también RESTA lo que el artefacto no dice.
const SOLO_PLAN = process.argv.includes('--plan');
// ⭐ `--realm=rubix-dev` para ensayar en desarrollo antes de tocar producción. Sin él se
//   aplican los dos, que es lo correcto por defecto: dos realms que divergen es el fallo del
//   que venimos — el 2026-08-28 los dos compartían la MISMA organización y las listas se
//   mezclaban entre entornos.
const SOLO_REALM = (process.argv.find((a) => a.startsWith('--realm=')) ?? '').slice(8) || null;

const leerContrasena = () => new Promise((res, rej) => {
  let d = '';
  process.stdin.setEncoding('utf8');
  process.stdin.on('data', (c) => { d += c; });
  process.stdin.on('end', () => (d.trim() ? res(d.trim()) : rej(new Error('sin contraseña en stdin'))));
  process.stdin.on('error', rej);
});

async function token(contrasena) {
  const r = await fetch(`${IDP}/realms/master/protocol/openid-connect/token`, {
    method: 'POST',
    headers: { 'content-type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams({
      grant_type: 'password', client_id: 'admin-cli', username: USUARIO, password: contrasena,
    }),
  });
  // ⛔ El cuerpo NO se imprime aunque falle: en un fallo de credenciales Keycloak devuelve
  //   un error inocuo, pero en otros devuelve trazas. No se arriesga.
  if (!r.ok) throw new Error(`el administrador no entra (HTTP ${r.status})`);
  return (await r.json()).access_token;
}

const api = (t) => async (metodo, ruta, cuerpo) => {
  const r = await fetch(`${IDP}/admin${ruta}`, {
    method: metodo,
    headers: { authorization: `Bearer ${t}`, 'content-type': 'application/json' },
    ...(cuerpo === undefined ? {} : { body: JSON.stringify(cuerpo) }),
  });
  if (!r.ok && r.status !== 404) throw new Error(`${metodo} ${ruta} → ${r.status} ${await r.text()}`);
  if (r.status === 404) return null;
  const txt = await r.text();
  return txt ? JSON.parse(txt) : null;
};

/**
 * ⭐⭐ RECONSTRUYE EL ÁRBOL desde la lista PLANA que devuelve la Admin API.
 *
 *   `/authentication/flows/{alias}/executions` no devuelve el grafo: devuelve una lista con
 *   un campo `level`. Rehacer el árbol es lo que permite medirlo con **el mismo
 *   `factoresMinimos`** que mide el artefacto — y que las dos medidas sean comparables es
 *   justo lo que hace que «desplegado» signifique algo.
 */
function arbolDesdePlano(alias, plano) {
  const flujos = [{ alias, providerId: 'basic-flow', topLevel: true, authenticationExecutions: [] }];
  const porNivel = { 0: flujos[0] };
  for (const e of plano) {
    const padre = porNivel[e.level];
    if (!padre) continue;
    if (e.authenticationFlow) {
      const hijo = {
        alias: e.displayName, providerId: 'basic-flow', topLevel: false, authenticationExecutions: [],
      };
      flujos.push(hijo);
      porNivel[e.level + 1] = hijo;
      padre.authenticationExecutions.push({
        flowAlias: e.displayName, requirement: e.requirement, autheticatorFlow: true,
      });
    } else {
      padre.authenticationExecutions.push({
        authenticator: e.providerId, requirement: e.requirement, autheticatorFlow: false,
      });
    }
  }
  return { authenticationFlows: flujos, browserFlow: alias };
}

async function medirVivo(llamar, realm) {
  const r = await llamar('GET', `/realms/${realm}`);
  if (!r) return { realm, existe: false };
  const plano = await llamar('GET', `/realms/${realm}/authentication/flows/${r.browserFlow}/executions`);
  const reposicion = r.resetCredentialsFlow
    ? await llamar('GET', `/realms/${realm}/authentication/flows/${encodeURIComponent(r.resetCredentialsFlow)}/executions`)
    : null;
  return {
    realm,
    existe: true,
    flujo: r.browserFlow,
    factores: plano ? factoresMinimos(arbolDesdePlano(r.browserFlow, plano)) : 0,
    'flujo-reposicion': r.resetCredentialsFlow ?? 'ninguno',
    // ⭐⭐ LA PUERTA DE ATRÁS, con el MISMO medidor y las credenciales que valen ALLÍ.
    //   `arbolDesdePlano` devuelve el árbol con la raíz ya puesta en `browserFlow`, así que
    //   aquí no hace falta `opciones.flujo`: sólo cambiar la lista de lo que cuenta.
    reposicion: reposicion
      ? factoresMinimos(arbolDesdePlano(r.resetCredentialsFlow, reposicion),
        { credenciales: CREDENCIALES_DE_REPOSICION })
      : 0,
    uv: r.webAuthnPolicyUserVerificationRequirement ?? 'sin-politica',
    aal: r.attributes?.['rubix.aal'] ?? 'sin-declarar',
    fal: r.attributes?.['rubix.fal'] ?? 'sin-declarar',
  };
}

/**
 * Pone el flujo tal cual lo describe el generador. ⭐ Borra antes de poner: idempotente.
 *
 * ⭐⭐ GENERALIZADO EL 2026-08-26 a CUALQUIER puerta, y no por elegancia: hasta ese día
 *   sólo sabía poner `browserFlow`, así que el flujo de REPOSICIÓN no tenía forma de
 *   aplicarse — y por eso seguía siendo el de FÁBRICA, que **borra el segundo factor**.
 *
 *   ⇒ La puerta de delante y la de atrás se ponen con **el mismo código**. Dos rutas
 *     distintas serían dos comportamientos distintos, y el de la puerta que nadie mira
 *     es siempre el que se queda atrás.
 */
async function aplicarFlujo(llamar, realm, deseado, opciones = {}) {
  const raizAlias = opciones.raiz ?? FLUJO_ENTRADA;
  const campo = opciones.campo ?? 'browserFlow';
  const porDefecto = opciones.porDefecto ?? 'browser';

  // ⛔ Primero se suelta la referencia: Keycloak no deja borrar el flujo que está en uso.
  const actual = await llamar('GET', `/realms/${realm}`);
  if (actual[campo] === raizAlias) {
    await llamar('PUT', `/realms/${realm}`, { ...actual, [campo]: porDefecto });
  }
  const existentes = await llamar('GET', `/realms/${realm}/authentication/flows`);
  for (const f of existentes ?? []) {
    if (f.topLevel && f.alias === raizAlias) await llamar('DELETE', `/realms/${realm}/authentication/flows/${f.id}`);
  }

  const flujos = new Map(deseado.authenticationFlows.map((f) => [f.alias, f]));
  const raiz = flujos.get(raizAlias);
  await llamar('POST', '/realms/' + realm + '/authentication/flows', {
    alias: raiz.alias, description: raiz.description, providerId: 'basic-flow',
    topLevel: true, builtIn: false,
  });

  // ⚠️ Se crea en ORDEN: la Admin API no acepta prioridades al crear, las deduce de la
  //    secuencia. Meterlas fuera de orden pone la contraseña detrás del segundo factor.
  const poner = async (alias) => {
    for (const e of flujos.get(alias).authenticationExecutions) {
      if (e.autheticatorFlow) {
        const hijo = flujos.get(e.flowAlias);
        await llamar('POST', `/realms/${realm}/authentication/flows/${alias}/executions/flow`, {
          alias: hijo.alias, type: 'basic-flow', description: hijo.description ?? '',
          provider: 'registration-page-form',
        });
        await poner(hijo.alias);
      } else {
        await llamar('POST', `/realms/${realm}/authentication/flows/${alias}/executions/execution`, {
          provider: e.authenticator,
        });
      }
    }
  };
  await poner(raizAlias);

  // Y ahora los requisitos, que es donde vive la exigencia. ⛔ Se crean en DISABLED por
  // defecto: sin este paso el flujo existe entero y no pide nada.
  // ⚠️ Los requisitos se recogen SÓLO de los flujos que cuelgan de ESTA raíz. Recorrerlos
  //    todos haría que dos puertas que compartan un autenticador —y la de entrada y la de
  //    reposición comparten la passkey y `auth-otp-form`— se pisaran el requisito: ganaría
  //    el último leído, y una de las dos quedaría aplicada al revés.
  const quiere = new Map();
  const recoger = (alias, visto = new Set()) => {
    if (!flujos.has(alias) || visto.has(alias)) return;
    visto.add(alias);
    for (const e of flujos.get(alias).authenticationExecutions) {
      quiere.set(e.autheticatorFlow ? e.flowAlias : e.authenticator, e.requirement);
      if (e.autheticatorFlow) recoger(e.flowAlias, visto);
    }
  };
  recoger(raizAlias);

  const plano = await llamar('GET', `/realms/${realm}/authentication/flows/${raizAlias}/executions`);
  for (const e of plano) {
    const clave = e.authenticationFlow ? e.displayName : e.providerId;
    const r = quiere.get(clave);
    if (r && e.requirement !== r) {
      await llamar('PUT', `/realms/${realm}/authentication/flows/${raizAlias}/executions`, { ...e, requirement: r });
    }
  }

  // Y por último el realm: el flujo en uso, y lo que sólo trae la puerta de entrada.
  const fresco = await llamar('GET', `/realms/${realm}`);
  await llamar('PUT', `/realms/${realm}`, {
    ...fresco,
    [campo]: raizAlias,
    ...(opciones.extras ? opciones.extras(fresco) : {}),
  });
}

/**
 * ⭐⭐⭐ LOS AJUSTES DEL REALM — y esto FALTABA, con la trampa ya denunciada en este fichero.
 *
 * ── ⛔⛔ EL HUECO, medido el 2026-08-28 ────────────────────
 *
 *   `realmSaaS()` declara `registrationAllowed`, `verifyEmail`, `resetPasswordAllowed`… y
 *   **nada de eso se aplicaba a un realm que ya existía**: `crearSiFalta` sólo escribe el
 *   objeto entero al NACER, y `aplicarFlujo` sólo toca el flujo y sus extras.
 *
 *   ⇒ Cambiar `registrationAllowed: false → true` en el artefacto no habría cambiado nada en
 *     el servidor, y la salida habría dicho «aplicado». Es LITERALMENTE la frase que este
 *     mismo fichero escribió sobre los ámbitos por defecto: *«un artefacto que declara algo
 *     que nadie aplica es una promesa»*.
 *
 * ── ⚠️ Y POR QUÉ UNA LISTA CORTA Y NO «todo el objeto» ─────────
 *
 *   Un `PUT` del artefacto entero pisaría lo que el realm vivo tiene y nosotros no
 *   gestionamos —SMTP, temas, `attributes`, la política de passkeys que pone `aplicarFlujo`—.
 *   Reconciliar los campos que decimos gobernar no es lo mismo que reponer el objeto.
 *
 * ⭐ Y se nombra lo que CAMBIA, no lo que se intentó: sin decir el valor viejo y el nuevo,
 *   «ajustes aplicados» no distingue «había que cambiar tres» de «no había que cambiar nada».
 */
const AJUSTES_GOBERNADOS = ['registrationAllowed', 'verifyEmail', 'resetPasswordAllowed', 'rememberMe'];

async function aplicarAjustes(llamar, realm, deseado) {
  const vivo = await llamar('GET', `/realms/${realm}`);
  const cambios = [];
  for (const campo of AJUSTES_GOBERNADOS) {
    if (deseado[campo] === undefined) continue;
    if (vivo[campo] !== deseado[campo]) cambios.push([campo, vivo[campo], deseado[campo]]);
  }
  if (!cambios.length) return { cambios: [] };
  await llamar('PUT', `/realms/${realm}`, {
    ...vivo, ...Object.fromEntries(cambios.map(([c, , n]) => [c, n])),
  });
  return { cambios };
}

/**
 * ⭐⭐ EL CLIENTE PÚBLICO — sus URIs de retorno, que es lo que hace posible un login
 *
 *   Hasta el 2026-08-26 `rubix-consola` no declaraba **ninguna**, así que Keycloak rechazaba
 *   toda petición de autorización. ⛔ Y el síntoma no dice eso: dice *«Invalid parameter:
 *   redirect_uri»*, que suena a que la URI está mal escrita y no a que no hay ninguna.
 *
 * ⚠️ Se hace por PATCH sobre lo que hay, no por reemplazo: el cliente vivo lleva sus
 *    mappers y sus ámbitos, y sobrescribirlo entero los tiraría. Reconciliar el campo que
 *    gestionamos no es lo mismo que reponer el objeto.
 */
/**
 * ⭐⭐⭐ LA CUENTA DE SÓLO LECTURA — `74` ③
 *
 * Crea el cliente si falta y le deja **EXACTAMENTE** los papeles de `PAPELES_DE_LECTURA`.
 *
 * ── ⛔⛔ Y LO QUE DE VERDAD HACE ESTA FUNCIÓN ES **QUITAR** ──────────
 *
 *   Añadir lo que falta es la mitad fácil y la que no protege de nada. Lo que convierte
 *   «sólo lectura» de una intención en un HECHO es la resta: cualquier papel que alguien
 *   haya añadido —a mano, en una prueba, «un momento para depurar»— se retira aquí.
 *
 *   ⚠️ Sin la resta, esta cuenta puede acumular `manage-users` un martes y nadie lo sabría:
 *      el `apply` seguiría diciendo que los cuatro papeles están puestos, porque lo están.
 *      Es la misma forma de `IAM·I5` — *«nadie deja de tener acceso porque llegue un aviso;
 *      deja de tenerlo porque ya no está en la lista»*.
 */
async function aplicarLector(llamar, realm) {
  const deseado = clienteLector();
  let vivos = await llamar('GET', `/realms/${realm}/clients?clientId=${CLIENTE_LECTOR}`);
  if (!vivos?.length) {
    await llamar('POST', `/realms/${realm}/clients`, deseado);
    vivos = await llamar('GET', `/realms/${realm}/clients?clientId=${CLIENTE_LECTOR}`);
  }
  const vivo = vivos?.[0];
  if (!vivo) throw new Error(`no se pudo crear ni encontrar ${CLIENTE_LECTOR} en ${realm}`);

  // ⛔ Los interruptores se reafirman en cada pasada. Un `standardFlowEnabled` que apareciera
  //   permitiría iniciar sesión COMO esta cuenta desde un navegador.
  await llamar('PUT', `/realms/${realm}/clients/${vivo.id}`, {
    ...vivo,
    publicClient: false,
    serviceAccountsEnabled: true,
    standardFlowEnabled: false,
    directAccessGrantsEnabled: false,
    implicitFlowEnabled: false,
  });

  // ⭐ El usuario de la cuenta de servicio y el cliente `realm-management`, que es quien
  //   posee los papeles de administración DE ESTE REALM.
  const su = await llamar('GET', `/realms/${realm}/clients/${vivo.id}/service-account-user`);
  const gestion = (await llamar('GET', `/realms/${realm}/clients?clientId=realm-management`))?.[0];
  if (!su || !gestion) throw new Error(`sin service-account-user o realm-management en ${realm}`);

  const disponibles = await llamar('GET', `/realms/${realm}/clients/${gestion.id}/roles`) ?? [];
  const puestos = await llamar('GET', `/realms/${realm}/users/${su.id}/role-mappings/clients/${gestion.id}`) ?? [];

  const faltan = PAPELES_DE_LECTURA
    .filter((n) => !puestos.some((r) => r.name === n))
    .map((n) => disponibles.find((r) => r.name === n))
    .filter(Boolean);
  const sobran = puestos.filter((r) => !PAPELES_DE_LECTURA.includes(r.name));

  if (faltan.length) {
    await llamar('POST', `/realms/${realm}/users/${su.id}/role-mappings/clients/${gestion.id}`, faltan);
  }
  if (sobran.length) {
    await llamar('DELETE', `/realms/${realm}/users/${su.id}/role-mappings/clients/${gestion.id}`, sobran);
  }
  return { puestos: faltan.length, retirados: sobran.map((r) => r.name) };
}

async function aplicarCliente(llamar, realm, deseado) {
  const quiere = (deseado.clients ?? []).find((c) => c.publicClient && c.standardFlowEnabled);
  if (!quiere) return null;
  const vivos = await llamar('GET', `/realms/${realm}/clients?clientId=${quiere.clientId}`);
  if (!vivos?.length) return null;
  const vivo = vivos[0];
  await llamar('PUT', `/realms/${realm}/clients/${vivo.id}`, {
    ...vivo,
    redirectUris: quiere.redirectUris ?? [],
    webOrigins: quiere.webOrigins ?? [],
    attributes: { ...(vivo.attributes ?? {}), ...(quiere.attributes ?? {}) },
  });

  // ── ⭐⭐ LOS ÁMBITOS POR DEFECTO, y esto NO estaba y costó una avería ──
  //
  //   ⛔ El 2026-08-28: el cliente vivía sin `basic`, y en Keycloak 24+ **ese ámbito es el
  //     que pone el claim `sub`**. Sin `sub` no hay identidad, y el token entero era
  //     inservible. Estaba en el artefacto desde antes y este `PUT` no lo aplicaba: sólo
  //     reafirmaba URIs. ⇒ un artefacto que declara algo que nadie aplica es una promesa.
  //
  //   ⭐ Y se RESTA además de sumar, igual que `aplicarLector`: sin la resta, «los ámbitos
  //     son estos» es una intención — el cliente podría acumular uno a mano y la salida
  //     seguiría diciendo que están los declarados, porque lo están.
  const quiereAmbitos = quiere.defaultClientScopes ?? [];
  let ambitos = null;
  if (quiereAmbitos.length) {
    const todos = await llamar('GET', `/realms/${realm}/client-scopes`);
    const porNombre = new Map((todos ?? []).map((sc) => [sc.name, sc]));
    const puestos = await llamar('GET', `/realms/${realm}/clients/${vivo.id}/default-client-scopes`) ?? [];
    const tiene = new Set(puestos.map((sc) => sc.name));

    const faltan = quiereAmbitos.filter((n) => !tiene.has(n) && porNombre.has(n));
    const sobran = puestos.filter((sc) => !quiereAmbitos.includes(sc.name));
    for (const n of faltan) {
      await llamar('PUT', `/realms/${realm}/clients/${vivo.id}/default-client-scopes/${porNombre.get(n).id}`);
    }
    for (const sc of sobran) {
      await llamar('DELETE', `/realms/${realm}/clients/${vivo.id}/default-client-scopes/${sc.id}`);
    }
    // ⚠️ Y se avisa de lo que se pidió y NO existe en el realm: un ámbito mal escrito se
    //    quedaría fuera en silencio, y el síntoma sería un claim que falta tres capas más allá.
    const inexistentes = quiereAmbitos.filter((n) => !porNombre.has(n));
    ambitos = { faltan, sobran: sobran.map((sc) => sc.name), inexistentes };
  }
  return { clientId: quiere.clientId, ambitos };
}

/**
 * ⭐⭐⭐ UN SOLO MAPPER ESCRIBIENDO `organization` — y la mitad que importa es la RESTA.
 *
 * ── ⛔⛔ El fallo, medido el 2026-08-28 con el propio Keycloak ───────
 *
 *     [ "o_01J…", { "o_01J…": { "id": "d56c3b36…" } } ]
 *        ↑ el INTEGRADO       ↑ el que añadíamos al cliente
 *
 *   El ámbito `organization` de Keycloak YA trae su mapper —multivaluado, pero sin
 *   `addOrganizationId`—. Añadir el nuestro daba dos escribiendo el mismo claim, y Keycloak
 *   **no avisa: los concatena**. El núcleo lo rechaza con «se esperaba un objeto por alias»,
 *   que es correcto y deja a todo el mundo fuera.
 *
 * ⇒ Se CONFIGURA el que existe y se BORRA el nuestro. Un claim, un mapper.
 *
 * ⭐ Y la resta se nombra en la salida, igual que en `aplicarLector`: sin decir qué se
 *   quitó, «hay un solo mapper» es una intención y no un hecho.
 */
async function aplicarClaimDeOrganizacion(llamar, realm) {
  const scopes = await llamar('GET', `/realms/${realm}/client-scopes`);
  const scope = (scopes ?? []).find((s) => s.name === 'organization');
  if (!scope) return null;

  const suyo = (scope.protocolMappers ?? [])
    .find((m) => m.protocolMapper === 'oidc-organization-membership-mapper');
  let configurado = false;
  if (suyo && suyo.config?.addOrganizationId !== 'true') {
    await llamar('PUT', `/realms/${realm}/client-scopes/${scope.id}/protocol-mappers/models/${suyo.id}`, {
      ...suyo,
      config: {
        ...suyo.config,
        multivalued: 'true',
        // ⭐ LA CLAVE: sin esto el mapper emite el ALIAS A SECAS, y el núcleo espera el
        //   objeto por alias — la única forma que deja ver cuántas organizaciones hay.
        addOrganizationId: 'true',
        'access.token.claim': 'true',
        'id.token.claim': 'true',
      },
    });
    configurado = true;
  }

  // ⛔⛔ LA RESTA: cualquier mapper de organización colgado de un CLIENTE sobra, porque el
  //   del ámbito ya escribe. Dos escribiendo el mismo claim es el fallo que esto arregla.
  const clientes = await llamar('GET', `/realms/${realm}/clients`);
  const retirados = [];
  for (const c of clientes ?? []) {
    for (const m of c.protocolMappers ?? []) {
      if (m.protocolMapper !== 'oidc-organization-membership-mapper') continue;
      await llamar('DELETE', `/realms/${realm}/clients/${c.id}/protocol-mappers/models/${m.id}`);
      retirados.push(`${c.clientId}/${m.name}`);
    }
  }
  return { configurado, retirados };
}

// ⭐⭐ LOS TRES, y los dos primeros salen del MISMO generador con un parámetro distinto.
//   `rubix-dev` entra el 2026-08-27: hasta ese día se desarrollaba contra el realm real, y
//   este mismo fichero **borra y recrea los flujos** en cada pasada. Un error de desarrollo
//   en el IdP no se deshace — no hay `outbox` del que replayarlo.
// ⭐⭐⭐ UNA ORGANIZACIÓN POR ENTORNO, y no es simetría por gusto.
//
// ⛔⛔ Hasta el 2026-08-28 los dos realms compartían `o_01J9ZKCP4N…`, y el síntoma se vio
//   en pantalla: `governance/users` listaba DOS personas con el mismo correo — el sujeto de
//   `rubix` y el de `rubix-dev`— porque `organizationMembers` filtra por organización y
//   ambos caían en la misma.
//
//   Que sean dos SUJETOS es correcto: la clave de `rubix.sujeto` es `(emisor, sub)` y un
//   emisor distinto es una persona distinta — `014` lo documenta y fusionarlos por el correo
//   sería justo el error que prohíbe. Lo que estaba mal era el CLIENTE compartido.
//
// ⚠️ Y tenía arista de seguridad: `admin-dev` acepta tokens de `rubix-dev` y lee la MISMA
//    base, así que una cuenta de desarrollo veía la lista de miembros de producción.
//
// ⇒ `72` partió la identidad en dos realms y no partió el cliente. Esto lo termina.
// ✏️ ORE (0048): los realms que se concilian son LOS DE ORE (`realmsDeOre()`), los
//   mismos que emite el manifiesto. Sin `rubix-dev` (no existe, y aquí se crearía).
const DESEADOS = realmsDeOre();

/**
 * ⭐⭐⭐ CREA EL REALM SI NO EXISTE — y hasta hoy NADA en este repositorio sabía hacerlo.
 *
 *   `KeycloakRealmImport` **se salta los realms que ya existen** y no tiene campo de
 *   override, así que se abandonó. Pero al abandonarlo se perdió también lo único que sí
 *   hacía bien: **crear uno desde cero**. ⇒ el realm de desarrollo no habría podido nacer
 *   sin que alguien lo hiciera a mano en la consola, que es la deriva que `69` vigila.
 *
 * ⚠️ Crear, y SÓLO crear. Si el realm existe no se toca aquí: lo reconcilian las funciones
 *    de abajo, campo a campo. Un `POST` sobre un realm vivo no es idempotente — es un 409, y
 *    forzarlo sería borrar usuarios reales para «dejarlo como el artefacto».
 */
async function crearSiFalta(llamar, realm, deseado) {
  const hay = await llamar('GET', `/realms/${realm}`);
  if (hay) return false;
  await llamar('POST', '/realms', deseado);
  const ahora = await llamar('GET', `/realms/${realm}`);
  if (!ahora) throw new Error(`no se pudo crear el realm ${realm}`);
  return true;
}

const contrasena = await leerContrasena();
const llamar = api(await token(contrasena));

/** ⭐ ORE (0048): las acciones requeridas —la del registro, sobre todo—. Devuelve los
 *  cambios `[alias, antes, después]` y aplica si `aplicar`. */
async function acciones(llamar, realm, deseado, aplicar) {
  const quiere = accionesDe(deseado);
  if (!Object.keys(quiere).length) return [];
  const vivas = await llamar('GET', `/realms/${realm}/authentication/required-actions`) ?? [];
  const cambios = [];
  for (const [alias, q] of Object.entries(quiere)) {
    const v = vivas.find((a) => a.alias === alias);
    if (!v) { cambios.push([alias, 'no registrada', q]); continue; }
    if (v.enabled === q.enabled && v.defaultAction === q.defaultAction) continue;
    cambios.push([alias, { enabled: v.enabled, defaultAction: v.defaultAction }, q]);
    if (aplicar) await llamar('PUT', `/realms/${realm}/authentication/required-actions/${alias}`, { ...v, ...q });
  }
  return cambios;
}

/**
 * ⭐⭐ EL CENSO DE CLIENTES (0048, deuda 1): cada cliente vivo, con su clase. Hasta aquí el
 *   reconciliador sólo miraba los clientes que el artefacto nombra, y un cliente que dejaba
 *   de nombrarse seguía vivo con su secreto. Ahora todos tienen que ser algo:
 *
 *     de fábrica · declarado · gestionado fuera (con quién) · RETIRADO (se borra) · desconocido
 *
 *   ⚠️ Un desconocido se AVISA y no se borra: lo creó alguien, y borrarlo a ciegas puede
 *   dejar sin entrada a quien lo use. Se decide y se nombra en `RETIRADOS` o se declara.
 */
async function censoDeClientes(llamar, realm, deseado) {
  const declarados = new Set((deseado.clients ?? []).map((c) => c.clientId));
  return (await llamar('GET', `/realms/${realm}/clients`) ?? []).map((c) => {
    const id = c.clientId;
    let clase = 'desconocido';
    if (RETIRADOS.includes(id)) clase = 'retirado';
    else if (DE_FABRICA.includes(id)) clase = 'de fábrica';
    else if (declarados.has(id)) clase = 'declarado';
    else {
      const g = GESTIONADOS_FUERA.find(([re]) => re.test(id));
      if (g) clase = `gestionado fuera: ${g[1]}`;
    }
    return { id: c.id, clientId: id, clase };
  });
}

/** Borra los retirados que sigan vivos. Devuelve los nombres borrados y los desconocidos. */
async function retirarClientes(llamar, realm, deseado) {
  const censo = await censoDeClientes(llamar, realm, deseado);
  const borrados = [];
  for (const c of censo.filter((x) => x.clase === 'retirado')) {
    await llamar('DELETE', `/realms/${realm}/clients/${c.id}`);
    borrados.push(c.clientId);
  }
  return { borrados, desconocidos: censo.filter((x) => x.clase === 'desconocido').map((x) => x.clientId) };
}

/** Lo que `aplicar` cambiaría en `realm`, sin tocar nada. */
async function planificar(llamar, realm, deseado) {
  const vivo = await llamar('GET', `/realms/${realm}`);
  if (!vivo) return [`✨ el realm ${realm} NO existe: se CREARÍA desde el artefacto`];
  const plan = [];
  const antes = await medirVivo(llamar, realm);
  const quiere = factoresMinimos(deseado);
  const quiereRep = factoresMinimos(deseado, { flujo: deseado.resetCredentialsFlow, credenciales: CREDENCIALES_DE_REPOSICION });
  plan.push(`flujos: entrada ${antes.factores} → ${quiere} factores · reposición ${antes.reposicion} → ${quiereRep} (se borran y se vuelven a poner \`${FLUJO_ENTRADA}\` y \`${FLUJO_REPOSICION}\`)`);
  for (const campo of AJUSTES_GOBERNADOS) {
    if (deseado[campo] !== undefined && vivo[campo] !== deseado[campo]) {
      plan.push(`ajuste ${campo}: ${JSON.stringify(vivo[campo])} → ${JSON.stringify(deseado[campo])}`);
    }
  }
  const extras = { ...politicaDePasskeys(vivo.displayName || realm), attributes: { ...(vivo.attributes ?? {}), ...declaracionDeGarantia() } };
  for (const [k, v] of Object.entries(extras)) {
    if (k === 'attributes') {
      for (const [a, x] of Object.entries(v)) {
        if ((vivo.attributes ?? {})[a] !== x) plan.push(`atributo ${a}: ${JSON.stringify((vivo.attributes ?? {})[a])} → ${JSON.stringify(x)}`);
      }
    } else if (JSON.stringify(vivo[k]) !== JSON.stringify(v)) {
      plan.push(`passkeys ${k}: ${JSON.stringify(vivo[k])} → ${JSON.stringify(v)}`);
    }
  }
  for (const [alias, antes, despues] of await acciones(llamar, realm, deseado, false)) {
    plan.push(`acción ${alias}: ${JSON.stringify(antes)} → ${JSON.stringify(despues)}`);
  }
  const pub = (deseado.clients ?? []).find((c) => c.publicClient && c.standardFlowEnabled);
  if (pub) {
    const c = (await llamar('GET', `/realms/${realm}/clients?clientId=${pub.clientId}`))?.[0];
    if (!c) plan.push(`cliente ${pub.clientId}: no existe (aplicar no lo crea)`);
    else {
      const dif = (nombre, a, b) => {
        const mas = (b ?? []).filter((x) => !(a ?? []).includes(x));
        const menos = (a ?? []).filter((x) => !(b ?? []).includes(x));
        if (mas.length) plan.push(`${pub.clientId} ${nombre} + ${mas.join(', ')}`);
        if (menos.length) plan.push(`${pub.clientId} ${nombre} − ${menos.join(', ')}`);
      };
      dif('redirectUris', c.redirectUris, pub.redirectUris);
      dif('webOrigins', c.webOrigins, pub.webOrigins);
      for (const [a, x] of Object.entries(pub.attributes ?? {})) {
        if ((c.attributes ?? {})[a] !== x) plan.push(`${pub.clientId} atributo ${a}: ${JSON.stringify((c.attributes ?? {})[a])} → ${JSON.stringify(x)}`);
      }
      const puestos = (await llamar('GET', `/realms/${realm}/clients/${c.id}/default-client-scopes`) ?? []).map((sc) => sc.name);
      const existen = new Set((await llamar('GET', `/realms/${realm}/client-scopes`) ?? []).map((sc) => sc.name));
      dif('ámbitos por defecto', puestos, (pub.defaultClientScopes ?? []).filter((n) => existen.has(n)));
      const inexistentes = (pub.defaultClientScopes ?? []).filter((n) => !existen.has(n));
      if (inexistentes.length) plan.push(`${pub.clientId} ámbitos pedidos que NO existen: ${inexistentes.join(', ')}`);
    }
  }
  for (const c of await censoDeClientes(llamar, realm, deseado)) {
    if (c.clase === 'retirado') plan.push(`⛔ cliente ${c.clientId}: RETIRADO, se BORRARÍA (con su secreto y su cuenta de servicio)`);
    else if (c.clase === 'desconocido') plan.push(`⚠️  cliente ${c.clientId}: vivo y NO declarado: retirarlo o declararlo (no se toca)`);
    else if (c.clase.startsWith('gestionado')) plan.push(`   cliente ${c.clientId}: ${c.clase}`);
  }
  const declaraLector = (deseado.clients ?? []).some((c) => c.clientId === CLIENTE_LECTOR);
  if (deseado.organizationsEnabled && declaraLector) {
    const l = (await llamar('GET', `/realms/${realm}/clients?clientId=${CLIENTE_LECTOR}`))?.[0];
    if (!l) plan.push(`${CLIENTE_LECTOR}: se CREARÍA`);
    else {
      const su = await llamar('GET', `/realms/${realm}/clients/${l.id}/service-account-user`);
      const g = (await llamar('GET', `/realms/${realm}/clients?clientId=realm-management`))?.[0];
      const tiene = su && g ? (await llamar('GET', `/realms/${realm}/users/${su.id}/role-mappings/clients/${g.id}`) ?? []).map((r) => r.name) : [];
      const faltan = PAPELES_DE_LECTURA.filter((n) => !tiene.includes(n));
      const sobran = tiene.filter((n) => !PAPELES_DE_LECTURA.includes(n));
      if (faltan.length) plan.push(`${CLIENTE_LECTOR} papeles + ${faltan.join(', ')}`);
      if (sobran.length) plan.push(`${CLIENTE_LECTOR} papeles − ${sobran.join(', ')}`);
      for (const k of ['publicClient', 'serviceAccountsEnabled', 'standardFlowEnabled', 'directAccessGrantsEnabled', 'implicitFlowEnabled']) {
        const x = k === 'serviceAccountsEnabled';
        if (l[k] !== x) plan.push(`${CLIENTE_LECTOR} ${k}: ${l[k]} → ${x}`);
      }
    }
  }
  if (deseado.organizationsEnabled) {
    const sc =(await llamar('GET', `/realms/${realm}/client-scopes`) ?? []).find((x) => x.name === 'organization');
    const m = (sc?.protocolMappers ?? []).find((x) => x.protocolMapper === 'oidc-organization-membership-mapper');
    if (m && m.config?.addOrganizationId !== 'true') plan.push('ámbito organization: addOrganizationId → true');
    for (const c of await llamar('GET', `/realms/${realm}/clients`) ?? []) {
      for (const x of c.protocolMappers ?? []) {
        if (x.protocolMapper === 'oidc-organization-membership-mapper') plan.push(`mapeador ${c.clientId}/${x.name}: se RETIRARÍA`);
      }
    }
  }
  return plan;
}

if (SOLO_PLAN) {
  for (const [realm, deseado] of DESEADOS) {
    if (SOLO_REALM && realm !== SOLO_REALM) continue;
    console.log(`── ${realm}`);
    for (const l of await planificar(llamar, realm, deseado)) console.log(`   ${l}`);
  }
  process.exit(0);
}

if (!SOLO_VERIFICAR) {
  for (const [realm, deseado] of DESEADOS) {
    if (SOLO_REALM && realm !== SOLO_REALM) continue;
    const nacido = await crearSiFalta(llamar, realm, deseado);
    if (nacido) console.log(`✨ realm ${realm} CREADO desde el artefacto`);
    await aplicarFlujo(llamar, realm, deseado, {
      extras: (fresco) => ({
        ...politicaDePasskeys(fresco.displayName || realm),
        attributes: { ...(fresco.attributes ?? {}), ...declaracionDeGarantia() },
      }),
    });
    // ⭐⭐ Y LA PUERTA DE ATRÁS, en la misma pasada. Aplicar una sin la otra deja el realm
    //   con la entrada endurecida y la recuperación de fábrica — que es PEOR que no tocar
    //   nada, porque la casa parece más segura de lo que está.
    await aplicarFlujo(llamar, realm, deseado, {
      raiz: FLUJO_REPOSICION,
      campo: 'resetCredentialsFlow',
      porDefecto: 'reset credentials',
    });
    // ⭐⭐ LOS AJUSTES DEL REALM, que hasta hoy se declaraban y no se aplicaban nunca.
    const ajustes = await aplicarAjustes(llamar, realm, deseado);
    // ⭐ ORE (0048): la tercera puerta, el registro.
    for (const [alias, antes, despues] of await acciones(llamar, realm, deseado, true)) {
      if (antes === 'no registrada') console.log(`   ⚠️  la acción ${alias} NO está registrada en ${realm}: el registro sigue abierto con un factor`);
      else console.log(`   ⭐ acción ${alias}: ${JSON.stringify(antes)} → ${JSON.stringify(despues)}`);
    }
    const cli = await aplicarCliente(llamar, realm, deseado);
    const cliente = cli?.clientId ?? null;
    // ⭐ Sólo en los realms SaaS: `rubix-interno` no administra clientes de nadie.
    let lector = null;
    // ✏️ ORE (0048): el lector sólo si el realm deseado lo declara (hoy, no).
    if (deseado.organizationsEnabled && (deseado.clients ?? []).some((c) => c.clientId === CLIENTE_LECTOR)) {
      lector = await aplicarLector(llamar, realm);
    }
    // ⭐⭐ UN SOLO MAPPER escribiendo `organization`. Va en la misma pasada porque un realm
    //   con el claim duplicado no deja entrar a NADIE — y el síntoma no nombra al culpable.
    const claim = deseado.organizationsEnabled ? await aplicarClaimDeOrganizacion(llamar, realm) : null;
    // ⛔ Y LOS RETIRADOS FUERA (0048, deuda 1): retirar es borrar, y se nombra.
    const ret = await retirarClientes(llamar, realm, deseado);
    console.log(`✅ aplicado en ${realm}${cliente ? ` · cliente ${cliente} con sus URIs` : ''}`);
    if (ret.borrados.length) console.log(`   ⛔ clientes RETIRADOS (borrados): ${ret.borrados.join(', ')}`);
    else console.log('   ✅ ningún cliente retirado sigue vivo');
    if (ret.desconocidos.length) console.log(`   ⚠️  vivos y NO declarados (no se tocan): ${ret.desconocidos.join(', ')}`);
    // ⭐ Se nombra lo que CAMBIÓ, con el valor viejo y el nuevo. «Ajustes aplicados» a secas no
    //   distingue «había tres que corregir» de «no había nada que hacer».
    if (ajustes.cambios.length) {
      for (const [campo, antes, ahora] of ajustes.cambios) {
        console.log(`   ⭐ ajuste ${campo}: ${JSON.stringify(antes)} → ${JSON.stringify(ahora)}`);
      }
    } else {
      console.log(`   ✅ los ${AJUSTES_GOBERNADOS.length} ajustes gobernados ya eran los declarados`);
    }
    if (cli?.ambitos) {
      const { faltan, sobran, inexistentes } = cli.ambitos;
      if (faltan.length) console.log(`   ⭐ ámbitos por defecto PUESTOS: ${faltan.join(', ')}`);
      if (sobran.length) console.log(`   ⛔ ámbitos por defecto RETIRADOS: ${sobran.join(', ')}`);
      if (inexistentes.length) console.log(`   ⚠️  ámbitos pedidos que NO existen en el realm: ${inexistentes.join(', ')}`);
      if (!faltan.length && !sobran.length) console.log('   ✅ los ámbitos por defecto ya eran los declarados');
    }
    if (claim) {
      if (claim.configurado) console.log(`   ⭐ ámbito «organization» → addOrganizationId=true`);
      if (claim.retirados.length) console.log(`   ⛔ mappers DUPLICADOS retirados: ${claim.retirados.join(', ')}`);
      if (!claim.configurado && !claim.retirados.length) console.log('   ✅ un solo mapper escribe «organization»');
    }
    if (lector) {
      // ⛔ Los retirados se NOMBRAN. Una resta silenciosa es indistinguible de no haber
      //   restado nada, y esto es exactamente lo que hay que poder leer en un despliegue.
      console.log(`   ↳ ${CLIENTE_LECTOR}: ${PAPELES_DE_LECTURA.length} papeles de lectura`
        + (lector.puestos ? ` · ${lector.puestos} puestos` : '')
        + (lector.retirados.length ? ` · ⛔ RETIRADOS: ${lector.retirados.join(', ')}` : ''));
    }
  }
}

for (const [realm] of DESEADOS) {
  if (SOLO_REALM && realm !== SOLO_REALM) continue;
  const m = await medirVivo(llamar, realm);
  // ⚠️ Los valores se sanean: esta línea la parte un `tr ' ' '\\n'`, y el flujo de
  //    FÁBRICA se llama `reset credentials` — CON ESPACIO. Sin esto, la guarda leería
  //    `flujo-reposicion=reset` y compararía contra media palabra, en verde.
  console.log(Object.entries(m).map(([k, v]) => `${k}=${String(v).replace(/\s+/g, '_')}`).join(' '));
}
