//! **Los codecs del plan** (0081 §7.5, C8): cómo viajan por Ballista los nodos que DataFusion no sabe serializar.
//! El cliente manda el plan lógico al scheduler (codec lógico) y el scheduler, cada etapa del físico a los ejecutores
//! (codec físico). Lo de DataFusion lo serializa `datafusion-proto`; lo de Ballista (el barajado, los formatos), sus
//! codecs; lo de los frentes, el codec que da el zócalo de [`Extensiones`](crate::zocalos::Extensiones).
//!
//! Ballista admite **un** codec por papel, y el suyo lleva sus propios nodos: cambiarlo por el de un frente los
//! perdería. Así que el suelo los envuelve: lo que el codec del frente codifica sale con [`PREFIJO`] delante, y lo
//! demás pasa al de Ballista. Al decodificar, el prefijo dice a cuál le toca. El codec del frente sólo conoce lo
//! suyo y falla con lo demás.

use ballista_core::serde::{BallistaLogicalExtensionCodec, BallistaPhysicalExtensionCodec};
use datafusion::arrow::datatypes::SchemaRef;
use datafusion::catalog::TableProvider;
use datafusion::common::{Result, TableReference};
use datafusion::execution::TaskContext;
use datafusion::logical_expr::{AggregateUDF, Extension, LogicalPlan, ScalarUDF, WindowUDF};
use datafusion::physical_plan::ExecutionPlan;
use datafusion_proto::logical_plan::LogicalExtensionCodec;
use datafusion_proto::physical_plan::{PhysicalExtensionCodec, PhysicalProtoConverterExtension};
use std::sync::Arc;

/// Lo que marca un nodo de ORE en el cable. Un mensaje de protobuf no empieza así: `O` sería el campo 9 con el tipo
/// de cable 7, que no existe.
pub const PREFIJO: &[u8] = b"ORE\x01";

/// **Los codecs de un frente**: los nodos que pone en el plan, y nada más.
#[derive(Debug, Clone)]
pub struct Codecs {
    pub logico: Arc<dyn LogicalExtensionCodec>,
    pub fisico: Arc<dyn PhysicalExtensionCodec>,
}

/// **Los codecs de un papel**: los del frente, si los hay, envueltos con los de Ballista; si no, los de Ballista.
pub fn del_papel(
    frente: Option<Codecs>,
) -> (
    Arc<dyn LogicalExtensionCodec>,
    Arc<dyn PhysicalExtensionCodec>,
) {
    let logico = Arc::new(BallistaLogicalExtensionCodec::default());
    let fisico = Arc::new(BallistaPhysicalExtensionCodec::default());
    match frente {
        Some(c) => (
            Arc::new(Envuelto {
                ore: c.logico,
                ballista: logico,
            }),
            Arc::new(Envuelto {
                ore: c.fisico,
                ballista: fisico,
            }),
        ),
        None => (logico, fisico),
    }
}

/// El codec de un frente con el de Ballista detrás.
#[derive(Debug)]
struct Envuelto<C: ?Sized> {
    ore: Arc<C>,
    ballista: Arc<C>,
}

/// Codifica con el de ORE, con su prefijo; si no es suyo, con el de Ballista.
fn codificar(
    buf: &mut Vec<u8>,
    ore: impl FnOnce(&mut Vec<u8>) -> Result<()>,
    ballista: impl FnOnce(&mut Vec<u8>) -> Result<()>,
) -> Result<()> {
    let inicio = buf.len();
    buf.extend_from_slice(PREFIJO);
    if ore(buf).is_ok() {
        return Ok(());
    }
    buf.truncate(inicio);
    ballista(buf)
}

impl LogicalExtensionCodec for Envuelto<dyn LogicalExtensionCodec> {
    fn try_decode(
        &self,
        buf: &[u8],
        inputs: &[LogicalPlan],
        ctx: &TaskContext,
    ) -> Result<Extension> {
        match buf.strip_prefix(PREFIJO) {
            Some(b) => self.ore.try_decode(b, inputs, ctx),
            None => self.ballista.try_decode(buf, inputs, ctx),
        }
    }

    fn try_encode(&self, node: &Extension, buf: &mut Vec<u8>) -> Result<()> {
        codificar(
            buf,
            |b| self.ore.try_encode(node, b),
            |b| self.ballista.try_encode(node, b),
        )
    }

