"""Genera `crates/ore-hbase-rpc/src/proto.rs` a partir de los `.proto` que trae el propio HBase.

    python generar_proto.py <hbase-protocol-2.4.18.jar> > crates/ore-hbase-rpc/src/proto.rs

Los mensajes del protocolo de HBase no se escriben a mano: se sacan del jar `hbase-protocol` (`*.proto` dentro
del jar, los mismos que compila HBase), se compilan con `protoc` (`grpcio-tools`) y se emite, con `prost` y
`derive` (sin `prost-build`, que no esta en el arbol de dependencias de ORE), solo lo que el cliente usa y lo
que eso nombra. Un numero de campo mal copiado no existe. Ficha 0072, §7.1.

Los `.proto` de HBase son proto2: hay campos `required`, `optional` con valor por defecto y enteros repetidos sin
empaquetar. Prost los lleva todos; los valores por defecto del `.proto` NO se emiten (el cliente pone a mano lo
que le importa, como `include_start_row`).
"""
import importlib
import os
import subprocess
import sys
import tempfile
import zipfile

jar = sys.argv[1]
tmp = tempfile.mkdtemp(prefix="hbase-proto-")
z = zipfile.ZipFile(jar)
protos = [n for n in z.namelist() if n.endswith(".proto") and "/" not in n]
for n in protos:
    open(os.path.join(tmp, n), "wb").write(z.read(n))
subprocess.run([sys.executable, "-m", "grpc_tools.protoc", f"-I{tmp}", f"--python_out={tmp}"]
               + [os.path.join(tmp, n) for n in protos], check=True)
sys.path.insert(0, tmp)

from google.protobuf import descriptor_pb2 as dp  # noqa: E402

# Lo que el cliente usa; el resto sale del cierre de lo que estos nombran.
RAIZ = [
    # RPC.proto
    "ConnectionHeader", "RequestHeader", "ResponseHeader", "ExceptionResponse", "CellBlockMeta",
    # ZooKeeper.proto
    "MetaRegionServer",
    # HBase.proto: el valor de info:regioninfo en hbase:meta
    "RegionInfo",
    # Table.proto: el estado de una tabla (info de hbase:meta, fila de la tabla, familia `table`)
    "TableState",
    # Client.proto
    "GetRequest", "GetResponse", "ScanRequest", "ScanResponse",
]
# `Result` taparia el `Result` de la biblioteca estandar dentro de `proto.rs`.
RENOMBRE = {"Result": "ResultadoPb"}

mensajes, enumerados = {}, {}  # los enumerados, por nombre completo (`.hbase.pb.TableState.State`)
for n in protos:
    mod = importlib.import_module(n[:-6] + "_pb2")
    f = dp.FileDescriptorProto.FromString(mod.DESCRIPTOR.serialized_pb)
    paquete = "." + f.package if f.package else ""
    for m in f.message_type:
        mensajes[m.name] = m
        for e in m.enum_type:
            enumerados[f"{paquete}.{m.name}.{e.name}"] = (e, m.name)
        for x in m.nested_type:
            mensajes.setdefault(x.name, x)
            for e in x.enum_type:
                enumerados[f"{paquete}.{m.name}.{x.name}.{e.name}"] = (e, x.name)
    for e in f.enum_type:
        enumerados[f"{paquete}.{e.name}"] = (e, "")

T = dp.FieldDescriptorProto
ESCALAR = {T.TYPE_DOUBLE: "double", T.TYPE_FLOAT: "float", T.TYPE_INT64: "int64", T.TYPE_UINT64: "uint64",
           T.TYPE_INT32: "int32", T.TYPE_BOOL: "bool", T.TYPE_STRING: "string", T.TYPE_BYTES: "bytes",
           T.TYPE_UINT32: "uint32", T.TYPE_SINT32: "sint32", T.TYPE_SINT64: "sint64",
           T.TYPE_FIXED32: "fixed32", T.TYPE_FIXED64: "fixed64", T.TYPE_SFIXED32: "sfixed32",
           T.TYPE_SFIXED64: "sfixed64"}
