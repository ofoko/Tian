//! C 代码生成器：AST → 可编译 C 源码（第一阶段后端）
//!
//! 输出 .c 文件由 gcc/cc 编译；中文标识符做 mangle 处理保证 C 兼容。

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::ast::*;
use crate::type_check::{
    collect_str_decls, consumes_var, is_owned, norm, structs, ty_of, FuncSig, Scope, VarInfo,
};

/// v3.0：C 类型名（结构体为指针类型 `Point*`，与 str 同为"堆值句柄"）
fn c_ty(t: Ty) -> String {
    match t {
        Ty::Struct(i) => {
            let sd = structs()[i as usize].clone();
            format!("{}*", cname(&sd.name))
        }
        other => other.c_name().to_string(),
    }
}

/// v3.0：释放一个堆值槽（str 用 __t_free，结构体用其深释放函数）
fn free_slot(var_c: &str, t: Ty) -> String {
    match t {
        Ty::Struct(i) => {
            let sd = structs()[i as usize].clone();
            format!("{}({});", free_fn(&sd), var_c)
        }
        _ => format!("__t_free({});", var_c),
    }
}

/// v3.0：结构体深释放函数名
fn free_fn(sd: &StructDef) -> String {
    format!("__t_free_{}", cname(&sd.name))
}

/// v3.0：结构体构造函数名
fn new_fn(sd: &StructDef) -> String {
    format!("__t_new_{}", cname(&sd.name))
}

