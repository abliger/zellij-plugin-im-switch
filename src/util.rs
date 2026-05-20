use zellij_tile::prelude::*;

/// 配置值中可能包含 "~"，而 sh 不会自动展开它。
pub fn expand_tilde(path: &str) -> String {
    if path == "~" {
        if let Ok(home) = std::env::var("HOME") {
            return home;
        }
    } else if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{}/{}", home, rest);
        }
    }
    path.to_string()
}

/// 对字符串做 shell 引号转义，使其可以安全地插入到 sh -c 脚本中。
/// 防止 im_select 或 state_dir 包含特殊字符时引发注入。
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace("'", "'\\''"))
}

/// 将 PaneId 编码为 "t{数字}" 或 "p{数字}"，用作 .ime 文件名。
pub fn file_name(id: &PaneId) -> String {
    match id {
        PaneId::Terminal(n) => format!("t{}", n),
        PaneId::Plugin(n) => format!("p{}", n),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_tilde() {
        std::env::set_var("HOME", "/home/user");
        assert_eq!(expand_tilde("~"), "/home/user");
        assert_eq!(expand_tilde("~/foo"), "/home/user/foo");
        assert_eq!(expand_tilde("/absolute/path"), "/absolute/path");
        assert_eq!(expand_tilde("relative"), "relative");
    }

    #[test]
    fn test_shell_quote() {
        assert_eq!(shell_quote("safe"), "'safe'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote("a'b'c"), "'a'\\''b'\\''c'");
    }

    #[test]
    fn test_file_name() {
        assert_eq!(file_name(&PaneId::Terminal(42)), "t42");
        assert_eq!(file_name(&PaneId::Plugin(7)), "p7");
    }
}