RUST = {"double": "f64", "float": "f32", "int64": "i64", "uint64": "u64", "int32": "i32", "bool": "bool",
        "string": "String", "bytes": "Vec<u8>", "uint32": "u32", "sint32": "i32", "sint64": "i64",
        "fixed32": "u32", "fixed64": "u64", "sfixed32": "i32", "sfixed64": "i64"}
RESERVADAS = {"type": "r#type", "ref": "r#ref", "in": "r#in", "match": "r#match"}


def corto(tn):  # ".hbase.pb.RegionSpecifier.RegionSpecifierType" -> "RegionSpecifierType"
    return tn.rsplit(".", 1)[-1]


def rust(n):
    return RENOMBRE.get(n, n)


def camel(n):
    return "".join(p.capitalize() for p in n.lower().split("_"))


usados, por_ver = [], list(RAIZ)
while por_ver:
    n = por_ver.pop(0)
    if n in usados or n not in mensajes:
        continue
    usados.append(n)
    for f in mensajes[n].field:
        if f.type in (T.TYPE_MESSAGE, T.TYPE_ENUM):
            por_ver.append(corto(f.type_name))
enums_usados = []  # nombres completos
for n in usados:
    for f in mensajes[n].field:
        if f.type == T.TYPE_ENUM and f.type_name not in enums_usados:
            enums_usados.append(f.type_name)
# Dos enumerados pueden llamarse igual (`State` en ZooKeeper.proto y en Table.proto): solo entonces se les
# antepone el mensaje que los contiene.
repetidos = {c for c in (corto(t) for t in enums_usados) if sum(corto(t) == c for t in enums_usados) > 1}


def enum_rust(tn):
    e, padre = enumerados[tn]
    return padre + e.name if e.name in repetidos else e.name

out = []
w = out.append
w("// GENERADO por `pruebas-de-fuego/hbase/generar_proto.py` desde los .proto de `hbase-protocol-2.4.18.jar`.")
w("// No se edita a mano: se vuelve a generar. Ficha 0072, §7.1.")
w("#![allow(clippy::all, missing_docs, non_snake_case)]")
w("")
for tn in sorted(enums_usados, key=enum_rust):
    e = enumerados[tn][0]
    w("#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]")
    w("#[repr(i32)]")
    w(f"pub enum {enum_rust(tn)} {{")
    for v in e.value:
        w(f"    {camel(v.name)} = {v.number},")
    w("}")
    w("")
for n in usados:
    m = mensajes[n]
    w("#[derive(Clone, PartialEq, prost::Message)]")
    w(f"pub struct {rust(n)} {{")
    for f in m.field:
        rep = f.label == T.LABEL_REPEATED
        req = f.label == T.LABEL_REQUIRED
        etiqueta = "repeated" if rep else "required" if req else "optional"
        if f.type == T.TYPE_MESSAGE:
            tipo, atr = rust(corto(f.type_name)), "message"
            if rep:
                campo = f"Vec<{tipo}>"
            elif req:
                campo = tipo
            else:
                campo = f"Option<Box<{tipo}>>" if tipo == rust(n) else f"Option<{tipo}>"
        elif f.type == T.TYPE_ENUM:
            atr = 'enumeration = "%s"' % enum_rust(f.type_name)
            campo = "Vec<i32>" if rep else "i32" if req else "Option<i32>"
        else:
            atr = ESCALAR[f.type]
            t = RUST[atr]
            campo = f"Vec<{t}>" if rep else t if req else f"Option<{t}>"
        extra = ', packed = "false"' if rep and f.type in ESCALAR and ESCALAR[f.type] not in ("string", "bytes") else ""
        w(f'    #[prost({atr}, {etiqueta}{extra}, tag = "{f.number}")]')
        w(f"    pub {RESERVADAS.get(f.name, f.name)}: {campo},")
    w("}")
    w("")
sys.stdout.reconfigure(encoding="utf-8", newline="\n")
sys.stdout.write("\n".join(out) + "\n")
