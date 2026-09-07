//! v3.9 源码格式化器（tc fmt，规范第 20 节）
//!
//! AST → 规范化源码：统一缩进（4 空格）、运算符两侧空格、
//! 顶层顺序规范化为 use → struct → fn → 顶层语句。
//! 验收标准：**幂等**——fmt(fmt(x)) == fmt(x)，且输出可再次编译。

use crate::ast::*;

const IND: &str = "    ";

/// 优先级：cmp(1) < add(2) < mul(3) < 一元(4) < 初等(5)
fn prec(e: &Expr) -> u8 {
    match e {
        Expr::Bin { op, .. } => match op {
            BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge | BinOp::Eq | BinOp::Ne => 1,
            BinOp::Add | BinOp::Sub => 2,
            BinOp::Mul | BinOp::Div => 3,
        },
        Expr::Neg(_) => 4,
        _ => 5,
    }
}

fn bin_op_str(op: BinOp) -> &'static str {
    op.c_str()
}

fn fmt_expr(e: &Expr, out: &mut String) {
    match e {
        Expr::Int(v) => out.push_str(&v.to_string()),
        Expr::Float(v) => {
            let s = v.to_string();
            out.push_str(&s);
            if !s.contains('.') && !s.contains('e') && !s.contains("inf") && !s.contains("nan") {
                out.push_str(".0");
            }
        }
        Expr::Str(s) => out.push_str(&c_str_lit(s)),
        Expr::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Expr::Var(n) => out.push_str(n),
        // v4.5：元组仅存在于返回边界，故 TupExpr 恒为 r/ 的实参，按逗号分隔输出（无括号，规范第 24 节）
        Expr::TupExpr { elems, .. } => {
            for (i, e) in elems.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                fmt_expr(e, out);
            }
        }
        Expr::Neg(x) => {
            out.push('-');
            if prec(x) < 5 {
                out.push('(');
                fmt_expr(x, out);
                out.push(')');
            } else {
                fmt_expr(x, out);
            }
        }
        Expr::Bin { op, lhs, rhs } => {
            let p = prec(e);
            if prec(lhs) < p {
                out.push('(');
                fmt_expr(lhs, out);
                out.push(')');
            } else {
                fmt_expr(lhs, out);
            }
            out.push(' ');
            out.push_str(bin_op_str(*op));
            out.push(' ');
            if prec(rhs) < p {
                out.push('(');
                fmt_expr(rhs, out);
                out.push(')');
            } else {
                fmt_expr(rhs, out);
            }
        }
        Expr::Call { name, args } => {
            out.push_str(name);
            out.push('(');
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                fmt_expr(a, out);
            }
            out.push(')');
        }
        Expr::Convert { name, arg } => {
            out.push_str(name);
            out.push('(');
            fmt_expr(arg, out);
            out.push(')');
        }
        Expr::Borrow(inner) => {
            out.push('&');
            fmt_expr(inner, out);
        }
        Expr::StructLit { name, fields, .. } => {
            out.push_str(name);
            out.push('{');
            for (i, (fname, v)) in fields.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                if let Some(n) = fname {
                    out.push_str(n);
                    out.push(':');
                }
                fmt_expr(v, out);
            }
            out.push('}');
        }
        Expr::Field(base, fname) => {
            fmt_expr(base, out);
            out.push('.');
            out.push_str(fname);
        }
        Expr::Index(base, idx) => {
            fmt_expr(base, out);
            out.push('[');
            fmt_expr(idx, out);
            out.push(']');
        }
        Expr::Len(inner) => {
            out.push_str("len(");
            fmt_expr(inner, out);
            out.push(')');
        }
        Expr::ArrLit { arr, elems } => {
            let table = crate::type_check::arrs();
            if let Some(a) = table.get(*arr as usize) {
                out.push_str(&format!("{}[{}]", a.elem.label(), a.len));
            }
            out.push('{');
            for (i, e) in elems.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                fmt_expr(e, out);
            }
            out.push('}');
        }
        Expr::Sel { cond, a, b } => {
            out.push_str("sel(");
            fmt_expr(cond, out);
            out.push_str(", ");
            fmt_expr(a, out);
            out.push_str(", ");
            fmt_expr(b, out);
            out.push(')');
        }
        // v4.0：动态数组字面量 []i64{...}（规范第 22 节）
        Expr::DArrLit { darr, elems } => {
            let table = crate::type_check::darrs();
            if let Some(d) = table.get(*darr as usize) {
                out.push_str(&format!("[]{}", d.elem.label()));
            }
            out.push('{');
            for (i, e) in elems.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                fmt_expr(e, out);
            }
            out.push('}');
        }
        Expr::Push { arr, value } => {
            out.push_str("push(");
            fmt_expr(arr, out);
            out.push_str(", ");
            fmt_expr(value, out);
            out.push(')');
        }
        Expr::Pop { arr } => {
            out.push_str("pop(");
            fmt_expr(arr, out);
            out.push(')');
        }
        // v4.2：sub(s, start, n)（规范第 23 节）
        Expr::Sub { s, start, n } => {
            out.push_str("sub(");
            fmt_expr(s, out);
            out.push_str(", ");
            fmt_expr(start, out);
            out.push_str(", ");
            fmt_expr(n, out);
            out.push(')');
        }
        // v4.7：map 字面量 map[K]V{ k1: v1, ... }（可空 {}；规范第 25 节）
        Expr::MapLit { map, entries } => {
            let minfo = crate::type_check::maps();
            if let Some(m) = minfo.get(*map as usize) {
                out.push_str(&format!("map[{}]{}", m.key.label(), m.val.label()));
            } else {
                out.push_str("map");
            }
            out.push('{');
            for (i, (k, v)) in entries.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                fmt_expr(k, out);
                out.push_str(": ");
                fmt_expr(v, out);
            }
            out.push('}');
        }
        // v4.7：has(m, k) → bool（表达式）
        Expr::Has { map, key } => {
            out.push_str("has(");
            fmt_expr(map, out);
            out.push_str(", ");
            fmt_expr(key, out);
            out.push(')');
        }
        // v4.7：keys(m) 键快照
        Expr::Keys(map) => {
            out.push_str("keys(");
            fmt_expr(map, out);
            out.push(')');
        }
        // v4.7：values(m) 值快照
        Expr::Values(map) => {
            out.push_str("values(");
            fmt_expr(map, out);
            out.push(')');
        }
        // v4.8：sort(a) 原地排序
        Expr::Sort(arr) => {
            out.push_str("sort(");
            fmt_expr(arr, out);
            out.push(')');
        }
        // v4.8：cat(a, sep) 连接动态数组为 str
        Expr::Cat { arr, sep } => {
            out.push_str("cat(");
            fmt_expr(arr, out);
            out.push_str(", ");
            fmt_expr(sep, out);
            out.push(')');
        }
        // v4.7：del(m, k)（语句级）
        Expr::Del { map, key } => {
            out.push_str("del(");
            fmt_expr(map, out);
            out.push_str(", ");
            fmt_expr(key, out);
            out.push(')');
        }
    }
}

