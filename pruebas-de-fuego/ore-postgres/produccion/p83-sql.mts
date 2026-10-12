// El SQL que corre Tables (ADR 0058 P8·3), sacado del propio catalogo.ts de la consola, para que
// p83.sh mida exactamente lo que ejecuta la vista. $CATALOGO = ruta de components/postgres/catalogo.ts.
//   tsx p83-sql.mts catalogo                          → el SQL de la lista
//   tsx p83-sql.mts relacion SCHEMA 'JSON' DESDE LIMITE → columnas, [conteo] y página
import { pathToFileURL } from 'node:url';

const c = await import(pathToFileURL(process.env.CATALOGO!).href);
const [que, ...a] = process.argv.slice(2);
if (que === 'catalogo') process.stdout.write(c.SQL_CATALOGO);
else if (que === 'relacion') {
  const r = JSON.parse(a[1]);
  const clase = { r: 'tabla', v: 'vista', m: 'materializada', p: 'particionada', f: 'foranea' }[r.clase as string] ?? r.clase;
  process.stdout.write(c.sqlDeRelacion(a[0], { ...r, clase }, Number(a[2]), Number(a[3])));
} else throw new Error(`¿${que}?`);