/// 生成完整 C 源码
pub fn generate(prog: &Program) -> String {
    let mut out = String::new();
    // v3.0：供 emit_* 内部查询结构体布局（线程局部表，见 type_check::set_structs）
    crate::type_check::set_structs(&prog.structs);

    // 头部与运行时辅助函数
    out.push_str("/* 由 tc（Tian Compiler v2.0，天权）生成 */\n");
    out.push_str("#include <stdio.h>\n#include <stdlib.h>\n#include <string.h>\n\n");
    out.push_str(
        "static char __t_sb[8][64]; static int __t_bi = 0;\n\
         static const char* __t_tos_ll(long long v){ char* b=__t_sb[__t_bi++&7]; snprintf(b,64,\"%lld\",v); return b; }\n\
         static const char* __t_tos_f(double v){ char* b=__t_sb[__t_bi++&7]; snprintf(b,64,\"%g\",v); return b; }\n\
         static const char* __t_tos_b(int v){ return v ? \"true\" : \"false\"; }\n\
         static char* __t_cat(const char* a, const char* b){ size_t la=strlen(a),lb=strlen(b); char* r=(char*)malloc(la+lb+1); memcpy(r,a,la); memcpy(r+la,b,lb+1); return r; }\n\
         static int __t_seq(const char* a, const char* b){ return strcmp(a,b)==0; }\n\
         static int __t_sne(const char* a, const char* b){ return strcmp(a,b)!=0; }\n\
         static char* __t_dup(const char* s){ size_t n=strlen(s?s:\"\")+1; char* r=(char*)malloc(n); memcpy(r,s?s:\"\",n); return r; }\n\
         static void __t_free(const char* p){ free((void*)p); }\n\n",
    );

    // v3.0：结构体类型定义 + 构造函数 + 深释放（规范第 14 节）
    for sd in &prog.structs {
        let mut fields_c = String::new();
        for f in &sd.fields {
            let _ = writeln!(fields_c, "    {} {};", c_ty(f.ty), cname(&f.name));
        }
        let _ = writeln!(out, "typedef struct {{\n{}}} {};\n", fields_c, cname(&sd.name));
        // 构造函数：str 字段由调用方传入**已拥有的**堆指针（字面量由调用方 __t_dup）
        let ctor_params = sd
            .fields
            .iter()
            .enumerate()
            .map(|(i, f)| format!("{} t{}", c_ty(f.ty), i))
            .collect::<Vec<_>>()
            .join(", ");
        let mut ctor_body = String::new();
        for (i, f) in sd.fields.iter().enumerate() {
            let _ = writeln!(ctor_body, "    p->{} = t{};", cname(&f.name), i);
        }
        let _ = writeln!(
            out,
            "static {S}* {new}({params}) {{ {S}* p = ({S}*)malloc(sizeof({S}));\n{body}    return p; }}\n",
            S = cname(&sd.name),
            new = new_fn(sd),
            params = ctor_params,
            body = ctor_body
        );
        // 深释放：先释放 str 字段（字段只借用、不可移出，故此处必然仍有效），再释放块
        let mut free_body = String::new();
        for f in &sd.fields {
            if f.ty == Ty::Str {
                let _ = writeln!(
                    free_body,
                    "    __t_free(p->{f}); p->{f} = NULL;",
                    f = cname(&f.name)
                );
            }
        }
        let _ = writeln!(
            out,
            "static void {free}({S}* p) {{ if (!p) return;\n{body}    free(p); }}\n",
            free = free_fn(sd),
            S = cname(&sd.name),
            body = free_body
        );
    }

    // 函数表（供 ty_of 校验函数调用表达式）
    let mut sigs: HashMap<String, FuncSig> = HashMap::new();
    for f in &prog.funcs {
        sigs.insert(
            f.name.clone(),
            FuncSig {
                params: f.params.iter().map(|p| p.ty.unwrap_or(Ty::I64)).collect(),
                ret: f.ret.unwrap_or(Ty::I64),
            },
        );
    }

    // 函数签名前向声明（规范 5.3）
    for f in &prog.funcs {
        let ret = f.ret.unwrap_or(Ty::I64);
        let params = f
            .params
            .iter()
            .map(|p| format!("{} {}", p.ty.unwrap_or(Ty::I64).c_name(), cname(&p.name)))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(
            out,
            "static {} {}({});",
            ret.c_name(),
            cname(&f.name),
            if params.is_empty() { "void".into() } else { params }
        );
    }
    out.push('\n');

    // v2.2：@post 出口契约辅助函数——每次返回前校验返回值（规范第 12 节）
    for f in &prog.funcs {
        let ret = f.ret.unwrap_or(Ty::I64);
        let post = f.contracts.iter().find_map(|c| match c {
            Contract::Post(e, line) => Some((e, *line)),
            _ => None,
        });
        if let Some((e, _)) = post {
            let mut ps: Scope = HashMap::new();
            ps.insert("ret".into(), VarInfo::new(ret, false));
            let mut x = String::new();
            emit_expr(e, &ps, &sigs, &mut x);
            let _ = writeln!(
                out,
                "static int __t_post_{}({} t_ret) {{ return ({}); }}",
                cname(&f.name),
                c_ty(ret),
                x
            );
        }
    }
    out.push('\n');

    // main：顶层语句按顺序执行（天权：str 变量提升声明 + 末尾释放）
    out.push_str("int main(void) {\n");
    let mut ty_scope: HashMap<String, Ty> = HashMap::new();
    let mut top_strs: Vec<String> = Vec::new();
    collect_str_decls(&prog.top, &mut ty_scope, &sigs, &prog.structs, &mut top_strs);
    let mut scope: Scope = HashMap::new();
    for n in &top_strs {
        let t = *ty_scope.get(n).unwrap_or(&Ty::Str);
        let _ = writeln!(out, "    {} {} = NULL;", c_ty(t), cname(n));
    }
    // v2.2：@example 自检样例——程序启动时运行，失败即非零退出（规范第 12 节）
    let empty_scope: Scope = HashMap::new();
    for f in &prog.funcs {
        let frt = sigs[&f.name].ret;
        for c in &f.contracts {
            if let Contract::Example { call, expected, line } = c {
                let mut call_s = String::new();
                let mut exp_s = String::new();
                emit_expr(call, &empty_scope, &sigs, &mut call_s);
                emit_expr(expected, &empty_scope, &sigs, &mut exp_s);
                let cond = if frt == Ty::Str {
                    format!("__t_seq({}, {})", call_s, exp_s)
                } else {
                    format!("({}) == ({})", call_s, exp_s)
                };
                let _ = writeln!(
                    out,
                    "    if (!{}) {{ fprintf(stderr, \"@example 契约失败：{}（第 {} 行）\\n\"); exit(1); }}",
                    cond,
                    f.name,
                    line
                );
            }
        }
    }
    for s in &prog.top {
        emit_stmt(s, &mut scope, &sigs, Ty::I64, None, &mut out, 1);
    }
    for n in &top_strs {
        let _ = writeln!(out, "    __t_free({}); {} = NULL;", cname(n), cname(n));
    }
    out.push_str("    return 0;\n}\n\n");

    // 函数定义（规范 5.3）
    for f in &prog.funcs {
        let ret = f.ret.unwrap_or(Ty::I64);
        let params = f
            .params
            .iter()
            .map(|p| format!("{} {}", p.ty.unwrap_or(Ty::I64).c_name(), cname(&p.name)))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(
            out,
            "static {} {}({}) {{",
            ret.c_name(),
            cname(&f.name),
            if params.is_empty() { "void".into() } else { params }
        );
        let mut scope: Scope = f
            .params
            .iter()
            .map(|p| {
                (
                    p.name.clone(),
                    VarInfo::new(p.ty.unwrap_or(Ty::I64), false),
                )
            })
            .collect();
        // v2.2：@post 出口契约（辅助函数名 + 锚点行号，供每次返回前校验）
        let post: Option<(String, usize)> = f.contracts.iter().find_map(|c| match c {
            Contract::Post(_, line) => Some((format!("__t_post_{}", cname(&f.name)), *line)),
            _ => None,
        });
        // 天权：收集函数内全部 str 声明，提升为 NULL 初始化的指针槽；
        // str 形参也是所有者，纳入函数出口释放清单
        let mut ty_scope: HashMap<String, Ty> = f
            .params
            .iter()
            .map(|p| (p.name.clone(), p.ty.unwrap_or(Ty::I64)))
            .collect();
        let mut str_names: Vec<String> = Vec::new();
        collect_str_decls(&f.body, &mut ty_scope, &sigs, &prog.structs, &mut str_names);
        let mut free_names = str_names.clone();
        free_names.extend(
            f.params
                .iter()
                .filter(|p| p.ty.map_or(false, is_owned))
                .map(|p| p.name.clone()),
        );
        for n in &str_names {
            let t = *ty_scope.get(n).unwrap_or(&Ty::Str);
            let _ = writeln!(out, "    {} {} = NULL;", c_ty(t), cname(n));
        }
        // v2.2：@pre 入口契约——假设不成立立即失败（规范第 12 节）
        for c in &f.contracts {
            if let Contract::Pre(e, line) = c {
                let mut x = String::new();
                emit_expr(e, &mut scope, &sigs, &mut x);
                let _ = writeln!(
                    out,
                    "    if (!({})) {{ fprintf(stderr, \"@pre 契约失败：{}（第 {} 行）\\n\"); exit(1); }}",
                    x,
                    f.name,
                    line
                );
            }
        }
        for s in &f.body {
            emit_stmt(s, &mut scope, &sigs, ret, post.as_ref(), &mut out, 1);
        }
        // 天权：落出路径释放全部堆值槽（str + 结构体，深释放；已移出的为 NULL，free 安全）
        for n in &free_names {
            let t = *ty_scope.get(n).unwrap_or(&Ty::Str);
            indent(&mut out, 1);
            let _ = writeln!(out, "    {} {} = NULL;", free_slot(&cname(n), t), cname(n));
        }
        // 函数末尾隐式 r/：返回该类型零值（规范 5.3；结构体返回 NULL 指针，可安全释放）
        let zero = match ret {
            Ty::I32 | Ty::I64 => "0",
            Ty::F64 => "0.0",
            Ty::Str => "__t_dup(\"\")",
            Ty::Bool => "0",
            Ty::Struct(_) => "NULL",
            Ty::BorrowStr => unreachable!("借用不可作为返回类型（检查器已拦截）"),
        };
        match &post {
            Some((helper, cline)) => {
                let _ = writeln!(
                    out,
                    "    {{ {} __t_tmp = {}; if (!{}(__t_tmp)) {{ fprintf(stderr, \"@post 契约失败：{}（第 {} 行）\\n\"); exit(1); }} return __t_tmp; }}\n}}\n",
                    c_ty(ret),
                    zero,
                    helper,
                    f.name,
                    cline
                );
            }
            None => {
                let _ = writeln!(out, "    return {};\n}}\n", zero);
            }
        }
    }

    out
}

