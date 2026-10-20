// GENERADO por `pruebas-de-fuego/phoenix/generar_proto.py` desde los descriptores de Avatica (los
// del cliente `phoenixdb`). No se edita a mano: se vuelve a generar. Ficha 0071, §4.
#![allow(clippy::all, missing_docs, non_snake_case)]

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]
#[repr(i32)]
pub enum Severity {
    UnknownSeverity = 0,
    FatalSeverity = 1,
    ErrorSeverity = 2,
    WarningSeverity = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]
#[repr(i32)]
pub enum StatementType {
    Select = 0,
    Insert = 1,
    Update = 2,
    Delete = 3,
    Upsert = 4,
    Merge = 5,
    OtherDml = 6,
    Create = 7,
    Drop = 8,
    Alter = 9,
    OtherDdl = 10,
    Call = 11,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]
#[repr(i32)]
pub enum Style {
    Object = 0,
    Record = 1,
    RecordProjection = 2,
    Array = 3,
    List = 4,
    Map = 5,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]
#[repr(i32)]
pub enum Rep {
    PrimitiveBoolean = 0,
    PrimitiveByte = 1,
    PrimitiveChar = 2,
    PrimitiveShort = 3,
    PrimitiveInt = 4,
    PrimitiveLong = 5,
    PrimitiveFloat = 6,
    PrimitiveDouble = 7,
    Boolean = 8,
    Byte = 9,
    Character = 10,
    Short = 11,
    Integer = 12,
    Long = 13,
    Float = 14,
    Double = 15,
    BigInteger = 25,
    BigDecimal = 26,
    JavaSqlTime = 16,
    JavaSqlTimestamp = 17,
    JavaSqlDate = 18,
    JavaUtilDate = 19,
    ByteString = 20,
    String = 21,
    Number = 22,
    Object = 23,
    Null = 24,
    Array = 27,
    Struct = 28,
    Multiset = 29,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct WireMessage {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(bytes, tag = "2")]
    pub wrapped_message: Vec<u8>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct OpenConnectionRequest {
    #[prost(string, tag = "1")]
    pub connection_id: String,
    #[prost(btree_map = "string, string", tag = "2")]
    pub info: std::collections::BTreeMap<String, String>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct OpenConnectionResponse {
    #[prost(message, optional, tag = "1")]
    pub metadata: Option<RpcMetadata>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct CloseConnectionRequest {
    #[prost(string, tag = "1")]
    pub connection_id: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct CloseConnectionResponse {
    #[prost(message, optional, tag = "1")]
    pub metadata: Option<RpcMetadata>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct CreateStatementRequest {
    #[prost(string, tag = "1")]
    pub connection_id: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct CreateStatementResponse {
    #[prost(string, tag = "1")]
    pub connection_id: String,
    #[prost(uint32, tag = "2")]
    pub statement_id: u32,
    #[prost(message, optional, tag = "3")]
    pub metadata: Option<RpcMetadata>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct CloseStatementRequest {
    #[prost(string, tag = "1")]
    pub connection_id: String,
    #[prost(uint32, tag = "2")]
    pub statement_id: u32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct CloseStatementResponse {
    #[prost(message, optional, tag = "1")]
    pub metadata: Option<RpcMetadata>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct PrepareAndExecuteRequest {
    #[prost(string, tag = "1")]
    pub connection_id: String,
    #[prost(string, tag = "2")]
    pub sql: String,
    #[prost(uint64, tag = "3")]
    pub max_row_count: u64,
    #[prost(uint32, tag = "4")]
    pub statement_id: u32,
    #[prost(int64, tag = "5")]
    pub max_rows_total: i64,
    #[prost(int32, tag = "6")]
    pub first_frame_max_size: i32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ExecuteResponse {
    #[prost(message, repeated, tag = "1")]
    pub results: Vec<ResultSetResponse>,
    #[prost(bool, tag = "2")]
    pub missing_statement: bool,
    #[prost(message, optional, tag = "3")]
    pub metadata: Option<RpcMetadata>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct FetchRequest {
    #[prost(string, tag = "1")]
    pub connection_id: String,
    #[prost(uint32, tag = "2")]
    pub statement_id: u32,
    #[prost(uint64, tag = "3")]
    pub offset: u64,
    #[prost(uint32, tag = "4")]
    pub fetch_max_row_count: u32,
    #[prost(int32, tag = "5")]
    pub frame_max_size: i32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct FetchResponse {
    #[prost(message, optional, tag = "1")]
    pub frame: Option<Frame>,
    #[prost(bool, tag = "2")]
    pub missing_statement: bool,
    #[prost(bool, tag = "3")]
    pub missing_results: bool,
    #[prost(message, optional, tag = "4")]
    pub metadata: Option<RpcMetadata>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ConnectionSyncRequest {
    #[prost(string, tag = "1")]
    pub connection_id: String,
    #[prost(message, optional, tag = "2")]
    pub conn_props: Option<ConnectionProperties>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ConnectionSyncResponse {
    #[prost(message, optional, tag = "1")]
    pub conn_props: Option<ConnectionProperties>,
    #[prost(message, optional, tag = "2")]
    pub metadata: Option<RpcMetadata>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct SchemasRequest {
    #[prost(string, tag = "1")]
    pub catalog: String,
    #[prost(string, tag = "2")]
    pub schema_pattern: String,
    #[prost(string, tag = "3")]
    pub connection_id: String,
    #[prost(bool, tag = "4")]
    pub has_catalog: bool,
    #[prost(bool, tag = "5")]
    pub has_schema_pattern: bool,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct TablesRequest {
    #[prost(string, tag = "1")]
    pub catalog: String,
    #[prost(string, tag = "2")]
    pub schema_pattern: String,
    #[prost(string, tag = "3")]
    pub table_name_pattern: String,
    #[prost(string, repeated, tag = "4")]
    pub type_list: Vec<String>,
    #[prost(bool, tag = "6")]
    pub has_type_list: bool,
    #[prost(string, tag = "7")]
    pub connection_id: String,
    #[prost(bool, tag = "8")]
    pub has_catalog: bool,
    #[prost(bool, tag = "9")]
    pub has_schema_pattern: bool,
    #[prost(bool, tag = "10")]
    pub has_table_name_pattern: bool,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ColumnsRequest {
    #[prost(string, tag = "1")]
    pub catalog: String,
    #[prost(string, tag = "2")]
    pub schema_pattern: String,
    #[prost(string, tag = "3")]
    pub table_name_pattern: String,
    #[prost(string, tag = "4")]
    pub column_name_pattern: String,
    #[prost(string, tag = "5")]
    pub connection_id: String,
    #[prost(bool, tag = "6")]
    pub has_catalog: bool,
    #[prost(bool, tag = "7")]
    pub has_schema_pattern: bool,
    #[prost(bool, tag = "8")]
    pub has_table_name_pattern: bool,
    #[prost(bool, tag = "9")]
    pub has_column_name_pattern: bool,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ErrorResponse {
    #[prost(string, repeated, tag = "1")]
    pub exceptions: Vec<String>,
    #[prost(bool, tag = "7")]
    pub has_exceptions: bool,
    #[prost(string, tag = "2")]
    pub error_message: String,
    #[prost(enumeration="Severity", tag = "3")]
    pub severity: i32,
    #[prost(uint32, tag = "4")]
    pub error_code: u32,
    #[prost(string, tag = "5")]
    pub sql_state: String,
    #[prost(message, optional, tag = "6")]
    pub metadata: Option<RpcMetadata>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct RpcMetadata {
    #[prost(string, tag = "1")]
    pub server_address: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ResultSetResponse {
    #[prost(string, tag = "1")]
    pub connection_id: String,
    #[prost(uint32, tag = "2")]
    pub statement_id: u32,
    #[prost(bool, tag = "3")]
    pub own_statement: bool,
    #[prost(message, optional, tag = "4")]
    pub signature: Option<Signature>,
    #[prost(message, optional, tag = "5")]
    pub first_frame: Option<Frame>,
    #[prost(uint64, tag = "6")]
    pub update_count: u64,
    #[prost(message, optional, tag = "7")]
    pub metadata: Option<RpcMetadata>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Frame {
    #[prost(uint64, tag = "1")]
    pub offset: u64,
    #[prost(bool, tag = "2")]
    pub done: bool,
    #[prost(message, repeated, tag = "3")]
    pub rows: Vec<Row>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ConnectionProperties {
    #[prost(bool, tag = "1")]
    pub is_dirty: bool,
    #[prost(bool, tag = "2")]
    pub auto_commit: bool,
    #[prost(bool, tag = "7")]
    pub has_auto_commit: bool,
    #[prost(bool, tag = "3")]
    pub read_only: bool,
    #[prost(bool, tag = "8")]
    pub has_read_only: bool,
    #[prost(uint32, tag = "4")]
    pub transaction_isolation: u32,
    #[prost(string, tag = "5")]
    pub catalog: String,
    #[prost(string, tag = "6")]
    pub schema: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Signature {
    #[prost(message, repeated, tag = "1")]
    pub columns: Vec<ColumnMetaData>,
    #[prost(string, tag = "2")]
    pub sql: String,
    #[prost(message, repeated, tag = "3")]
    pub parameters: Vec<AvaticaParameter>,
    #[prost(message, optional, tag = "4")]
    pub cursor_factory: Option<CursorFactory>,
    #[prost(enumeration="StatementType", tag = "5")]
    pub statementType: i32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Row {
    #[prost(message, repeated, tag = "1")]
    pub value: Vec<ColumnValue>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ColumnMetaData {
    #[prost(uint32, tag = "1")]
    pub ordinal: u32,
    #[prost(bool, tag = "2")]
    pub auto_increment: bool,
    #[prost(bool, tag = "3")]
    pub case_sensitive: bool,
    #[prost(bool, tag = "4")]
    pub searchable: bool,
    #[prost(bool, tag = "5")]
    pub currency: bool,
    #[prost(uint32, tag = "6")]
    pub nullable: u32,
    #[prost(bool, tag = "7")]
    pub signed: bool,
    #[prost(uint32, tag = "8")]
    pub display_size: u32,
    #[prost(string, tag = "9")]
    pub label: String,
    #[prost(string, tag = "10")]
    pub column_name: String,
    #[prost(string, tag = "11")]
    pub schema_name: String,
    #[prost(uint32, tag = "12")]
    pub precision: u32,
    #[prost(uint32, tag = "13")]
    pub scale: u32,
    #[prost(string, tag = "14")]
    pub table_name: String,
    #[prost(string, tag = "15")]
    pub catalog_name: String,
    #[prost(bool, tag = "16")]
    pub read_only: bool,
    #[prost(bool, tag = "17")]
    pub writable: bool,
    #[prost(bool, tag = "18")]
    pub definitely_writable: bool,
    #[prost(string, tag = "19")]
    pub column_class_name: String,
    #[prost(message, optional, tag = "20")]
    pub r#type: Option<AvaticaType>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct AvaticaParameter {
    #[prost(bool, tag = "1")]
    pub signed: bool,
    #[prost(uint32, tag = "2")]
    pub precision: u32,
    #[prost(uint32, tag = "3")]
    pub scale: u32,
    #[prost(uint32, tag = "4")]
    pub parameter_type: u32,
    #[prost(string, tag = "5")]
    pub type_name: String,
    #[prost(string, tag = "6")]
    pub class_name: String,
    #[prost(string, tag = "7")]
    pub name: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct CursorFactory {
    #[prost(enumeration="Style", tag = "1")]
    pub style: i32,
    #[prost(string, tag = "2")]
    pub class_name: String,
    #[prost(string, repeated, tag = "3")]
    pub field_names: Vec<String>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ColumnValue {
    #[prost(message, repeated, tag = "1")]
    pub value: Vec<TypedValue>,
    #[prost(message, repeated, tag = "2")]
    pub array_value: Vec<TypedValue>,
    #[prost(bool, tag = "3")]
    pub has_array_value: bool,
    #[prost(message, optional, tag = "4")]
    pub scalar_value: Option<TypedValue>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct AvaticaType {
    #[prost(uint32, tag = "1")]
    pub id: u32,
    #[prost(string, tag = "2")]
    pub name: String,
    #[prost(enumeration="Rep", tag = "3")]
    pub rep: i32,
    #[prost(message, repeated, tag = "4")]
    pub columns: Vec<ColumnMetaData>,
    #[prost(message, optional, tag = "5")]
    pub component: Option<Box<AvaticaType>>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct TypedValue {
    #[prost(enumeration="Rep", tag = "1")]
    pub r#type: i32,
    #[prost(bool, tag = "2")]
    pub bool_value: bool,
    #[prost(string, tag = "3")]
    pub string_value: String,
    #[prost(sint64, tag = "4")]
    pub number_value: i64,
    #[prost(bytes, tag = "5")]
    pub bytes_value: Vec<u8>,
    #[prost(double, tag = "6")]
    pub double_value: f64,
    #[prost(bool, tag = "7")]
    pub null: bool,
    #[prost(message, repeated, tag = "8")]
    pub array_value: Vec<TypedValue>,
    #[prost(enumeration="Rep", tag = "9")]
    pub component_type: i32,
    #[prost(bool, tag = "10")]
    pub implicitly_null: bool,
}

