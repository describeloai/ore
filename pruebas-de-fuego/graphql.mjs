// El SDL que emitimos, enfrentado a `graphql-js` — la implementacion de
// referencia, no la nuestra.
//
// `buildSchema` analiza y `assertValidSchema` comprueba la validez del esquema
// resultante, que son dos cosas distintas: un SDL puede analizar y describir un
// esquema invalido. Un tipo que nadie alcanza, una directiva mal declarada, un
// campo cuyo tipo no existe — nada de eso lo ve quien lo escribio.
//
// Se lee el SDL por stdin, como todo lo demas en este repositorio.
import { buildSchema, assertValidSchema } from "graphql";

const sdl = await new Promise((res, rej) => {
  let t = "";
  process.stdin.setEncoding("utf8");
  process.stdin.on("data", (d) => (t += d));
  process.stdin.on("end", () => res(t));
  process.stdin.on("error", rej);
});

if (!sdl.trim()) {
  console.error("no llego ningun SDL por stdin");
  process.exit(2);
}

try {
  assertValidSchema(buildSchema(sdl));
} catch (e) {
  console.error(`graphql-js RECHAZA el esquema que emitimos:\n  ${e.message}`);
  process.exit(1);
}