/// 天权前置收集（规范 11.3）：见 type_check::collect_str_decls（两个后端共享）

/// 天权：绑定 str 值到变量——字面量/tos 结果需复制成独立堆内存（规范 11.1.1）
fn binds_dup(e: &Expr) -> bool {
    match e {
        Expr::Str(_) => true,
        Expr::Convert { name, .. } => name == "tos",
        _ => false,
    }
}

fn emit_bind(e: &Expr, scope: &Scope, sigs: &HashMap<String, FuncSig>, out: &mut String) {
    if binds_dup(e) {
        out.push_str("__t_dup(");
        emit_expr(e, scope, sigs, out);
        out.push(')');
    } else {
        emit_expr(e, scope, sigs, out);
    }
}

/// 天权：对 RHS 中被移动的 str 变量发射置空语句（规范 11.2.1）。
/// is_top 表示当前位置是"移动位"（str 绑定的顶层 / str 形参的实参）；
/// 其余位置（拼接、比较、tos/copy 参数、打印）只借用（规范 11.2.4）。
fn emit_move_nulls(
    e: &Expr,
    is_top: bool,
    scope: &Scope,
    sigs: &HashMap<String, FuncSig>,
    out: &mut String,
    level: usize,
) {
    match e {
        Expr::Var(name) => {
            // v3.0：str 与结构体变量在移动位都要置空（规范 14.4）
            if is_top && scope.get(name).map(|v| is_owned(v.ty)).unwrap_or(false) {
                indent(out, level);
                let _ = writeln!(out, "{} = NULL;", cname(name));
            }
        }
        Expr::Neg(x) => emit_move_nulls(x, false, scope, sigs, out, level),
        Expr::Bin { lhs, rhs, .. } => {
            emit_move_nulls(lhs, false, scope, sigs, out, level);
            emit_move_nulls(rhs, false, scope, sigs, out, level);
        }
        Expr::Call { name, args } => {
            if let Some(sig) = sigs.get(name) {
                for (a, pt) in args.iter().zip(&sig.params) {
                    if *pt == Ty::Str {
                        emit_move_nulls(a, true, scope, sigs, out, level);
                    }
                }
            }
        }
        // copy()/tos() 的参数只借用，但其参数内部的调用仍可能移动
        Expr::Convert { arg, .. } => emit_move_nulls(arg, false, scope, sigs, out, level),
        _ => {}
    }
}

