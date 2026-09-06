//! CLI 数据库名称解析 —— 裸名映射到集中存储，含 `/` 的参数按路径直开

use std::path::PathBuf;

/// 解析 `<db>` 参数为数据库文件路径。
///
/// - 含 `/`：按路径直接使用（相对或绝对路径）。
/// - 裸名：`$RTSQL_HOME`（未设置时 `$HOME/.rtsql`）下的 `db/<name>.db`。
/// - `$RTSQL_HOME` 与 `$HOME` 均未设置：返回 Err。
///
/// 不做 `~` 字面量展开，不创建目录。
pub fn resolve_db_path(arg: &str) -> Result<PathBuf, String> {
    if arg.contains('/') {
        return Ok(PathBuf::from(arg));
    }
    let base = match std::env::var("RTSQL_HOME") {
        Ok(home) => PathBuf::from(home),
        Err(_) => {
            let home = std::env::var("HOME").map_err(|_| {
                "neither RTSQL_HOME nor HOME is set; cannot resolve database name".to_string()
            })?;
            PathBuf::from(home).join(".rtsql")
        }
    };
    Ok(base.join("db").join(format!("{}.db", arg)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// env 是进程级全局状态而 cargo test 默认并行：
    /// 涉及 env 的用例集中在单个 #[test] 内顺序执行，Guard 保证断言失败时也恢复现场。
    struct EnvGuard {
        home: Option<String>,
        rtsql_home: Option<String>,
    }

    impl EnvGuard {
        fn capture() -> Self {
            Self {
                home: std::env::var("HOME").ok(),
                rtsql_home: std::env::var("RTSQL_HOME").ok(),
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.home {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
            match &self.rtsql_home {
                Some(v) => std::env::set_var("RTSQL_HOME", v),
                None => std::env::remove_var("RTSQL_HOME"),
            }
        }
    }

    #[test]
    fn test_path_arg_used_as_is() {
        assert_eq!(resolve_db_path("./x.db").unwrap(), PathBuf::from("./x.db"));
        assert_eq!(
            resolve_db_path("/abs/x.db").unwrap(),
            PathBuf::from("/abs/x.db")
        );
    }

    #[test]
    fn test_bare_name_env_cases() {
        let _guard = EnvGuard::capture();

        // 裸名默认：RTSQL_HOME 未设置，HOME 决定基目录
        std::env::remove_var("RTSQL_HOME");
        std::env::set_var("HOME", "/home/testuser");
        assert_eq!(
            resolve_db_path("foo").unwrap(),
            PathBuf::from("/home/testuser/.rtsql/db/foo.db")
        );

        // RTSQL_HOME 覆盖默认基目录
        std::env::set_var("RTSQL_HOME", "/tmp/rtshome");
        assert_eq!(
            resolve_db_path("foo").unwrap(),
            PathBuf::from("/tmp/rtshome/db/foo.db")
        );

        // HOME 与 RTSQL_HOME 均缺失 → Err
        std::env::remove_var("RTSQL_HOME");
        std::env::remove_var("HOME");
        assert!(resolve_db_path("foo").is_err());
    }
}
