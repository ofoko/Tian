//! tc 词法分析器
//!
//! 核心难点（规范 2.7）：区分 `/a`（只读声明）、`//b`（可变声明）与 `a/2`（除法）。
//! 判定逻辑：读到 `/` 后向前看一个字符：
//!   1. `//` + 字母/下划线 → 可变声明前缀
//!   2. `/` + 字母/下划线   → 只读声明前缀
//!   3. 其余               → 除号

use std::fmt;

/// Token 类型
#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    /// `/` 只读声明前缀（紧贴标识符）
    ReadOnlyDecl,
    /// `//` 可变声明前缀（紧贴标识符）
    MutableDecl,
    /// `w/` while
    While,
    /// `i/` if
    If,
    /// `e/` else
    Else,
    /// `f/` 函数定义
    Fn,
    /// `r/` 函数返回
    Return,
    /// v3.0 结构体声明
    Struct,
    /// v3.6 模块导入
    Use,
    /// v3.8 跳出循环
    Break,
    /// v3.8 进入下一轮循环
    Continue,
    /// 标识符（含中文）
    Ident(String),
    /// 整数字面量
    Int(i64),
    /// 浮点字面量
    Float(f64),
    /// 字符串字面量（已解转义）
    Str(String),
    /// true / false
    Bool(bool),
    /// `` ` `` 打印前缀
    Backtick,
    /// 转换函数 toi / tos
    ConvertFn(String),
    /// 赋值 =
    Assign,
    /// + - * /
    Plus,
    Minus,
    Star,
    Slash,
    /// 比较：<= >= == != < >
    Le,
    Ge,
    Eq,
    Ne,
    Lt,
    Gt,
    /// ( ) { } , :
    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Colon,
    /// v2.1 借用前缀 &
    Amp,
    /// v2.2 语义锚点前缀 @
    At,
    /// v2.2 @example 中的 ->
    Arrow,
    /// v3.0 字段访问 .
    Dot,
    /// v3.3 数组下标 [ ]
    LBracket,
    RBracket,
    /// 换行（语句结束符）
    Newline,
    /// 文件结束
    Eof,
}

impl fmt::Display for Tok {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Tok::ReadOnlyDecl => write!(f, "/"),
            Tok::MutableDecl => write!(f, "//"),
            Tok::While => write!(f, "w/"),
            Tok::If => write!(f, "i/"),
            Tok::Else => write!(f, "e/"),
            Tok::Fn => write!(f, "f/"),
            Tok::Return => write!(f, "r/"),
            Tok::Struct => write!(f, "struct"),
            Tok::Use => write!(f, "use"),
            Tok::Break => write!(f, "break"),
            Tok::Continue => write!(f, "continue"),
            Tok::Ident(s) => write!(f, "标识符:{}", s),
            Tok::Int(n) => write!(f, "整数:{}", n),
            Tok::Float(v) => write!(f, "浮点:{}", v),
            Tok::Str(s) => write!(f, "字符串:{:?}", s),
            Tok::Bool(b) => write!(f, "布尔:{}", b),
            Tok::Backtick => write!(f, "`"),
            Tok::ConvertFn(name) => write!(f, "转换函数:{}", name),
            Tok::Assign => write!(f, "="),
            Tok::Plus => write!(f, "+"),
            Tok::Minus => write!(f, "-"),
            Tok::Star => write!(f, "*"),
            Tok::Slash => write!(f, "/ (除号)"),
            Tok::Le => write!(f, "<="),
            Tok::Ge => write!(f, ">="),
            Tok::Eq => write!(f, "=="),
            Tok::Ne => write!(f, "!="),
            Tok::Lt => write!(f, "<"),
            Tok::Gt => write!(f, ">"),
            Tok::LParen => write!(f, "("),
            Tok::RParen => write!(f, ")"),
            Tok::LBrace => write!(f, "{{"),
            Tok::RBrace => write!(f, "}}"),
            Tok::Comma => write!(f, ","),
            Tok::Colon => write!(f, ":"),
            Tok::Amp => write!(f, "&"),
            Tok::At => write!(f, "@"),
            Tok::Arrow => write!(f, "->"),
            Tok::Dot => write!(f, "."),
            Tok::LBracket => write!(f, "["),
            Tok::RBracket => write!(f, "]"),
            Tok::Newline => write!(f, "⏎"),
            Tok::Eof => write!(f, "EOF"),
        }
    }
}