/// 是否存在需要置空的移动（决定 r/ 是否走临时变量形式，保证置空先于返回）
fn has_move_nulls(e: &Expr, is_top: bool, sigs: &HashMap<String, FuncSig>) -> bool {
    match e {
        Expr::Var(_) => is_top,
        Expr::Neg(x) => has_move_nulls(x, false, sigs),
        Expr::Bin { lhs, rhs, .. } => {
            has_move_nulls(lhs, false, sigs) || has_move_nulls(rhs, false, sigs)
        }
        Expr::Call { name, args } => sigs
            .get(name)
            .map(|sig| {
                args.iter()
                    .zip(&sig.params)
                    .any(|(a, pt)| is_owned(*pt) && has_move_nulls(a, true, sigs))
            })
            .unwrap_or(false),
        Expr::Convert { arg, .. } => has_move_nulls(arg, false, sigs),
        _ => false,
    }
}

/// 标识符 mangle：非 ASCII 字符编码为 xNNNN，保证 C 编译器兼容
fn cname(s: &str) -> String {
    let mut r = String::from("t_");
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            r.push(c);
        } else {
            let _ = write!(r, "x{:x}", c as u32);
        }
    }
    r
}

/// C 字符串字面量转义
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

fn indent(out: &mut String, level: usize) {
    for _ in 0..level {
        out.push_str("    ");
    }
}

