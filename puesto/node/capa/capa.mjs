// RESOLVER LA CAPA DE NODE (ORE 0050 R3 T5b) — dentro de `capa-node:1`.
//
// El gemelo de `puesto/jvm/capa/Capa.java`, para npm:
//
//   declarar <árbol> <alcance> <trabajo>   los package.json del alcance → deps.txt · digest.txt
//   package  <trabajo> <provisto>          lo que npm resuelve: lo declarado sin lo que la sesión trae
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

/** `dependencies` de un package.json, como `nombre@rango` (ver `entorno.rs`). */
function declaradas(texto) {
  let j;
  try {
    j = JSON.parse(texto);
  } catch {
    return [];
  }
  const d = j?.dependencies;
  if (!d || typeof d !== "object" || Array.isArray(d)) return [];
  const fuera = [];
  for (const [k, v] of Object.entries(d)) {
    if (typeof v !== "string") continue;
    const nombre = k.trim();
    const rango = v.trim();
    if (!nombre || /\s/.test(nombre) || ["file:", "link:", "workspace:"].some((p) => rango.startsWith(p))) continue;
    fuera.push(`${nombre}@${rango || "*"}`);
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

/** Lo que la sesión ya trae (`provisto.txt`: `nombre@versión` por línea). */
function provisto(f) {
  return new Map(
    readFileSync(f, "utf8")
      .split("\n")
      .map((l) => l.trim())
      .filter((l) => l && !l.startsWith("#"))
      .map(partir),
  );
}

function paquete(trabajo, ficheroProvisto) {
  const deps = readFileSync(join(trabajo, "deps.txt"), "utf8").split("\n").filter(Boolean);
  const trae = provisto(ficheroProvisto);
  const dependencies = {};
  const avisos = [];
  for (const d of deps) {
    const [n, r] = partir(d);
    if (trae.has(n)) {
      // ⭐ MANDA EL CONTENEDOR, como en la JVM: el SDK está hecho contra lo que
      //   la imagen pone, y dos copias del mismo paquete son dos módulos.
      if (r !== "*" && r !== trae.get(n)) avisos.push(`pediste ${n} ${r}, y esta sesión trae la ${trae.get(n)}: se usa la de la sesión`);
      continue;
    }
    dependencies[n] = r;
  }
  writeFileSync(join(trabajo, "package.json"), JSON.stringify({ name: "capa", private: true, type: "module", dependencies }, null, 2) + "\n");
  writeFileSync(join(trabajo, "avisos.txt"), avisos.join("\n") + (avisos.length ? "\n" : ""));
  console.log(`### a npm: ${Object.keys(dependencies).length} paquete(s)${avisos.length ? ` · ${avisos.length} aviso(s)` : ""}`);
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
  // El lock: el conjunto exacto que npm resolvió (`package-lock.json`).
  const lock = [];
  const lockf = join(trabajo, "capa", "package-lock.json");
  if (existsSync(lockf)) {
    const l = JSON.parse(readFileSync(lockf, "utf8"));
    for (const [ruta, p] of Object.entries(l.packages ?? {})) {
      if (!ruta) continue;
      lock.push(`${ruta.replace(/^.*node_modules\//, "")}@${p.version}`);
    }
    lock.sort(porBytes);
  }
  for (const n of nativos(join(trabajo, "capa", "node_modules"))) {
    avisos.push(`${n} compila código nativo al instalarse, y la capa no ejecuta scripts de instalación (--ignore-scripts): no funcionará`);
  }
  const caja = join(trabajo, "capa.tgz");
  let suma = "";
  let mb = 0;
  if (existsSync(caja)) {
    const b = readFileSync(caja);
    suma = createHash("sha256").update(b).digest("hex");
    mb = Math.ceil(b.length / 1048576);
  }
  const tope = Number(process.env.TOPE_MB ?? "512");
  let error = "";
  if (estado === "error") error = ultimas(join(trabajo, "npm.log"), 800);
  else if (mb > tope) {
    estado = "error";
    error = `la capa pesa ${mb} MB y el tope es ${tope} MB: un puesto que tarda dos minutos en arrancar no es un puesto`;
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
    lock,
    mb: String(mb),
    avisos,
    cuando: new Date().toISOString().replace(/\.\d+Z$/, "Z"),
    entorno: "puesto-node:1",
    ...(error ? { error } : {}),
  };
  writeFileSync(join(trabajo, "informe.json"), JSON.stringify(j, null, 1) + "\n");
  console.log(`### informe ${estado} · ${lock.length} paquete(s) · ${mb} MB${avisos.length ? ` · ${avisos.length} aviso(s)` : ""}`);
  for (const a of avisos) console.log(`    ⚠️ ${a}`);
}

switch (orden) {
  case "declarar": declarar(args[0], args[1], args[2]); break;
  case "package": paquete(args[0], args[1]); break;
  case "informe": informe(args[0], args[1]); break;
  default:
    console.error("capa.mjs declarar|package|informe …");
    process.exit(2);
}