/// 词法错误，附带行号
#[derive(Debug, Clone, PartialEq)]
pub struct LexError {
    pub msg: String,
    pub line: usize,
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "词法错误（第 {} 行）：{}", self.line, self.msg)
    }
}

/// 词法分析入口：源码 → Token 流（以 Eof 结尾）
pub fn lex(src: &str) -> Result<Vec<Tok>, LexError> {
    Ok(lex_spanned(src)?.into_iter().map(|(t, _)| t).collect())
}

/// 带 Token 起始行号的版本（供解析器报错用）
pub fn lex_spanned(src: &str) -> Result<Vec<(Tok, usize)>, LexError> {
    let mut lexer = Lexer::new(src);
    let mut toks = Vec::new();
    loop {
        let line = lexer.line;
        let tok = lexer.next_token()?;
        let is_eof = tok == Tok::Eof;
        toks.push((tok, line));
        if is_eof {
            return Ok(toks);
        }
    }
}

/// 该 Token 是否为"表达式结尾"（其后出现 / 应判定为除号，规范 2.7 v0.1.1）
fn ends_expr(t: Option<&Tok>) -> bool {
    matches!(
        t,
        Some(Tok::Ident(_))
            | Some(Tok::Int(_))
            | Some(Tok::Float(_))
            | Some(Tok::Str(_))
            | Some(Tok::Bool(_))
            | Some(Tok::RParen)
    )
}

struct Lexer {
    chars: Vec<char>,
    pos: usize,
    line: usize,
    prev: Option<Tok>,
}