fn emit_stmt(
    s: &Stmt,
    scope: &mut Scope,
    sigs: &HashMap<String, FuncSig>,
    fn_ret: Ty,
    post: Option<&(String, usize)>,
    out: &mut String,
    level: usize,
) {
    match s {
        Stmt::Decl { name, ty, value, mutable, .. } => {
            let t = ty.unwrap_or_else(|| ty_of(value, scope, sigs, &structs()).unwrap());
            scope.insert(name.clone(), VarInfo::new(t, *mutable));
            let mut e = String::new();
            if t == Ty::Str {
                emit_bind(value, scope, sigs, &mut e);
            } else {
                emit_expr(value, scope, sigs, &mut e);
            }
            indent(out, level);
            let cn = cname(name);
            if is_owned(t) {
                // 槽已提升为 NULL 初始化；先深释放旧值（首次声明释放 NULL 安全），
                // 再接管移动来的值的指针
                if t == Ty::Str {
                    let _ = writeln!(out, "__t_free({});", cn);
                } else {
                    let _ = writeln!(out, "{}", free_slot(&cn, t));
                }
                indent(out, level);
                let _ = writeln!(out, "{} = {};", cn, e);
                emit_move_nulls(value, true, scope, sigs, out, level);
            } else {
                let _ = writeln!(out, "{} {} = {};", t.c_name(), cn, e);
                // RHS 中的调用可能以 str/结构体 形参移动了实参
                emit_move_nulls(value, false, scope, sigs, out, level);
            }
        }
        // v3.0：左侧可为变量或 变量.字段（规范 14.3）
        Stmt::Assign { target, value, .. } => {
            // 解析左侧 C 表达式、类型、以及（若是变量）其名称（用于自移动判定）
            let (cn, t, target_var): (String, Ty, Option<String>) = match target {
                Expr::Var(n) => (
                    cname(n),
                    scope.get(n).map(|v| v.ty).unwrap_or(Ty::I64),
                    Some(n.clone()),
                ),
                Expr::Field(base, fname) => {
                    let bt = norm(ty_of(base, scope, sigs, &structs()).unwrap_or(Ty::I64));
                    let ft = match bt {
                        Ty::Struct(i) => structs()[i as usize]
                            .fields
                            .iter()
                            .find(|f| f.name == *fname)
                            .map(|f| f.ty),
                        _ => None,
                    }
                    .unwrap_or(Ty::I64);
                    let be = match base.as_ref() {
                        Expr::Var(n) => cname(n),
                        _ => String::new(),
                    };
                    (format!("{}->{}", be, cname(fname)), ft, None)
                }
                _ => unreachable!("类型检查已拦截非法赋值左侧"),
            };
            let mut e = String::new();
            if t == Ty::Str {
                emit_bind(value, scope, sigs, &mut e);
            } else {
                emit_expr(value, scope, sigs, &mut e);
            }
            indent(out, level);
            let self_move = target_var
                .as_ref()
                .map_or(false, |n| matches!(value, Expr::Var(v) if v == n));
            if is_owned(t) {
                if self_move {
                    // 自我移动 a=a / p.x=p.x：所有权不变（复活语义，规范 11.2.3）
                } else if t == Ty::Str && target_var.is_some() {
                    // 天权 11.3.2：先求值到临时（RHS 可能读取自身旧值），置空被移动的源变量；
                    // 若 RHS 内部已消费自身（a=g(a)）则不再释放旧值
                    let consumed = match &target_var {
                        Some(n) => consumes_var(value, n, true, sigs),
                        None => true,
                    };
                    let _ = writeln!(out, "{{ const char* __t_tmp = {};", e);
                    emit_move_nulls(value, true, scope, sigs, out, level);
                    indent(out, level);
                    if !consumed {
                        let _ = writeln!(out, "__t_free({});", cn);
                    }
                    let _ = writeln!(out, "{} = __t_tmp; }}", cn);
                } else if t == Ty::Str {
                    // str 字段赋值：先释放旧字段值，再接管
                    let _ = writeln!(out, "__t_free({}); {} = {};", cn, cn, e);
                    emit_move_nulls(value, true, scope, sigs, out, level);
                } else {
                    // 结构体变量赋值：深释放旧值后接管
                    let consumed = match &target_var {
                        Some(n) => consumes_var(value, n, true, sigs),
                        None => true,
                    };
                    let _ = writeln!(out, "{{ {} __t_tmp = {};", c_ty(t), e);
                    emit_move_nulls(value, true, scope, sigs, out, level);
                    indent(out, level);
                    if !consumed {
                        let _ = writeln!(out, "{}", free_slot(&cn, t));
                    }
                    let _ = writeln!(out, "{} = __t_tmp; }}", cn);
                }
            } else {
                let _ = writeln!(out, "{} = {};", cn, e);
                emit_move_nulls(value, false, scope, sigs, out, level);
            }
        }
        Stmt::Print(e, _) => {
            let t = norm(ty_of(e, scope, sigs, &structs()).unwrap());
            let mut x = String::new();
            emit_expr(e, scope, sigs, &mut x);
            indent(out, level);
            let fmt = match t {
                Ty::I32 | Ty::I64 => "%lld\\n",
                Ty::F64 => "%g\\n",
                Ty::Str => "%s\\n",
                Ty::Bool => "%s\\n",
                Ty::BorrowStr => unreachable!("Print 类型已归一化"),
            };
            let cast = match t {
                Ty::I32 | Ty::I64 => "(long long)",
                Ty::F64 => "(double)",
                // v1.1：bool 统一打印为 true/false（规范第 4 节）
                Ty::Str => "",
                Ty::Bool => "",
                Ty::BorrowStr => unreachable!("Print 类型已归一化"),
            };
            let val = if t == Ty::Bool {
                format!("__t_tos_b((int)({}))", x)
            } else {
                format!("{}({})", cast, x)
            };
            let _ = writeln!(out, "printf(\"{}\", {});", fmt, val);
        }
        Stmt::While { cond, body, .. } => {
            let mut c = String::new();
            emit_expr(cond, scope, sigs, &mut c);
            indent(out, level);
            let _ = writeln!(out, "while ({}) {{", c);
            for s in body {
                emit_stmt(s, scope, sigs, fn_ret, post, out, level + 1);
            }
            indent(out, level);
            out.push_str("}\n");
        }
        Stmt::If { cond, body, else_body, .. } => {
            let mut c = String::new();
            emit_expr(cond, scope, sigs, &mut c);
            indent(out, level);
            let _ = writeln!(out, "if ({}) {{", c);
            for s in body {
                emit_stmt(s, scope, sigs, fn_ret, post, out, level + 1);
            }
            if let Some(eb) = else_body {
                indent(out, level);
                out.push_str("} else {\n");
                for s in eb {
                    emit_stmt(s, scope, sigs, fn_ret, post, out, level + 1);
                }
            }
            indent(out, level);
            out.push_str("}\n");
        }
        Stmt::Return(e, _) => {
            match e {
                Some(expr) => {
                    let mut x = String::new();
                    if fn_ret == Ty::Str {
                        emit_bind(expr, scope, sigs, &mut x);
                    } else {
                        emit_expr(expr, scope, sigs, &mut x);
                    }
                    // v3.0：struct 返回值同样需要在返回位置将源变量置空
                    let needs_nulls = has_move_nulls(expr, is_owned(fn_ret), sigs);
                    indent(out, level);
                    if is_owned(fn_ret) || needs_nulls || post.is_some() {
                        // 天权：返回前先求值到临时、置空被移动的源变量，再返回；
                        // v2.2：返回前校验 @post 出口契约
                        let _ = writeln!(out, "{{ {} __t_tmp = {};", c_ty(fn_ret), x);
                        emit_move_nulls(expr, is_owned(fn_ret), scope, sigs, out, level);
                        if let Some((helper, cline)) = post {
                            indent(out, level);
                            let _ = writeln!(
                                out,
                                "if (!{}(__t_tmp)) {{ fprintf(stderr, \"@post 契约失败（第 {} 行）\\n\"); exit(1); }}",
                                helper, cline
                            );
                        }
                        indent(out, level);
                        let _ = writeln!(out, "return __t_tmp; }}");
                    } else {
                        let cast = match fn_ret {
                            Ty::I32 => "(int)",
                            Ty::I64 => "(long long)",
                            Ty::F64 => "(double)",
                            Ty::Str => "(const char*)",
                            Ty::Bool => "(int)",
                            // 结构体指针原样返回
                            Ty::Struct(_) => "",
                            Ty::BorrowStr => unreachable!("借用不可作为返回类型（检查器已拦截）"),
                        };
                        let _ = writeln!(out, "return {}({});", cast, x);
                    }
                }
                // 裸 r/ 返回该类型零值（规范 5.3）
                None => {
                    let zero = match fn_ret {
                        Ty::I32 | Ty::I64 => "0",
                        Ty::F64 => "0.0",
                        Ty::Str => "__t_dup(\"\")",
                        Ty::Bool => "0",
                        Ty::Struct(_) => "NULL",
                        Ty::BorrowStr => unreachable!("借用不可作为返回类型（检查器已拦截）"),
                    };
                    match post {
                        Some((helper, cline)) => {
                            let _ = writeln!(
                                out,
                                "{{ {} __t_tmp = {}; if (!{}(__t_tmp)) {{ fprintf(stderr, \"@post 契约失败（第 {} 行）\\n\"); exit(1); }} return __t_tmp; }}",
                                c_ty(fn_ret),
                                zero,
                                helper,
                                cline
                            );
                        }
                        None => {
                            let _ = writeln!(out, "return {};", zero);
                        }
                    }
                }
            }
        }
        Stmt::Expr(e, _) => {
            let mut x = String::new();
            emit_expr(e, scope, sigs, &mut x);
            indent(out, level);
            let _ = writeln!(out, "{};", x);
            // 语句级调用可能移动了 str 实参
            emit_move_nulls(e, false, scope, sigs, out, level);
        }
    }
}