/// 字符串字面量重建（转义 " \ \n \t，与 lexer 支持集一致）
fn c_str_lit(s: &str) -> String {
    let mut r = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => r.push_str("\\\""),
            '\\' => r.push_str("\\\\"),
            '\n' => r.push_str("\\n"),
            '\t' => r.push_str("\\t"),
            c => r.push(c),
        }
    }
    r.push('"');
    r
}

fn fmt_stmt(s: &Stmt, out: &mut String, level: usize) {
    // 先格式化语句本体，再按层级缩进每一行
    let mut body = String::new();
    fmt_stmt_inner(s, &mut body);
    let pad = IND.repeat(level);
    for (i, line) in body.lines().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&pad);
        out.push_str(line);
    }
    out.push('\n');
}

fn fmt_stmt_inner(s: &Stmt, out: &mut String) {
    match s {
        Stmt::Decl { mutable, name, ty, value, .. } => {
            out.push_str(if *mutable { "//" } else { "/" });
            out.push_str(name);
            if let Some(t) = ty {
                out.push(':');
                out.push_str(&ty_str(*t));
            }
            out.push('=');
            fmt_expr(value, out);
        }
        Stmt::Assign { target, value, .. } => {
            fmt_expr(target, out);
            out.push('=');
            fmt_expr(value, out);
        }
        Stmt::Print(e, _) => {
            out.push('`');
            fmt_expr(e, out);
        }
        Stmt::While { cond, body, .. } => {
            out.push_str("w/");
            fmt_expr(cond, out);
            out.push_str("{\n");
            for st in body {
                fmt_stmt(st, out, 1);
            }
            out.push('}');
        }
        Stmt::If { cond, body, else_body, .. } => {
            out.push_str("i/");
            fmt_expr(cond, out);
            out.push_str("{\n");
            for st in body {
                fmt_stmt(st, out, 1);
            }
            out.push('}');
            if let Some(eb) = else_body {
                out.push_str("e/{\n");
                for st in eb {
                    fmt_stmt(st, out, 1);
                }
                out.push('}');
            }
        }
        Stmt::Return(e, _) => {
            out.push_str("r/");
            if let Some(e) = e {
                fmt_expr(e, out);
            }
        }
        Stmt::Break(_) => out.push_str("break"),
        Stmt::Continue(_) => out.push_str("continue"),
        // v4.0：panic / check（规范第 21 节）
        Stmt::Panic(msg, _) => {
            out.push_str("panic(");
            fmt_expr(msg, out);
            out.push(')');
        }
        Stmt::Check(cond, msg, _) => {
            out.push_str("check(");
            fmt_expr(cond, out);
            out.push_str(", ");
            fmt_expr(msg, out);
            out.push(')');
        }
        Stmt::Expr(e, _) => {
            fmt_expr(e, out);
        }
        // v4.5：多声明解构 /a, b = f(...)（规范第 24 节）
        Stmt::MultiDecl { mutable, names, value, .. } => {
            out.push_str(if *mutable { "//" } else { "/" });
            for (i, n) in names.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(n);
            }
            out.push('=');
            fmt_expr(value, out);
        }
    }
}

