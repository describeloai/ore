"""Genera `crates/ore-avatica/src/proto.rs` a partir de los descriptores de Avatica.

    python generar_proto.py > crates/ore-avatica/src/proto.rs

Los mensajes de Avatica (calcite-avatica, common/requests/responses.proto) no se escriben a mano:
cada número de campo sale del descriptor que trae `phoenixdb` (el cliente de referencia de Python,
que los compila de los .proto de Avatica). Así un número mal copiado no existe. Solo se emiten los
mensajes que el cliente usa y los que estos nombran; `prost` con `derive` basta, sin `prost-build`
(que no está en el árbol de dependencias de ORE).

phoenixdb importa gssapi aunque no se use Kerberos; en Windows no hay KfW, y se le pone un doble.
"""
import sys
import types

sys.modules.setdefault("gssapi", types.ModuleType("gssapi"))
_m = types.ModuleType("requests_gssapi")
_m.HTTPSPNEGOAuth = object
sys.modules.setdefault("requests_gssapi", _m)
from google.protobuf import descriptor_pb2 as dp  # noqa: E402
from phoenixdb.avatica.proto import common_pb2, requests_pb2, responses_pb2  # noqa: E402

RAIZ = [
    "WireMessage",
    "OpenConnectionRequest", "OpenConnectionResponse",
    "CloseConnectionRequest", "CloseConnectionResponse",
    "CreateStatementRequest", "CreateStatementResponse",
    "CloseStatementRequest", "CloseStatementResponse",
    "PrepareAndExecuteRequest", "ExecuteResponse",
    "FetchRequest", "FetchResponse",
    "ConnectionSyncRequest", "ConnectionSyncResponse",
    "SchemasRequest", "TablesRequest", "ColumnsRequest",
    "ErrorResponse",
]

mensajes, enumerados = {}, {}
for mod in (common_pb2, requests_pb2, responses_pb2):
    f = dp.FileDescriptorProto.FromString(mod.DESCRIPTOR.serialized_pb)
    assert f.syntax in ("", "proto2", "proto3"), f.syntax
    for m in f.message_type:
        mensajes[m.name] = (m, f.syntax)
        for e in m.enum_type:  # enumerados anidados (p. ej. `Style`)
            enumerados[e.name] = e
    for e in f.enum_type:
        enumerados[e.name] = e

T = dp.FieldDescriptorProto
ESCALAR = {T.TYPE_DOUBLE: "double", T.TYPE_FLOAT: "float", T.TYPE_INT64: "int64", T.TYPE_UINT64: "uint64",
           T.TYPE_INT32: "int32", T.TYPE_BOOL: "bool", T.TYPE_STRING: "string", T.TYPE_BYTES: "bytes",
           T.TYPE_UINT32: "uint32", T.TYPE_SINT32: "sint32", T.TYPE_SINT64: "sint64"}
RUST = {"double": "f64", "float": "f32", "int64": "i64", "uint64": "u64", "int32": "i32", "bool": "bool",
        "string": "String", "bytes": "Vec<u8>", "uint32": "u32", "sint32": "i32", "sint64": "i64"}


def nombre_corto(tn):  # ".org.apache.calcite.avatica.proto.Common.Frame" -> "Frame"
    return tn.rsplit(".", 1)[-1]


def snake(n):
    return n


def camel(n):
    return "".join(p.capitalize() for p in n.split("_"))


usados, por_ver = [], list(RAIZ)
while por_ver:
    n = por_ver.pop(0)
    if n in usados or n not in mensajes:
        continue
    usados.append(n)
    for f in mensajes[n][0].field:
        if f.type in (T.TYPE_MESSAGE, T.TYPE_ENUM):
            por_ver.append(nombre_corto(f.type_name))
enums_usados = []
for n in usados:
    for f in mensajes[n][0].field:
        if f.type == T.TYPE_ENUM:
            e = nombre_corto(f.type_name)
            if e not in enums_usados:
                enums_usados.append(e)

# Campo cuyo nombre es palabra reservada de Rust
RESERVADAS = {"type": "r#type", "ref": "r#ref", "in": "r#in", "match": "r#match"}

out = []
w = out.append
w("// GENERADO por `pruebas-de-fuego/phoenix/generar_proto.py` desde los descriptores de Avatica (los")
w("// del cliente `phoenixdb`). No se edita a mano: se vuelve a generar. Ficha 0071, §4.")
w("#![allow(clippy::all, missing_docs, non_snake_case)]")
w("")
for en in enums_usados:
    e = enumerados[en]
    w("#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]")
    w("#[repr(i32)]")
    w(f"pub enum {en} {{")
    for v in e.value:
        w(f"    {camel(v.name.lower())} = {v.number},")
    w("}")
    w("")
for n in usados:
    m, sintaxis = mensajes[n]
    w("#[derive(Clone, PartialEq, prost::Message)]")
    w(f"pub struct {n} {{")
    for f in m.field:
        rep = f.label == T.LABEL_REPEATED
        base = f.type
        if base == T.TYPE_MESSAGE:
            tipo, atr = nombre_corto(f.type_name), "message"
        elif base == T.TYPE_ENUM:
            tipo, atr = "i32", "enumeration=\"%s\"" % nombre_corto(f.type_name)
        else:
            atr = ESCALAR[base]
            tipo = RUST[atr]
        nombre = RESERVADAS.get(f.name, f.name)
        entrada = next((x for x in m.nested_type if x.options.map_entry and x.name == nombre_corto(f.type_name)), None)             if base == T.TYPE_MESSAGE else None
        if entrada is not None:  # map<string, string> (solo `OpenConnectionRequest.info`)
            k, v = entrada.field
            assert ESCALAR[k.type] == "string" and ESCALAR[v.type] == "string"
            w(f'    #[prost(btree_map = "string, string", tag = "{f.number}")]')
            w(f"    pub {nombre}: std::collections::BTreeMap<String, String>,")
            continue
        if base == T.TYPE_MESSAGE:
            if rep:
                campo = f"Vec<{tipo}>"; cuerpo = f'#[prost({atr}, repeated, tag = "{f.number}")]'
            else:
                # un mensaje que se contiene a sí mismo (`AvaticaType.component`) necesita indirección
                campo = f"Option<Box<{tipo}>>" if tipo == n else f"Option<{tipo}>"; cuerpo = f'#[prost({atr}, optional, tag = "{f.number}")]'
        elif rep:
            campo = f"Vec<{tipo}>"
            cuerpo = f'#[prost({atr}, repeated, tag = "{f.number}")]'
        else:
            campo = tipo
            cuerpo = f'#[prost({atr}, tag = "{f.number}")]'
        w(f"    {cuerpo}")
        w(f"    pub {nombre}: {campo},")
    w("}")
    w("")
sys.stdout.reconfigure(encoding="utf-8", newline="\n")
sys.stdout.write("\n".join(out) + "\n")
