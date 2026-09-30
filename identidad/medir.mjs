// ═══════════════════════════════════════════════════════════════════
// LA ENTRADA, MEDIDA — lo que cada realm de ORE EXIGE de verdad (0048 I3)
//
//   node identidad/medir.mjs        → una línea por realm y, si algo no cumple, sale con 1
//
// Viene de la plataforma (`scripts/medir-entrada.mjs` y las reglas de `check-entrada.sh`)
// con un cambio de fondo: mide `realmsDeOre()`, que es lo que el manifiesto importa y
// `aplicar.mjs` concilia en vivo — no `realmSaaS()` a secas, que no es lo que corre.
//
// ⭐ Se mide RECORRIENDO los flujos, no leyendo un campo: un realm puede declarar
//   `rubix.aal = AAL2` y entrar con uno (fue el caso del 2026-08-26 al 2026-09-30).
//
// Las reglas:
//   ① la entrada exige DOS factores, y al menos uno resistente a phishing (WebAuthn)
//   ② lo declarado (`rubix.aal`) es lo medido
//   ③ la reposición no pide MENOS que la entrada, y ningún paso quita un factor
//   ④ ninguna credencial de correo en el artefacto
//   ⑤ ningún retorno `http://` a una máquina que no sea la propia (localhost es la consola en local)
// ═══════════════════════════════════════════════════════════════════

import {
  factoresMinimos, CREDENCIALES, FLUJO_ENTRADA, CREDENCIALES_DE_REPOSICION, PASOS_QUE_QUITAN_FACTOR,
} from './realm.mjs';
import { realmsDeOre } from './ore.mjs';

/** CISA los nombra resistentes a phishing; excluye SMS, voz, correo y el push de aprobar. */
const RESISTENTES = ['webauthn-authenticator', 'webauthn-authenticator-passwordless'];

/** Los autenticadores que un flujo usa de verdad, recorriendo los subflujos. */
function autenticadoresDe(realm, raiz) {
  const flujos = new Map((realm.authenticationFlows ?? []).map((f) => [f.alias, f]));
  const vistos = new Set();
  const usados = new Set();
  const andar = (alias) => {
    if (!flujos.has(alias) || vistos.has(alias)) return;
    vistos.add(alias);
    for (const e of flujos.get(alias).authenticationExecutions ?? []) {
      if (e.requirement === 'DISABLED') continue;
      if (e.autheticatorFlow) andar(e.flowAlias);
      else usados.add(e.authenticator);
    }
  };
  andar(raiz ?? realm.browserFlow);
  return [...usados];
}

/** Las URIs del cliente PÚBLICO que viajan sin cifrar (el que recibe el código en el navegador). */
const enClaro = (r) => (r.clients ?? []).filter((c) => c.publicClient)
  .flatMap((c) => [...(c.redirectUris ?? []), ...(c.webOrigins ?? [])])
  .filter((u) => u.startsWith('http://'));
const propia = (u) => /^http:\/\/(localhost|127\.0\.0\.1)(:|\/|$)/.test(u);

const fallos = [];
for (const [nombre, r] of realmsDeOre()) {
  const usados = autenticadoresDe(r);
  const factores = factoresMinimos(r);
  const resistentes = usados.filter((a) => RESISTENTES.includes(a)).length;
  const reposicion = factoresMinimos(r, { flujo: r.resetCredentialsFlow, credenciales: CREDENCIALES_DE_REPOSICION });
  const quitan = autenticadoresDe(r, r.resetCredentialsFlow).filter((a) => PASOS_QUE_QUITAN_FACTOR.includes(a));
  const aal = r.attributes?.['rubix.aal'] ?? 'sin-declarar';
  const ajenos = enClaro(r).filter((u) => !propia(u));
  console.log([
    `realm=${nombre}`, `factores=${factores}`, `flujo=${r.browserFlow ?? 'ninguno'}`,
    `propio=${r.browserFlow === FLUJO_ENTRADA}`, `resistentes=${resistentes}`,
    `credenciales=${usados.filter((a) => CREDENCIALES.includes(a)).length}`,
    `uv=${r.webAuthnPolicyUserVerificationRequirement ?? 'sin-politica'}`, `aal=${aal}`,
    `reposicion=${reposicion}`, `quitan=${quitan.length}`,
    `smtp=${r.smtpServer?.host ?? 'ninguno'}`, `retornos-en-claro=${enClaro(r).length}`, `ajenos=${ajenos.length}`,
  ].join(' '));
  if (factores < 2) fallos.push(`${nombre}: la entrada exige ${factores} factor(es), no dos`);
  if (resistentes < 1) fallos.push(`${nombre}: ningún factor resistente a phishing en la entrada`);
  if (aal === 'AAL2' && factores < 2) fallos.push(`${nombre}: declara AAL2 y entra con ${factores}`);
  if (reposicion < factores) fallos.push(`${nombre}: la reposición pide ${reposicion} y la entrada ${factores}`);
  if (quitan.length) fallos.push(`${nombre}: la reposición quita un factor (${quitan.join(', ')})`);
  if (r.smtpServer && 'password' in r.smtpServer) fallos.push(`${nombre}: una credencial de correo en el artefacto`);
  if (ajenos.length) fallos.push(`${nombre}: retornos en claro a otra máquina: ${ajenos.join(', ')}`);
}
if (fallos.length) {
  for (const f of fallos) console.error(`✗ ${f}`);
  process.exit(1);
}
console.log('✓ la entrada y la reposición exigen dos factores, uno resistente a phishing, en todos los realms');