/// v3.3/v3.0 类型名：i64[3]、Point（fmt 需要数组/结构体表，format 入口已设置）
fn ty_str(t: Ty) -> String {
    match t {
        Ty::Arr(i) => {
            let a = crate::type_check::arrs();
            match a.get(i as usize) {
                Some(a) => format!("{}[{}]", a.elem.label(), a.len),
                None => "array".into(),
            }
        }
        Ty::Struct(i) => {
            let st = crate::type_check::structs();
            match st.get(i as usize) {
                Some(sd) => sd.name.clone(),
                None => "struct".into(),
            }
        }
        Ty::DArr(i) => {
            let dt = crate::type_check::darrs();
            match dt.get(i as usize) {
                Some(d) => format!("[]{}", d.elem.label()),
                None => "darr".into(),
            }
        }
        // v4.5：元组类型还原为 (e1, e2, ...)（规范第 24 节）
        Ty::Tuple(i) => {
            let tt = crate::type_check::tuples();
            match tt.get(i as usize) {
                Some(td) => {
                    let mut s = String::from("(");
                    for (j, e) in td.elems.iter().enumerate() {
                        if j > 0 {
                            s.push_str(", ");
                        }
                        s.push_str(&ty_str(*e));
                    }
                    s.push(')');
                    s
                }
                None => "tuple".into(),
            }
        }
        // v4.7：map 类型还原为 map[K]V（规范第 25 节）
        Ty::Map(i) => {
            let mt = crate::type_check::maps();
            match mt.get(i as usize) {
                Some(m) => format!("map[{}]{}", m.key.label(), m.val.label()),
                None => "map".into(),
            }
        }
        other => other.label().to_string(),
    }
}

