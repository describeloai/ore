// RESOLVER LA CAPA DE NODE (ORE 0050 R3 T5b) — dentro de `capa-node:1`.
//
// El gemelo de `puesto/jvm/capa/Capa.java`, para npm:
//
//   declarar <árbol> <alcance> <trabajo>   los package.json del alcance → deps.txt · digest.txt
//   package  <trabajo> <provisto>          lo que npm resuelve: lo declarado sin lo que la sesión trae
//   tipos    <trabajo>                     lo de desarrollo: lo que la resolución entera tiene y la de ejecución no (L3·1)
//   informe  <trabajo> <estado>            lo resuelto → informe.json (lock, suma de la caja, avisos)
//                                          y, si está lista, lock-del-repositorio.json (L2)
//
// ⚠️ LA DECLARACIÓN Y EL DIGEST SON LOS DE `ore-serve` (`entorno.rs`,
//   `dependencias_de_package` y `digest_de`), byte a byte: si difieren, el Job
//   resuelve una capa con otro nombre del que la sesión espera. Lo mismo que
//   allí: sólo `dependencies`, como `nombre@rango`, sin lo local (`file:`,
//   `link:`, `workspace:`), la unión ordenada y sin repetidos, y el digest es
//   `capa-` + 12 hex del sha256 de `node\n` + las líneas.
import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const [orden, ...args] = process.argv.slice(2);

/** Lo de desarrollo lleva esta marca delante (L3·1): `dev:@types/lodash@^4`. */
const DEV = "dev:";

/**
 * `dependencies` de un package.json, como `nombre@rango`, y —L3·1—
 * `devDependencies`, como `dev:nombre@rango` (ver `entorno.rs`).
 */
function declaradas(texto) {
  let j;
  try {
    j = JSON.parse(texto);
  } catch {
    return [];
  }
  const fuera = [];
  for (const [seccion, prefijo] of [["dependencies", ""], ["devDependencies", DEV]]) {
    const d = j?.[seccion];
    if (!d || typeof d !== "object" || Array.isArray(d)) continue;
    for (const [k, v] of Object.entries(d)) {
      if (typeof v !== "string") continue;
      const nombre = k.trim();
      const rango = v.trim();
      if (!nombre || /\s/.test(nombre) || ["file:", "link:", "workspace:"].some((p) => rango.startsWith(p))) continue;
      fuera.push(`${prefijo}${nombre}@${rango || "*"}`);
    }
  }
  return fuera;
}

/** Los package.json de un alcance, como `declaracion_en`. */
function ficheros(arbol, alcance) {
  const f = [join(arbol, "package.json")];
  const a = (alcance ?? "").trim().replace(/^\/+|\/+$/g, "");
  if (!a) {
    const p = join(arbol, "packages");
    if (existsSync(p)) for (const e of readdirSync(p)) f.push(join(p, e, "package.json"));
  } else {
    const partes = a.split("/");
    if (partes.length >= 2 && partes[0] === "packages") {
      let acc = join(arbol, "packages", partes[1]);
      f.push(join(acc, "package.json"));
      for (const p of partes.slice(2)) {
        acc = join(acc, p);
        f.push(join(acc, "package.json"));
      }
    }
  }
  return f;
}

/** El orden de `Vec<String>::sort` de Rust: por bytes. */
const porBytes = (a, b) => Buffer.compare(Buffer.from(a), Buffer.from(b));

/** `nombre@rango` → [nombre, rango] (el nombre puede empezar por `@`). */
function partir(dep) {
  const i = dep.lastIndexOf("@");
  return i > 0 ? [dep.slice(0, i), dep.slice(i + 1)] : [dep, "*"];
}

function declarar(arbol, alcance, trabajo) {
  const deps = [...new Set(ficheros(arbol, alcance).filter((f) => existsSync(f)).flatMap((f) => declaradas(readFileSync(f, "utf8"))))].sort(porBytes);
  const digest = deps.length ? "capa-" + createHash("sha256").update("node\n" + deps.join("\n")).digest("hex").slice(0, 12) : "";
  writeFileSync(join(trabajo, "deps.txt"), deps.join("\n") + "\n");
  writeFileSync(join(trabajo, "digest.txt"), digest);
  console.log(`### alcance: ${alcance || "(la celda)"}`);
  console.log(`### declarado: ${deps.length ? deps.join(", ") : "(nada)"} → ${digest || "(sin capa)"}`);
}