    fn try_decode_table_provider(
        &self,
        buf: &[u8],
        table_ref: &TableReference,
        schema: SchemaRef,
        ctx: &TaskContext,
    ) -> Result<Arc<dyn TableProvider>> {
        match buf.strip_prefix(PREFIJO) {
            Some(b) => self
                .ore
                .try_decode_table_provider(b, table_ref, schema, ctx),
            None => self
                .ballista
                .try_decode_table_provider(buf, table_ref, schema, ctx),
        }
    }

    fn try_encode_table_provider(
        &self,
        table_ref: &TableReference,
        node: Arc<dyn TableProvider>,
        buf: &mut Vec<u8>,
    ) -> Result<()> {
        codificar(
            buf,
            |b| {
                self.ore
                    .try_encode_table_provider(table_ref, node.clone(), b)
            },
            |b| {
                self.ballista
                    .try_encode_table_provider(table_ref, node.clone(), b)
            },
        )
    }

    // Las funciones viajan por su nombre y se buscan en el registro de cada papel: las del frente están en la
    // plantilla (S3). Lo demás, como lo haga Ballista.
    fn try_decode_file_format(
        &self,
        buf: &[u8],
        ctx: &TaskContext,
    ) -> Result<Arc<dyn datafusion::datasource::file_format::FileFormatFactory>> {
        self.ballista.try_decode_file_format(buf, ctx)
    }

    fn try_encode_file_format(
        &self,
        buf: &mut Vec<u8>,
        node: Arc<dyn datafusion::datasource::file_format::FileFormatFactory>,
    ) -> Result<()> {
        self.ballista.try_encode_file_format(buf, node)
    }

    fn try_decode_udf(&self, name: &str, buf: &[u8]) -> Result<Arc<ScalarUDF>> {
        self.ballista.try_decode_udf(name, buf)
    }

    fn try_encode_udf(&self, node: &ScalarUDF, buf: &mut Vec<u8>) -> Result<()> {
        self.ballista.try_encode_udf(node, buf)
    }

    fn try_decode_udaf(&self, name: &str, buf: &[u8]) -> Result<Arc<AggregateUDF>> {
        self.ballista.try_decode_udaf(name, buf)
    }

    fn try_encode_udaf(&self, node: &AggregateUDF, buf: &mut Vec<u8>) -> Result<()> {
        self.ballista.try_encode_udaf(node, buf)
    }

    fn try_decode_udwf(&self, name: &str, buf: &[u8]) -> Result<Arc<WindowUDF>> {
        self.ballista.try_decode_udwf(name, buf)
    }

    fn try_encode_udwf(&self, node: &WindowUDF, buf: &mut Vec<u8>) -> Result<()> {
        self.ballista.try_encode_udwf(node, buf)
    }
}

impl PhysicalExtensionCodec for Envuelto<dyn PhysicalExtensionCodec> {
    fn try_decode(
        &self,
        buf: &[u8],
        inputs: &[Arc<dyn ExecutionPlan>],
        ctx: &TaskContext,
        proto_converter: &dyn PhysicalProtoConverterExtension,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        match buf.strip_prefix(PREFIJO) {
            Some(b) => self.ore.try_decode(b, inputs, ctx, proto_converter),
            None => self.ballista.try_decode(buf, inputs, ctx, proto_converter),
        }
    }

    fn try_encode(
        &self,
        node: Arc<dyn ExecutionPlan>,
        buf: &mut Vec<u8>,
        proto_converter: &dyn PhysicalProtoConverterExtension,
    ) -> Result<()> {
        codificar(
            buf,
            |b| self.ore.try_encode(node.clone(), b, proto_converter),
            |b| self.ballista.try_encode(node.clone(), b, proto_converter),
        )
    }

    fn try_decode_udf(&self, name: &str, buf: &[u8]) -> Result<Arc<ScalarUDF>> {
        self.ballista.try_decode_udf(name, buf)
    }

    fn try_encode_udf(&self, node: &ScalarUDF, buf: &mut Vec<u8>) -> Result<()> {
        self.ballista.try_encode_udf(node, buf)
    }

    fn try_decode_udaf(&self, name: &str, buf: &[u8]) -> Result<Arc<AggregateUDF>> {
        self.ballista.try_decode_udaf(name, buf)
    }

    fn try_encode_udaf(&self, node: &AggregateUDF, buf: &mut Vec<u8>) -> Result<()> {
        self.ballista.try_encode_udaf(node, buf)
    }

    fn try_decode_udwf(&self, name: &str, buf: &[u8]) -> Result<Arc<WindowUDF>> {
        self.ballista.try_decode_udwf(name, buf)
    }

    fn try_encode_udwf(&self, node: &WindowUDF, buf: &mut Vec<u8>) -> Result<()> {
        self.ballista.try_encode_udwf(node, buf)
    }
}