/// 表达式 → C 表达式文本（type_check 已通过，推导不会失败）
fn emit_expr(e: &Expr, scope: &Scope, sigs: &HashMap<String, FuncSig>, out: &mut String) {
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
        Expr::Bool(b) => out.push_str(if *b { "1" } else { "0" }),
        Expr::Var(name) => out.push_str(&cname(name)),
        // v3.0：结构体字面量 → 构造函数调用；str 字段由 emit_bind 决定 dup/移动（规范 14.2）
        Expr::StructLit { name, fields, .. } => {
            let sd = structs()
                .into_iter()
                .find(|s| s.name == *name)
                .expect("未声明结构体（类型检查已拦截）");
            let mut args = Vec::new();
            for (i, (_, v)) in fields.iter().enumerate() {
                let ft = sd.fields[i].ty;
                let mut a = String::new();
                if ft == Ty::Str {
                    emit_bind(v, scope, sigs, &mut a);
                } else {
                    emit_expr(v, scope, sigs, &mut a);
                }
                args.push(a);
            }
            out.push_str(&format!("{}({})", new_fn(&sd), args.join(", ")));
        }
        // v3.0：字段读取 p.x —— base 是结构体指针，用 -> 取成员；
        // str 字段返回其指针（只读借用，不复制，规范 14.3）
        Expr::Field(base, fname) => {
            let bt = norm(ty_of(base, scope, sigs, &structs()).unwrap_or(Ty::I64));
            let field = match bt {
                Ty::Struct(i) => structs()[i as usize]
                    .fields
                    .iter()
                    .find(|f| f.name == *fname)
                    .map(|f| cname(&f.name))
                    .unwrap_or_else(|| cname(fname)),
                _ => cname(fname),
            };
            let mut b = String::new();
            emit_expr(base, scope, sigs, &mut b);
            out.push_str(&format!("{}->{}", b, field));
        }
        // v2.1 借用：按只读指针传递，无复制、无移动、无置空（规范 11.6）
        Expr::Borrow(inner) => emit_expr(inner, scope, sigs, out),
        Expr::Neg(inner) => {
            out.push_str("(-");
            emit_expr(inner, scope, sigs, out);
            out.push(')');
        }
        Expr::Bin { op, lhs, rhs } => {
            // v2.1：借用值在只读运算位视同 str（规范 11.6.2）
            let lt = norm(ty_of(lhs, scope, sigs, &structs()).unwrap_or(Ty::I64));
            let rt = norm(ty_of(rhs, scope, sigs, &structs()).unwrap_or(Ty::I64));
            // v0.3：str + str → 运行时拼接 __t_cat
            if *op == BinOp::Add && lt == Ty::Str && rt == Ty::Str {
                out.push_str("__t_cat(");
                emit_expr(lhs, scope, sigs, out);
                out.push_str(", ");
                emit_expr(rhs, scope, sigs, out);
                out.push(')');
                return;
            }
            // str ==/!= → 值比较（strcmp），不能用 C 指针相等
            if matches!(op, BinOp::Eq | BinOp::Ne) && lt == Ty::Str && rt == Ty::Str {
                out.push_str(if *op == BinOp::Eq { "__t_seq(" } else { "__t_sne(" });
                emit_expr(lhs, scope, sigs, out);
                out.push_str(", ");
                emit_expr(rhs, scope, sigs, out);
                out.push(')');
                return;
            }
            out.push('(');
            emit_expr(lhs, scope, sigs, out);
            let _ = write!(out, " {} ", op.c_str());
            emit_expr(rhs, scope, sigs, out);
            out.push(')');
        }
        Expr::Call { name, args } => {
            out.push_str(&cname(name));
            out.push('(');
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                // 不做强转：type_check 已保证实参与形参兼容（规范 5.3 提升规则）
                emit_expr(a, scope, sigs, out);
            }
            out.push(')');
        }
        Expr::Convert { name, arg } => {
            if name == "copy" {
                // copy(s)：显式复制为独立所有者（规范 11.2.5）
                out.push_str("__t_dup(");
                emit_expr(arg, scope, sigs, out);
                out.push(')');
                return;
            }
            let t = norm(ty_of(arg, scope, sigs, &structs()).unwrap());
            if name == "toi" {
                if t == Ty::Str {
                    // 字符串字面量已在类型检查时验证可解析
                    if let Expr::Str(s) = arg.as_ref() {
                        out.push_str(&format!("({}LL)", s.parse::<i64>().unwrap()));
                    }
                } else {
                    out.push_str("((long long)");
                    emit_expr(arg, scope, sigs, out);
                    out.push(')');
                }
            } else {
                // tos
                match t {
                    Ty::Str => emit_expr(arg, scope, sigs, out),
                    Ty::F64 => {
                        out.push_str("__t_tos_f(");
                        emit_expr(arg, scope, sigs, out);
                        out.push(')');
                    }
                    _ => {
                        out.push_str("__t_tos_ll((long long)");
                        emit_expr(arg, scope, sigs, out);
                        out.push(')');
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex_spanned;
    use crate::parser::parse;
    use crate::type_check::check;

    #[test]
    fn test_gen_main_t() {
        let src = "# main.t\n/a=20\n`a/2\n//cnt=1\nw/cnt<=3{\n    cnt=cnt+1\n    `cnt/a\n}\nf/add(x,y){\n    /res=x+y\n    `res\n}\nadd(10,90)\n";
        let prog = parse(lex_spanned(src).unwrap()).unwrap();
        check(&prog).unwrap();
        let c = generate(&prog);
        assert!(c.contains("long long t_a = 20;"));
        assert!(c.contains("(t_a / 2)"));
        assert!(c.contains("while ((t_cnt <= 3))"));
        assert!(c.contains("static long long t_add(long long t_x, long long t_y)"));
        assert!(c.contains("t_add(10, 90);"));
        // 生成代码可被本机 C 编译器接受（仅语法层面验证，不执行）
        assert!(c.contains("int main(void)"));
    }

    #[test]
    fn test_chinese_ident_mangling() {
        let src = "/天=1\n`天\n";
        let prog = parse(lex_spanned(src).unwrap()).unwrap();
        check(&prog).unwrap();
        let c = generate(&prog);
        assert!(c.contains("t_x5929 = 1;"));
    }

    #[test]
    fn test_gen_ownership() {
        // 天权 v2.0：字面量 dup、copy 借用、移动置空、出口释放
        let src = "//s=\"天\"\n//c=copy(s)\n`s+\"道\"\n//t=s\n`t\n";
        let prog = parse(lex_spanned(src).unwrap()).unwrap();
        check(&prog).unwrap();
        let c = generate(&prog);
        // str 槽提升为 NULL 初始化
        assert!(c.contains("const char* t_s = NULL;"));
        // 字面量绑定走 __t_dup
        assert!(c.contains("t_s = __t_dup(\"天\");"), "生成的 C：\n{}", c);
        // copy 只借用：不置空源
        assert!(c.contains("__t_dup(t_s)"));
        // 移动（声明绑定）：t 接管指针，源置空
        assert!(c.contains("t_t = t_s;"));
        assert!(c.contains("t_s = NULL;"));
        // 移动（赋值绑定）：经临时变量，释放旧值后接管
        // 作用域出口释放
        assert!(c.contains("__t_free(t_s); t_s = NULL;"));
    }

    #[test]
    fn test_gen_self_consuming_assign() {
        // a = g(a)：RHS 以 str 形参消费自身 → 不释放旧值（避免双重释放）
        let src = "f/echo(x:str):str{\nr/x\n}\n//s=\"a\"\ns=echo(s)\n`s\n";
        let prog = parse(lex_spanned(src).unwrap()).unwrap();
        check(&prog).unwrap();
        let c = generate(&prog);
        assert!(!c.contains("__t_free(t_s);\n    t_s = __t_tmp;"), "不应释放被消费的旧值：\n{}", c);
        assert!(c.contains("t_echo(t_s)"));
    }

    #[test]
    fn test_gen_borrow() {
        // 天权 v2.1：借用形参不释放、不置空；借用实参不移动源（规范 11.6）
        let src = "f/shout(x:&str):str{\nr/x+\"!\"\n}\n//s=\"天\"\n`shout(&s)\n`s\n";
        let prog = parse(lex_spanned(src).unwrap()).unwrap();
        check(&prog).unwrap();
        let c = generate(&prog);
        // 形参按只读指针传递
        assert!(c.contains("const char* t_x)"), "生成的 C：\n{}", c);
        // 借用形参不是所有者：函数出口不释放
        assert!(!c.contains("__t_free(t_x)"), "借用形参不应被释放：\n{}", c);
        // 借用实参不置空源（t_s = NULL 仅出现在槽位初始化与出口释放两处）
        assert_eq!(
            c.matches("t_s = NULL;").count(),
            2,
            "借用不应移动源变量：\n{}",
            c
        );
        // 借用实参直接传指针
        assert!(c.contains("t_shout(t_s)"));
    }

    #[test]
    fn test_gen_contracts() {
        // v2.2 语义锚点：@pre 入口守卫、@post 出口校验、@example 启动自检
        let src = "f/wrap(s:&str):str{\n@pre: s != \"\"\n@post: ret != \"\"\n@example: wrap(\"hi\") -> \"hi!\"\nr/ s+\"!\"\n}\n`wrap(\"x\")\n";
        let prog = parse(lex_spanned(src).unwrap()).unwrap();
        check(&prog).unwrap();
        let c = generate(&prog);
        // @pre：函数入口 if 守卫，失败 exit(1)
        assert!(c.contains("@pre 契约失败：wrap"), "生成的 C：\n{}", c);
        // @post：helper 函数 __t_post_<cname>（cname 自带 t_ 前缀），返回点调用
        assert!(c.contains("__t_post_t_wrap"), "生成的 C：\n{}", c);
        // @example：main 启动自检，str 用 __t_seq 比较
        assert!(c.contains("__t_seq("), "生成的 C：\n{}", c);
        assert!(c.contains("@example 契约失败：wrap"), "生成的 C：\n{}", c);
    }
}
