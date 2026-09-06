//! 语法解析器：Token 流 → AST（手写递归下降）

use crate::ast::*;
use crate::lexer::Tok;

#[derive(Debug, Clone)]
pub struct ParseError {
    pub msg: String,
    pub line: usize,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "语法错误（第 {} 行）：{}", self.line, self.msg)
    }
}

pub fn parse(toks: Vec<(Tok, usize)>) -> Result<Program, ParseError> {
    Parser {
        toks,
        pos: 0,
        structs: Vec::new(),
    }
    .parse_program()
}

struct Parser {
    toks: Vec<(Tok, usize)>,
    pos: usize,
    /// v3.0：已声明的结构体（顺序即 Ty::Struct 索引；必须先声明后使用）
    structs: Vec<StructDef>,
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos].0
    }

    fn peek2(&self) -> Option<&Tok> {
        self.toks.get(self.pos + 1).map(|t| &t.0)
    }

    fn line(&self) -> usize {
        self.toks[self.pos.min(self.toks.len() - 1)].1
    }

    fn bump(&mut self) -> Tok {
        let t = self.toks[self.pos].0.clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn err(&self, msg: impl Into<String>) -> ParseError {
        ParseError {
            msg: msg.into(),
            line: self.line(),
        }
    }

    fn expect(&mut self, want: &Tok) -> Result<(), ParseError> {
        if self.peek() == want {
            self.bump();
            Ok(())
        } else {
            Err(self.err(format!("期望 '{}'，实际 '{}'", want, self.peek())))
        }
    }

    fn expect_ident(&mut self) -> Result<String, ParseError> {
        match self.peek().clone() {
            Tok::Ident(s) => {
                self.bump();
                Ok(s)
            }
            t => Err(self.err(format!("期望标识符，实际 '{}'", t))),
        }
    }

    fn skip_newlines(&mut self) {
        while *self.peek() == Tok::Newline {
            self.bump();
        }
    }

    fn expect_newline(&mut self) -> Result<(), ParseError> {
        match self.peek() {
            // 块内最后一条语句可省略换行，直接以 } 结束（如 i/x>5{ `100 }）
            Tok::Newline | Tok::RBrace | Tok::Eof => {
                if *self.peek() == Tok::Newline {
                    self.bump();
                }
                Ok(())
            }
            t => Err(self.err(format!("语句应以换行结束，实际 '{}'", t))),
        }
    }

    fn parse_program(&mut self) -> Result<Program, ParseError> {
        let mut prog = Program::default();
        self.skip_newlines();
        while *self.peek() != Tok::Eof {
            match self.peek() {
                Tok::Fn => prog.funcs.push(self.parse_fn()?),
                // v3.0：顶层结构体声明（规范第 14 节）
                Tok::Struct => prog.structs.push(self.parse_struct()?),
                _ => prog.top.push(self.parse_stmt()?),
            }
            self.skip_newlines();
        }
        Ok(prog)
    }

    /// v3.0：struct Point{ x:i64  y:str }（规范第 14 节）
    fn parse_struct(&mut self) -> Result<StructDef, ParseError> {
        let line = self.line();
        self.bump(); // struct
        let name = self.expect_ident()?;
        if self.structs.iter().any(|s| s.name == name) {
            return Err(self.err(format!("结构体 '{}' 重复定义", name)));
        }
        self.expect(&Tok::LBrace)?;
        let mut fields = Vec::new();
        self.skip_newlines();
        while *self.peek() != Tok::RBrace {
            if *self.peek() == Tok::Eof {
                return Err(self.err("结构体未闭合（缺少 }）"));
            }
            let fline = self.line();
            let fname = self.expect_ident()?;
            self.expect(&Tok::Colon)?;
            let fty = self.parse_type()?;
            if fty.is_struct() {
                // 结构体按值内嵌在 v3.0 不支持（会让深释放与拷贝语义复杂化）
                return Err(self.err(format!(
                    "字段 '{}' 不能是结构体类型（v3.0 仅支持 i32/i64/f64/str/bool 字段）",
                    fname
                )));
            }
            if fields.iter().any(|f: &FieldDef| f.name == fname) {
                return Err(self.err(format!("结构体 '{}' 的字段 '{}' 重复", name, fname)));
            }
            fields.push(FieldDef {
                name: fname,
                ty: fty,
                line: fline,
            });
            if *self.peek() == Tok::Comma {
                self.bump();
            }
            self.skip_newlines();
        }
        self.bump(); // }
        if *self.peek() == Tok::Newline {
            self.bump();
        }
        Ok(StructDef { name, fields, line })
    }

    /// v3.0：结构体字面量 Point{1,2} / Point{x:1,y:2}（规范 14.2）
    fn parse_struct_lit(&mut self, name: String, line: usize) -> Result<Expr, ParseError> {
        self.expect(&Tok::LBrace)?;
        let mut fields: Vec<(Option<String>, Expr)> = Vec::new();
        self.skip_newlines();
        while *self.peek() != Tok::RBrace {
            if *self.peek() == Tok::Eof {
                return Err(self.err("结构体字面量未闭合（缺少 }）"));
            }
            // 命名式：ident 后紧跟 :
            let fname = if matches!(self.peek(), Tok::Ident(_)) && matches!(self.peek2(), Some(Tok::Colon))
            {
                let n = self.expect_ident()?;
                self.bump(); // :
                Some(n)
            } else {
                None
            };
            let v = self.parse_expr()?;
            fields.push((fname, v));
            if *self.peek() == Tok::Comma {
                self.bump();
            }
            self.skip_newlines();
        }
        self.bump(); // }
        Ok(Expr::StructLit { name, fields, line })
    }

    fn parse_fn(&mut self) -> Result<FnDef, ParseError> {
        self.bump(); // f/
        let name = self.expect_ident()?;
        self.expect(&Tok::LParen)?;
        let mut params = Vec::new();
        if *self.peek() != Tok::RParen {
            loop {
                let name = self.expect_ident()?;
                // 参数类型标注可省略（规范 5.3）
                let ty = if *self.peek() == Tok::Colon {
                    self.bump();
                    Some(self.parse_type()?)
                } else {
                    None
                };
                params.push(Param { name, ty });
                if *self.peek() == Tok::Comma {
                    self.bump();
                } else {
                    break;
                }
            }
        }
        self.expect(&Tok::RParen)?;
        // 返回类型标注可省略（规范 5.3）
        let ret = if *self.peek() == Tok::Colon {
            self.bump();
            Some(self.parse_type()?)
        } else {
            None
        };
        self.expect(&Tok::LBrace)?;
        // v2.2 语义锚点：函数体首部的 @pre/@post/@example 行（规范第 12 节）
        let contracts = self.parse_contracts()?;
        let body = self.parse_block()?;
        Ok(FnDef { name, params, ret, body, contracts })
    }

    /// v2.2：解析函数体首部的语义锚点行（规范第 12 节）
    fn parse_contracts(&mut self) -> Result<Vec<Contract>, ParseError> {
        let mut contracts = Vec::new();
        self.skip_newlines();
        while *self.peek() == Tok::At {
            self.bump(); // @
            let line = self.line();
            let word = self.expect_ident()?;
            self.expect(&Tok::Colon)?;
            match word.as_str() {
                "pre" => {
                    let e = self.parse_expr()?;
                    self.expect_newline()?;
                    contracts.push(Contract::Pre(e, line));
                }
                "post" => {
                    let e = self.parse_expr()?;
                    self.expect_newline()?;
                    contracts.push(Contract::Post(e, line));
                }
                "example" => {
                    let call = self.parse_expr()?;
                    self.expect(&Tok::Arrow)?;
                    let expected = self.parse_expr()?;
                    self.expect_newline()?;
                    contracts.push(Contract::Example { call, expected, line });
                }
                other => {
                    return Err(self.err(format!(
                        "未知锚点 '@{}'（可用：@pre @post @example）",
                        other
                    )))
                }
            }
            self.skip_newlines();
        }
        Ok(contracts)
    }

    fn parse_block(&mut self) -> Result<Vec<Stmt>, ParseError> {
        self.skip_newlines();
        let mut body = Vec::new();
        while *self.peek() != Tok::RBrace {
            if *self.peek() == Tok::Eof {
                return Err(self.err("块未闭合（缺少 }）"));
            }
            body.push(self.parse_stmt()?);
            self.skip_newlines();
        }
        self.bump(); // }
        // 块后的换行可选吞掉
        if *self.peek() == Tok::Newline {
            self.bump();
        }
        Ok(body)
    }

    fn parse_stmt(&mut self) -> Result<Stmt, ParseError> {
        // 行号在语句开始处捕获（解析后 pos 已越过换行符）
        let start_line = self.line();
        match self.peek().clone() {
            Tok::ReadOnlyDecl | Tok::MutableDecl => {
                let mutable = *self.peek() == Tok::MutableDecl;
                self.bump();
                let name = self.expect_ident()?;
                let ty = if *self.peek() == Tok::Colon {
                    self.bump();
                    Some(self.parse_type()?)
                } else {
                    None
                };
                self.expect(&Tok::Assign)?;
                let value = self.parse_expr()?;
                self.expect_newline()?;
                Ok(Stmt::Decl {
                    mutable,
                    name,
                    ty,
                    value,
                    line: start_line,
                })
            }
            Tok::While => {
                self.bump();
                let cond = self.parse_expr()?;
                self.expect(&Tok::LBrace)?;
                let body = self.parse_block()?;
                Ok(Stmt::While { cond, body, line: start_line })
            }
            Tok::If => {
                self.bump();
                let cond = self.parse_expr()?;
                self.expect(&Tok::LBrace)?;
                let body = self.parse_block()?;
                // e/ 可选，紧跟 i/ 块（规范 5.2）
                let else_body = if *self.peek() == Tok::Else {
                    self.bump();
                    self.expect(&Tok::LBrace)?;
                    Some(self.parse_block()?)
                } else {
                    None
                };
                Ok(Stmt::If { cond, body, else_body, line: start_line })
            }
            Tok::Return => {
                self.bump();
                // r/表达式 返回表达式值；r/ 裸返回 0（规范 5.3）
                if matches!(self.peek(), Tok::Newline | Tok::Eof) {
                    self.expect_newline()?;
                    Ok(Stmt::Return(None, start_line))
                } else {
                    let e = self.parse_expr()?;
                    self.expect_newline()?;
                    Ok(Stmt::Return(Some(e), start_line))
                }
            }
            Tok::Backtick => {
                self.bump();
                let e = self.parse_expr()?;
                self.expect_newline()?;
                Ok(Stmt::Print(e, start_line))
            }
            Tok::Ident(_) => {
                // v3.0：先解析左侧（可为 x 或 p.x），遇 = 即赋值，否则为表达式语句
                let target = self.parse_expr()?;
                if *self.peek() == Tok::Assign {
                    self.bump();
                    let value = self.parse_expr()?;
                    self.expect_newline()?;
                    Ok(Stmt::Assign {
                        target,
                        value,
                        line: start_line,
                    })
                } else {
                    self.expect_newline()?;
                    Ok(Stmt::Expr(target, start_line))
                }
            }
            t => Err(self.err(format!("无法识别的语句开头 '{}'", t))),
        }
    }

    /// 类型标注：i32 / i64 / f64 / str / bool / &str（v2.1 借用）
    fn parse_type(&mut self) -> Result<Ty, ParseError> {
        // &str 借用类型（规范 11.6）
        if *self.peek() == Tok::Amp {
            self.bump();
            let name = self.expect_ident()?;
            return match name.as_str() {
                "str" => Ok(Ty::BorrowStr),
                other => Err(self.err(format!("&只能修饰 str（实际 &{}）", other))),
            };
        }
        let name = self.expect_ident()?;
        match name.as_str() {
            "i32" => Ok(Ty::I32),
            "i64" => Ok(Ty::I64),
            "f64" => Ok(Ty::F64),
            "str" => Ok(Ty::Str),
            "bool" => Ok(Ty::Bool),
            _ => {
                // v3.0：已声明的结构体可作为类型（必须先声明后使用）
                if let Some(i) = self.structs.iter().position(|s| s.name == name) {
                    return Ok(Ty::Struct(i as u32));
                }
                Err(self.err(format!(
                    "未知类型 '{}'（可用：i32 i64 f64 str bool &str 或已声明的结构体名）",
                    name
                )))
            }
        }
    }

    /// v3.0：初等表达式 + 后缀链（调用、字段访问），如 f(1).x.y
    fn parse_postfix(&mut self) -> Result<Expr, ParseError> {
        let mut e = self.parse_primary()?;
        loop {
            if *self.peek() == Tok::LParen {
                // 仅 Var 后可调用：f(...)
                let callee = match e {
                    Expr::Var(name) => name,
                    other => {
                        return Err(self.err(format!("只有函数名可直接调用（实际 {:?}）", other)))
                    }
                };
                self.bump();
                let mut args = Vec::new();
                if *self.peek() != Tok::RParen {
                    loop {
                        args.push(self.parse_expr()?);
                        if *self.peek() == Tok::Comma {
                            self.bump();
                        } else {
                            break;
                        }
                    }
                }
                self.expect(&Tok::RParen)?;
                e = Expr::Call { name: callee, args };
            } else if *self.peek() == Tok::Dot {
                self.bump();
                let f = self.expect_ident()?;
                e = Expr::Field(Box::new(e), f);
            } else {
                break;
            }
        }
        Ok(e)
    }

    // 表达式：比较 < 加减 < 乘除 < 一元负号 < 初等
    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_cmp()
    }

    fn parse_cmp(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_add()?;
        loop {
            let op = match self.peek() {
                Tok::Le => BinOp::Le,
                Tok::Ge => BinOp::Ge,
                Tok::Eq => BinOp::Eq,
                Tok::Ne => BinOp::Ne,
                Tok::Lt => BinOp::Lt,
                Tok::Gt => BinOp::Gt,
                _ => break,
            };
            self.bump();
            let rhs = self.parse_add()?;
            lhs = Expr::Bin {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn parse_add(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_mul()?;
        loop {
            let op = match self.peek() {
                Tok::Plus => BinOp::Add,
                Tok::Minus => BinOp::Sub,
                _ => break,
            };
            self.bump();
            let rhs = self.parse_mul()?;
            lhs = Expr::Bin {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn parse_mul(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Tok::Star => BinOp::Mul,
                Tok::Slash => BinOp::Div,
                _ => break,
            };
            self.bump();
            let rhs = self.parse_unary()?;
            lhs = Expr::Bin {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        if *self.peek() == Tok::Minus {
            self.bump();
            let e = self.parse_unary()?;
            return Ok(Expr::Neg(Box::new(e)));
        }
        // v2.1 借用：&s（规范 11.6；合法性由类型检查器约束）
        if *self.peek() == Tok::Amp {
            self.bump();
            let e = self.parse_unary()?;
            return Ok(Expr::Borrow(Box::new(e)));
        }
        // v3.0：后缀链（调用/字段访问）优先级高于一元符号之后的一切
        self.parse_postfix()
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        match self.peek().clone() {
            Tok::Int(v) => {
                self.bump();
                Ok(Expr::Int(v))
            }
            Tok::Float(v) => {
                self.bump();
                Ok(Expr::Float(v))
            }
            Tok::Str(s) => {
                self.bump();
                Ok(Expr::Str(s))
            }
            Tok::Bool(b) => {
                self.bump();
                Ok(Expr::Bool(b))
            }
            Tok::Ident(name) => {
                let line = self.line();
                self.bump();
                // v3.0：Point{...} 结构体字面量（调用与字段访问交给 parse_postfix）
                if *self.peek() == Tok::LBrace {
                    if !self.structs.iter().any(|s| s.name == name) {
                        return Err(self.err(format!("未知结构体 '{}'（需先声明）", name)));
                    }
                    return self.parse_struct_lit(name, line);
                }
                Ok(Expr::Var(name))
            }
            Tok::ConvertFn(name) => {
                self.bump();
                self.expect(&Tok::LParen)?;
                let arg = self.parse_expr()?;
                self.expect(&Tok::RParen)?;
                Ok(Expr::Convert {
                    name,
                    arg: Box::new(arg),
                })
            }
            Tok::LParen => {
                self.bump();
                let e = self.parse_expr()?;
                self.expect(&Tok::RParen)?;
                Ok(e)
            }
            t => Err(self.err(format!("期望表达式，实际 '{}'", t))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex_spanned;

    fn parse_src(src: &str) -> Program {
        parse(lex_spanned(src).unwrap()).unwrap()
    }

    #[test]
    fn test_main_t_program() {
        let src = "# main.t 天道示例\n/a=20\n`a/2\n//cnt=1\nw/cnt<=3{\n    cnt=cnt+1\n    `cnt/a\n}\nf/add(x,y){\n    /res=x+y\n    `res\n}\nadd(10,90)\n";
        let prog = parse_src(src);
        assert_eq!(prog.top.len(), 5); // 声明a、打印、声明cnt、while、调用add
        assert_eq!(prog.funcs.len(), 1);
        assert_eq!(prog.funcs[0].name, "add");
        assert_eq!(prog.funcs[0].params[0].name, "x");
        assert_eq!(prog.funcs[0].params[1].name, "y");
        assert!(prog.funcs[0].params.iter().all(|p| p.ty.is_none()));
        assert_eq!(prog.funcs[0].ret, None);
        // add(10,90) 必须解析为函数调用语句
        match &prog.top[4] {
            Stmt::Expr(Expr::Call { name, args }, _) => {
                assert_eq!(name, "add");
                assert_eq!(args.len(), 2);
            }
            other => panic!("期望函数调用语句，实际 {:?}", other),
        }
    }

    #[test]
    fn test_division_in_expr() {
        let prog = parse_src("`cnt/a\n");
        match &prog.top[0] {
            Stmt::Print(Expr::Bin { op, .. }, _) => assert_eq!(*op, BinOp::Div),
            other => panic!("期望除法表达式，实际 {:?}", other),
        }
    }

    #[test]
    fn test_type_annotation_and_precedence() {
        let prog = parse_src("/num:i32=10\n");
        match &prog.top[0] {
            Stmt::Decl { ty: Some(Ty::I32), value: Expr::Int(10), .. } => {}
            other => panic!("期望 i32 声明，实际 {:?}", other),
        }
        // 优先级：1+2*3 → Add(1, Mul(2,3))
        let prog = parse_src("/x=1+2*3\n");
        match &prog.top[0] {
            Stmt::Decl { value: Expr::Bin { op: BinOp::Add, rhs, .. }, .. } => {
                assert!(matches!(rhs.as_ref(), Expr::Bin { op: BinOp::Mul, .. }));
            }
            other => panic!("期望加法，实际 {:?}", other),
        }
    }

    #[test]
    fn test_parse_errors() {
        // 只读声明缺少初始化
        assert!(parse(lex_spanned("/a\n").unwrap()).is_err());
        // 赋值无换行结尾
        assert!(parse(lex_spanned("//a=1 //b=2\n").unwrap()).is_err());
        // 未闭合块
        assert!(parse(lex_spanned("w/1<2{\n/a=1\n").unwrap()).is_err());
    }

    #[test]
    fn test_annotated_fn() {
        // v0.3：参数与返回类型标注
        let prog = parse_src("f/join(a:str, b:str):str{\nr/a+b\n}\n");
        let f = &prog.funcs[0];
        assert_eq!(f.params[0].ty, Some(Ty::Str));
        assert_eq!(f.params[1].ty, Some(Ty::Str));
        assert_eq!(f.ret, Some(Ty::Str));
        // 混合标注与省略
        let prog = parse_src("f/mix(x:f64, n):f64{\nr/x*n\n}\n");
        let f = &prog.funcs[0];
        assert_eq!(f.params[0].ty, Some(Ty::F64));
        assert_eq!(f.params[1].ty, None);
        assert_eq!(f.ret, Some(Ty::F64));
    }

    #[test]
    fn test_return_and_else() {
        let src = "f/f(x){\nr/x\n}\n/sum=add(3,4)\n`sum\ni/sum>5{\n`1\n}e/{\n`0\n}\n".replace("add", "f");
        let prog = parse_src(&src);
        // 函数体含 r/x
        match &prog.funcs[0].body[0] {
            Stmt::Return(Some(Expr::Var(v)), _) => assert_eq!(v, "x"),
            other => panic!("期望返回语句，实际 {:?}", other),
        }
        // i/ 带 e/ 块
        match &prog.top[2] {
            Stmt::If { else_body: Some(b), .. } => assert_eq!(b.len(), 1),
            other => panic!("期望带 else 的 if，实际 {:?}", other),
        }
        // 裸 r/ → None
        let prog = parse_src("f/g(){\nr/\n}\n");
        match &prog.funcs[0].body[0] {
            Stmt::Return(None, _) => {}
            other => panic!("期望裸返回，实际 {:?}", other),
        }
    }
}