/// 格式化整个程序（规范第 20 节：顶层顺序规范化为 use → struct → fn → 顶层语句）
pub fn format(prog: &Program) -> String {
    // 数组/结构体表供 ty_str 使用
    crate::type_check::set_arrs(&prog.arrs);
    crate::type_check::set_darrs(&prog.darrs);
    crate::type_check::set_structs(&prog.structs);
    crate::type_check::set_maps(&prog.maps);
    let mut out = String::new();
    for u in &prog.uses {
        out.push_str(&format!("use {}\n", u));
    }
    if !prog.uses.is_empty() {
        out.push('\n');
    }
    for sd in prog.structs.iter().filter(|s| !s.imported) {
        out.push_str(&format!("struct {}{{", sd.name));
        for (i, f) in sd.fields.iter().enumerate() {
            if i > 0 {
                out.push_str("  ");
            }
            out.push_str(&format!("{}:{}", f.name, ty_str(f.ty)));
        }
        out.push_str("}\n");
    }
    if !prog.structs.is_empty() {
        out.push('\n');
    }
    for f in prog.funcs.iter().filter(|f| !f.imported) {
        out.push_str(&format!("f/{}(", f.name));
        for (i, p) in f.params.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            if p.borrow {
                out.push('&');
            }
            out.push_str(&p.name);
            if let Some(t) = p.ty {
                out.push(':');
                out.push_str(&ty_str(t));
            }
        }
        out.push(')');
        if let Some(r) = f.ret {
            out.push(':');
            out.push_str(&ty_str(r));
        }
        out.push_str("{\n");
        for c in &f.contracts {
            match c {
                Contract::Pre(e, _) => {
                    out.push_str("@pre: ");
                    fmt_expr(e, &mut out);
                    out.push('\n');
                }
                Contract::Post(e, _) => {
                    out.push_str("@post: ");
                    fmt_expr(e, &mut out);
                    out.push('\n');
                }
                Contract::Example { call, expected, .. } => {
                    out.push_str("@example: ");
                    fmt_expr(call, &mut out);
                    out.push_str(" -> ");
                    // v4.6：元组返回的 @example 期望值 (e1, e2) 必须带括号才能重新解析
                    if matches!(expected, Expr::TupExpr { .. }) {
                        out.push('(');
                        fmt_expr(expected, &mut out);
                        out.push(')');
                    } else {
                        fmt_expr(expected, &mut out);
                    }
                    out.push('\n');
                }
            }
        }
        for s in &f.body {
            fmt_stmt(s, &mut out, 1);
        }
        out.push_str("}\n");
    }
    if !prog.funcs.is_empty() && !prog.top.is_empty() {
        out.push('\n');
    }
    for s in &prog.top {
        fmt_stmt(s, &mut out, 0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex_spanned;
    use crate::parser;

    fn roundtrip(src: &str) -> String {
        let toks = lex_spanned(src).expect("词法失败");
        let prog = parser::parse(toks).expect("解析失败");
        format(&prog)
    }

    #[test]
    fn test_fmt_idempotent() {
        let src = "//a=20\n`a/2\n//cnt=1\nw/cnt<=3{\n    cnt=cnt+1\n    `cnt/a\n}\n";
        let once = roundtrip(src);
        // 幂等：第二次 fmt 与第一次完全一致
        let twice = roundtrip(&once);
        assert_eq!(once, twice, "fmt 必须幂等");
        assert!(once.contains("`a / 2"));
    }

    #[test]
    fn test_fmt_reorders_and_compiles() {
        // 函数在顶层语句之后使用 → fmt 规范化顺序后仍可解析
        let src = "`add(1,2)\nf/add(x:i64,y:i64):i64{\n    r/x+y\n}\n";
        let out = roundtrip(src);
        assert!(out.find("f/add").unwrap() < out.find("`add").unwrap());
    }

    #[test]
    fn test_fmt_literals() {
        let src = "//s=\"天\\n\" \n`\"引\\\"号\"\n`3.50\n`true\n";
        let out = roundtrip(src);
        assert!(out.contains("\"天\\n\""));
        assert!(out.contains("\"引\\\"号\""));
        assert!(out.contains("3.5"));
    }
}
