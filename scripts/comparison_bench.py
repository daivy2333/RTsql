#!/usr/bin/env python3
"""
RTsql vs SQLite 全面对比基准测试
M17.5-T3: 测量多个维度的性能差异

待测维度：
- 内存消耗：启动后 RSS + 10K 行 INSERT 峰值 RSS
- 启动时间：冷启动 + 热启动时间
- 数据文件大小：10K/100K 行后 .db 文件大小
- 编译产物大小：release binary 大小
- 大规模加载：批量 INSERT 吞吐
- 并发资源：不同并发度下 CPU + 内存

依赖：psutil, rusqlite (通过 subprocess 调用)
"""

import os
import sys
import time
import subprocess
import tempfile
import shutil
import json
from pathlib import Path

try:
    import psutil
except ImportError:
    print("Error: psutil not installed. Install with: pip install psutil")
    sys.exit(1)

# 项目根目录
PROJECT_ROOT = Path(__file__).parent.parent.absolute()
RTSQL_BIN = PROJECT_ROOT / "target" / "release" / "rtsql"
RTSQL_LIB = PROJECT_ROOT / "target" / "release" / "librtsql.a"


def check_dependencies():
    """检查依赖是否满足"""
    if not RTSQL_BIN.exists():
        print(f"Error: RTsql binary not found at {RTSQL_BIN}")
        print("Build with: cargo build --release")
        return False

    if not shutil.which("sqlite3"):
        print("Error: sqlite3 command not found")
        return False

    return True


def measure_process_memory(pid):
    """测量进程内存消耗（RSS）"""
    try:
        process = psutil.Process(pid)
        return process.memory_info().rss / 1024 / 1024  # MB
    except psutil.NoSuchProcess:
        return 0


def measure_file_size(path):
    """测量文件大小"""
    return os.path.getsize(path) / 1024 / 1024  # MB


def create_test_table_rtsql(db_path, table_name="test"):
    """创建 RTsql 测试表"""
    cmd = [
        str(RTSQL_BIN),
        "--file", str(db_path),
        "--command",
        f"CREATE TABLE {table_name} (id INT PRIMARY KEY, name VARCHAR(100), value INT)"
    ]
    subprocess.run(cmd, check=True, capture_output=True)


def create_test_table_sqlite(db_path, table_name="test"):
    """创建 SQLite 测试表"""
    subprocess.run([
        "sqlite3", str(db_path),
        f"CREATE TABLE {table_name} (id INTEGER PRIMARY KEY, name TEXT, value INTEGER);"
    ], check=True, capture_output=True)


def insert_batch_rtsql(db_path, start, n, table_name="test"):
    """批量插入 RTsql 数据"""
    # RTsql 通过命令行一次插入多行（简化版本）
    for i in range(start, start + n):
        cmd = [
            str(RTSQL_BIN),
            "--file", str(db_path),
            "--command",
            f"INSERT INTO {table_name} VALUES ({i}, 'user_{i}', {i * 10})"
        ]
        subprocess.run(cmd, check=True, capture_output=True)


def insert_batch_sqlite(db_path, start, n, table_name="test"):
    """批量插入 SQLite 数据"""
    subprocess.run([
        "sqlite3", str(db_path),
        f"INSERT INTO {table_name} SELECT {start + i}, 'user_{start + i}', {(start + i) * 10} FROM (SELECT 1) WHERE {start + i} < {start + n};"
    ] * n, check=True, capture_output=True)


def benchmark_startup_time(db_type, db_path):
    """测量启动时间"""
    start_time = time.time()

    if db_type == "rtsql":
        create_test_table_rtsql(db_path)
    else:
        create_test_table_sqlite(db_path)

    elapsed = time.time() - start_time
    return elapsed


def benchmark_memory_consumption(db_type, db_path, num_rows=10000):
    """测量内存消耗（启动后 + 工作峰值）"""
    # 创建临时数据库
    if db_type == "rtsql":
        create_test_table_rtsql(db_path)
    else:
        create_test_table_sqlite(db_path)

    # 启动后内存（通过进程测量）
    # 注意：RTsql 是短生命周期进程，SQLite 是嵌入式
    # 这里我们测量文件大小作为间接指标

    # 工作峰值内存（插入数据后）
    start_time = time.time()
    if db_type == "rtsql":
        insert_batch_rtsql(db_path, 0, num_rows)
    else:
        insert_batch_sqlite(db_path, 0, num_rows)
    elapsed = time.time() - start_time

    # 返回文件大小和插入时间
    file_size = measure_file_size(db_path)
    return file_size, elapsed


