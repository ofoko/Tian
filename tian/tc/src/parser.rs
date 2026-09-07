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
        arrs: Vec::new(),
        tuples: Vec::new(),
        ret_stack: Vec::new(),
        uses: Vec::new(),
        last_type_borrow: false,
        files: vec!["<内存源码>".to_string()],
        visited: std::collections::HashSet::new(),
        saved: Vec::new(),
        maps: Vec::new(),
        // v4.7：预注册 keys() 结果的基础 darr 类型（[]str、[]i64；map 键仅限这两种，规范 25.4）
        darrs: Parser::seed_darrs(),
    }
    .parse_program()
}

/// v3.6：从源文件解析（处理 use 导入，规范第 17 节）。
/// use 名字 → 把 名字.t 的 token 流接入同一个 Parser：结构体表/数组表天然共享，
/// 跨文件类型索引无需重排；visited 集合防止循环导入与重复导入。
pub fn parse_file(path: &str) -> Result<Program, ParseError> {
    let src = std::fs::read_to_string(path)
        .map_err(|e| ParseError { msg: format!("无法读取源文件 {}：{}", path, e), line: 0 })?;
    let toks = crate::lexer::lex_spanned(&src).map_err(|e| ParseError { msg: e.msg, line: e.line })?;
    let canon = std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string());
    let mut p = Parser {
        toks,
        pos: 0,
        structs: Vec::new(),
        arrs: Vec::new(),
        tuples: Vec::new(),
        ret_stack: Vec::new(),
        uses: Vec::new(),
        last_type_borrow: false,
        files: vec![path.to_string()],
        visited: std::collections::HashSet::from([canon]),
        saved: Vec::new(),
        maps: Vec::new(),
        darrs: Parser::seed_darrs(),
    };
    p.parse_program()
}

struct Parser {
    toks: Vec<(Tok, usize)>,
    pos: usize,
    /// v3.0：已声明的结构体（顺序即 Ty::Struct 索引；必须先声明后使用）
    structs: Vec<StructDef>,
    /// v3.3：已出现的数组类型（按 (元素, 长度) 去重，顺序即 Ty::Arr 索引）
    arrs: Vec<ArrDef>,
    /// v4.0：动态数组类型（按元素去重，顺序即 Ty::DArr 索引）
    darrs: Vec<DArrDef>,
    /// v3.6：文件栈（use 导入时压栈；报错定位到具体文件）
    files: Vec<String>,
    /// v3.6：已导入文件的规范路径集合（防循环/重复导入）
    visited: std::collections::HashSet<String>,
    // v3.6：use 导入时保存的调用方 token 流（文件解析完毕后弹回）
    saved: Vec<(Vec<(Tok, usize)>, usize)>,
    // v3.9：use 导入的模块名（供 fmt 重建源码）
    uses: Vec<String>,
    // v4.3：parse_type 是否消费了 & 前缀（&[]T 借用标记，供 parse_fn 读取）
    last_type_borrow: bool,
    // v4.5：元组类型表（按元素列表去重）
    tuples: Vec<TupleDef>,
    // v4.5：当前函数返回类型栈（元组多值返回解析用）
    ret_stack: Vec<Ty>,
    // v4.7：关联数组类型表（按 (key,val) 去重）
    maps: Vec<MapDef>,
}

