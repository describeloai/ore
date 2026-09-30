// ═══════════════════════════════════════════════════════════════════
// LOS REALMS DE ORE — lo que la definición (`realm.mjs`) no sabe de ORE, y el
// manifiesto (`malla/61-realms.yaml`). ADR 0048: la identidad es de ORE.
//
//   node identidad/ore.mjs          → escribe malla/61-realms.yaml
//
// ⭐⭐ UN SOLO REALM DESEADO. `realmsDeOre()` es lo que se importa en un realm nuevo
//   (el manifiesto) Y lo que el reconciliador (`aplicar.mjs`) concilia en uno vivo.
//   Hasta 0048 eran dos: el reconciliador de la plataforma conciliaba `realmSaaS()` a
//   secas, sin lo de ORE — y ejecutado habría encendido `verifyEmail` (sin correo: quien
//   se registra se queda esperando) y quitado `localhost` de la consola.
//
// Portado de `malla/gen-realm.py` sin cambiar lo que emite: mismas transformaciones,
// mismo orden, y el perfil de usuario serializado como lo hacía Python (`, ` y `: `).
// ═══════════════════════════════════════════════════════════════════

import { writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { realmSaaS, realmInterno } from './realm.mjs';

const AQUI = dirname(fileURLToPath(import.meta.url));
export const MANIFIESTO = join(AQUI, '..', 'malla', '61-realms.yaml');

/** La organización del realm de producción (una Organization de Keycloak): la primera
 *  de la plataforma. La pertenencia la decide `ore-iam` (0047 A9′), no esta lista. */
export const ORGANIZACIONES = ['o_01J9ZKCP4N7QR2S5T8V0W3X6Y9'];

// ── Lo que ORE añade, en los realms que lo llevan ──────────────────────────
// (las razones, en la cabecera del manifiesto y en el historial de gen-realm.py)
const REALMS_CON_ORE = ['rubix', 'rubix-interno'];

/** `ore-serve`: una AUDIENCIA. No inicia sesión de nadie. */
const AUDIENCIA = {
  "clientId": "ore-serve",
  "name": "ORE · el plano de control",
  "description": "Audiencia. No inicia sesion: existe para poder decir que un token es PARA nosotros.",
  "enabled": true,
  "protocol": "openid-connect",
  "publicClient": false,
  "standardFlowEnabled": false,
  "directAccessGrantsEnabled": false,
  "serviceAccountsEnabled": false,
  "implicitFlowEnabled": false
};

/** El mapeador que mete `ore-serve` en el `aud` (en `ore-agente` y en `rubix-consola`). */
const MAPEADOR_AUDIENCIA = {
  "name": "audiencia-ore-serve",
  "protocol": "openid-connect",
  "protocolMapper": "oidc-audience-mapper",
  "consentRequired": false,
  "config": {
    "included.client.audience": "ore-serve",
    "id.token.claim": "false",
    "access.token.claim": "true"
  }
};

/** `ore-agente`: la cuenta de servicio genérica (las de cada celda las crea el
 *  aprovisionador, ⑦). Lleva `modelos` en el `aud` (0027 E2) y `rubix_tipo=agente`. */
const AGENTE = {
  "clientId": "ore-agente",
  "name": "ORE · agente",
  "description": "Cuenta de servicio: pide tokens para `ore-serve`. Es un agente, no una persona.",
  "enabled": true,
  "protocol": "openid-connect",
  "publicClient": false,
  "standardFlowEnabled": false,
  "directAccessGrantsEnabled": false,
  "serviceAccountsEnabled": true,
  "implicitFlowEnabled": false,
  "attributes": {
    "access.token.lifespan": "300"
  },
  "protocolMappers": [
    {
      "name": "audiencia-ore-serve",
      "protocol": "openid-connect",
      "protocolMapper": "oidc-audience-mapper",
      "consentRequired": false,
      "config": {
        "included.client.audience": "ore-serve",
        "id.token.claim": "false",
        "access.token.claim": "true"
      }
    },
    {
      "name": "audiencia-modelos",
      "protocol": "openid-connect",
      "protocolMapper": "oidc-audience-mapper",
      "consentRequired": false,
      "config": {
        "included.custom.audience": "modelos",
        "id.token.claim": "false",
        "access.token.claim": "true"
      }
    },
    {
      "name": "rubix-tipo-agente",
      "protocol": "openid-connect",
      "protocolMapper": "oidc-hardcoded-claim-mapper",
      "consentRequired": false,
      "config": {
        "claim.name": "rubix_tipo",
        "claim.value": "agente",
        "jsonType.label": "String",
        "access.token.claim": "true"
      }
    }
  ]
};

/** El ámbito que lleva `sub` (Keycloak 24+): sin él, un token válido y ANÓNIMO. */
const AMBITO_DEL_SUJETO = 'basic';

/** El registro abierto en `rubix` (0025 E6), sin `verifyEmail` mientras no haya correo. */
const REGISTRO_EN_PRODUCCION = {
  "realm": "rubix",
  "registrationAllowed": true,
  "verifyEmail": false
};

/** El registro pregunta por la organización (035): atributo del perfil y su claim. */
const ATRIBUTO_ORGANIZACION = {
  "name": "organizacion",
  "displayName": "Organización",
  "validations": {
    "length": {
      "min": 2,
      "max": 80
    }
  },
  "annotations": {
    "inputHelperTextBefore": "El nombre de tu empresa o equipo. Sera tu cuenta en Rubix; el identificador (minusculas y guiones) se deriva de el."
  },
  "required": {
    "roles": [
      "user"
    ]
  },
  "permissions": {
    "view": [
      "admin",
      "user"
    ],
    "edit": [
      "admin",
      "user"
    ]
  },
  "multivalued": false
};
const MAPEADOR_ORGANIZACION = {
  "name": "rubix-organizacion",
  "protocol": "openid-connect",
  "protocolMapper": "oidc-usermodel-attribute-mapper",
  "consentRequired": false,
  "config": {
    "user.attribute": "organizacion",
    "claim.name": "rubix_organizacion",
    "jsonType.label": "String",
    "id.token.claim": "true",
    "access.token.claim": "true",
    "userinfo.token.claim": "true"
  }
};
const PERFIL_BASE = [
  {
    "name": "username",
    "displayName": "${username}",
    "validations": {
      "length": {
        "min": 3,
        "max": 255
      },
      "username-prohibited-characters": {},
      "up-username-not-idn-homograph": {}
    },
    "permissions": {
      "view": [
        "admin",
        "user"
      ],
      "edit": [
        "admin",
        "user"
      ]
    },
    "multivalued": false
  },
  {
    "name": "email",
    "displayName": "${email}",
    "validations": {
      "email": {},
      "length": {
        "max": 255
      }
    },
    "required": {
      "roles": [
        "user"
      ]
    },
    "permissions": {
      "view": [
        "admin",
        "user"
      ],
      "edit": [
        "admin",
        "user"
      ]
    },
    "multivalued": false
  },
  {
    "name": "firstName",
    "displayName": "${firstName}",
    "validations": {
      "length": {
        "max": 255
      },
      "person-name-prohibited-characters": {}
    },
    "required": {
      "roles": [
        "user"
      ]
    },
    "permissions": {
      "view": [
        "admin",
        "user"
      ],
      "edit": [
        "admin",
        "user"
      ]
    },
    "multivalued": false
  },
  {
    "name": "lastName",
    "displayName": "${lastName}",
    "validations": {
      "length": {
        "max": 255
      },
      "person-name-prohibited-characters": {}
    },
    "required": {
      "roles": [
        "user"
      ]
    },
    "permissions": {
      "view": [
        "admin",
        "user"
      ],
      "edit": [
        "admin",
        "user"
      ]
    },
    "multivalued": false
  }
];

/** La consola en local entra por `rubix`: cliente público con PKCE (S256). */
const CONSOLA_LOCAL = "http://localhost:3000";

const copia = (x) => JSON.parse(JSON.stringify(x));

/** JSON compacto como `json.dumps` de Python (`, ` y `: `): el perfil de usuario viaja
 *  como cadena dentro del realm, y así el manifiesto no cambia al cambiar de lenguaje. */
export function jsonPy(v) {
  if (Array.isArray(v)) return `[${v.map(jsonPy).join(', ')}]`;
  if (v && typeof v === 'object') {
    return `{${Object.entries(v).map(([k, x]) => `${JSON.stringify(k)}: ${jsonPy(x)}`).join(', ')}}`;
  }
  return JSON.stringify(v);
}

function conSujeto(realm) {
  for (const c of realm.clients ?? []) {
    // ⛔ Sólo si la lista está FIJADA: si no, hereda la del realm.
    if (Array.isArray(c.defaultClientScopes) && !c.defaultClientScopes.includes(AMBITO_DEL_SUJETO)) {
      c.defaultClientScopes.push(AMBITO_DEL_SUJETO);
    }
  }
  return realm;
}

function conOre(realm) {
  realm.clients ??= [];
  const ya = new Set(realm.clients.map((c) => c.clientId));
  if (!ya.has('ore-serve')) realm.clients.push(copia(AUDIENCIA));
  if (!ya.has('ore-agente')) realm.clients.push(copia(AGENTE));
  for (const c of realm.clients) {
    if (c.clientId !== 'rubix-consola') continue;
    c.protocolMappers ??= [];
    if (!c.protocolMappers.some((x) => x.name === 'audiencia-ore-serve')) c.protocolMappers.push(copia(MAPEADOR_AUDIENCIA));
  }
  return realm;
}

function conOrganizacionEnElRegistro(realm) {
  realm.components ??= {};
  const lista = (realm.components['org.keycloak.userprofile.UserProfileProvider'] ??= []);
  if (!lista.length) lista.push({ name: 'declarative-user-profile', providerId: 'declarative-user-profile', subComponents: {}, config: {} });
  const cfg = (lista[0].config ??= {});
  const perfil = cfg['kc.user.profile.config']?.length
    ? JSON.parse(cfg['kc.user.profile.config'][0])
    : { attributes: copia(PERFIL_BASE), groups: [{ name: 'user-metadata', displayHeader: 'User metadata', displayDescription: 'Attributes, which refer to user metadata' }] };
  if (!perfil.attributes.some((a) => a.name === 'organizacion')) perfil.attributes.push(copia(ATRIBUTO_ORGANIZACION));
  cfg['kc.user.profile.config'] = [jsonPy(perfil)];
  for (const c of realm.clients ?? []) {
    if (c.clientId !== 'rubix-consola') continue;
    c.protocolMappers ??= [];
    if (!c.protocolMappers.some((x) => x.name === 'rubix-organizacion')) c.protocolMappers.push(copia(MAPEADOR_ORGANIZACION));
  }
  return realm;
}

function conRegistro(realm) {
  if (realm.realm !== REGISTRO_EN_PRODUCCION.realm) return realm;
  realm.registrationAllowed = REGISTRO_EN_PRODUCCION.registrationAllowed;
  realm.verifyEmail = REGISTRO_EN_PRODUCCION.verifyEmail;
  return conOrganizacionEnElRegistro(realm);
}

// ⛔ EL LECTOR NO (0048). `rubix-consola-lector` era la cuenta con la que la consola de
//   administración de la plataforma leía organizaciones y usuarios del IdP (papeles
//   `view-users`, `query-users`…). En ORE nadie la usa —la pertenencia la decide `ore-iam`
//   (0047 A9′)— y su secreto vivía en el proyecto viejo. Medido con `aplicar.mjs --plan`
//   el 2026-09-30: en vivo existe SIN papeles, y conciliarla se los habría DADO. Una cuenta
//   que puede leer a todos los usuarios y que nadie usa es sólo riesgo: no se declara.
const SIN_USO = ['rubix-consola-lector'];
function sinLoQueNadieUsa(realm) {
  realm.clients = (realm.clients ?? []).filter((c) => !SIN_USO.includes(c.clientId));
  return realm;
}

function conConsolaLocal(realm) {
  if (realm.realm !== REGISTRO_EN_PRODUCCION.realm) return realm;
  for (const c of realm.clients ?? []) {
    if (c.clientId !== 'rubix-consola') continue;
    c.redirectUris ??= [];
    if (!c.redirectUris.includes(`${CONSOLA_LOCAL}/auth/callback`)) c.redirectUris.push(`${CONSOLA_LOCAL}/auth/callback`);
    c.webOrigins ??= [];
    if (!c.webOrigins.includes(CONSOLA_LOCAL)) c.webOrigins.push(CONSOLA_LOCAL);
    c.attributes ??= {};
    const salidas = (c.attributes['post.logout.redirect.uris'] ?? '').split('##').filter(Boolean);
    if (!salidas.includes(`${CONSOLA_LOCAL}/`)) salidas.push(`${CONSOLA_LOCAL}/`);
    c.attributes['post.logout.redirect.uris'] = salidas.join('##');
  }
  return realm;
}

// ⛔⛔ LA TERCERA PUERTA: EL REGISTRO (medido en vivo el 2026-09-30, recién saldada AAL2).
//   Con la entrada y la reposición en REQUIRED, un usuario NUEVO se registró y entró en la
//   consola de su cuenta con UN factor: el registro no pasa por el flujo de entrada, y
//   Keycloak abre la sesión al terminarlo. Lo que lo cierra es una ACCIÓN POR DEFECTO: se le
//   pone a todo usuario al crearse y se ejecuta antes de abrir la sesión. La passkey
//   (`webauthn-register`), porque es el factor principal de `realm.mjs` (resistente a
//   phishing); TOTP queda habilitado —no obligatorio— como recuperación.
//
//   ⚠️ No va dentro del realm: `requiredActions` en un import SUSTITUYE la lista entera de
//     Keycloak. Lo concilia `aplicar.mjs` acción a acción, y `medir.mjs` lo exige.
export const ACCIONES_POR_DEFECTO = {
  'webauthn-register': { enabled: true, defaultAction: true },
  CONFIGURE_TOTP: { enabled: true, defaultAction: false },
};
/** Las acciones que el realm `nombre` quiere, o nada si no abre el registro. */
export function accionesDe(realm) {
  return realm.registrationAllowed ? ACCIONES_POR_DEFECTO : {};
}
/** Las acciones requeridas que dejan al usuario con un segundo factor. */
export const ACCIONES_QUE_ENROLAN = ['webauthn-register', 'webauthn-register-passwordless', 'CONFIGURE_TOTP'];

/** ⭐ Los realms que ORE quiere, en el orden del manifiesto: `[[nombre, realm], …]`. */
export function realmsDeOre() {
  const base = [
    realmInterno(),
    realmSaaS({ organizaciones: ORGANIZACIONES, entorno: 'produccion' }),
  ].map(copia);
  return base.map((realm) => {
    realm = conSujeto(realm);
    if (REALMS_CON_ORE.includes(realm.realm)) realm = conOre(realm);
    realm = conRegistro(realm);
    realm = conConsolaLocal(realm);
    realm = sinLoQueNadieUsa(realm);
    return [realm.realm, realm];
  });
}

const CABECERA = "# LOS REALMS — GENERADOS. No se editan aqui.\n#\n#   node identidad/ore.mjs\n#\n# Salen de `identidad/realm.mjs` (la definicion: flujos, passkeys, factores,\n# correo, clientes) y de `identidad/ore.mjs` (lo que ORE anade). Desde el\n# 2026-09-30 los dos viven en ORE (ADR 0048): antes el primero estaba en la\n# plataforma, escribiendo para un cluster donde ya no vive el IdP.\n#\n# ── Los dos, y ninguno sobra ────────────────────────────────────────────────\n#\n#   rubix          produccion, y el UNICO con gente (el 2026-09-14 `rubix-dev`\n#                  se renombro a `rubix`, 034). Registro abierto; la consola en\n#                  local (`localhost:3000`) entra tambien por aqui\n#   rubix-interno  sin clientes propios\n#\n# ✏️ `rubix-dev` ya no sale (0048): no existe en vivo, `ore-iam` rechaza su\n#   emisor, y el reconciliador lo CREARIA al no encontrarlo.\n#\n# ── Lo que ORE anade, en LOS TRES ───────────────────────────────────────────\n#\n#   ore-serve    una AUDIENCIA. Todos los flujos apagados: no inicia sesion de\n#                nadie. Existe para poder decir que un token es PARA nosotros\n#   ore-agente   una cuenta de servicio que pide tokens para esa audiencia\n#   y en `rubix-consola`, el mapeador que mete `ore-serve` en el `aud`\n#\n# Todo lo demas viaja tal cual: la politica de clave, los cinco flujos propios,\n# las passkeys y las organizaciones.\n#\n# ── ⭐ AAL2 en las dos puertas ───────────────────────────────────────────────\n#\n# `EXIGIR_SEGUNDO_FACTOR = true` (saldada el 2026-09-30): la entrada y la\n# reposicion piden passkey o TOTP. `pruebas-de-fuego/el-segundo-factor.sh` lo\n# vigila sobre este fichero.\n#\n# ── ⚠️ Lo que un `KeycloakRealmImport` NO hace ──────────────────────────────\n#\n# Mantener: se salta un realm que ya existe y dice `Done: True` igualmente. Por\n# eso este fichero NO esta en `malla/kustomization.yaml`, y el realm vivo lo\n# concilia `identidad/aplicar.mjs` desde ESTOS mismos realms.\n\n";

/** El manifiesto: un `KeycloakRealmImport` por realm, en JSON (que es YAML). */
export function manifiesto() {
  const docs = realmsDeOre().map(([nombre, realm]) => ({
    apiVersion: 'k8s.keycloak.org/v2alpha1',
    kind: 'KeycloakRealmImport',
    metadata: {
      name: nombre,
      namespace: 'identidad',
      labels: { 'ore.dev/tenant': 'system', 'ore.dev/rol': 'identidad' },
    },
    spec: { keycloakCRName: 'idp', realm },
  }));
  return `${CABECERA}${docs.map((d) => JSON.stringify(d, null, 2)).join('\n---\n')}\n`;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1].replace(/\//g, '\\').replace(/^([a-z]):/, (_, l) => `${l.toUpperCase()}:`)
    || (process.argv[1] ?? '').replace(/\\/g, '/').endsWith('identidad/ore.mjs')) {
  writeFileSync(MANIFIESTO, manifiesto(), 'utf8');
  for (const [nombre, realm] of realmsDeOre()) {
    console.log(`${nombre.padEnd(14)} ${(realm.clients ?? []).length} clientes: ${(realm.clients ?? []).map((c) => c.clientId).join(', ') || '—'}`);
  }
  console.log(`escrito ${MANIFIESTO}`);
}