def benchmark_file_size(db_type, db_path, num_rows):
    """测量数据文件大小"""
    if db_type == "rtsql":
        create_test_table_rtsql(db_path)
        insert_batch_rtsql(db_path, 0, num_rows)
    else:
        create_test_table_sqlite(db_path)
        insert_batch_sqlite(db_path, 0, num_rows)

    return measure_file_size(db_path)


def benchmark_binary_size():
    """测量编译产物大小"""
    results = {}

    if RTSQL_BIN.exists():
        results["rtsql_binary"] = measure_file_size(RTSQL_BIN)

    if RTSQL_LIB.exists():
        results["rtsql_static_lib"] = measure_file_size(RTSQL_LIB)

    return results


def run_all_benchmarks():
    """运行所有基准测试"""
    if not check_dependencies():
        return

    results = {
        "timestamp": time.strftime("%Y-%m-%d %H:%M:%S"),
        "rtsql": {},
        "sqlite": {}
    }

    # 1. 启动时间对比
    print("=== 1. Startup Time Benchmark ===")
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as tmp:
        db_path_rtsql = tmp.name
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as tmp:
        db_path_sqlite = tmp.name

    startup_rtsql = benchmark_startup_time("rtsql", db_path_rtsql)
    startup_sqlite = benchmark_startup_time("sqlite", db_path_sqlite)

    results["rtsql"]["startup_time"] = startup_rtsql
    results["sqlite"]["startup_time"] = startup_sqlite

    print(f"RTsql startup time: {startup_rtsql:.3f}s")
    print(f"SQLite startup time: {startup_sqlite:.3f}s")

    os.unlink(db_path_rtsql)
    os.unlink(db_path_sqlite)

    # 2. 数据文件大小对比（10K 和 100K）
    print("\n=== 2. File Size Benchmark ===")
    for num_rows in [10000, 100000]:
        with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as tmp:
            db_path_rtsql = tmp.name
        with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as tmp:
            db_path_sqlite = tmp.name

        size_rtsql = benchmark_file_size("rtsql", db_path_rtsql, num_rows)
        size_sqlite = benchmark_file_size("sqlite", db_path_sqlite, num_rows)

        results["rtsql"][f"file_size_{num_rows//1000}k"] = size_rtsql
        results["sqlite"][f"file_size_{num_rows//1000}k"] = size_sqlite

        print(f"{num_rows//1000}K rows:")
        print(f"  RTsql: {size_rtsql:.2f} MB")
        print(f"  SQLite: {size_sqlite:.2f} MB")

        os.unlink(db_path_rtsql)
        os.unlink(db_path_sqlite)

    # 3. 编译产物大小
    print("\n=== 3. Binary Size Benchmark ===")
    binary_sizes = benchmark_binary_size()
    results["rtsql"]["binary_size"] = binary_sizes

    for key, value in binary_sizes.items():
        print(f"{key}: {value:.2f} MB")

    # 4. 大规模加载性能对比（10K INSERT）
    print("\n=== 4. Bulk Insert Benchmark (10K rows) ===")
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as tmp:
        db_path_rtsql = tmp.name
    with tempfile.NamedTemporaryFile(suffix=".db", delete=False) as tmp:
        db_path_sqlite = tmp.name

    # RTsql
    create_test_table_rtsql(db_path_rtsql)
    start_time = time.time()
    insert_batch_rtsql(db_path_rtsql, 0, 10000)
    elapsed_rtsql = time.time() - start_time

    # SQLite
    create_test_table_sqlite(db_path_sqlite)
    start_time = time.time()
    insert_batch_sqlite(db_path_sqlite, 0, 10000)
    elapsed_sqlite = time.time() - start_time

    results["rtsql"]["bulk_insert_10k"] = elapsed_rtsql
    results["sqlite"]["bulk_insert_10k"] = elapsed_sqlite

    print(f"RTsql 10K INSERT: {elapsed_rtsql:.2f}s ({10000/elapsed_rtsql:.0f} rows/s)")
    print(f"SQLite 10K INSERT: {elapsed_sqlite:.2f}s ({10000/elapsed_sqlite:.0f} rows/s)")

    os.unlink(db_path_rtsql)
    os.unlink(db_path_sqlite)

    # 输出 JSON 结果
    print("\n=== Full Results (JSON) ===")
    print(json.dumps(results, indent=2))

    # 保存结果到文件
    result_file = PROJECT_ROOT / ".claude" / "docs" / "comparison_results.json"
    with open(result_file, "w") as f:
        json.dump(results, f, indent=2)
    print(f"\nResults saved to {result_file}")

    return results


if __name__ == "__main__":
    run_all_benchmarks()