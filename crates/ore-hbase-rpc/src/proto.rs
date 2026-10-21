// GENERADO por `pruebas-de-fuego/hbase/generar_proto.py` desde los .proto de `hbase-protocol-2.4.18.jar`.
// No se edita a mano: se vuelve a generar. Ficha 0072, §7.1.
#![allow(clippy::all, missing_docs, non_snake_case)]

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]
#[repr(i32)]
pub enum CellType {
    Minimum = 0,
    Put = 4,
    Delete = 8,
    DeleteFamilyVersion = 10,
    DeleteColumn = 12,
    DeleteFamily = 14,
    Maximum = 255,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]
#[repr(i32)]
pub enum Consistency {
    Strong = 0,
    Timeline = 1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]
#[repr(i32)]
pub enum ReadType {
    Default = 0,
    Stream = 1,
    Pread = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]
#[repr(i32)]
pub enum RegionSpecifierType {
    RegionName = 1,
    EncodedRegionName = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]
#[repr(i32)]
pub enum State {
    Enabled = 0,
    Disabled = 1,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ConnectionHeader {
    #[prost(message, optional, tag = "1")]
    pub user_info: Option<UserInformation>,
    #[prost(string, optional, tag = "2")]
    pub service_name: Option<String>,
    #[prost(string, optional, tag = "3")]
    pub cell_block_codec_class: Option<String>,
    #[prost(string, optional, tag = "4")]
    pub cell_block_compressor_class: Option<String>,
    #[prost(message, optional, tag = "5")]
    pub version_info: Option<VersionInfo>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct RequestHeader {
    #[prost(uint32, optional, tag = "1")]
    pub call_id: Option<u32>,
    #[prost(message, optional, tag = "2")]
    pub trace_info: Option<RPCTInfo>,
    #[prost(string, optional, tag = "3")]
    pub method_name: Option<String>,
    #[prost(bool, optional, tag = "4")]
    pub request_param: Option<bool>,
    #[prost(message, optional, tag = "5")]
    pub cell_block_meta: Option<CellBlockMeta>,
    #[prost(uint32, optional, tag = "6")]
    pub priority: Option<u32>,
    #[prost(uint32, optional, tag = "7")]
    pub timeout: Option<u32>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ResponseHeader {
    #[prost(uint32, optional, tag = "1")]
    pub call_id: Option<u32>,
    #[prost(message, optional, tag = "2")]
    pub exception: Option<ExceptionResponse>,
    #[prost(message, optional, tag = "3")]
    pub cell_block_meta: Option<CellBlockMeta>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ExceptionResponse {
    #[prost(string, optional, tag = "1")]
    pub exception_class_name: Option<String>,
    #[prost(string, optional, tag = "2")]
    pub stack_trace: Option<String>,
    #[prost(string, optional, tag = "3")]
    pub hostname: Option<String>,
    #[prost(int32, optional, tag = "4")]
    pub port: Option<i32>,
    #[prost(bool, optional, tag = "5")]
    pub do_not_retry: Option<bool>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct CellBlockMeta {
    #[prost(uint32, optional, tag = "1")]
    pub length: Option<u32>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct MetaRegionServer {
    #[prost(message, required, tag = "1")]
    pub server: ServerName,
    #[prost(uint32, optional, tag = "2")]
    pub rpc_version: Option<u32>,
    #[prost(enumeration = "State", optional, tag = "3")]
    pub state: Option<i32>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct RegionInfo {
    #[prost(uint64, required, tag = "1")]
    pub region_id: u64,
    #[prost(message, required, tag = "2")]
    pub table_name: TableName,
    #[prost(bytes, optional, tag = "3")]
    pub start_key: Option<Vec<u8>>,
    #[prost(bytes, optional, tag = "4")]
    pub end_key: Option<Vec<u8>>,
    #[prost(bool, optional, tag = "5")]
    pub offline: Option<bool>,
    #[prost(bool, optional, tag = "6")]
    pub split: Option<bool>,
    #[prost(int32, optional, tag = "7")]
    pub replica_id: Option<i32>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct GetRequest {
    #[prost(message, required, tag = "1")]
    pub region: RegionSpecifier,
    #[prost(message, required, tag = "2")]
    pub get: Get,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct GetResponse {
    #[prost(message, optional, tag = "1")]
    pub result: Option<ResultadoPb>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ScanRequest {
    #[prost(message, optional, tag = "1")]
    pub region: Option<RegionSpecifier>,
    #[prost(message, optional, tag = "2")]
    pub scan: Option<Scan>,
    #[prost(uint64, optional, tag = "3")]
    pub scanner_id: Option<u64>,
    #[prost(uint32, optional, tag = "4")]
    pub number_of_rows: Option<u32>,
    #[prost(bool, optional, tag = "5")]
    pub close_scanner: Option<bool>,
    #[prost(uint64, optional, tag = "6")]
    pub next_call_seq: Option<u64>,
    #[prost(bool, optional, tag = "7")]
    pub client_handles_partials: Option<bool>,
    #[prost(bool, optional, tag = "8")]
    pub client_handles_heartbeats: Option<bool>,
    #[prost(bool, optional, tag = "9")]
    pub track_scan_metrics: Option<bool>,
    #[prost(bool, optional, tag = "10")]
    pub renew: Option<bool>,
    #[prost(uint32, optional, tag = "11")]
    pub limit_of_rows: Option<u32>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ScanResponse {
    #[prost(uint32, repeated, packed = "false", tag = "1")]
    pub cells_per_result: Vec<u32>,
    #[prost(uint64, optional, tag = "2")]
    pub scanner_id: Option<u64>,
    #[prost(bool, optional, tag = "3")]
    pub more_results: Option<bool>,
    #[prost(uint32, optional, tag = "4")]
    pub ttl: Option<u32>,
    #[prost(message, repeated, tag = "5")]
    pub results: Vec<ResultadoPb>,
    #[prost(bool, optional, tag = "6")]
    pub stale: Option<bool>,
    #[prost(bool, repeated, packed = "false", tag = "7")]
    pub partial_flag_per_result: Vec<bool>,
    #[prost(bool, optional, tag = "8")]
    pub more_results_in_region: Option<bool>,
    #[prost(bool, optional, tag = "9")]
    pub heartbeat_message: Option<bool>,
    #[prost(message, optional, tag = "10")]
    pub scan_metrics: Option<ScanMetrics>,
    #[prost(uint64, optional, tag = "11")]
    pub mvcc_read_point: Option<u64>,
    #[prost(message, optional, tag = "12")]
    pub cursor: Option<Cursor>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct UserInformation {
    #[prost(string, required, tag = "1")]
    pub effective_user: String,
    #[prost(string, optional, tag = "2")]
    pub real_user: Option<String>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct VersionInfo {
    #[prost(string, required, tag = "1")]
    pub version: String,
    #[prost(string, required, tag = "2")]
    pub url: String,
    #[prost(string, required, tag = "3")]
    pub revision: String,
    #[prost(string, required, tag = "4")]
    pub user: String,
    #[prost(string, required, tag = "5")]
    pub date: String,
    #[prost(string, required, tag = "6")]
    pub src_checksum: String,
    #[prost(uint32, optional, tag = "7")]
    pub version_major: Option<u32>,
    #[prost(uint32, optional, tag = "8")]
    pub version_minor: Option<u32>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct RPCTInfo {
    #[prost(int64, optional, tag = "1")]
    pub trace_id: Option<i64>,
    #[prost(int64, optional, tag = "2")]
    pub parent_id: Option<i64>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ServerName {
    #[prost(string, required, tag = "1")]
    pub host_name: String,
    #[prost(uint32, optional, tag = "2")]
    pub port: Option<u32>,
    #[prost(uint64, optional, tag = "3")]
    pub start_code: Option<u64>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct TableName {
    #[prost(bytes, required, tag = "1")]
    pub namespace: Vec<u8>,
    #[prost(bytes, required, tag = "2")]
    pub qualifier: Vec<u8>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct RegionSpecifier {
    #[prost(enumeration = "RegionSpecifierType", required, tag = "1")]
    pub r#type: i32,
    #[prost(bytes, required, tag = "2")]
    pub value: Vec<u8>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Get {
    #[prost(bytes, required, tag = "1")]
    pub row: Vec<u8>,
    #[prost(message, repeated, tag = "2")]
    pub column: Vec<Column>,
    #[prost(message, repeated, tag = "3")]
    pub attribute: Vec<NameBytesPair>,
    #[prost(message, optional, tag = "4")]
    pub filter: Option<Filter>,
    #[prost(message, optional, tag = "5")]
    pub time_range: Option<TimeRange>,
    #[prost(uint32, optional, tag = "6")]
    pub max_versions: Option<u32>,
    #[prost(bool, optional, tag = "7")]
    pub cache_blocks: Option<bool>,
    #[prost(uint32, optional, tag = "8")]
    pub store_limit: Option<u32>,
    #[prost(uint32, optional, tag = "9")]
    pub store_offset: Option<u32>,
    #[prost(bool, optional, tag = "10")]
    pub existence_only: Option<bool>,
    #[prost(bool, optional, tag = "11")]
    pub closest_row_before: Option<bool>,
    #[prost(enumeration = "Consistency", optional, tag = "12")]
    pub consistency: Option<i32>,
    #[prost(message, repeated, tag = "13")]
    pub cf_time_range: Vec<ColumnFamilyTimeRange>,
    #[prost(bool, optional, tag = "14")]
    pub load_column_families_on_demand: Option<bool>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ResultadoPb {
    #[prost(message, repeated, tag = "1")]
    pub cell: Vec<Cell>,
    #[prost(int32, optional, tag = "2")]
    pub associated_cell_count: Option<i32>,
    #[prost(bool, optional, tag = "3")]
    pub exists: Option<bool>,
    #[prost(bool, optional, tag = "4")]
    pub stale: Option<bool>,
    #[prost(bool, optional, tag = "5")]
    pub partial: Option<bool>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Scan {
    #[prost(message, repeated, tag = "1")]
    pub column: Vec<Column>,
    #[prost(message, repeated, tag = "2")]
    pub attribute: Vec<NameBytesPair>,
    #[prost(bytes, optional, tag = "3")]
    pub start_row: Option<Vec<u8>>,
    #[prost(bytes, optional, tag = "4")]
    pub stop_row: Option<Vec<u8>>,
    #[prost(message, optional, tag = "5")]
    pub filter: Option<Filter>,
    #[prost(message, optional, tag = "6")]
    pub time_range: Option<TimeRange>,
    #[prost(uint32, optional, tag = "7")]
    pub max_versions: Option<u32>,
    #[prost(bool, optional, tag = "8")]
    pub cache_blocks: Option<bool>,
    #[prost(uint32, optional, tag = "9")]
    pub batch_size: Option<u32>,
    #[prost(uint64, optional, tag = "10")]
    pub max_result_size: Option<u64>,
    #[prost(uint32, optional, tag = "11")]
    pub store_limit: Option<u32>,
    #[prost(uint32, optional, tag = "12")]
    pub store_offset: Option<u32>,
    #[prost(bool, optional, tag = "13")]
    pub load_column_families_on_demand: Option<bool>,
    #[prost(bool, optional, tag = "14")]
    pub small: Option<bool>,
    #[prost(bool, optional, tag = "15")]
    pub reversed: Option<bool>,
    #[prost(enumeration = "Consistency", optional, tag = "16")]
    pub consistency: Option<i32>,
    #[prost(uint32, optional, tag = "17")]
    pub caching: Option<u32>,
    #[prost(bool, optional, tag = "18")]
    pub allow_partial_results: Option<bool>,
    #[prost(message, repeated, tag = "19")]
    pub cf_time_range: Vec<ColumnFamilyTimeRange>,
    #[prost(uint64, optional, tag = "20")]
    pub mvcc_read_point: Option<u64>,
    #[prost(bool, optional, tag = "21")]
    pub include_start_row: Option<bool>,
    #[prost(bool, optional, tag = "22")]
    pub include_stop_row: Option<bool>,
    #[prost(enumeration = "ReadType", optional, tag = "23")]
    pub readType: Option<i32>,
    #[prost(bool, optional, tag = "24")]
    pub need_cursor_result: Option<bool>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ScanMetrics {
    #[prost(message, repeated, tag = "1")]
    pub metrics: Vec<NameInt64Pair>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Cursor {
    #[prost(bytes, optional, tag = "1")]
    pub row: Option<Vec<u8>>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Column {
    #[prost(bytes, required, tag = "1")]
    pub family: Vec<u8>,
    #[prost(bytes, repeated, tag = "2")]
    pub qualifier: Vec<Vec<u8>>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct NameBytesPair {
    #[prost(string, required, tag = "1")]
    pub name: String,
    #[prost(bytes, optional, tag = "2")]
    pub value: Option<Vec<u8>>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Filter {
    #[prost(string, required, tag = "1")]
    pub name: String,
    #[prost(bytes, optional, tag = "2")]
    pub serialized_filter: Option<Vec<u8>>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct TimeRange {
    #[prost(uint64, optional, tag = "1")]
    pub from: Option<u64>,
    #[prost(uint64, optional, tag = "2")]
    pub to: Option<u64>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ColumnFamilyTimeRange {
    #[prost(bytes, required, tag = "1")]
    pub column_family: Vec<u8>,
    #[prost(message, required, tag = "2")]
    pub time_range: TimeRange,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Cell {
    #[prost(bytes, optional, tag = "1")]
    pub row: Option<Vec<u8>>,
    #[prost(bytes, optional, tag = "2")]
    pub family: Option<Vec<u8>>,
    #[prost(bytes, optional, tag = "3")]
    pub qualifier: Option<Vec<u8>>,
    #[prost(uint64, optional, tag = "4")]
    pub timestamp: Option<u64>,
    #[prost(enumeration = "CellType", optional, tag = "5")]
    pub cell_type: Option<i32>,
    #[prost(bytes, optional, tag = "6")]
    pub value: Option<Vec<u8>>,
    #[prost(bytes, optional, tag = "7")]
    pub tags: Option<Vec<u8>>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct NameInt64Pair {
    #[prost(string, optional, tag = "1")]
    pub name: Option<String>,
    #[prost(int64, optional, tag = "2")]
    pub value: Option<i64>,
}

