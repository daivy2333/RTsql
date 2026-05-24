<pir>
<meta>
my_project|.|PY,Rust
</meta>
<units>
u0|benches/micro_bench.rs|Rust|benches
u1|benches/cache_bench.rs|Rust|benches
u2|benches/wal_group_commit_bench.rs|Rust|benches
u3|benches/sqlite_compare.rs|Rust|benches
u4|benches/scale_bench.rs|Rust|benches
u5|benches/concurrent_bench.rs|Rust|benches
u6|benches/common/mod.rs|Rust|common
u7|scripts/comparison_bench.py|PY|scripts
u8|src/profiling.rs|Rust|src
u9|src/pipeline.rs|Rust|src
u10|src/lib.rs|Rust|src
u11|src/database.rs|Rust|src
u12|src/main.rs|Rust|entry|src
u13|src/plan_cache.rs|Rust|src
u14|src/wal/record.rs|Rust|wal
u15|src/wal/reader.rs|Rust|wal
u16|src/wal/buffer.rs|Rust|wal
u17|src/wal/checkpoint.rs|Rust|wal
u18|src/wal/mod.rs|Rust|wal
u19|src/wal/writer.rs|Rust|wal
u20|src/wal/recovery.rs|Rust|wal
u21|src/executor/drop_table.rs|Rust|executor
u22|src/executor/subquery_eval.rs|Rust|executor
u23|src/executor/sort.rs|Rust|executor
u24|src/executor/predicate.rs|Rust|executor
u25|src/executor/create_table.rs|Rust|executor
u26|src/executor/delete.rs|Rust|executor
u27|src/executor/semi_join.rs|Rust|executor
u28|src/executor/update.rs|Rust|executor
u29|src/executor/correlated.rs|Rust|executor
u30|src/executor/insert.rs|Rust|executor
u31|src/executor/anti_join.rs|Rust|executor
u32|src/executor/filter.rs|Rust|executor
u33|src/executor/limit.rs|Rust|executor
u34|src/executor/join_config.rs|Rust|executor
u35|src/executor/scan.rs|Rust|executor
u36|src/executor/value.rs|Rust|executor
u37|src/executor/having.rs|Rust|executor
u38|src/executor/index_scan.rs|Rust|executor
u39|src/executor/join_related_config.rs|Rust|executor
u40|src/executor/result.rs|Rust|executor
u41|src/executor/index_scan_all.rs|Rust|executor
u42|src/executor/mod.rs|Rust|executor
u43|src/executor/join.rs|Rust|executor
u44|src/executor/plan.rs|Rust|executor
u45|src/executor/executor_trait.rs|Rust|executor
u46|src/executor/aggregate.rs|Rust|executor
u47|src/executor/derived_scan.rs|Rust|executor
u48|src/transaction/tx_id.rs|Rust|transaction
u49|src/transaction/version_chain.rs|Rust|transaction
u50|src/transaction/snapshot.rs|Rust|transaction
u51|src/transaction/mod.rs|Rust|transaction
u52|src/transaction/error.rs|Rust|transaction
u53|src/transaction/manager.rs|Rust|transaction
u54|src/transaction/row_lock.rs|Rust|transaction
u55|src/storage/async_storage.rs|Rust|storage
u56|src/storage/page.rs|Rust|storage
u57|src/storage/file_storage.rs|Rust|storage
u58|src/storage/data_page.rs|Rust|storage
u59|src/storage/page_id.rs|Rust|storage
u60|src/storage/buffer_pool.rs|Rust|storage
u61|src/storage/mod.rs|Rust|storage
u62|src/storage/error.rs|Rust|storage
u63|src/storage/page_frame.rs|Rust|storage
u64|src/storage/data/table_manager.rs|Rust|data
u65|src/storage/data/mod.rs|Rust|data
u66|src/storage/btree/btree.rs|Rust|btree
u67|src/storage/btree/node.rs|Rust|btree
u68|src/storage/btree/index_manager.rs|Rust|btree
u69|src/storage/btree/async_loader.rs|Rust|btree
u70|src/storage/btree/mod.rs|Rust|btree
u71|src/storage/btree/sync_loader.rs|Rust|btree
u72|src/storage/page_format/key.rs|Rust|page_format
u73|src/storage/page_format/row_id.rs|Rust|page_format
u74|src/storage/page_format/mod.rs|Rust|page_format
u75|src/storage/page_format/slotted_page.rs|Rust|page_format
u76|src/storage/page_format/tuple.rs|Rust|page_format
u77|src/network/connection.rs|Rust|network
u78|src/network/handler.rs|Rust|network
u79|src/network/pg_messages.rs|Rust|network
u80|src/network/pg_protocol.rs|Rust|network
u81|src/network/server.rs|Rust|network
u82|src/network/protocol.rs|Rust|network
u83|src/network/mod.rs|Rust|network
u84|src/network/error.rs|Rust|network
u85|src/parser/planner.rs|Rust|parser
u86|src/parser/value.rs|Rust|parser
u87|src/parser/mod.rs|Rust|parser
u88|src/parser/ast.rs|Rust|parser
u89|src/parser/error.rs|Rust|parser
</units>
<pool>
d0|import|[psutil]
d1|import|[shutil]
d2|import|[stdlib:py]
d3|import|[subprocess]
d4|import|[tempfile]
d5|use|[async_trait::async_trait]
d6|use|[common::*]
d7|use|[crate::Value]
d8|use|[crate::database::Database]
d9|use|[crate::executor::ExecResult]
d10|use|[crate::executor::PhysicalPlan]
d11|use|[crate::executor::Value]
d12|use|[crate::executor::aggregate::AggregateFunc]
d13|use|[crate::executor::executor_trait::Executor]
d14|use|[crate::executor::predicate::PredicateRef]
d15|use|[crate::executor::result::ExecResult]
d16|use|[crate::executor::value::Value]
d17|use|[crate::executor::{
        ColumnExpression, ComparisonOp, ComparisonPredicate, FilterNode, ParameterExpression,
        PredicateRef, ScanNode,
    }]