/**
 * Lo que la sesión ya trae (`provisto.txt`: `nombre@versión` por línea; con
 * ` tipos` detrás, lo que sólo tipa —L3·2—: el compilador, el servidor de
 * lenguaje, `@types/node`). Para la capa todo es lo mismo: no se copia.
 */
function provisto(f) {
  return new Map(
    readFileSync(f, "utf8")
      .split("\n")
      .map((l) => l.trim())
      .filter((l) => l && !l.startsWith("#"))
      .map((l) => partir(l.split(/\s+/)[0])),
  );
}

function paquete(trabajo, ficheroProvisto) {
  const deps = readFileSync(join(trabajo, "deps.txt"), "utf8").split("\n").filter(Boolean);
  const trae = provisto(ficheroProvisto);
  const dependencies = {};
  const devDependencies = {};
  const avisos = [];
  for (const d of deps) {
    const dev = d.startsWith(DEV);
    const [n, r] = partir(dev ? d.slice(DEV.length) : d);
    if (trae.has(n)) {
      // ⭐ MANDA EL CONTENEDOR, como en la JVM: el SDK está hecho contra lo que
      //   la imagen pone, y dos copias del mismo paquete son dos módulos.
      //   También en lo de desarrollo: el `tsc` y los tipos de Node son los de
      //   la imagen, y un `@types/node` de otra versión diría otro Node.
      if (r !== "*" && r !== trae.get(n)) avisos.push(`you asked for ${n} ${r}, and this session brings ${trae.get(n)}: the session's version is used`);
      continue;
    }
    (dev ? devDependencies : dependencies)[n] = r;
  }
  writeFileSync(join(trabajo, "package.json"), JSON.stringify({ name: "capa", private: true, type: "module", dependencies, devDependencies }, null, 2) + "\n");
  writeFileSync(join(trabajo, "avisos.txt"), avisos.join("\n") + (avisos.length ? "\n" : ""));
  const nd = Object.keys(devDependencies).length;
  console.log(`### a npm: ${Object.keys(dependencies).length} paquete(s)${nd ? ` + ${nd} de desarrollo` : ""}${avisos.length ? ` · ${avisos.length} aviso(s)` : ""}`);
}

/** Los paquetes de primer nivel de un `node_modules` (`dayjs`, `@types/lodash`). */
function primerNivel(dir) {
  if (!existsSync(dir)) return [];
  const fuera = [];
  for (const e of readdirSync(dir)) {
    if (e.startsWith(".")) continue;
    if (e.startsWith("@")) for (const s of readdirSync(join(dir, e))) fuera.push(`${e}/${s}`);
    else fuera.push(e);
  }
  return fuera.sort(porBytes);
}

/**
 * ⭐ L3·1: LA CAJA DE TIPOS es lo que la resolución entera (`capa/`, con lo de
 * desarrollo) tiene y la de ejecución (`run/`, `npm ci --omit=dev` desde el
 * MISMO lock) no. Una resolución, dos cajas: lo de desarrollo nunca llega a
 * ejecución, y lo que tipa es exactamente lo que corre. Deja `tipos.txt`, las
 * rutas para `tar -T`.
 */
function tipos(trabajo) {
  const run = new Set(primerNivel(join(trabajo, "run", "node_modules")));
  const dev = primerNivel(join(trabajo, "capa", "node_modules")).filter((p) => !run.has(p));
  writeFileSync(join(trabajo, "tipos.txt"), dev.map((p) => `node_modules/${p}\n`).join(""));
  console.log(`### tipos: ${dev.length} paquete(s) de desarrollo${dev.length ? ` (${dev.join(", ")})` : ""}`);
}

/** Los paquetes de node_modules que compilan código nativo al instalarse. */
function nativos(dir, fuera = []) {
  if (!existsSync(dir)) return fuera;
  for (const e of readdirSync(dir)) {
    if (e.startsWith(".")) continue;
    const p = join(dir, e);
    if (e.startsWith("@")) {
      nativos(p, fuera);
      continue;
    }
    if (!statSync(p).isDirectory()) continue;
    if (existsSync(join(p, "binding.gyp"))) fuera.push(e);
    nativos(join(p, "node_modules"), fuera);
  }
  return fuera;
}

function ultimas(f, n) {
  if (!existsSync(f)) return "";
  const t = readFileSync(f, "utf8");
  return t.length > n ? "…" + t.slice(-n) : t;
}