impl Parser {
    /// v4.7：keys()/values() 返回的基础 darr 类型预注册（键仅 []str、[]i64；值含 i32/i64/f64/bool/str），
    /// 确保键/值快照类型 id 恒有效（规范 25.4）。顺序固定：现存 []str=[0]、[]i64=[1] 不位移。
    fn seed_darrs() -> Vec<DArrDef> {
        vec![
            DArrDef { elem: Ty::Str },
            DArrDef { elem: Ty::I64 },
            DArrDef { elem: Ty::I32 },
            DArrDef { elem: Ty::F64 },
            DArrDef { elem: Ty::Bool },
        ]
    }

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
        // v3.6：非入口文件的错误带文件名前缀（报错定位到具体 use 文件）
        let msg = msg.into();
        let msg = if self.files.len() > 1 {
            format!("[{}] {}", self.files.last().unwrap(), msg)
        } else {
            msg
        };
        ParseError {
            msg,
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
        loop {
            match self.peek() {
                Tok::Eof => {
                    // v3.6：use 导入的文件结束，弹回调用方继续解析
                    if self.files.len() > 1 {
                        self.files.pop();
                        let (toks, pos) = self.saved.pop().expect("saved 流为空（内部错误）");
                        self.toks = toks;
                        self.pos = pos;
                    } else {
                        break;
                    }
                }
                Tok::Fn => prog.funcs.push(self.parse_fn()?),
                // v3.0：顶层结构体声明（规范第 14 节）
                Tok::Struct => prog.structs.push(self.parse_struct()?),
                // v3.6：use 导入（规范第 17 节）
                Tok::Use => self.parse_use()?,
                _ => prog.top.push(self.parse_stmt()?),
            }
            self.skip_newlines();
        }
        prog.arrs = std::mem::take(&mut self.arrs);
        prog.darrs = std::mem::take(&mut self.darrs);
        prog.tuples = std::mem::take(&mut self.tuples);
        prog.maps = std::mem::take(&mut self.maps);
        prog.uses = std::mem::take(&mut self.uses);
        Ok(prog)
    }

