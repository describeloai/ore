# The ORE SDK

The `ore` module is what code in a session (a notebook cell, a script run with `ore run`, a
function invoked by a pipeline) uses to read, write and create things in the tree. Python is the
primary SDK; Node and the JVM cover reading and writing with the same contract (see
[Node](#node) and [JVM](#jvm)).

The SDK never sees a bucket or a credential: it asks `ore-serve` what to read or where to write,
on behalf of the person who opened the session, and with their permissions.

**Names.** Everything in the tree is named `<database>.<schema>.<name>`. A name in the `default`
schema can also be written `<database>.<name>`: `sales.orders` and `sales.default.orders` are the
same thing.

## The session

A Python cell starts with these already in scope:

| name | what it is |
|---|---|
| `ore` | the module |
| `over`, `sql` | read ([Reading](#reading)) |
| `write`, `declare` | write a dataset, declare a document ([Writing](#writing)) |
| `transform` | the decorator ([Transforms](#transforms)) |
| `person` | `person()` → who opened the session (`persona:…`), the identity everything here runs as |

Everything else is `ore.<name>` (`ore.collection(…)`, `ore.create_view(…)`). `ore.session` is
the session object: `ore.session.id` is the session, `ore.session.person` who opened it.

If the last line of a cell is a DataFrame (pandas or polars), a Series or an Arrow Table, the
console shows it as a table. `ore.table(value, limit=200)` builds that same output yourself, and
`ore.to_json(v, oos_type=None)` turns a single value into the contract's JSON (decimals as
strings, instants in UTC with `Z`, bytes in base64, integers beyond 2⁵³ as strings).

### `ore.API`

`ore.API` is the version of the SDK's interface: `2` is the English names. Code that ORE
generates for a session (an SQL cell, a script) checks it first and fails with *"this session
runs an older ORE SDK … close the session and open a new one"* if the session's SDK is older.
You only need it if you write code that must run on sessions of different ages.

## Reading

```python
df  = over("sales.orders")                      # pandas, with Arrow types
tbl = over("sales.orders", format="arrow")      # pyarrow.Table
pl_ = over("sales.orders", format="polars")     # polars DataFrame

top = sql("""
    select customer, sum(amount) as total
    from sales.orders
    group by customer order by total desc limit 10
""")
```

- `over(view, format="pandas")` reads a dataset or a view. `format` is `"pandas"` (the default),
  `"arrow"` or `"polars"`.
- pandas DataFrames use `pd.ArrowDtype`: an integer column with nulls stays an integer, a
  `Decimal` stays exact, an instant keeps its zone. Columns the tree says are never null come out
  non-nullable in the Arrow schema.
- `sql(query, format="pandas")` runs DuckDB SQL over the tree. `ore-serve` says which tree names
  the query reads (a name inside a comment or a string does not count), and each one becomes a
  DuckDB view under its own name. It returns the same as `over()` for `format`, or `None` for a
  statement without a result.
- Errors: a name that does not exist is `LookupError`; one you may not read from a session is
  `PermissionError`; a copy that is not made yet is `RuntimeError`.

## Writing

### `write()`

```python
r = write("sales.daily_totals", df)                               # overwrite
write("sales.events", new_rows, mode="append")
write("sales.customers", changes, mode="upsert", key=["customer_id"])
r["rows"], r["snapshot"], r["repeated"]
```

`write(name, data, mode="overwrite", key=None, anchored_to=None)` writes `data` (a pandas or
polars DataFrame, a pandas Series, or an Arrow Table / RecordBatch) as the lake dataset `name`.

- `mode`:
  - `"overwrite"` (default): the dataset becomes `data`.
  - `"append"`: `data` is added.
  - `"upsert"`: needs `key=[…]`, the columns that identify a row. Rows of `data` replace the rows
    with the same key, the rest are added; the key is then declared on the dataset. `key` with any
    other mode is a `ValueError`.
- **Idempotent**: writing the same table to the same name with the same mode again leaves no new
  snapshot (`repeated` is `True`). If someone writes at the same time, it retries on top of what
  is there; if the catalog fails, it checks whether the commit went in before reporting an error.
- **Types** follow the type contract: nested structs and lists are fine; `uint64`, a column of
  type `null`, a struct without fields and a list of lists are refused, naming the column. An
  empty table is refused.
- **Provenance**: what you write records where it came from — inside a transform, its inputs and
  name; outside one, what the session has read so far — plus the code (`<path>@<commit>` when run
  with `ore run`) and the session.
- Returns `{table, rows, snapshot, metadata_location, operation, repeated, mode, added, before}`.
- `anchored_to="db.schema.collection"` writes an **anchored table** on that collection: it must
  carry the system columns (see [`apply()`](#incremental-derivation-apply)), and it merges by
  `_anchor_id`, so it cannot be combined with `upsert`. You normally let `Collection.apply()` do
  this.

### `declare()`

`declare(document)` adds an ontology document to the tree from code — a `View`, `Entity`,
`Interface`, `Concept`, `Table`… — as its YAML text or as a dict `{kind, metadata, spec}`. It is
compiled before it is pushed, committed as the person who opened the session, on the session's
branch. Returns `{kind, name, file, commit, created}`. A document the compiler rejects is a
`ValueError` with the diagnostics.

## Creating things

These are what an SQL script's `create …` statements run; you can call them from Python too.
Each one acts as the person who opened the session, on its branch. With `if_not_exists=True`,
something that already exists is not an error.

| call | returns |
|---|---|
| `create_database(name, kind="standard", origin=None, include=None, if_not_exists=False)` | `{database, kind, created}` |
| `create_schema(database, schema, if_not_exists=False)` | `{schema, created}` |
| `create_dataset(name, columns, key=None, if_not_exists=False)` | `{dataset, created}` |
| `create_view(name, sql, columns=None, comment=None, owner=None, or_replace=False, if_not_exists=False, schema_evolution=False, materialized=False)` | `{view, status, columns}` (+ `copy`) |
| `drop_view(name, if_exists=False)` | `{view, status}` |
| `create_collection(name, media, formats, owner=None, comment=None, labels=None, retention=None, if_not_exists=False)` | `{collection, created}` |

- `create_database`: without `origin`, an empty standard database; with `origin` (and
  optionally `include=[…]`, what to take from it), a database created from that source;
  `kind` is `"standard"` or `"foreign"`.
- `create_dataset`: `columns` is `[(name, iceberg_type), …]` (`"long"`, `"string"`,
  `"decimal(18, 2)"`, `"timestamptz"`…); `key` declares the primary key that upserts use.
- `create_view`: the view's column contract is described by DuckDB without reading a row.
  `columns=[(name, comment), …]` renames the select's columns by position. Replacing a view may
  add columns; removing one or changing its type breaks its readers and needs
  `schema_evolution=True`. `status` is `created`, `replaced` or `already exists`.
  `materialized=True` also creates its copy, the dataset `<view>_copia`, returned as `copy`.
- `drop_view`: if something reads the view it is not removed, and the error says why. `status` is
  `dropped` or `not found`.
- `create_collection`: an empty **written** media collection, which code fills with
  [transactions](#writing-into-a-collection). `media` is one of `document`, `image`, `audio`,
  `video`, `spreadsheet`, `email`; `formats` the extensions it accepts (the first is the primary
  one). `labels` can raise what is derived, not lower it. Without `owner` it belongs to whoever
  creates it.
- A rejected document (an OOS code) is a `ValueError`; anything else is a `RuntimeError`.

## Transforms

```python
@transform(inputs=["sales.orders", "sales.customers"], output="sales.daily_totals")
def daily_totals():
    orders = over("sales.orders")
    ...
    write("sales.daily_totals", totals)

daily_totals()
```

`@transform(inputs, output)` declares what a function reads and writes, and that is enforced
while it runs (by the SDK and by `ore-serve`):

- `over()`, `sql()` or a collection read of something not in `inputs` is a `PermissionError`;
  so is `write()` (or a collection transaction) to anything other than `output`. Reading `output`
  itself is allowed (an incremental transform reads what it already wrote).
- `inputs` and `output` may be collections: `inputs=[ore.collection("legal.archive.contracts")]`.
  While the transform runs, each input collection is read **as of the transaction it had when the
  transform started**, even if it changes meanwhile.
- What it writes records `{inputs, transform}` as its provenance: the lineage output ← code ←
  inputs.
- A transform cannot call another one. `output` cannot also be an input.

## Collections

```python
c = ore.collection("legal.archive.contracts")

for item in c.items(prefix="2026/"):            # lazy, by cursor, no bytes
    print(item.ref.path, item.ref.size, item.ref.content_type)

item = c.stat(path="2026/a.pdf")                 # one item, fresh
with item.open() as f:                           # a file pinned to its version
    head = f.read(5)                             # b"%PDF-"
    f.seek(-1024, 2)                             # the tail, with one range request
    tail = f.read()

data = item.read_bytes()                         # whole, verified
part = item.read_range(0, 4096)

for item, data, error in ore.read_many(c.items(), threads=16):
    ...
```

The code never talks to the store or the origin: the session says where the bytes are and the SDK
reads them, without ORE's token. Media collections, items and their errors are specified in
[`media.md`](media.md).

- `ore.collection(name)` → a `Collection`. Its `short_name` is the name a transform declares.
- `Collection.items(prefix=None, state=None, limit=1000)` → a lazy iterator of `Item`s, one
  consistent transaction of the collection (`Collection.as_of` says which). `prefix` filters by
  path, `state` by item state, `limit` is the page size.
- `Collection.stat(path=None, digest=None, version=None)` → one `Item`, fresh; `item.current`
  says whether that version is still the current one.
- `Item` has `ref` (a `MediaRef`) and `collection`, and reads its bytes:
  - `open()` → a read-only binary file (`read`, `seek`, `tell`), pinned to the item's version.
    Use it with `with`. Reading is streamed; only a `seek` starts a new request; closing halfway
    does not download the rest. A whole read is checked against `size` and `digest`.
  - `read_bytes(threads=8)` → all the bytes, verified; a large item is read by ranges in
    parallel.
  - `read_range(offset, length)` → `length` bytes from `offset`.
  - If the item had no digest, `item.sha256_seen` is the sha256 a whole read computed.
- `ore.read_many(items, threads=16)` reads many items at once and yields `(item, data, None)` or
  `(item, None, error)` as they finish; one item's error does not stop the others. `items` may be
  lazy (`c.items()`).
- `MediaRef` is an immutable dataclass with the grammar's fields: `uri`, `collection`, `path`,
  `version`, `digest`, `size`, `content_type`, `content_type_detected`, `checksum`,
  `annotations`, `modified`, `state`. `MediaRef.from_json(d)` builds one, ignoring unknown
  fields.
- Access expires after a few minutes; the SDK renews it **for the same version** and carries on.
  If that version can no longer be read, it raises `MediaChanged` — never bytes of another
  version.

### Writing into a collection

```python
ore.create_collection("legal.archive.pages", "image", ["png"])

with ore.collection("legal.archive.pages").transaction() as t:
    t.put("c1/p0.png", png_bytes)             # bytes
    t.put("c1/original.pdf", "/tmp/c1.pdf")   # a path, streamed
# on exit: commit; on an exception: abort
```

Only a **written** collection (one made with `create_collection`, without an origin) accepts
writes; otherwise `MediaNotWritable`.

- `Collection.transaction(ttl_s=3600)` → a `Transaction`. As a `with` block it commits on exit
  and aborts on an exception. Inside a transform, only on its `output`.
- `Transaction.put(path, data, content_type=None)` uploads one item to `path` (relative to the
  collection). `data` is bytes, a path or a file object. The served type is detected from the
  bytes; `content_type` counts only if the bytes say nothing. An interrupted upload is retried.
  Returns the item's `MediaRef`. Uploading what is already there is free (by digest).
- `Transaction.put_many(pairs, threads=8)` uploads many: `pairs` yields `(path, data)` or
  `(path, data, content_type)`, and it yields `(path, ref, error)` as each finishes. At most
  `2 × threads` are in flight.
- `Transaction.commit()` makes everything uploaded visible at once, with its provenance. If
  someone else committed at the same time, it commits again on top. Returns
  `{transaction, items, commit, metadata_location, changes, provenance, …}`.
- `Transaction.abort()` leaves nothing. A transaction has no item limit; it commits whole or not
  at all.
- `t.id`, `t.uploaded` (the `MediaRef`s), `t.closed` and `t.result` (what `commit()` returned).

### Incremental derivation: `apply()`

```python
def pages(item):
    with item.open() as f:
        for n, text in enumerate(extract(f)):
            yield {"text": text, "anchor": {"kind": "page", "page": n + 1}}

summary = ore.collection("legal.archive.contracts").apply(
    pages, output="legal.archive.contract_pages", version="2")
# {'items': 120, 'new': 3, 'recomputed': 0, 'skipped': 117, 'errors': 0,
#  'removed': 0, 'rows': 940, 'written': True}
```

`Collection.apply(fn, version=None, params=None, output=None, retry_errors=False, threads=4,
save_every_s=300)` runs `fn(item)` on each item that needs it and writes the result as an
**anchored table** in `output` (inside a transform, `output` defaults to the transform's).

- `fn(item)` returns or yields rows: dicts with your columns. A row that is a part of the item
  carries `anchor` (an `Anchor`: `{"kind": "page", "page": 3}`; fields `kind`, `page`, `bbox`,
  `polygon`, `space`, `t_start`, `t_end`, `frame`, `char_start`, `char_end`, `text_of`,
  `offset`, `length`) and optionally `anchor_parent`. With no rows, the item gets one row of
  `kind: item`. Column names starting with `_` are reserved.
- **Only what changed is computed.** Each item's key is its identity (its `digest`, or
  collection + path + version for an unread virtual item), the function's name, `version` (by
  default, a hash of `fn`'s source) and `params`. An unchanged key keeps its rows (moving an item
  does not recompute it); a changed key is recomputed; rows of items that are gone are removed.
- An exception in `fn` is a result: that item gets a row with `_status.state = "error"` and the
  others go on. `retry_errors=True` retries those items.
- `threads` items are computed at once. It saves every `save_every_s` seconds and at the end;
  with nothing to do, nothing is written.
- Returns `{items, new, recomputed, skipped, errors, removed, rows, written}`.

The anchored table has these system columns next to yours:

| column | content |
|---|---|
| `_item` | the item: `uri`, `collection`, `path`, `version`, `digest`, `size`, `content_type`, `content_type_detected`, `checksum` |
| `_anchor` | the `Anchor` of the row (`kind: item` for the whole item) |
| `_anchor_id` | a stable id of the row: item identity + anchor + function |
| `_anchor_parent` | the `_anchor_id` of its parent, if any |
| `_derivation` | `key`, `fn`, `fn_version`, `model`, `model_rev`, `params_hash`, `run`, `created` |
| `_status` | `state` (`ok`/`error`), `error_type`, `error_message`, `attempts` |

### Collections and functions in SQL (ORE 0049 B7)

**A collection is a relation in `FROM`**, one row per item, read by its listing (no bytes):
`item` (the `MediaRef`, as a struct of the `_item` fields), `path`, `digest`, `size`,
`content_type`, `modified`.

```sql
select path, size, content_type from legal.archive.contracts
```

**A tree function (`@function`, published in Functions) is called from SQL by its name**,
`functions.<def>(…)` (ORE 0056: functions have their own space, outside any database), with
its contract: as a value, `f(x)`, or as rows, `cross join lateral f(x)` (`from f(x)`, `join f(x)`).
Its parameters are positional, in the order of its `def`; one with a default can be left out.
Its types are its document's: a `@dataclass` is a struct, `list[D]` gives one row per element,
`Media[c]` takes an `item`. Only code functions (`runtime: python`) without `over` or `models`.
In a session, the function is read from **the session's branch**.

```sql
select c.path, functions.language(c.item) as lang from legal.archive.contracts as c
```

**A dataset written from a collection is an anchored table**, computed item by item exactly like
`apply()` (same registry, same keys, same system columns): there is nothing new to write.

```sql
-- transforms/contract_pages.sql
create or replace dataset legal.archive.contract_pages as
select p.page, p.text, p.anchor
from legal.archive.contracts as c
cross join lateral functions.pdf_pages(c.item) as p
```

- A column named `anchor` (an `Anchor` struct) is each row's anchor; without it, the item's.
- The version is the query and the documents of the functions it calls: changing either
  recomputes every item; running it again with nothing changed computes and writes nothing.
- The cell's result is one row: `items`, `new`, `recomputed`, `skipped`, `errors`, `removed`,
  `rows`.
- Limits, each one an error that says what to do instead: the dataset is written whole
  (`create or replace`, not `insert into`); it reads its collection and nothing else (join it
  afterwards, in a view); nothing that needs more than one item (`group by`, aggregates, windows,
  `order by`, `limit`, `distinct`, `union`: in a view that reads it); and it is a dataset of its
  own (not an existing one that is not anchored, or anchored to another collection).
- An item the `where` leaves out keeps one row of `kind: item` with no values, as in `apply()`:
  so it is not computed again.

### Serving media to a browser

A `Media<c>` property of an Entity holds the fingerprint of an item of collection `c`.

- `ore.media_columns(view)` → `{column: "db.schema.collection"}` for the `Media<c>` columns of a
  view.
- `ore.media_url(collection, fingerprint, ttl=None)` → `{url, content_type, disposition, seconds,
  expires_ms, path, …}`. The `url` opens without a credential for `ttl` seconds (300 by default,
  30 to 3600): do not store or share it.
- `ore.media_urls(collection, fingerprints, ttl=None)` → `{fingerprint: {url, …}}`, in batches of
  a hundred; missing ones are left out.

Code in ORE does not need URLs to read bytes: it has `item.open()`.

## Functions and models

```python
from decimal import Decimal
from datetime import date

@function
def net_amount(amount: Decimal, day: date) -> Decimal:
    ...

echo = ore.get_function("echo_types")
echo(amount=Decimal("12.50"), day=date(2026, 10, 2))
```

- `@function` or `@function(over=…, reads=[…], models=[…], timeout="…")` marks a `def` as a tree
  function. Its annotated signature **is the contract**: the `Function` document is derived from
  the file without running it, so the decorator's arguments must be literals. Called in a session
  it behaves as it will when invoked: each argument is converted to the annotated type (the string
  `"2026-10-02"` becomes a `date`, `12.5` a `Decimal`; `"3"` does not become an `int`), and the
  return value is checked. A value that does not fit raises `ore.contrato.ContractError` (a
  `TypeError`). OOS types Python lacks are in `ore.tipos`: `DateTimeTz`, `Money["EUR", 2]`,
  `Quantity["km", 1]`, `Annotated[Decimal, Precision(p, s)]`, `Media["db.schema.collection"]`.
- `ore.get_function(name)` (`<def>`: a function has its own space, outside any database —in SQL,
  `functions.<def>`—; the name of before, `<database>.<def>`, still finds it) loads a published
  code function and returns it as a callable with its contract; it runs in your process. A
  function with `over` or `models` is not called this way: a pipeline invokes it.
- `ore.model(ref)` → a `Model`, only inside a function that declares `ref` in `models` (by the
  name it uses there, or the full `db.schema.name`); elsewhere it is a `PermissionError`.
  - `Model.chat(messages, timeout=120, **options)` posts OpenAI-shaped `messages` to the model
    gateway; extra `options` go in the request body. Returns the whole response.
  - `Model.ask(text, **options)` sends one user message and returns the answer's text
    (`temperature` 0 unless given).

## Errors

| exception | when |
|---|---|
| `LookupError` | a name that does not exist (`over`, `sql`, `get_function`) |
| `PermissionError` | not allowed: a read outside a transform's inputs, a write outside its output, a model not declared |
| `ValueError` | a bad argument, or a document/statement the compiler rejects (with its OOS codes) |
| `RuntimeError` | `ore-serve` or the catalog failed |
| `ore.contrato.ContractError` | a function argument or return value does not fit its annotation |
| `ore.MediaError` | base of every media error; `.type` (the wire code, `media/…`), `.status`, `.detail` |
| `ore.MediaNotFound` | the collection or item does not exist (also a `LookupError`) |
| `ore.MediaForbidden` | not allowed, or the collection is not declared by the transform (also a `PermissionError`) |
| `ore.MediaChanged` | the pinned version can no longer be read whole |
| `ore.MediaCorrupt` | the bytes do not match `size` or `digest`, or the stream was cut (also an `IOError`) |
| `ore.MediaRangeError` | a range that cannot be served (also a `ValueError`) |
| `ore.MediaNotWritable` | writing into a collection that is not a written one |
| `ore.MediaTransactionError` | the transaction is not open (expired, closed, another collection's) |

## Node

`import ore from "ore"` (a cell already has `ore`, `over`, `sql`, `write`, `declare`, `transform`
and `person`). Everything is `async` except `person`, `table`, `toJson` and `arrowName`.

- `over(view, { limit, strict, as })` and `sql(text, { limit, strict, as })` return every row,
  as Python does (`ore.LIMIT` is `Infinity`), or at most `limit` if given, as objects with DuckDB's
  typed values (a 64-bit integer is a `bigint`, a decimal is exact). The array has `.types`,
  `.total` and `.truncated`. `strict: true` throws instead of truncating. `as: "columns"` returns
  `{ names, types, columns, total, truncated }`, one array per column, without an object per row.
- `write(name, data, { mode, key })`: `data` is rows, what `over`/`sql` returned, or
  `{ names, types, columns }`. Same modes and idempotency as Python. Returns
  `{ table, rows, snapshot, metadata_location, operation, repeated }`. No `anchored_to`.
- `transform({ inputs, output }, fn)` returns a function that runs `fn` under the same rules.
- `declare(document)` returns `{ kind, name, file, commit, created }`.
- `person()`, `session`, `table(value, limit = 200)`, `toJson(v)`, `arrowName(type)`,
  `mediaColumns(view)`, `mediaUrl(collection, fingerprint, { ttl })`,
  `mediaUrls(collection, fingerprints, { ttl })` (these two return the items as `ore-serve`
  serves them), `ore.API`.
- No collections, no `create_*`, no models, no calling a function (`get_function`): use Python for those.

### Functions (OOS v1alpha23)

A TypeScript function is the `export default function` of a `.ts` under a `functions/` folder,
named like its file, with an optional `export const config = { over, reads, models, timeout }`
(all literal). Its `Function` document is derived from the file without running it.

- **Types** (`import type { … } from "ore"`, `index.d.ts`): `Integer` (a `number` the contract
  requires exact; `bigint` for the full 64 bits), `Decimal` / `Decimal<p, s>`, `Money<"EUR", 2>`,
  `Quantity<"km", 1>` (decimals travel as **strings with their digits**, never `number`),
  `LocalDate`, `LocalTime`, `LocalDateTime` (ISO strings, no zone), `Media<"db.schema.collection">`,
  `Config`. A JS `Date` is an instant (`DateTimeTz`); `number` is `Float`; `Uint8Array` is `Opaque`.
  They are aliases: `const d: LocalDate = "2026-10-03"` needs no conversion.
- **The contract** (`contract.call(fn, signature, args)`): each argument converted to what the
  derived signature declares, the row first with `over`, and the output checked; a value that does
  not fit throws `ContractError` (`.side` is `input` or `output`, `.parameter` the culprit).
  Node erases types before running, so the contract reads the **derived signature**, not the code.
  `contract.toWire(v)` turns a result into JSON (`bigint` → string, `Date` → ISO).

## JVM

A Java cell has `import static ore.Ore.*;`.

- `over(view)`, `over(view, limit, strict)`, `sql(text)`, `sql(text, limit, strict)` → `Rows`, a
  `List<Map<String, Object>>` with `types`, `total` and `truncated`: every row, as Python does,
  or at most `limit` if given. Values follow the type contract: `Long`, exact `BigDecimal`, `LocalDate`, `LocalTime`,
  `LocalDateTime`, `Instant` (UTC), `byte[]`, `List`, `Map`.
- `arrow(view)` / `arrowSql(text)` → an `ArrowReader` over batches, without an object per row;
  close it when done.
- `write(name, data)`, `write(name, data, mode)`, `write(name, data, mode, key)`: `data` is
  `Rows`, a `List<Map>`, a `VectorSchemaRoot` or an `ArrowReader`. Returns a `Result` (a `Map`)
  `{table, rows, snapshot, metadata_location, operation, repeated}`.
- `transform(name, inputs, output, body)` runs a `Callable` under the transform rules.
- `declare(yaml)` or `declare(map)` → `{kind, name, file, commit, created}`.
- `person()`, `session`, `table(value, limit)`, `toJson(v)`, `API`.

## Migrating from the Spanish names

Until API 2 the SDK used Spanish names. **They still work today, silently**: each old name is
the same object as the new one, old keyword arguments are translated, and the dicts the SDK
returns also answer to their old keys (`r["filas"]` is `r["rows"]`). A later release will first
emit a `DeprecationWarning` for each old name and then remove them, so new code should use the
English names.

| before | now |
|---|---|
| `persona()`, `ore.puesto`, `ore.Puesto` | `person()`, `ore.session`, `ore.Session` |
| `over(vista, como=…)`, `sql(texto, como=…)` | `over(view, format=…)`, `sql(query, format=…)` |
| `write(nombre, datos, modo=, clave=, anclada_a=)` | `write(name, data, mode=, key=, anchored_to=)` |
| `modo="sobrescribir"` / `"anexar"` | `mode="overwrite"` / `"append"` |
| `declare(documento)` | `declare(document)` |
| `ore.crear_base`, `crear_schema`, `crear_dataset`, `crear_vista`, `crear_coleccion` | `create_database`, `create_schema`, `create_dataset`, `create_view`, `create_collection` |
| `ore.borrar_vista` | `drop_view` |
| `si_no_existe=`, `si_existe=`, `o_reemplaza=`, `evolucion=`, `materializada=` | `if_not_exists=`, `if_exists=`, `or_replace=`, `schema_evolution=`, `materialized=` |
| `nombre=`, `columnas=`, `clave=`, `comentario=`, `dueno=`, `etiquetas=`, `retencion=`, `formatos=`, `clase=`, `origen=`, `incluye=` | `name=`, `columns=`, `key=`, `comment=`, `owner=`, `labels=`, `retention=`, `formats=`, `kind=`, `origin=`, `include=` |
| `ore.coleccion`, `ore.Coleccion` | `ore.collection`, `ore.Collection` |
| `items(prefijo=, estado=, limite=)` | `items(prefix=, state=, limit=)` |
| `.aplicar(fn, salida=, reintentar_errores=, hilos=, guardar_cada_s=)` | `.apply(fn, output=, retry_errors=, threads=, save_every_s=)` |
| row keys `ancla`, `ancla_padre` | `anchor`, `anchor_parent` |
| `.transaccion()`, `ore.Transaccion` | `.transaction()`, `ore.Transaction` |
| `t.put(path, datos, tipo=)`, `t.put_varios(pares, hilos=)` | `t.put(path, data, content_type=)`, `t.put_many(pairs, threads=)` |
| `t.subidos`, `t.resultado`, `t.cerrada` | `t.uploaded`, `t.result`, `t.closed` |
| `ore.leer_varios(items, hilos=)`, `item.read_bytes(hilos=)` | `ore.read_many(items, threads=)`, `item.read_bytes(threads=)` |
| `item.sha256_visto`, `item.actual`, `item.coleccion`, `col.nombre_corto` | `item.sha256_seen`, `item.current`, `item.collection`, `col.short_name` |
| `MediaRef.de_json` | `MediaRef.from_json` |
| `MediaNoExiste`, `MediaSinPermiso`, `MediaCambiado`, `MediaCorrupto`, `MediaRango`, `MediaNoEscribible`, `MediaTransaccion` | `MediaNotFound`, `MediaForbidden`, `MediaChanged`, `MediaCorrupt`, `MediaRangeError`, `MediaNotWritable`, `MediaTransactionError` |
| `error.tipo`, `error.detalle` | `error.type`, `error.detail` |
| `ore.media_de`, `ore.media`, `ore.medias` (`coleccion=`, `huella(s)=`) | `ore.media_columns`, `ore.media_url`, `ore.media_urls` (`collection=`, `fingerprint(s)=`) |
| `ore.funcion(nombre)` | `ore.get_function(name)` |
| `ore.modelo(referencia)`, `ore.Modelo`, `.pide(texto)`, `.chat(mensajes, plazo=)` | `ore.model(ref)`, `ore.Model`, `.ask(text)`, `.chat(messages, timeout=)` |
| `ore.tabla(valor, limite=)`, `ore.json_de(v, tipo=)` | `ore.table(value, limit=)`, `ore.to_json(v, oos_type=)` |
| `ore.contrato.ErrorDeContrato` | `ore.contrato.ContractError` |

Returned keys: `tabla`→`table`, `filas`→`rows`, `operacion`→`operation`,
`repetida`→`repeated`, `modo`→`mode`, `anadidas`→`added`, `antes`→`before`, `nombre`→`name`,
`fichero`→`file`, `nueva`→`created`, `base`→`database`,
`clase`→`kind`, `creada`/`creado`→`created`, `coleccion`→`collection`, `vista`→`view`,
`estado`→`status`, `columnas`→`columns`, `copia`→`copy`, `nuevos`→`new`,
`recalculados`→`recomputed`, `saltados`→`skipped`, `errores`→`errors`, `borrados`→`removed`,
`escrito`→`written`, `transaccion`→`transaction`, `cambios`→`changes`,
`procedencia`→`provenance`, `huella`→`fingerprint`, `tipo`→`content_type`,
`disposicion`→`disposition`, `segundos`→`seconds`, `caduca_ms`→`expires_ms`, `camino`→`path`.

Node: `persona`→`person`, `puesto`→`session`, `tabla`→`table`, `jsonDe`→`toJson`,
`nombreArrow`→`arrowName`, `media`/`medias`/`mediaDe`→`mediaUrl`/`mediaUrls`/`mediaColumns`,
`LIMITE`→`LIMIT`, `EXTENSIONES`→`EXTENSIONS`; options `limite`/`estricto`/`como`
(`"filas"`/`"columnas"`)/`modo`/`clave` → `limit`/`strict`/`as` (`"rows"`/`"columns"`)/`mode`/`key`;
result keys `nombres`/`tipos`/`columnas`/`truncada` → `names`/`types`/`columns`/`truncated`.

JVM: `persona()`→`person()`, `puesto`→`session`, `Filas`→`Rows` (`tipos`/`truncada` →
`types`/`truncated`), `tabla()`→`table()`, `jsonDe()`→`toJson()`, `nombreArrow()`→`arrowName()`,
`valorDe()`→`valueAt()`, `tipoInferido()`→`inferredType()`, `llano()`→`plain()`,
`LIMITE`→`LIMIT`, `EXTENSIONES`→`EXTENSIONS`.

`ore.table()` still returns the console's wire format (`{columnas, filas, total, limite}`) in all
three SDKs: those keys are a protocol, not part of the naming change.