function informe(trabajo, estadoDado) {
  let estado = estadoDado;
  const deps = readFileSync(join(trabajo, "deps.txt"), "utf8").split("\n").filter(Boolean);
  const digest = existsSync(join(trabajo, "digest.txt")) ? readFileSync(join(trabajo, "digest.txt"), "utf8").trim() : "";
  const avisos = existsSync(join(trabajo, "avisos.txt")) ? readFileSync(join(trabajo, "avisos.txt"), "utf8").split("\n").filter(Boolean) : [];
  // El lock: el conjunto exacto que npm resolvió (`package-lock.json`), el de
  // la resolución ENTERA; lo de desarrollo, con `dev:` delante (L3·1).
  const lock = [];
  const lockf = join(trabajo, "capa", "package-lock.json");
  if (existsSync(lockf)) {
    const l = JSON.parse(readFileSync(lockf, "utf8"));
    for (const [ruta, p] of Object.entries(l.packages ?? {})) {
      if (!ruta) continue;
      lock.push(`${p.dev ? DEV : ""}${ruta.replace(/^.*node_modules\//, "")}@${p.version}`);
    }
    lock.sort(porBytes);
  }
  // Lo nativo sólo importa en lo que corre: un paquete de tipos no se ejecuta.
  for (const n of nativos(join(trabajo, "run", "node_modules"))) {
    avisos.push(`${n} compiles native code when it installs, and install scripts don't run here (--ignore-scripts): it won't work`);
  }
  const sumaDe = (f) => {
    if (!existsSync(f)) return ["", 0];
    const b = readFileSync(f);
    return [createHash("sha256").update(b).digest("hex"), Math.ceil(b.length / 1048576)];
  };
  const [suma, mb] = sumaDe(join(trabajo, "capa.tgz"));
  const [sumaTipos, mbTipos] = sumaDe(join(trabajo, "tipos.tgz"));
  const tope = Number(process.env.TOPE_MB ?? "512");
  let error = "";
  if (estado === "error") error = ultimas(join(trabajo, "npm.log"), 800);
  else if (mb > tope) {
    estado = "error";
    error = `the libraries take ${mb} MB and the limit is ${tope} MB: a session that takes minutes to start is not a session`;
  } else if (mbTipos > tope) {
    estado = "error";
    error = `the dev packages take ${mbTipos} MB and the limit is ${tope} MB`;
  }
  // ⭐ L2: el lock, para el repositorio —`package-lock.json` junto a su
  //   `package.json`, lo que se versiona—. El de npm tal cual (`resolved`,
  //   `integrity`), sin el nombre de la caja de trabajo (`capa`). Sólo de una
  //   capa lista: una que falló no fija nada. Lo que la sesión trae no está
  //   en él: no lo instala la capa.
  const delRepositorio = join(trabajo, "lock-del-repositorio.json");
  if (estado === "lista" && existsSync(lockf)) {
    const l = JSON.parse(readFileSync(lockf, "utf8"));
    delete l.name;
    if (l.packages?.[""]) delete l.packages[""].name;
    writeFileSync(delRepositorio, JSON.stringify(l, null, 2) + "\n");
  }
  const j = {
    estado,
    digest,
    declarado: deps,
    ...(suma ? { caja: "capa.tgz", suma } : {}),
    // L3·1: lo de desarrollo, en su caja: lo lee lo que tipa, nunca lo que corre.
    ...(sumaTipos ? { cajaTipos: "tipos.tgz", sumaTipos, mbTipos: String(mbTipos) } : {}),
    lock,
    mb: String(mb),
    avisos,
    cuando: new Date().toISOString().replace(/\.\d+Z$/, "Z"),
    entorno: "puesto-node:1",
    ...(error ? { error } : {}),
  };
  writeFileSync(join(trabajo, "informe.json"), JSON.stringify(j, null, 1) + "\n");
  console.log(`### informe ${estado} · ${lock.length} paquete(s) · ${mb} MB${sumaTipos ? ` + ${mbTipos} MB de tipos` : ""}${avisos.length ? ` · ${avisos.length} aviso(s)` : ""}`);
  for (const a of avisos) console.log(`    ⚠️ ${a}`);
}

switch (orden) {
  case "declarar": declarar(args[0], args[1], args[2]); break;
  case "package": paquete(args[0], args[1]); break;
  case "tipos": tipos(args[0]); break;
  case "informe": informe(args[0], args[1]); break;
  default:
    console.error("capa.mjs declarar|package|tipos|informe …");
    process.exit(2);
}