impl Lexer {
    fn new(src: &str) -> Self {
        Lexer {
            chars: src.chars().collect(),
            pos: 0,
            line: 1,
            prev: None,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek2(&self) -> Option<char> {
        self.chars.get(self.pos + 1).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
            if c == Some('\n') {
                self.line += 1;
            }
        }
        c
    }

    fn err(&self, msg: impl Into<String>) -> LexError {
        LexError {
            msg: msg.into(),
            line: self.line,
        }
    }

    /// 斜杠判定：/ + 字母或下划线 = 声明前缀（规范 2.7）
    fn is_decl_start(c: Option<char>) -> bool {
        match c {
            Some(c) => c.is_alphabetic() || c == '_',
            None => false,
        }
    }

    fn next_token(&mut self) -> Result<Tok, LexError> {
        let tok = self.next_token_inner()?;
        self.prev = Some(tok.clone());
        Ok(tok)
    }

    fn next_token_inner(&mut self) -> Result<Tok, LexError> {
        // 跳过空格与制表符
        while matches!(self.peek(), Some(' ') | Some('\t') | Some('\r')) {
            self.bump();
        }

        let c = match self.peek() {
            None => return Ok(Tok::Eof),
            Some(c) => c,
        };

        // 表达式内（前一 Token 为操作数结尾）→ / 恒为除号（规范 2.7 v0.1.1）
        let div_ctx = ends_expr(self.prev.as_ref());

        // 换行：语句结束符
        if c == '\n' {
            self.bump();
            // 连续换行合并为一个
            while matches!(self.peek(), Some('\n') | Some('\r') | Some(' ') | Some('\t')) {
                self.bump();
            }
            return Ok(Tok::Newline);
        }

        // 注释：# 到行尾
        if c == '#' {
            while let Some(c) = self.peek() {
                if c == '\n' {
                    break;
                }
                self.bump();
            }
            return self.next_token();
        }

        // ★ 核心：斜杠歧义判定（规范 2.7 v0.1.1）
        if c == '/' {
            if self.peek2() == Some('/') {
                if !div_ctx && Self::is_decl_start(self.chars.get(self.pos + 2).copied()) {
                    // //b → 可变声明
                    self.bump();
                    self.bump();
                    return Ok(Tok::MutableDecl);
                }
                // 表达式内或 // 后非标识符 → 两个除号
                self.bump();
                return Ok(Tok::Slash);
            }
            if !div_ctx && Self::is_decl_start(self.peek2()) {
                // /a → 只读声明
                self.bump();
                return Ok(Tok::ReadOnlyDecl);
            }
            // 其余情况 → 除号
            self.bump();
            return Ok(Tok::Slash);
        }

        // v2.2：-> 箭头（@example 用）
        if c == '-' && self.peek2() == Some('>') {
            self.bump();
            self.bump();
            return Ok(Tok::Arrow);
        }

        // 单字符符号
        let single = match c {
            '`' => Some(Tok::Backtick),
            '+' => Some(Tok::Plus),
            '-' => Some(Tok::Minus),
            '*' => Some(Tok::Star),
            '&' => Some(Tok::Amp),
            '@' => Some(Tok::At),
            // v3.0：字段访问（数字后的 . 已在 lex_number 中作为小数点消费）
            '.' => Some(Tok::Dot),
            '(' => Some(Tok::LParen),
            ')' => Some(Tok::RParen),
            '[' => Some(Tok::LBracket),
            ']' => Some(Tok::RBracket),
            '{' => Some(Tok::LBrace),
            '}' => Some(Tok::RBrace),
            ',' => Some(Tok::Comma),
            ':' => Some(Tok::Colon),
            // '=' '<' '>' '!' 走下方专门分支（可能构成 == / <= / >= / !=）
            _ => None,
        };
        if let Some(t) = single {
            self.bump();
            return Ok(t);
        }

        // 双字符比较运算符与单字符 < >
        if c == '<' || c == '>' || c == '!' {
            self.bump();
            if self.peek() == Some('=') {
                self.bump();
                return Ok(match c {
                    '<' => Tok::Le,
                    '>' => Tok::Ge,
                    _ => Tok::Ne,
                });
            }
            if c == '!' {
                return Err(self.err("孤立的 '!'（期望 != ）"));
            }
            return Ok(if c == '<' { Tok::Lt } else { Tok::Gt });
        }

        if c == '=' {
            // = 或 ==
            self.bump();
            if self.peek() == Some('=') {
                self.bump();
                return Ok(Tok::Eq);
            }
            return Ok(Tok::Assign);
        }

        // 数字字面量（含浮点）
        if c.is_ascii_digit() {
            return self.lex_number();
        }

        // 字符串字面量
        if c == '"' {
            return self.lex_string();
        }

        // 标识符 / 保留标记字
        if c.is_alphabetic() || c == '_' {
            return self.lex_ident();
        }

        Err(self.err(format!("无法识别的字符 '{}'", c)))
    }

    fn lex_number(&mut self) -> Result<Tok, LexError> {
        let start = self.pos;
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.bump();
        }
        // 浮点：小数点后必须有数字（规范 2.4）
        if self.peek() == Some('.') {
            match self.peek2() {
                Some(n) if n.is_ascii_digit() => {
                    self.bump(); // .
                    while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                        self.bump();
                    }
                    let s: String = self.chars[start..self.pos].iter().collect();
                    // 数字后紧贴字母/下划线视为非法（如 1.5x），避免静默拆分
                    if matches!(self.peek(), Some(c) if c.is_alphabetic() || c == '_') {
                        return Err(self.err(format!("数字 '{}' 后不能直接跟标识符字符", s)));
                    }
                    let v = s
                        .parse::<f64>()
                        .map_err(|_| self.err(format!("非法浮点数 '{}'", s)))?;
                    return Ok(Tok::Float(v));
                }
                _ => {
                    let s: String = self.chars[start..self.pos + 1].iter().collect();
                    return Err(self.err(format!(
                        "非法数字 '{}'（浮点小数点后必须有数字，若为整数请去掉小数点）",
                        s
                    )));
                }
            }
        }
        // 数字后紧贴字母/下划线视为非法（如 1.5x、2天），避免静默拆分
        if matches!(self.peek(), Some(c) if c.is_alphabetic() || c == '_') {
            let s: String = self.chars[start..self.pos].iter().collect();
            return Err(self.err(format!("数字 '{}' 后不能直接跟标识符字符", s)));
        }
        let s: String = self.chars[start..self.pos].iter().collect();
        let v = s
            .parse::<i64>()
            .map_err(|_| self.err(format!("整数超出 i64 范围 '{}'", s)))?;
        Ok(Tok::Int(v))
    }

    fn lex_string(&mut self) -> Result<Tok, LexError> {
        self.bump(); // 吃掉开引号
        let mut s = String::new();
        loop {
            match self.bump() {
                None | Some('\n') => return Err(self.err("字符串未闭合（字符串不可跨行）")),
                Some('"') => return Ok(Tok::Str(s)),
                Some('\\') => match self.bump() {
                    Some('"') => s.push('"'),
                    Some('\\') => s.push('\\'),
                    Some('n') => s.push('\n'),
                    Some('t') => s.push('\t'),
                    Some(other) => return Err(self.err(format!("未知转义 '\\{}'", other))),
                    None => return Err(self.err("字符串未闭合")),
                },
                Some(c) => s.push(c),
            }
        }
    }

    fn lex_ident(&mut self) -> Result<Tok, LexError> {
        let start = self.pos;
        while matches!(self.peek(), Some(c) if c.is_alphanumeric() || c == '_') {
            self.bump();
        }
        let word: String = self.chars[start..self.pos].iter().collect();

        // 保留标记字（规范 2.2）：不在表达式内时 w/ i/ e/ f/ r/
        let div_ctx = ends_expr(self.prev.as_ref());
        if !div_ctx
            && matches!(word.as_str(), "w" | "i" | "f" | "r" | "e")
            && self.peek() == Some('/')
        {
            self.bump(); // 吃掉 /
            return Ok(match word.as_str() {
                "w" => Tok::While,
                "i" => Tok::If,
                "e" => Tok::Else,
                "r" => Tok::Return,
                _ => Tok::Fn,
            });
        }

        // bool 字面量
        if word == "true" {
            return Ok(Tok::Bool(true));
        }
        if word == "false" {
            return Ok(Tok::Bool(false));
        }

        // v3.0 保留字：struct（结构体声明，规范第 14 节）
        if word == "struct" {
            return Ok(Tok::Struct);
        }

        // v3.6 保留字：use（模块导入，规范第 17 节）
        if word == "use" {
            return Ok(Tok::Use);
        }

        // v3.8 保留字：break / continue（循环控制，规范第 19 节）
        if word == "break" {
            return Ok(Tok::Break);
        }
        if word == "continue" {
            return Ok(Tok::Continue);
        }

        // 转换/内建函数（标识符紧贴左括号时视为调用，此处仅记录名字）
        if word == "toi" || word == "tos" || word == "copy" || word == "len" || word == "sel" || word == "panic" || word == "check"
            || word == "push"
            || word == "pop"
            || word == "sub"
            || word == "tof"
            || word == "has"
            || word == "del"
            || word == "keys"
        {
            return Ok(Tok::ConvertFn(word));
        }

        Ok(Tok::Ident(word))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(src: &str) -> Vec<Tok> {
        // 统一补充行尾换行（模拟真实源文件的行结束符）
        lex(&format!("{}\n", src)).unwrap()
    }

    #[test]
    fn test_slash_decl_vs_division() {
        // 核心难点：/a 声明 vs a/2 除法
        assert_eq!(toks("/a=20"), vec![Tok::ReadOnlyDecl, Tok::Ident("a".into()), Tok::Assign, Tok::Int(20), Tok::Newline, Tok::Eof]);
        assert_eq!(toks("10/2"), vec![Tok::Int(10), Tok::Slash, Tok::Int(2), Tok::Newline, Tok::Eof]);
        assert_eq!(toks("a/2"), vec![Tok::Ident("a".into()), Tok::Slash, Tok::Int(2), Tok::Newline, Tok::Eof]);
        assert_eq!(toks("/ a"), vec![Tok::Slash, Tok::Ident("a".into()), Tok::Newline, Tok::Eof]); // 空格 → 除号
        // 规范 2.7 v0.1.1：表达式内 / 恒为除号（a /b、cnt/a 均按除法）
        assert_eq!(toks("a /b"), vec![Tok::Ident("a".into()), Tok::Slash, Tok::Ident("b".into()), Tok::Newline, Tok::Eof]);
        assert_eq!(toks("cnt/a"), vec![Tok::Ident("cnt".into()), Tok::Slash, Tok::Ident("a".into()), Tok::Newline, Tok::Eof]);
        // 表达式结束后换行，下一行的 /a 仍是声明
        assert_eq!(toks("10/2\n/a=1"), vec![Tok::Int(10), Tok::Slash, Tok::Int(2), Tok::Newline, Tok::ReadOnlyDecl, Tok::Ident("a".into()), Tok::Assign, Tok::Int(1), Tok::Newline, Tok::Eof]);
    }

    #[test]
    fn test_mutable_decl() {
        assert_eq!(toks("//cnt=1"), vec![Tok::MutableDecl, Tok::Ident("cnt".into()), Tok::Assign, Tok::Int(1), Tok::Newline, Tok::Eof]);
        // // 后不是标识符 → 除法除法
        assert_eq!(toks("8//2"), vec![Tok::Int(8), Tok::Slash, Tok::Slash, Tok::Int(2), Tok::Newline, Tok::Eof]);
    }

    #[test]
    fn test_flow_markers() {
        assert_eq!(toks("w/cnt<=3{"), vec![Tok::While, Tok::Ident("cnt".into()), Tok::Le, Tok::Int(3), Tok::LBrace, Tok::Newline, Tok::Eof]);
        assert_eq!(toks("i/x==1{"), vec![Tok::If, Tok::Ident("x".into()), Tok::Eq, Tok::Int(1), Tok::LBrace, Tok::Newline, Tok::Eof]);
        assert_eq!(toks("f/add(x,y){"), vec![Tok::Fn, Tok::Ident("add".into()), Tok::LParen, Tok::Ident("x".into()), Tok::Comma, Tok::Ident("y".into()), Tok::RParen, Tok::LBrace, Tok::Newline, Tok::Eof]);
        // w 单独作为变量名合法
        assert_eq!(toks("/w=1"), vec![Tok::ReadOnlyDecl, Tok::Ident("w".into()), Tok::Assign, Tok::Int(1), Tok::Newline, Tok::Eof]);
        // v0.2：r/ e/ 标记
        assert_eq!(toks("r/x"), vec![Tok::Return, Tok::Ident("x".into()), Tok::Newline, Tok::Eof]);
        assert_eq!(toks("}e/{"), vec![Tok::RBrace, Tok::Else, Tok::LBrace, Tok::Newline, Tok::Eof]);
        // 表达式内标记字不生效：a e/b 中 / 仍是除号（a e 相邻本身语法非法，由 parser 拦截）
        assert_eq!(toks("a e/b"), vec![Tok::Ident("a".into()), Tok::Ident("e".into()), Tok::Slash, Tok::Ident("b".into()), Tok::Newline, Tok::Eof]);
    }

    #[test]
    fn test_print_and_type_annotation() {
        assert_eq!(toks("`a"), vec![Tok::Backtick, Tok::Ident("a".into()), Tok::Newline, Tok::Eof]);
        assert_eq!(toks("/num:i32=10"), vec![Tok::ReadOnlyDecl, Tok::Ident("num".into()), Tok::Colon, Tok::Ident("i32".into()), Tok::Assign, Tok::Int(10), Tok::Newline, Tok::Eof]);
    }

    #[test]
    fn test_literals() {
        assert_eq!(toks("3.14"), vec![Tok::Float(3.14), Tok::Newline, Tok::Eof]);
        assert_eq!(toks("\"你好\\n\""), vec![Tok::Str("你好\n".into()), Tok::Newline, Tok::Eof]);
        assert_eq!(toks("true false"), vec![Tok::Bool(true), Tok::Bool(false), Tok::Newline, Tok::Eof]);
        assert_eq!(toks("toi(\"5\")"), vec![Tok::ConvertFn("toi".into()), Tok::LParen, Tok::Str("5".into()), Tok::RParen, Tok::Newline, Tok::Eof]);
    }

    #[test]
    fn test_comment() {
        // 注释不吞换行符：换行仍是语句结束符，因此 Newline 在最前
        assert_eq!(toks("# 这是注释\n/a=1"), vec![Tok::Newline, Tok::ReadOnlyDecl, Tok::Ident("a".into()), Tok::Assign, Tok::Int(1), Tok::Newline, Tok::Eof]);
    }

    #[test]
    fn test_errors() {
        assert!(lex("\"未闭合").is_err());
        assert!(lex("1.5x").is_err()); // 数字后直接跟标识符
        assert_eq!(lex("1.5x").unwrap_err().line, 1);
        assert!(lex("3./2").is_err()); // 小数点后无数字
    }

    /// 集成测试：规范第 9 节定稿样例 main.t 全文分词
    #[test]
    fn test_main_t_sample() {
        let src = "# main.t 天道示例\n/a=20\n`a/2\n//cnt=1\nw/cnt<=3{\n    cnt=cnt+1\n    `cnt/a\n}\nf/add(x,y){\n    /res=x+y\n    `res\n}\nadd(10,90)\n";
        let t = lex(src).unwrap();
        assert_eq!(t[0], Tok::Newline); // 注释行后
        assert_eq!(t[1], Tok::ReadOnlyDecl);
        // 包含中文注释与中文环境，只要不报错即通过
        assert_eq!(*t.last().unwrap(), Tok::Eof);
    }
}