d18|use|[crate::executor::{
    AggregateExecutor, AggregateNode, AntiJoinExecutor, CreateTableExecutor, DeleteExecutor,
    DerivedScanExecutor, DropTableExecutor, ExecResult, Executor, FilterExecutor, HavingExecutor,
    IndexScanExecutor, IndexScanAllExecutor, InsertExecutor, JoinConfig, JoinExecutor, JoinRelatedConfig, LimitExecutor, PhysicalPlan, ScanExecutor,
    SemiJoinExecutorV2, SortExecutor, SubqueryEvalExecutor, UpdateExecutor, Value,
}]
d19|use|[crate::executor::{
    AggregateFunc, AggregateNode, AntiJoinNode, ColumnConstraint, ColumnDef, ColumnRef, ColumnType,
    ComparisonOp, ComparisonPredicate, ConstantExpression, CorrelatedParam, CreateTableNode,
    DeleteNode, DerivedScanNode, DropTableNode, ExpressionRef, FilterNode, HavingNode,
    IndexScanNode, InsertNode, JoinCondition, LimitNode, LogicalOp, LogicalPredicate,
    OrderByColumn, OutputColumn, ParameterExpression, PhysicalPlan, PredicateRef, ScanNode,
    SemiJoinNode, SortNode, SubqueryEvalNode, UpdateNode, Value,
}]
d20|use|[crate::executor::{
    CorrelatedParam, ExecResult, Executor, JoinCondition, JoinRelatedConfig, OutputColumn, PhysicalPlan, Value,
}]
d21|use|[crate::executor::{ColumnType, Value}]
d22|use|[crate::executor::{CorrelatedParam, ExecResult, Executor, PhysicalPlan, Value}]
d23|use|[crate::executor::{CorrelatedParam, Executor, JoinCondition, OutputColumn, PhysicalPlan}]
d24|use|[crate::executor::{ExecResult, Executor, JoinCondition, JoinRelatedConfig, OutputColumn, PhysicalPlan, Value}]
d25|use|[crate::executor::{ExecResult, Executor, JoinConfig, JoinCondition, OutputColumn, Value}]
d26|use|[crate::executor::{ExecResult, Executor, OrderByColumn, Value}]
d27|use|[crate::executor::{ExecResult, Executor, PhysicalPlan}]
d28|use|[crate::executor::{ExecResult, Executor, PredicateRef}]
d29|use|[crate::executor::{ExecResult, Executor, Value}]
d30|use|[crate::executor::{ExecResult, Executor}]
d31|use|[crate::executor::{Executor, JoinCondition, OutputColumn}]
d32|use|[crate::executor::{PhysicalPlan, Value}]
d33|use|[crate::network::NetworkError]
d34|use|[crate::network::connection::ConnectionHandler]
d35|use|[crate::network::error::NetworkError]
d36|use|[crate::network::handler::SqlHandler]
d37|use|[crate::network::pg_messages]
d38|use|[crate::network::pg_protocol::PgProtocol]
d39|use|[crate::network::protocol::Protocol]
d40|use|[crate::network::protocol::Response]
d41|use|[crate::network::protocol::{Protocol, Request, Response}]
d42|use|[crate::network::protocol::{Request, Response}]
d43|use|[crate::parser::ast::*]
d44|use|[crate::parser::ast::extract_join_table_name]
d45|use|[crate::parser::error::PlanError]
d46|use|[crate::parser::value::value_from_sqlparser]
d47|use|[crate::parser::{parse_sql, PlanBuilder}]
d48|use|[crate::plan_cache::PlanCache]
d49|use|[crate::profiling::{
    init_profiling, is_profiling_enabled, print_timings, record_time, with_profiling_scope,
}]
d50|use|[crate::profiling::{is_profiling_enabled, record_time}]
d51|use|[crate::storage::BufferPool]
d52|use|[crate::storage::FileStorage]
d53|use|[crate::storage::PageId]
d54|use|[crate::storage::Page]
d55|use|[crate::storage::Result]
d56|use|[crate::storage::RowId]
d57|use|[crate::storage::btree::IndexManager]
d58|use|[crate::storage::buffer_pool::BufferPool]
d59|use|[crate::storage::data::TableManager]
d60|use|[crate::storage::data::TableMeta]
d61|use|[crate::storage::page_format::ColumnType as StorageColumnType]
d62|use|[crate::storage::page_format::ColumnType]
d63|use|[crate::storage::page_format::Key]
d64|use|[crate::storage::page_format::RowId]
d65|use|[crate::storage::page_format::{
    compute_tuple_size, deserialize_tuple, serialize_tuple, ColumnType,
}]
d66|use|[crate::storage::page_format::{RowId, SlottedPage, SlottedPageRef}]
d67|use|[crate::storage::page_format::{compute_tuple_size, serialize_tuple, ColumnType}]
d68|use|[crate::storage::page_format::{deserialize_tuple, ColumnType, RowId}]
d69|use|[crate::storage::page_format::{deserialize_tuple, ColumnType}]
d70|use|[crate::storage::page_frame::PageGuard]
d71|use|[crate::storage::page_id::PageId]
d72|use|[crate::storage::{
    btree::node::{
        InternalNode, InternalNodeRef, LeafNode, LeafNodeRef, INTERNAL_NODE, LEAF_NODE,
    },
    page_format::{Key, RowId},
    PageId, PageGuard, Result, StorageError,
}]
d73|use|[crate::storage::{
    btree::node::{InternalNodeRef, LeafNodeRef, LEAF_NODE},
    page_format::{Key, RowId},
    BufferPool, PageId, Result,
}]
d74|use|[crate::storage::{
    page_format::{Key, RowId, Slot, SlottedPage, SlottedPageHeader, SlottedPageRef, MAX_KEY_LEN},
    Page, PageId, StorageError,
}]
d75|use|[crate::storage::{
    page_frame::{PageFrame, PageGuard},
    AsyncStorage, Page, PageId, Result, RowId, StorageError,
}]
d76|use|[crate::storage::{
    read_tuple_from_data_page, write_tuple_to_data_page, BufferPool, Result, StorageError,
    TableMeta,
}]
d77|use|[crate::storage::{AsyncStorage, Page, PageId, Result, StorageError}]
d78|use|[crate::storage::{BufferPool, ColumnType, FileStorage, Result, TableManager, TableMeta}]
d79|use|[crate::storage::{BufferPool, FileStorage, TableManager}]
d80|use|[crate::storage::{BufferPool, PageGuard, PageId, Result}]
d81|use|[crate::storage::{BufferPool, Result, RowId, TableMeta}]
d82|use|[crate::storage::{Page, PageId, Result}]
d83|use|[crate::storage::{Page, PageId}]
d84|use|[crate::storage::{PageId, Result, StorageError}]
d85|use|[crate::storage::{Result, StorageError}]
d86|use|[crate::storage::{btree::IndexManager, Result}]
d87|use|[crate::storage::{delete_tuple_from_data_page, BufferPool, Result, StorageError}]
d88|use|[crate::storage::{read_tuple_from_data_page, BufferPool, Result, TableMeta}]
d89|use|[crate::storage::{write_tuple_to_data_page, BufferPool, Result, StorageError, TableMeta}]
d90|use|[crate::storage::{write_tuple_to_data_page, BufferPool, TableManager}]
d91|use|[crate::storage]
d92|use|[crate::transaction::Snapshot]
d93|use|[crate::transaction::TransactionError]
d94|use|[crate::transaction::TransactionManager]
d95|use|[crate::transaction::VersionHeader]
d96|use|[crate::transaction::{Snapshot, TransactionError, TransactionId}]
d97|use|[crate::transaction::{Snapshot, VersionHeader}]
d98|use|[crate::transaction::{TransactionManager, VersionHeader}]
d99|use|[crate::wal::{RecoveryManager, WALBuffer, WalWriter}]
d100|use|[crate::wal::{WALBuffer, WalRecord}]
d101|use|[criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput}]
d102|use|[criterion::{criterion_group, criterion_main, BenchmarkId, Criterion}]
d103|use|[criterion::{criterion_group, criterion_main, Criterion}]
d104|use|[rand::Rng]
d105|use|[rtsql::database::Database]
d106|use|[rtsql::network::Server]
d107|use|[rtsql::storage::ColumnType]
d108|use|[rtsql::storage::RowId]
d109|use|[rtsql::wal::{WalRecord, WalWriter, WALBuffer}]
d110|use|[rusqlite::Connection]
d111|use|[serde::{Deserialize, Serialize}]
d112|use|[sqlparser::ast::*]
d113|use|[sqlparser::ast::BinaryOperator as SqlOp]
d114|use|[sqlparser::ast::BinaryOperator]
d115|use|[sqlparser::ast::DataType]
d116|use|[sqlparser::ast::JoinOperator]
d117|use|[sqlparser::ast::Value as SqlValue]
d118|use|[sqlparser::ast::{Expr, ObjectType, Query, SetExpr, Statement, TableFactor}]
d119|use|[sqlparser::ast::{Expr, Query, SetExpr, Statement, TableFactor, TableWithJoins}]
d120|use|[stdlib:rust]
d121|use|[super::*]
d122|use|[super::RowId]
d123|use|[super::record::{WalError, WalRecord, WalRecordType}]
d124|use|[super::record::{WalError, WalRecord}]
d125|use|[super::writer::WalWriter]
d126|use|[super::{AsyncPageLoader, BTree, SyncPageLoader}]
d127|use|[super::{AsyncPageLoader, SyncPageLoader}]
d128|use|[super::{WalError, WalReader, WalRecord}]
d129|use|[super::{WalError, WalRecord, WalWriter}]
d130|use|[tempfile::tempdir]
d131|use|[tempfile::{tempdir, NamedTempFile, TempDir}]
d132|use|[thiserror::Error]
d133|use|[tokio::io::{AsyncReadExt, AsyncWriteExt}]
d134|use|[tokio::net::TcpListener]
d135|use|[tokio::net::TcpStream]
d136|use|[tokio::runtime::Handle]
d137|use|[tokio::runtime::Runtime]
d138|use|[tokio::sync::RwLock]
d139|use|[tokio::sync::{Mutex, Notify}]
d140|use|[tokio::sync::{Mutex, RwLock}]
d141|use|[tokio::task::JoinError]
d142|use|[tokio::task::JoinHandle]
d143|use|[tokio::task::spawn_blocking]
d144|use|[tokio::task_local]
d145|use|[tokio::time]
d146|use|[tokio_util::sync::CancellationToken]
</pool>
<deps>
u0|d6 d137 d102
u1|d6 d137 d103
u2|d120 d108 d109 d101
u3|d120 d133 d135 d131 d137 d110 d101
u4|d6 d101
u5|d6 d120 d105 d101
u6|d120 d105
u7|d1 d4 d3 d0 d2
u8|d120 d144
u9|d120 d40 d47 d55 d119 d8 d49 d18
u11|d120 d40 d94 d99 d48 d78
u12|d120 d107 d105 d106
u13|d120 d10
u14|d120 d56 d121
u15|d120 d123
u16|d120 d142 d124 d125 d139 d145
u17|d120 d129 d51
u19|d120 d124 d143
u20|d95 d90 d128 d120
u21|d8 d120 d85 d27
u22|d8 d120 d55 d22
u23|d121 d120 d55 d26
u24|d11 d120 d121
u25|d8 d120 d85 d27
u26|d100 d30 d120 d86
u27|d8 d120 d55 d24
u28|d120 d100 d98 d76 d65 d29
u29|d32 d120 d17 d121
u30|d89 d120 d100 d98 d29 d67
u31|d8 d20 d120 d55
u32|d28 d55
u33|d30 d55
u34|d120 d31
u35|d120 d29 d88 d69 d92
u36|d120 d63
u37|d13 d91 d14 d15
u38|d30 d120 d92 d88 d69 d50
u39|d8 d120 d23
u40|d64 d16
u41|d50 d30 d120 d68 d88 d92
u43|d120 d55 d25
u44|d61 d120 d63 d21 d14 d12
u45|d55 d9 d5
u46|d15 d11 d120 d13 d91 d5
u47|d29 d55
u48|d120 d121
u49|d53 d56 d121
u50|d120 d121
u52|d132
u53|d120 d121 d100 d138 d81 d130 d96 d79
u54|d140 d120 d56 d121
u55|d5 d82
u56|d84
u57|d120 d77 d5 d143
u58|d58 d120 d121 d60 d95 d66 d52 d59 d84 d130 d62
u59|d120
u60|d138 d75 d120 d97
u62|d93 d132 d141 d122
u63|d120 d54
u64|d11 d120 d138 d87 d57 d71 d62
u66|d127 d120 d72
u67|d121 d83 d74
u68|d138 d120 d73 d126
u69|d70 d58 d120 d55 d71
u71|d120 d80 d136
u72|d120 d121
u73|d120 d121
u75|d83 d54 d121
u76|d120 d85 d7 d121
u77|d35 d39 d135 d36
u78|d8 d120 d42
u79|d11 d33
u80|d11 d133 d135 d104 d35 d41 d5 d37
u81|d120 d36 d38 d134 d8 d35 d146 d34
u82|d133 d135 d111 d35 d5
u84|d132
u85|d43 d46 d120 d121 d113 d115 d19 d114 d118 d44 d116 d45
u86|d11 d117 d45
u88|d112 d45
u89|d120
</deps>
<syms>
bench_insert|u0|func
bench_select|u0|func
bench_update|u0|func
bench_delete|u0|func
bench_scan|u0|func
bench_filter|u0|func
bench_sort|u0|func
bench_limit|u0|func
bench_join|u0|func
bench_pk_lookup_cached|u1|func
bench_pk_lookup_uncached|u1|func
create_wal_buffer|u2|func
make_insert_record|u2|func
bench_wal_baseline|u2|func
bench_wal_group_commit|u2|func
bench_wal_capacity_impact|u2|func
setup_sqlite|u3|func
insert_sqlite_rows|u3|func
start|u3|func
addr|u3|func
shutdown|u3|func
drop|u3|func
find_available_port|u3|func
new|u3|func
setup_bench_table|u3|func
execute|u3|func
file_path|u3|func
connect|u3|func
close|u3|func
bench_sqlite_insert|u3|func
bench_sqlite_select|u3|func
bench_sqlite_scan|u3|func
bench_sqlite_join|u3|func
bench_rtsql_insert|u3|func
bench_rtsql_select|u3|func
bench_rtsql_scan|u3|func
bench_compare_insert|u3|func
bench_compare_pk_lookup|u3|func
bench_compare_full_scan|u3|func
bench_split_performance|u3|func
bench_pk_lookup_after_split|u3|func
bench_non_unique_index|u3|func
bench_file_size|u3|func
bench_binary_size|u3|func
RTsqlServer|u3|struct
RTsqlDirect|u3|struct
PgClient|u3|struct
RTsqlServer|u3|impl
RTsqlDirect|u3|impl
PgClient|u3|impl
scale_insert|u4|func
scale_select|u4|func
scale_scan|u4|func
scale_join|u4|func
bench_concurrent_read|u5|func
bench_concurrent_write|u5|func
bench_concurrent_mixed|u5|func
bench_concurrent_conflict|u5|func
setup_db|u6|func
cleanup_db|u6|func
create_test_table|u6|func
insert_rows|u6|func
create_join_tables|u6|func
insert_join_data|u6|func
check_dependencies|u7|func
measure_process_memory|u7|func
measure_file_size|u7|func
create_test_table_rtsql|u7|func
create_test_table_sqlite|u7|func
insert_batch_rtsql|u7|func
insert_batch_sqlite|u7|func
benchmark_startup_time|u7|func
benchmark_memory_consumption|u7|func
benchmark_file_size|u7|func
benchmark_binary_size|u7|func
run_all_benchmarks|u7|func
init_profiling|u8|func
record_time|u8|func
get_timings|u8|func
print_timings|u8|func
is_profiling_enabled|u8|func
with_profiling_scope|u8|func
execute|u9|func
execute_inner|u9|func
execute_executor|u9|func
value_to_json|u9|func
extract_column_indices|u9|func
extract_all_table_names|u9|func
extract_all_query_table_names|u9|func
extract_subquery_tables_from_expr|u9|func
extract_all_from_table_with_joins_item|u9|func
register_table|u9|func
is_cacheable|u9|func
open|u11|func
create_table|u11|func
get_table|u11|func
execute_sql|u11|func
plan_cache_len|u11|func
Database|u11|struct
Database|u11|impl
main|u12|func|entry
new|u13|func
with_capacity|u13|func
get|u13|func
put|u13|func
clear|u13|func
len|u13|func
is_empty|u13|func
default|u13|func
PlanCache|u13|struct
PlanCache|u13|impl
try_from|u14|func
record_type|u14|func
tx_id|u14|func
serialize|u14|func
deserialize|u14|func
serialize_with_lsn|u14|func
deserialize_with_lsn|u14|func
serialize_data|u14|func
deserialize_data|u14|func
fmt|u14|func
serialize_bytes|u14|func
deserialize_bytes|u14|func
deserialize_bytes_with_len|u14|func
serialize_string|u14|func
read_string|u14|func
serialize_row_id|u14|func
read_row_id|u14|func
test_record_type_conversion|u14|func
test_delete_record|u14|func
test_abort_record|u14|func
test_commit_record|u14|func
test_checkpoint_record|u14|func
WalRecordType|u14|enum
WalRecord|u14|enum
WalError|u14|enum
WalRecord|u14|impl
open|u15|func
read_next|u15|func
read_all|u15|func
seek_to|u15|func
current_position|u15|func
path|u15|func
read|u15|func
seek|u15|func
WalReader|u15|struct
WalReader|u15|impl
new|u16|func
append|u16|func
append_commit_and_wait|u16|func
start_flush_loop|u16|func
flush_loop|u16|func
do_flush|u16|func
shutdown|u16|func
fmt|u16|func
WALBuffer|u16|struct
WALBuffer|u16|impl
new|u17|func
read_checkpoint_site|u17|func
write_checkpoint_site|u17|func
checkpoint|u17|func
CheckpointManager|u17|struct
CheckpointManager|u17|impl
open|u19|func
write_record|u19|func
fsync|u19|func
truncate_to|u19|func
get_write_count|u19|func
get_checkpoint_threshold|u19|func
set_checkpoint_threshold|u19|func
should_checkpoint|u19|func
reset_write_count|u19|func
get_current_lsn|u19|func
write_batch|u19|func
WalWriter|u19|struct
WalWriter|u19|impl
recover|u20|func
full_recover|u20|func
redo_record|u20|func
mark_uncommitted_aborted|u20|func
needs_recovery|u20|func
read_wal|u20|func
RecoveryResult|u20|struct
RecoveryManager|u20|struct
RecoveryManager|u20|impl
new|u21|func
next|u21|func
DropTableExecutor|u21|struct
DropTableExecutor|u21|impl
new|u22|func
eval_subquery|u22|func
extract_param_values|u22|func
next|u22|func
SubqueryEvalExecutor|u22|struct
SubqueryEvalExecutor|u22|impl
new|u23|func
initialize|u23|func
compare_rows|u23|func
compare_values|u23|func
next|u23|func
test_compare_values_int|u23|func
test_compare_values_float|u23|func
test_compare_values_cross_type|u23|func
test_compare_values_null|u23|func
test_compare_values_string|u23|func
SortExecutor|u23|struct
SortExecutor|u23|impl
inject_parameters|u24|func
set_parameter_value|u24|func
evaluate|u24|func
new|u24|func
set_value|u24|func
fmt|u24|func
test_column_expression|u24|func
test_constant_expression|u24|func
test_comparison_eq|u24|func
test_logical_and|u24|func
test_parameter_expression|u24|func
Predicate|u24|trait
Expression|u24|trait
ComparisonOp|u24|enum
ComparisonPredicate|u24|struct
LogicalOp|u24|enum
LogicalPredicate|u24|struct
ColumnExpression|u24|struct
ConstantExpression|u24|struct
ParameterExpression|u24|struct
ComparisonPredicate|u24|impl
LogicalPredicate|u24|impl
ColumnExpression|u24|impl
ConstantExpression|u24|impl
ParameterExpression|u24|impl
new|u25|func
next|u25|func
CreateTableExecutor|u25|struct
CreateTableExecutor|u25|impl
new|u26|func
next|u26|func
DeleteExecutor|u26|struct
DeleteExecutor|u26|impl
new|u27|func
build_right_key|u27|func
build_left_key|u27|func
build_output_row|u27|func
extract_param_values|u27|func
next|u27|func
SemiJoinPhase|u27|enum
SemiJoinExecutorV2|u27|struct
SemiJoinExecutorV2|u27|impl
new|u28|func
next|u28|func
UpdateExecutor|u28|struct
UpdateExecutor|u28|impl
inject_correlated_values|u29|func
test_inject_into_filter|u29|func
new|u30|func
next|u30|func
InsertExecutor|u30|struct
InsertExecutor|u30|impl
new|u31|func
build_right_key|u31|func
build_left_key|u31|func
build_output_row|u31|func
extract_param_values|u31|func
next|u31|func
AntiJoinPhase|u31|enum
AntiJoinExecutor|u31|struct
AntiJoinExecutor|u31|impl
new|u32|func
next|u32|func
FilterExecutor|u32|struct
FilterExecutor|u32|impl
new|u33|func
next|u33|func
LimitExecutor|u33|struct
LimitExecutor|u33|impl
JoinConfig|u34|struct
new|u35|func
next|u35|func
ScanExecutor|u35|struct
ScanExecutor|u35|impl
fmt|u36|func
hash|u36|func
to_key|u36|func
is_null|u36|func
as_float|u36|func
as_bool|u36|func
equals|u36|func
gt|u36|func
lt|u36|func
ge|u36|func
le|u36|func
add|u36|func
lt_agg|u36|func
div|u36|func
ColumnType|u36|enum
ValueError|u36|enum
Value|u36|enum
Value|u36|impl
new|u37|func
next|u37|func
HavingExecutor|u37|struct
HavingExecutor|u37|impl
new|u38|func
next|u38|func
IndexScanExecutor|u38|struct
IndexScanExecutor|u38|impl
JoinRelatedConfig|u39|struct
ExecResult|u40|enum
new|u41|func
next|u41|func
IndexScanAllExecutor|u41|struct
IndexScanAllExecutor|u41|impl
new|u43|func
build_hash_key_right|u43|func
build_hash_key_left|u43|func
build_output_row|u43|func
next|u43|func
JoinPhase|u43|enum
JoinExecutor|u43|struct
JoinExecutor|u43|impl
new|u44|func
with_constraint|u44|func
to_schema_column|u44|func
OrderByColumn|u44|struct
PhysicalPlan|u44|enum
ScanNode|u44|struct
IndexScanNode|u44|struct
IndexScanAllNode|u44|struct
FilterNode|u44|struct
InsertNode|u44|struct
UpdateNode|u44|struct
DeleteNode|u44|struct
ColumnDef|u44|struct
ColumnConstraint|u44|enum
CreateTableNode|u44|struct
DropTableNode|u44|struct
SortNode|u44|struct
LimitNode|u44|struct
JoinCondition|u44|struct
ColumnRef|u44|struct
OutputColumn|u44|struct
JoinNode|u44|struct
AggregateNode|u44|struct
HavingNode|u44|struct
CorrelatedParam|u44|struct
SemiJoinNode|u44|struct
AntiJoinNode|u44|struct
SubqueryEvalNode|u44|struct
DerivedScanNode|u44|struct
ColumnDef|u44|impl
CorrelatedParam|u44|impl
Executor|u45|trait
result_column_name|u46|func
new|u46|func
update|u46|func
finalize|u46|func
consume_input|u46|func
extract_value|u46|func
extract_group_key|u46|func
build_output_rows|u46|func
next|u46|func
AggregateFunc|u46|enum
AggregateState|u46|enum
AggregateExecutor|u46|struct
AggregateFunc|u46|impl
AggregateState|u46|impl
AggregateExecutor|u46|impl
new|u47|func
next|u47|func
DerivedScanExecutor|u47|struct
DerivedScanExecutor|u47|impl
new|u48|func
allocate|u48|func
current|u48|func
default|u48|func
test_tx_id_allocate_single_thread|u48|func
test_tx_id_allocate_multi_thread|u48|func
TransactionId|u48|struct
TransactionId|u48|impl
new|u49|func
create_tx_id|u49|func
commit_tx_id|u49|func
next_version|u49|func
with_next_version|u49|func
commit|u49|func
to_bytes|u49|func
from_bytes|u49|func
test_version_header_new|u49|func
test_version_header_with_next_version|u49|func
test_version_header_commit|u49|func
test_version_header_serialize|u49|func
test_version_header_size|u49|func
VersionHeader|u49|struct
VersionHeader|u49|impl
new|u50|func
tx_id|u50|func
is_visible|u50|func
is_visible_self|u50|func
test_snapshot_visible_committed_before|u50|func
test_snapshot_not_visible_uncommitted|u50|func
test_snapshot_not_visible_active_tx|u50|func
test_snapshot_not_visible_after_snapshot|u50|func
test_snapshot_visible_self_created|u50|func
Snapshot|u50|struct
Snapshot|u50|impl
TransactionError|u52|enum
new|u53|func
id|u53|func
snapshot|u53|func
state|u53|func
set_wal_buffer|u53|func
begin|u53|func
commit|u53|func
abort|u53|func
active_transactions|u53|func
record_version|u53|func
get_tx_versions|u53|func
tx_versions|u53|func
current_tx_id|u53|func
commit_by_id|u53|func
commit_mark_versions|u53|func
abort_cleanup_versions|u53|func
default|u53|func
create_test_buffer_pool|u53|func
create_test_table|u53|func
test_transaction_begin|u53|func
test_transaction_commit|u53|func
test_transaction_abort|u53|func
test_transaction_multiple|u53|func
test_transaction_snapshot_active_list|u53|func
test_double_commit_error|u53|func
test_tx_versions_initialization|u53|func
test_record_version_single|u53|func
test_record_version_multiple|u53|func
test_get_tx_versions_empty|u53|func
TransactionState|u53|enum
Transaction|u53|struct
TransactionManager|u53|struct
Transaction|u53|impl
TransactionManager|u53|impl
new|u54|func
get_lock|u54|func
default|u54|func
test_row_lock_acquire_release|u54|func
test_row_lock_concurrent_same_row|u54|func
test_row_lock_different_rows|u54|func
RowLockTable|u54|struct
RowLockTable|u54|impl
page_size|u55|func
AsyncStorage|u55|trait
new|u56|func
from_bytes|u56|func
Page|u56|struct
Page|u56|impl
open|u57|func
page_count|u57|func
read_page_blocking|u57|func
read_page|u57|func
write_page|u57|func
allocate_page|u57|func
free_page|u57|func
sync|u57|func
FileStorage|u57|struct
FileStorage|u57|impl
write_tuple_to_data_page|u58|func
read_tuple_from_data_page|u58|func
update_version_header_in_data_page|u58|func
delete_tuple_from_data_page|u58|func
setup|u58|func
write_read_single_tuple|u58|func
write_read_multiple_tuples|u58|func
page_full_auto_allocate|u58|func
read_invalid_slot|u58|func
version_header_roundtrip|u58|func
to_offset|u59|func
page_num|u59|func
fmt|u59|func
PageId|u59|struct
PageId|u59|impl
new|u60|func
capacity|u60|func
storage|u60|func
get_page|u60|func
evict_one|u60|func
flush_all|u60|func
read_version_header|u60|func
write_commit_tx_id|u60|func
find_visible_version|u60|func
mark_tx_aborted|u60|func
free_page|u60|func
BufferPool|u60|struct
BufferPool|u60|impl
from|u62|func
StorageError|u62|enum
new|u63|func
deref|u63|func
mark_dirty|u63|func
ref_count|u63|func
page|u63|func
page_data|u63|func
modify_page|u63|func
drop|u63|func
PageFrame|u63|struct
PageGuard|u63|struct
PageDataGuard|u63|struct
PageFrame|u63|impl
PageGuard|u63|impl
new|u64|func
to_tuple|u64|func
gc_table|u64|func
create_table|u64|func
get_table|u64|func
table_exists|u64|func
drop_table|u64|func
ColumnSchema|u64|struct
TableMeta|u64|struct
TableManager|u64|struct
ColumnSchema|u64|impl
TableMeta|u64|impl
TableManager|u64|impl
find_child_position|u66|func
sibling_ids|u66|func
new|u66|func
root_page_id|u66|func
from_root|u66|func
search|u66|func
search_from_page|u66|func
search_async|u66|func
search_from_page_async|u66|func
insert|u66|func
insert_into_page|u66|func
delete|u66|func
delete_from_page|u66|func
delete_from_leaf|u66|func
handle_leaf_underflow|u66|func
leaf_key_count|u66|func
internal_key_count|u66|func
redistribute_leaf_right|u66|func
redistribute_leaf_left|u66|func
merge_leaves|u66|func
delete_from_internal|u66|func
handle_child_merge|u66|func
find_separator_slot|u66|func
handle_internal_underflow|u66|func
shrink_root|u66|func
redistribute_internal_right|u66|func
redistribute_internal_left|u66|func
merge_internal_nodes|u66|func
get_parent_separator_key|u66|func
update_parent_separator|u66|func
read_leaf_pair|u66|func
rebuild_leaf|u66|func
read_internal_seps|u66|func
rebuild_internal|u66|func
scan_all|u66|func
update|u66|func
update_in_page|u66|func
search_all|u66|func
search_all_from_page|u66|func
delete_by_key|u66|func
delete_all_from_page|u66|func
delete_exact|u66|func
delete_exact_from_page|u66|func
SplitResult|u66|struct
MergeInfo|u66|struct
BTree|u66|struct
BTree|u66|impl
from_page|u67|func
init|u67|func
key_count|u67|func
get_key|u67|func
get_row_id|u67|func
find_key_position|u67|func
insert|u67|func
shift_slots_right|u67|func
insert_simple|u67|func
delete|u67|func
delete_slot|u67|func
update|u67|func
next_leaf_page_id|u67|func
set_next_leaf_page_id|u67|func
free_space|u67|func
min_keys|u67|func
split|u67|func
can_merge_with|u67|func
merge_right|u67|func
redistribute_right|u67|func
new|u67|func
find_all_matches|u67|func
find_key_position_binary|u67|func
set_leftmost_child|u67|func
get_child_page_id|u67|func
find_child_page_id|u67|func
insert_separator|u67|func
find_insert_position|u67|func
shift_slots_right_internal|u67|func
insert_separator_simple|u67|func
remove_separator|u67|func
get_all_separators|u67|func
leftmost_child|u67|func
find_child_page_id_binary|u67|func
test_leaf_node_init|u67|func
test_leaf_node_insert_single|u67|func
test_leaf_node_insert_multiple|u67|func
test_leaf_node_find_position|u67|func
test_leaf_node_ref_from_page_data|u67|func
test_leaf_node_ref_find_key_position|u67|func
test_internal_node_ref_from_page_data|u67|func
test_leaf_node_ref_binary_search_matches_linear|u67|func
test_internal_node_ref_binary_search_matches_linear|u67|func
test_leaf_node_split|u67|func
test_leaf_node_split_empty_fails|u67|func
test_leaf_node_split_single_entry|u67|func
test_leaf_node_split_odd_count|u67|func
test_internal_node_split|u67|func
test_internal_node_split_empty_fails|u67|func
test_internal_node_split_single_separator|u67|func
test_internal_node_split_odd_count|u67|func
test_internal_remove_separator|u67|func
test_internal_merge_right|u67|func
test_internal_can_merge_with|u67|func
test_internal_min_keys|u67|func
test_leaf_can_merge_with|u67|func
test_leaf_merge_right|u67|func
test_leaf_merge_chain|u67|func
test_leaf_redistribute_right|u67|func
test_leaf_merge_overflow|u67|func
LeafSplitData|u67|struct
LeafMergeResult|u67|struct
InternalSplitData|u67|struct
InternalMergeResult|u67|struct
LeafNode|u67|struct
LeafNodeRef|u67|struct
InternalNode|u67|struct
InternalNodeRef|u67|struct
Node|u67|enum
new|u68|func
search|u68|func
search_all|u68|func
search_from_page_async|u68|func
search_all_from_page_async|u68|func
insert|u68|func
delete|u68|func
scan_all|u68|func
scan_all_async_from_root|u68|func
update|u68|func
find_key_by_row_id|u68|func
IndexManager|u68|struct
IndexManager|u68|impl
new|u69|func
load_page|u69|func
AsyncPageLoader|u69|struct
AsyncPageLoader|u69|impl
new|u71|func
load_page|u71|func
allocate_page|u71|func
free_page|u71|func
SyncPageLoader|u71|struct
SyncPageLoader|u71|impl
new|u72|func
len|u72|func
is_empty|u72|func
as_bytes|u72|func
serialize|u72|func
deserialize|u72|func
partial_cmp|u72|func
cmp|u72|func
test_key_new|u72|func
test_key_empty|u72|func
test_key_max_length|u72|func
test_key_too_long|u72|func
test_key_serialize_deserialize|u72|func
test_key_ordering|u72|func
Key|u72|struct
Key|u72|impl
new|u73|func
serialize|u73|func
deserialize|u73|func
fmt|u73|func
test_row_id_new|u73|func
test_row_id_serialize_deserialize|u73|func
test_row_id_size|u73|func
RowId|u73|struct
RowId|u73|impl
new|u75|func
serialize|u75|func
deserialize|u75|func
slot_count|u75|func
get_slot|u75|func
get_slot_by_logical_id|u75|func
get_slot_data|u75|func
header|u75|func
init|u75|func
add_slot|u75|func
delete_slot|u75|func
delete_slot_by_logical_id|u75|func
free_space|u75|func
sync_header|u75|func
reload_header|u75|func
page_id|u75|func
page_data|u75|func
test_slotted_page_init|u75|func
test_slotted_page_add_slot|u75|func
test_slotted_page_add_multiple_slots|u75|func
test_slotted_page_free_space|u75|func
test_slotted_page_no_space|u75|func
test_logical_id_increment|u75|func
test_delete_preserves_logical_id|u75|func
test_get_by_logical_id_after_compact|u75|func
Slot|u75|struct
SlottedPageHeader|u75|struct
SlottedPage|u75|struct
SlottedPageRef|u75|struct
Slot|u75|impl
SlottedPageHeader|u75|impl
compute_tuple_size|u76|func
serialize_tuple|u76|func
err_too_small|u76|func
deserialize_tuple|u76|func
eof|u76|func
roundtrip_single|u76|func
serialize_int_roundtrip|u76|func
serialize_string_roundtrip|u76|func
serialize_null_roundtrip|u76|func
serialize_mixed|u76|func
deserialize_truncated|u76|func
large_string|u76|func
serialize_float_roundtrip|u76|func
serialize_bool_roundtrip|u76|func
serialize_mixed_types|u76|func
ColumnType|u76|enum
new|u77|func
handle|u77|func
ConnectionHandler|u77|struct
new|u78|func
execute|u78|func
SqlHandler|u78|struct
SqlHandler|u78|impl
authentication_ok|u79|func
parameter_status|u79|func
backend_key_data|u79|func
ready_for_query|u79|func
row_description|u79|func
data_row|u79|func
command_complete|u79|func
error_response|u79|func
map_error_to_sqlstate|u79|func
new|u80|func
state|u80|func
process_id|u80|func
secret_key|u80|func
read_exact|u80|func
handle_startup|u80|func
send_startup_response|u80|func
handle_query|u80|func
handle_terminate|u80|func
json_to_value|u80|func
default|u80|func
parse_request|u80|func
write_response|u80|func
ProtocolState|u80|enum
PgProtocol|u80|struct
PgProtocol|u80|impl
new|u81|func
shutdown_token|u81|func
run|u81|func
Server|u81|struct
Server|u81|impl
sql|u82|func
new|u82|func
default|u82|func
parse_request|u82|func
write_response|u82|func
Protocol|u82|trait
Request|u82|enum
Response|u82|enum
JsonProtocol|u82|struct
Request|u82|impl
JsonProtocol|u82|impl
NetworkError|u84|enum
new|u85|func
register_table|u85|func
build_plan|u85|func
validate_table|u85|func
resolve_column_ref|u85|func
extract_join_conditions|u85|func
get_plan_output_columns|u85|func
build_from_clause_with_projection|u85|func
build_query|u85|func
is_simple_pk_equality|u85|func
extract_pk_from_where|u85|func
convert_comparison_op|u85|func
build_having_expression|u85|func
build_having|u85|func
build_expression|u85|func
build_where|u85|func
try_build_where_subquery|u85|func
extract_subquery_table_names|u85|func
extract_correlated_params|u85|func
collect_outer_column_refs|u85|func
has_outer_refs_outside|u85|func
resolve_column_in_plan|u85|func
get_subquery_first_column|u85|func
build_output_columns_for_table|u85|func
build_insert|u85|func
extract_insert_values|u85|func
convert_data_type|u85|func
extract_column_constraints|u85|func
extract_default_value|u85|func
extract_primary_key|u85|func
build_create_table|u85|func
build_drop_table|u85|func
build_update|u85|func
build_delete|u85|func
extract_column_name|u85|func
parse_limit_value|u85|func
parse_offset_value|u85|func
is_aggregate_expr|u85|func
extract_aggregate_func|u85|func
extract_single_column_arg|u85|func
expr_to_column_name|u85|func
default|u85|func
test_plan_builder_new|u85|func
test_register_table|u85|func
test_validate_table|u85|func
test_build_query_scan|u85|func
test_build_query_index_scan|u85|func
test_build_insert|u85|func
test_build_update|u85|func
test_build_delete|u85|func
test_extract_pk_from_where_reversed|u85|func
test_nonexistent_table|u85|func
test_unsupported_where|u85|func
test_insert_multiple_rows|u85|func
PlanBuilder|u85|struct
PlanBuilder|u85|impl
value_from_sqlparser|u86|func
parse_sql|u88|func
extract_select_body|u88|func
extract_table_name|u88|func
extract_columns|u88|func
extract_qualified_columns|u88|func
extract_name_from_object|u88|func
extract_join_table_name|u88|func
expr_to_column_name_static|u88|func
extract_single_col_static|u88|func
fmt|u89|func
PlanError|u89|enum
</syms>
</pir>