    /// v3.6：use 名字 —— 把 名字.t 接入当前解析流（规范第 17 节）。
    /// 循环导入与重复导入按已访问集合跳过；被导入文件内可继续 use。
    fn parse_use(&mut self) -> Result<(), ParseError> {
        self.bump(); // use
        let name = self.expect_ident()?;
        let cur = self.files.last().unwrap().clone();
        let dir = std::path::Path::new(&cur)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default();
        let path = dir.join(format!("{}.t", name));
        let canon = std::fs::canonicalize(&path)
            .map(|p| p.to_string_lossy().to_string())
            .map_err(|_| self.err(format!("use 找不到文件 '{}'（期望 {}）", name, path.display())))?;
        if !self.visited.insert(canon) {
            return Ok(()); // 已导入过（含循环导入），跳过
        }
        self.uses.push(name.clone());
        let src = std::fs::read_to_string(&path)
            .map_err(|e| self.err(format!("无法读取导入文件 {}：{}", path.display(), e)))?;
        let toks = crate::lexer::lex_spanned(&src)
            .map_err(|e| ParseError { msg: format!("[{}] {}", path.display(), e.msg), line: e.line })?;
        self.saved.push((std::mem::take(&mut self.toks), self.pos));
        self.toks = toks;
        self.pos = 0;
        self.files.push(path.display().to_string());
        Ok(())
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
        let sd = StructDef { name, fields, line, imported: self.files.len() > 1 };
        self.structs.push(sd.clone());
        Ok(sd)
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

    /// v3.3：数组字面量 {e1, e2, ...}（元素个数与类型由类型检查器校验）
    fn parse_arr_lit(&mut self, arr: u32, line: usize) -> Result<Expr, ParseError> {
        self.expect(&Tok::LBrace)?;
        let mut elems = Vec::new();
        self.skip_newlines();
        while *self.peek() != Tok::RBrace {
            if *self.peek() == Tok::Eof {
                return Err(self.err("数组字面量未闭合（缺少 }）"));
            }
            elems.push(self.parse_expr()?);
            if *self.peek() == Tok::Comma {
                self.bump();
            }
            self.skip_newlines();
        }
        self.bump(); // }
        let _ = line;
        Ok(Expr::ArrLit { arr, elems })
    }

    fn parse_fn(&mut self) -> Result<FnDef, ParseError> {
        self.bump(); // f/
        let name = self.expect_ident()?;
        self.expect(&Tok::LParen)?;
        let mut params = Vec::new();
        if *self.peek() != Tok::RParen {
            loop {
                // v4.3：&[]T 借用形参——& 可写在形参名前或类型标注前（规范 22.8）
                let mut is_borrow = *self.peek() == Tok::Amp;
                if is_borrow {
                    self.bump();
                }
                self.last_type_borrow = false;
                let name = self.expect_ident()?;
                // 参数类型标注可省略（规范 5.3）
                let ty = if *self.peek() == Tok::Colon {
                    self.bump();
                    Some(self.parse_type()?)
                } else {
                    None
                };
                if self.last_type_borrow {
                    is_borrow = true;
                }
                params.push(Param { name, ty, borrow: is_borrow });
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
        self.ret_stack.push(ret.unwrap_or(Ty::I64));
        self.expect(&Tok::LBrace)?;
        // v2.2 语义锚点：函数体首部的 @pre/@post/@example 行（规范第 12 节）
        let contracts = self.parse_contracts()?;
        let body = self.parse_block()?;
        self.ret_stack.pop();
        Ok(FnDef { name, params, ret, body, contracts, imported: self.files.len() > 1 })
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
                    // v4.6：函数返回元组时，@example 期望值允许 (e1, e2) 元组字面量
                    // 逐元素结构比较（规范 24.1 例外；仅返回边界，不引入元素访问）
                    let expected = if let Some(Ty::Tuple(tid)) = self.ret_stack.last() {
                        self.parse_tuple_literal_for_contract(*tid, line)?
                    } else {
                        self.parse_expr()?
                    };
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

    /// v4.6：解析 @example 期望值的元组字面量 (e1, e2, ...)（规范 24.1 例外）。
    /// 元素个数必须与返回元组类型一致；tuple id 直接取返回类型的 id，保证与 ret 类型一致。
    fn parse_tuple_literal_for_contract(
        &mut self,
        tid: u32,
        line: usize,
    ) -> Result<Expr, ParseError> {
        self.expect(&Tok::LParen)?;
        let n = self
            .tuples
            .get(tid as usize)
            .map(|t| t.elems.len())
            .unwrap_or(0);
        let mut elems = Vec::new();
        self.skip_newlines();
        while elems.len() < n {
            if *self.peek() == Tok::Eof {
                return Err(self.err(format!(
                    "元组 @example 期望 {} 个元素，实际 {} 个",
                    n,
                    elems.len()
                )));
            }
            if elems.len() > 0 {
                self.expect(&Tok::Comma)?;
                self.skip_newlines();
            }
            elems.push(self.parse_expr()?);
            self.skip_newlines();
        }
        if *self.peek() == Tok::Comma {
            return Err(self.err(format!(
                "元组 @example 期望 {} 个元素，实际更多",
                n
            )));
        }
        self.expect(&Tok::RParen)?;
        let _ = line;
        Ok(Expr::TupExpr { elems, tup: tid })
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
            // v3.8：break / continue（合法性由类型检查器按循环上下文校验）
            Tok::Break => {
                self.bump();
                self.expect_newline()?;
                Ok(Stmt::Break(start_line))
            }
            Tok::Continue => {
                self.bump();
                self.expect_newline()?;
                Ok(Stmt::Continue(start_line))
            }
            // v4.0：panic(msg) / check(cond, msg) 快速失败语句（规范第 21 节）
            Tok::ConvertFn(ref n) if n == "panic" || n == "check" || n == "push" || n == "pop" || n == "del" => {
                self.bump();
                self.expect(&Tok::LParen)?;
                let first = self.parse_expr()?;
                if n == "panic" {
                    self.expect(&Tok::RParen)?;
                    self.expect_newline()?;
                    Ok(Stmt::Panic(Box::new(first), start_line))
                } else if n == "pop" {
                    // v4.1：pop(arr) 移除末元素（规范第 22 节）
                    self.expect(&Tok::RParen)?;
                    self.expect_newline()?;
                    Ok(Stmt::Expr(
                        Expr::Pop { arr: Box::new(first) },
                        start_line,
                    ))
                } else if n == "del" {
                    // v4.7：del(m, key) 删除键（规范第 25 节）
                    self.expect(&Tok::Comma)?;
                    let second = self.parse_expr()?;
                    self.expect(&Tok::RParen)?;
                    self.expect_newline()?;
                    Ok(Stmt::Expr(
                        Expr::Del {
                            map: Box::new(first),
                            key: Box::new(second),
                        },
                        start_line,
                    ))
                } else {
                    self.expect(&Tok::Comma)?;
                    let second = self.parse_expr()?;
                    self.expect(&Tok::RParen)?;
                    self.expect_newline()?;
                    if n == "check" {
                        // v4.0：check(cond, msg)（规范第 21 节）
                        Ok(Stmt::Check(Box::new(first), Box::new(second), start_line))
                    } else {
                        // v4.0：push(arr, v) 语句（规范第 22 节）
                        Ok(Stmt::Expr(
                            Expr::Push {
                                arr: Box::new(first),
                                value: Box::new(second),
                            },
                            start_line,
                        ))
                    }
                }
            }
            Tok::ReadOnlyDecl | Tok::MutableDecl => {
                let mutable = *self.peek() == Tok::MutableDecl;
                self.bump();
                let name = self.expect_ident()?;
                // v4.5：//a, b = f(...) 多声明解构（规范第 24 节；无逐名类型标注）
                if *self.peek() == Tok::Comma {
                    let mut names = vec![name];
                    while *self.peek() == Tok::Comma {
                        self.bump();
                        names.push(self.expect_ident()?);
                    }
                    self.expect(&Tok::Assign)?;
                    let value = self.parse_expr()?;
                    self.expect_newline()?;
                    return Ok(Stmt::MultiDecl {
                        mutable,
                        names,
                        value,
                        line: start_line,
                    });
                }
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
                // r/表达式 返回表达式值；r/e1, e2 多值返回（v4.5）；r/ 裸返回 0（规范 5.3）
                if matches!(self.peek(), Tok::Newline | Tok::Eof) {
                    self.expect_newline()?;
                    Ok(Stmt::Return(None, start_line))
                } else {
                    // v4.5：元组返回类型 → 按元素个数解析 r/e1, e2（规范第 24 节）
                    let tup = match self.ret_stack.last() {
                        Some(Ty::Tuple(t)) => Some(*t),
                        _ => None,
                    };
                    let e = self.parse_expr()?;
                    if *self.peek() == Tok::Comma {
                        let tid = tup.ok_or_else(|| {
                            self.err("多值返回要求函数返回类型是元组 (T1, T2, ...)")
                        })?;
                        let want = self.tuples[tid as usize].elems.len();
                        let mut elems = vec![e];
                        while *self.peek() == Tok::Comma {
                            self.bump();
                            if elems.len() >= want {
                                return Err(self.err(format!(
                                    "元组返回期望 {} 个元素，实际更多",
                                    want
                                )));
                            }
                            elems.push(self.parse_expr()?);
                        }
                        if elems.len() != want {
                            return Err(self.err(format!(
                                "元组返回期望 {} 个元素，实际 {} 个",
                                want,
                                elems.len()
                            )));
                        }
                        self.expect_newline()?;
                        return Ok(Stmt::Return(
                            Some(Expr::TupExpr { elems, tup: tid }),
                            start_line,
                        ));
                    }
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

    /// v4.5：按元素类型列表取元组类型 id（去重）
    fn tuple_id(&mut self, elems: Vec<Ty>) -> Ty {
        if let Some(i) = self.tuples.iter().position(|t| t.elems == elems) {
            return Ty::Tuple(i as u32);
        }
        self.tuples.push(TupleDef { elems: elems.clone() });
        Ty::Tuple((self.tuples.len() - 1) as u32)
    }

    /// 类型标注：i32 / i64 / f64 / str / bool / &str（v2.1 借用）/ []T（v4.0）/ (T, T)（v4.5 元组）
    fn parse_type(&mut self) -> Result<Ty, ParseError> {
        // v4.5：(T1, T2, ...) 元组类型（仅返回位置，规范第 24 节）
        if *self.peek() == Tok::LParen {
            self.bump();
            let mut elems = Vec::new();
            while *self.peek() != Tok::RParen {
                if *self.peek() == Tok::Eof {
                    return Err(self.err("元组类型未闭合（缺少 ））"));
                }
                let t = self.parse_type()?;
                if t.is_tuple() {
                    return Err(self.err("元组元素不能是元组（不支持嵌套）"));
                }
                elems.push(t);
                if *self.peek() == Tok::Comma {
                    self.bump();
                }
            }
            self.expect(&Tok::RParen)?;
            if elems.len() < 2 || elems.len() > 4 {
                return Err(self.err(format!(
                    "元组元素个数须为 2–4 个（实际 {} 个）",
                    elems.len()
                )));
            }
            return Ok(self.tuple_id(elems));
        }
        // v4.7：map[K]V 关联数组类型（规范第 25 节）
        if let Tok::Ident(name) = self.peek() {
            if name == "map" {
                return self.parse_map_type();
            }
        }
        // v4.0：[]T 动态数组类型（规范第 22 节）
        if *self.peek() == Tok::LBracket {
            self.bump(); // [
            self.expect(&Tok::RBracket)?;
            let name = self.expect_ident()?;
            let elem = match name.as_str() {
                "i32" => Ty::I32,
                "i64" => Ty::I64,
                "f64" => Ty::F64,
                "bool" => Ty::Bool,
                // v4.2：str 元素（push 移交所有权，容器深释放，规范 22.6）
                "str" => Ty::Str,
                other => {
                    return Err(self.err(format!(
                        "动态数组元素类型须为 i32/i64/f64/bool/str（实际 '{}'）",
                        other
                    )))
                }
            };
            return Ok(self.darr_id(elem));
        }
        // &str 借用类型（规范 11.6）；&[]T 借用动态数组（规范 22.8）
        if *self.peek() == Tok::Amp {
            self.bump();
            // v4.3：&[]T —— 记录借用标志并返回 DArr
            if *self.peek() == Tok::LBracket {
                self.bump();
                self.expect(&Tok::RBracket)?;
                let name = self.expect_ident()?;
                let elem = match name.as_str() {
                    "i32" => Ty::I32,
                    "i64" => Ty::I64,
                    "f64" => Ty::F64,
                    "bool" => Ty::Bool,
                    "str" => Ty::Str,
                    other => {
                        return Err(self.err(format!(
                            "动态数组元素类型须为 i32/i64/f64/bool/str（实际 '{}'）",
                            other
                        )))
                    }
                };
                self.last_type_borrow = true;
                return Ok(self.darr_id(elem));
            }
            // v4.7：&map[K]V —— 只读借用关联数组（规范 25.6）
            if let Tok::Ident(n) = self.peek() {
                if n == "map" {
                    self.last_type_borrow = true;
                    return self.parse_map_type();
                }
            }
            let name = self.expect_ident()?;
            return match name.as_str() {
                "str" => Ok(Ty::BorrowStr),
                other => Err(self.err(format!("&只能修饰 str（实际 &{}）", other))),
            };
        }
        let name = self.expect_ident()?;
        match name.as_str() {
            "i32" => self.maybe_arr_suffix(Ty::I32),
            "i64" => self.maybe_arr_suffix(Ty::I64),
            "f64" => self.maybe_arr_suffix(Ty::F64),
            "bool" => self.maybe_arr_suffix(Ty::Bool),
            "str" => {
                // v3.3：str 数组不支持（str 参与天权，数组值语义会破坏所有权模型）
                if *self.peek() == Tok::LBracket {
                    return Err(self.err("str 不能声明为数组（v3.3 数组仅支持 i32/i64/f64/bool 元素）"));
                }
                Ok(Ty::Str)
            }
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

    /// v4.0：按元素类型取动态数组类型 id（去重）
    fn darr_id(&mut self, elem: Ty) -> Ty {
        if let Some(i) = self.darrs.iter().position(|d| d.elem == elem) {
            return Ty::DArr(i as u32);
        }
        self.darrs.push(DArrDef { elem });
        Ty::DArr((self.darrs.len() - 1) as u32)
    }

    /// v4.7：按 (key,val) 取关联数组类型 id（去重，规范第 25 节）
    fn map_id(&mut self, key: Ty, val: Ty) -> Ty {
        if let Some(i) = self.maps.iter().position(|m| m.key == key && m.val == val) {
            return Ty::Map(i as u32);
        }
        self.maps.push(MapDef { key, val });
        Ty::Map((self.maps.len() - 1) as u32)
    }

    /// v4.7：map[K]V 关联数组类型（K ∈ {i64,str}；V ∈ {i32,i64,f64,bool,str}）
    fn parse_map_type(&mut self) -> Result<Ty, ParseError> {
        self.expect_ident()?; // map
        self.expect(&Tok::LBracket)?;
        let key_name = self.expect_ident()?;
        let key = match key_name.as_str() {
            "i64" => Ty::I64,
            "str" => Ty::Str,
            other => {
                return Err(self.err(format!(
                    "map 键类型须为 i64 或 str（实际 '{}'）",
                    other
                )))
            }
        };
        self.expect(&Tok::RBracket)?;
        let val_name = self.expect_ident()?;
        let val = match val_name.as_str() {
            "i32" => Ty::I32,
            "i64" => Ty::I64,
            "f64" => Ty::F64,
            "bool" => Ty::Bool,
            "str" => Ty::Str,
            other => {
                return Err(self.err(format!(
                    "map 值类型须为 i32/i64/f64/bool/str（实际 '{}'）",
                    other
                )))
            }
        };
        Ok(self.map_id(key, val))
    }

    /// v3.3：类型后缀 [N] → 固定长度数组（仅值类型元素；str/结构体数组不支持）
    fn maybe_arr_suffix(&mut self, elem: Ty) -> Result<Ty, ParseError> {
        if *self.peek() != Tok::LBracket {
            return Ok(elem);
        }
        let line = self.line();
        self.bump(); // [
        let len = match self.peek().clone() {
            Tok::Int(n) if n > 0 => n as u64,
            Tok::Int(0) => return Err(self.err("数组长度必须为正整数")),
            Tok::Int(_) => return Err(self.err("数组长度必须为非负整数字面量")),
            t => return Err(self.err(format!("期望数组长度（正整数字面量），实际 '{}'", t))),
        };
        self.bump();
        self.expect(&Tok::RBracket)?;
        if let Some(i) = self.arrs.iter().position(|a| a.elem == elem && a.len == len) {
            return Ok(Ty::Arr(i as u32));
        }
        self.arrs.push(ArrDef { elem, len });
        let _ = line;
        Ok(Ty::Arr((self.arrs.len() - 1) as u32))
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
            } else if *self.peek() == Tok::LBracket {
                // v3.3：数组下标 a[i]（读）
                self.bump();
                let idx = self.parse_expr()?;
                self.expect(&Tok::RBracket)?;
                e = Expr::Index(Box::new(e), Box::new(idx));
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
                // v4.7：map[K]V{ k1: v1, ... } 关联数组字面量（规范第 25 节）。
                // 此处尚未 bump map 关键字，parse_map_type 会消费之。
                if name == "map" && self.peek2() == Some(&Tok::LBracket) {
                    let ty = self.parse_map_type()?;
                    let map = match ty {
                        Ty::Map(i) => i,
                        _ => unreachable!(),
                    };
                    let mut entries = Vec::new();
                    self.expect(&Tok::LBrace)?;
                    self.skip_newlines();
                    while *self.peek() != Tok::RBrace {
                        if *self.peek() == Tok::Eof {
                            return Err(self.err("map 字面量未闭合（缺少 }）"));
                        }
                        let k = self.parse_expr()?;
                        self.expect(&Tok::Colon)?;
                        let v = self.parse_expr()?;
                        entries.push((k, v));
                        if *self.peek() == Tok::Comma {
                            self.bump();
                        }
                        self.skip_newlines();
                    }
                    self.bump(); // }
                    return Ok(Expr::MapLit { map, entries });
                }
                self.bump();
                // v3.0：Point{...} 结构体字面量（调用与字段访问交给 parse_postfix）
                // 仅当标识符是已声明的结构体名时才解析为结构体字面量；
                // 否则按普通变量处理，{ 留给 while/if 等语句块解析器消费
                if *self.peek() == Tok::LBrace
                    && self.structs.iter().any(|s| s.name == name)
                {
                    return self.parse_struct_lit(name, line);
                }
                // v3.3：i64[3]{1,2,3} 数组字面量（仅四种值类型名 + [N] + {元素}）
                if *self.peek() == Tok::LBracket
                    && matches!(name.as_str(), "i32" | "i64" | "f64" | "bool")
                {
                    let elem = match name.as_str() {
                        "i32" => Ty::I32,
                        "i64" => Ty::I64,
                        "f64" => Ty::F64,
                        _ => Ty::Bool,
                    };
                    let arr = match self.maybe_arr_suffix(elem)? {
                        Ty::Arr(i) => i,
                        _ => unreachable!(),
                    };
                    return self.parse_arr_lit(arr, line);
                }
                // str 数组不支持（str 参与天权，规范 16.5）
                if *self.peek() == Tok::LBracket && name == "str" {
                    return Err(self.err("str 不能声明为数组（v3.3 数组仅支持 i32/i64/f64/bool 元素）"));
                }
                Ok(Expr::Var(name))
            }
            Tok::ConvertFn(name) => {
                self.bump();
                // v4.0：push(a, v) 追加元素（语句级，规范第 22 节）
                if name == "push" {
                    self.expect(&Tok::LParen)?;
                    let arr = self.parse_expr()?;
                    self.expect(&Tok::Comma)?;
                    let value = self.parse_expr()?;
                    self.expect(&Tok::RParen)?;
                    return Ok(Expr::Push {
                        arr: Box::new(arr),
                        value: Box::new(value),
                    });
                }
                // v4.7：has(m, k) 键存在性（表达式，规范第 25 节）
                if name == "has" {
                    self.expect(&Tok::LParen)?;
                    let map = self.parse_expr()?;
                    self.expect(&Tok::Comma)?;
                    let key = self.parse_expr()?;
                    self.expect(&Tok::RParen)?;
                    return Ok(Expr::Has {
                        map: Box::new(map),
                        key: Box::new(key),
                    });
                }
                // v4.2：sub(s, start, n) 子串（规范第 23 节）
                if name == "sub" {
                    self.expect(&Tok::LParen)?;
                    let s = self.parse_expr()?;
                    self.expect(&Tok::Comma)?;
                    let start = self.parse_expr()?;
                    self.expect(&Tok::Comma)?;
                    let n = self.parse_expr()?;
                    self.expect(&Tok::RParen)?;
                    return Ok(Expr::Sub {
                        s: Box::new(s),
                        start: Box::new(start),
                        n: Box::new(n),
                    });
                }
                // v3.7：sel(条件, 值1, 值2) 三参数条件表达式（规范第 18 节）
                if name == "sel" {
                    self.expect(&Tok::LParen)?;
                    let cond = self.parse_expr()?;
                    self.expect(&Tok::Comma)?;
                    let a = self.parse_expr()?;
                    self.expect(&Tok::Comma)?;
                    let b = self.parse_expr()?;
                    self.expect(&Tok::RParen)?;
                    return Ok(Expr::Sel {
                        cond: Box::new(cond),
                        a: Box::new(a),
                        b: Box::new(b),
                    });
                }
                self.expect(&Tok::LParen)?;
                let arg = self.parse_expr()?;
                self.expect(&Tok::RParen)?;
                if name == "len" {
                    // v3.3：len(a) 数组/字符串长度
                    return Ok(Expr::Len(Box::new(arg)));
                }
                // v4.7：keys(m) map 键快照 → []K（规范 25.4）
                if name == "keys" {
                    return Ok(Expr::Keys(Box::new(arg)));
                }
                // v4.7：values(m) map 值快照 → []V（规范 25.4）
                if name == "values" {
                    return Ok(Expr::Values(Box::new(arg)));
                }
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
            // v4.0：[]i64{1,2} 动态数组字面量（规范第 22 节）
            Tok::LBracket => {
                let line = self.line();
                self.bump(); // [
                self.expect(&Tok::RBracket)?;
                let name = self.expect_ident()?;
                let elem = match name.as_str() {
                    "i32" => Ty::I32,
                    "i64" => Ty::I64,
                    "f64" => Ty::F64,
                    "bool" => Ty::Bool,
                    // v4.2：str 元素
                    "str" => Ty::Str,
                    other => {
                        return Err(self.err(format!(
                            "动态数组元素类型须为 i32/i64/f64/bool/str（实际 '{}'）",
                            other
                        )))
                    }
                };
                let darr = match self.darr_id(elem) {
                    Ty::DArr(i) => i,
                    _ => unreachable!(),
                };
                self.expect(&Tok::LBrace)?;
                let mut elems = Vec::new();
                self.skip_newlines();
                while *self.peek() != Tok::RBrace {
                    if *self.peek() == Tok::Eof {
                        return Err(self.err("动态数组字面量未闭合（缺少 }）"));
                    }
                    elems.push(self.parse_expr()?);
                    if *self.peek() == Tok::Comma {
                        self.bump();
                    }
                    self.skip_newlines();
                }
                self.bump(); // }
                let _ = line;
                Ok(Expr::DArrLit { darr, elems })
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
