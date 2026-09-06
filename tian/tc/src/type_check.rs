//! 类型检查：AST 静态校验（规范第 6、7 节）
//!
//! 拦截：str/bool 与数字混用、只读变量赋值、未声明/重复声明、
//! 未定义函数、参数数量不符、条件非 bool、函数带返回值使用等。

use std::collections::HashMap;

use crate::ast::*;

#[derive(Debug)]
pub struct CheckError {
    pub msg: String,
    pub line: usize,
    /// v2.3 差分诊断：修复提示（"怎么改"）
    pub hint: String,
    /// v2.3 差分诊断：候选名（"可能你想写的是"）
    pub candidates: Vec<String>,
}

impl std::fmt::Display for CheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "编译错误（第 {} 行）：{}", self.line, self.msg)?;
        if !self.hint.is_empty() {
            write!(f, "；提示：{}", self.hint)?;
        }
        if !self.candidates.is_empty() {
            write!(f, "；候选：{}", self.candidates.join(" / "))?;
        }
        Ok(())
    }
}

fn err(line: usize, msg: impl Into<String>) -> CheckError {
    CheckError {
        msg: msg.into(),
        line,
        hint: String::new(),
        candidates: Vec::new(),
    }
}

/// v2.3 差分诊断：按编辑距离与前后缀相似度给出候选名（最多 3 个）
pub fn similar_candidates(name: &str, pool: &[String]) -> Vec<String> {
    let mut scored: Vec<(usize, &String)> = pool
        .iter()
        .filter(|k| k.as_str() != name)
        .filter(|k| {
            levenshtein(name, k) <= 2
                || k.starts_with(name)
                || name.starts_with(k.as_str())
                || k.contains(name)
        })
        .map(|k| (levenshtein(name, k), k))
        .collect();
    scored.sort_by_key(|(d, _)| *d);
    scored.into_iter().map(|(_, k)| k.clone()).take(3).collect()
}

/// 编辑距离（DP，O(mn)；标识符都很短，足够）
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

#[derive(Debug, Clone, Copy)]
pub struct VarInfo {
    pub ty: Ty,
    pub mutable: bool,
    /// 天权 v2.0：str 所有权是否已被移走（仅 str 有意义）
    pub moved: bool,
}

impl VarInfo {
    pub fn new(ty: Ty, mutable: bool) -> Self {
        VarInfo {
            ty,
            mutable,
            moved: false,
        }
    }
}

pub type Scope = HashMap<String, VarInfo>;

/// 函数签名（v0.3：全量类型标注，省略默认 i64）
#[derive(Debug, Clone)]
pub struct FuncSig {
    pub params: Vec<Ty>,
    pub ret: Ty,
}

/// 入口：校验整个程序
pub fn check(prog: &Program) -> Result<(), CheckError> {
    // 函数表（定义顺序无关）
    let mut funcs: HashMap<String, FuncSig> = HashMap::new();
    for f in &prog.funcs {
        let sig = FuncSig {
            params: f.params.iter().map(|p| p.ty.unwrap_or(Ty::I64)).collect(),
            ret: f.ret.unwrap_or(Ty::I64),
        };
        if funcs.insert(f.name.clone(), sig).is_some() {
            return Err(err(0, format!("函数 '{}' 重复定义", f.name)));
        }
        // v2.1：返回类型不能是借用（借用不可逃逸，规范 11.6.5）
        if f.ret == Some(Ty::BorrowStr) {
            return Err(err(0, format!("函数 '{}' 的返回类型不能是借用 &str", f.name)));
        }
        // 参数重复检查
        let mut seen = std::collections::HashSet::new();
        for p in &f.params {
            if !seen.insert(p.name.clone()) {
                return Err(err(0, format!("函数 '{}' 参数 '{}' 重复", f.name, p.name)));
            }
        }
    }

    // v3.0：结构体声明校验（规范 14.1）
    for sd in &prog.structs {
        if sd.fields.is_empty() {
            return Err(err(sd.line, format!("结构体 '{}' 至少需要一个字段", sd.name)));
        }
        for f in &sd.fields {
            if f.ty == Ty::BorrowStr {
                return Err(err(
                    f.line,
                    format!("结构体 '{}' 的字段 '{}' 不能是借用 &str（借用不可存储在结构体中）", sd.name, f.name),
                ));
            }
        }
    }

    // 顶层语句（全局作用域，r/ 禁止出现在顶层）
    let mut top_scope: Scope = HashMap::new();
    for s in &prog.top {
        check_stmt(s, &mut top_scope, &funcs, &prog.structs, false, Ty::I64)?;
    }

    // 函数体：作用域仅含自身参数（规范 5.3：函数不可访问顶层变量）
    for f in &prog.funcs {
        let ret = f.ret.unwrap_or(Ty::I64);
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
        for s in &f.body {
            check_stmt(s, &mut scope, &funcs, &prog.structs, true, ret)?;
        }
        // v2.2：语义锚点静态校验（规范第 12 节）
        check_contracts(f, &funcs, &prog.structs, ret)?;
    }
    Ok(())
}

/// v2.2 语义锚点：@pre/@post 必须是 bool 表达式（@post 中 ret 指返回值）；
/// @example 必须调用本函数自身，且期望值类型与返回类型一致。
/// 锚点表达式只能引用函数形参（与函数体隔离，防锚点漂移）。
fn check_contracts(
    f: &FnDef,
    funcs: &HashMap<String, FuncSig>,
    structs: &[StructDef],
    ret: Ty,
) -> Result<(), CheckError> {
    let base: Scope = f
        .params
        .iter()
        .map(|p| (p.name.clone(), VarInfo::new(p.ty.unwrap_or(Ty::I64), false)))
        .collect();
    for c in &f.contracts {
        match c {
            Contract::Pre(e, line) => {
                let mut cs = base.clone();
                let t = check_expr(e, &mut cs, funcs, structs, *line)?;
                if t != Ty::Bool {
                    return Err(err(*line, format!("@pre 必须是 bool 表达式，实际 {}", ty_label(t, structs))));
                }
            }
            Contract::Post(e, line) => {
                let mut cs = base.clone();
                cs.insert("ret".into(), VarInfo::new(ret, false));
                let t = check_expr(e, &mut cs, funcs, structs, *line)?;
                if t != Ty::Bool {
                    return Err(err(*line, format!("@post 必须是 bool 表达式，实际 {}", ty_label(t, structs))));
                }
                // v3.0：结构体返回值不能直接参与比较（规范 14.5）
                if let Expr::Bin { lhs, .. } = e {
                    if matches!(lhs.as_ref(), Expr::Var(n) if n == "ret") && ret.is_struct() {
                        return Err(err(*line, "结构体不能用 ==/!= 比较（请比较字段，如 ret.x == 1）"));
                    }
                }
            }
            Contract::Example { call, expected, line } => {
                match call {
                    Expr::Call { name, .. } if name == &f.name => {}
                    _ => return Err(err(*line, "@example 必须调用本函数自身（自测样例）")),
                }
                let mut cs = base.clone();
                let ct = check_expr(call, &mut cs, funcs, structs, *line)?;
                let et = check_expr(expected, &mut cs, funcs, structs, *line)?;
                if ct != ret {
                    return Err(err(
                        *line,
                        format!(
                            "@example 调用返回类型 {} 与函数返回类型 {} 不符",
                            ty_label(ct, structs),
                            ty_label(ret, structs)
                        ),
                    ));
                }
                if et != ret {
                    return Err(err(
                        *line,
                        format!(
                            "@example 期望值类型 {} 与返回类型 {} 不符",
                            ty_label(et, structs),
                            ty_label(ret, structs)
                        ),
                    ));
                }
                // v3.0：结构体返回值不能用 @example 直接比对（规范 14.5）
                if ret.is_struct() {
                    return Err(err(
                        *line,
                        "结构体返回值不能用 @example 直接比对（请为返回字段的函数写样例）",
                    ));
                }
            }
        }
    }
    Ok(())
}

/// 块作用域包装：块内声明不外泄；外层变量的移动状态保守外传（规范 11.4）
fn with_block_scope(
    scope: &mut Scope,
    f: impl FnOnce(&mut Scope) -> Result<(), CheckError>,
) -> Result<(), CheckError> {
    let snapshot = scope.clone();
    f(scope)?;
    // 外层变量的移动状态外传（任一分支中移动即视为已移动）
    let moved_outer: Vec<String> = scope
        .iter()
        .filter(|(k, v)| v.moved && snapshot.contains_key(k.as_str()))
        .map(|(k, _)| k.clone())
        .collect();
    *scope = snapshot;
    for k in moved_outer {
        if let Some(v) = scope.get_mut(&k) {
            v.moved = true;
        }
    }
    Ok(())
}

/// 天权：语句顶层的移动（赋值/声明的 RHS、返回值为 Var 时，源变量失效）
/// v3.0 起对结构体同样生效（结构体是堆值，规范 14.4）
fn mark_top_move(expr: &Expr, scope: &mut Scope) {
    if let Expr::Var(name) = expr {
        if let Some(v) = scope.get_mut(name) {
            if is_owned(v.ty) {
                v.moved = true;
            }
        }
    }
}

fn check_stmt(
    s: &Stmt,
    scope: &mut Scope,
    funcs: &HashMap<String, FuncSig>,
    structs: &[StructDef],
    in_fn: bool,
    fn_ret: Ty,
) -> Result<(), CheckError> {
    match s {
        Stmt::Decl {
            mutable,
            name,
            ty,
            value,
            line,
        } => {
            if scope.contains_key(name) {
                return Err(err(*line, format!("变量 '{}' 重复声明", name)));
            }
            let vt = check_expr(value, scope, funcs, structs, *line)?;
            let target = ty.unwrap_or(vt);
            // v2.1：借用不能取得所有权；局部变量不能声明/推断为借用类型（规范 11.6.5）
            if target == Ty::BorrowStr {
                let msg = if ty.is_some() {
                    "局部变量不能声明为借用 &str（借用只存在于调用期间）"
                } else {
                    "借用不能取得所有权（&str 形参不可绑定给 str 变量；需要副本请用 copy()）"
                };
                return Err(err(*line, msg));
            }
            if let Some(ann) = ty {
                if !value_assignable(*ann, vt, value) {
                    return Err(err(
                        *line,
                        format!("类型标注 {} 与实际类型 {} 不符", ann.label(), vt.label()),
                    ));
                }
            }
            // 天权：str / 结构体绑定移动 RHS 的所有权（规范 11.2 / 14.4）
            if is_owned(target) {
                // v3.0：结构体字段只能借用、不能移出（否则字段与结构体双释放，规范 14.4）
                if matches!(value, Expr::Field(..)) {
                    return Err(err(
                        *line,
                        format!(
                            "不能把结构体字段移出为 {} 所有者（请用 copy(p.x) 取副本）",
                            ty_label(target, structs)
                        ),
                    ));
                }
                mark_top_move(value, scope);
            }
            scope.insert(name.clone(), VarInfo::new(target, *mutable));
            Ok(())
        }
        // v3.0：左侧可为变量或字段（p.x = e，规范 14.3）
        Stmt::Assign { target, value, line } => {
            let (var_name, target_ty) = match target {
                Expr::Var(name) => {
                    let info = match scope.get(name) {
                        Some(v) => *v,
                        None => {
                            return Err(err(*line, format!("变量 '{}' 未声明（声明用 / 或 // 前缀）", name)))
                        }
                    };
                    if !info.mutable {
                        return Err(err(*line, format!("变量 '{}' 是只读的（/ 声明），不能赋值", name)));
                    }
                    (Some(name.clone()), info.ty)
                }
                Expr::Field(base, fname) => {
                    let bt = norm(check_expr(base, scope, funcs, structs, *line)?);
                    let sd = struct_by_id(structs, bt).ok_or_else(|| {
                        err(
                            *line,
                            format!("类型 {} 没有字段（只有结构体能用 . 取字段）", ty_label(bt, structs)),
                        )
                    })?;
                    let ft = field_ty(sd, fname).ok_or_else(|| {
                        err(*line, format!("结构体 '{}' 没有字段 '{}'", sd.name, fname))
                    })?;
                    // 字段写入要求基表达式是可写变量（结构体本身只读则不可改字段）
                    match base.as_ref() {
                        Expr::Var(bn) => match scope.get(bn) {
                            None => return Err(err(*line, format!("变量 '{}' 未声明", bn))),
                            Some(v) if !v.mutable => {
                                return Err(err(
                                    *line,
                                    format!("变量 '{}' 是只读的（/ 声明），不能修改其字段", bn),
                                ))
                            }
                            Some(v) if v.moved => {
                                return Err(err(*line, format!("变量 '{}' 的值已被移动，不能修改其字段", bn)))
                            }
                            _ => {}
                        },
                        _ => return Err(err(*line, "只能给'变量.字段'赋值（如 p.x = 1）")),
                    }
                    (None, ft)
                }
                _ => return Err(err(*line, "赋值的左侧只能是变量或 变量.字段")),
            };
            let vt = check_expr(value, scope, funcs, structs, *line)?;
            if !assign_compatible(target_ty, vt, value) {
                return Err(err(
                    *line,
                    format!(
                        "不能把 {} 赋给 {} 类型的左侧",
                        ty_label(vt, structs),
                        ty_label(target_ty, structs)
                    ),
                ));
            }
            // 天权：整体接收所有权（移动进入 + 复活，规范 11.2.3 / 14.4）
            if is_owned(target_ty) {
                if matches!(value, Expr::Field(..)) {
                    return Err(err(
                        *line,
                        "不能把结构体字段移出（字段只能借用：请用 copy(p.x) 取副本）",
                    ));
                }
                mark_top_move(value, scope);
            }
            if let Some(name) = var_name {
                if let Some(v) = scope.get_mut(&name) {
                    v.moved = false;
                }
            }
            Ok(())
        }
        Stmt::Print(e, line) => {
            // 打印只借用（规范 11.2.4）；结构体整体不可直接打印（规范 14.5）
            let t = check_expr(e, scope, funcs, structs, *line)?;
            if t.is_struct() {
                return Err(err(
                    *line,
                    format!(
                        "不能直接打印结构体 '{}'（请打印具体字段，如 p.x）",
                        ty_label(t, structs)
                    ),
                ));
            }
            Ok(())
        }
        Stmt::While { cond, body, line } => {
            let ct = check_expr(cond, scope, funcs, structs, *line)?;
            if ct != Ty::Bool {
                return Err(err(*line, format!("while 条件必须是 bool，实际 {}", ty_label(ct, structs))));
            }
            with_block_scope(scope, |s| {
                for st in body {
                    check_stmt(st, s, funcs, structs, in_fn, fn_ret)?;
                }
                Ok(())
            })
        }
        Stmt::If { cond, body, else_body, line } => {
            let ct = check_expr(cond, scope, funcs, structs, *line)?;
            if ct != Ty::Bool {
                return Err(err(*line, format!("if 条件必须是 bool，实际 {}", ty_label(ct, structs))));
            }
            with_block_scope(scope, |s| {
                for st in body {
                    check_stmt(st, s, funcs, structs, in_fn, fn_ret)?;
                }
                Ok(())
            })?;
            if let Some(eb) = else_body {
                with_block_scope(scope, |s| {
                    for st in eb {
                        check_stmt(st, s, funcs, structs, in_fn, fn_ret)?;
                    }
                    Ok(())
                })?;
            }
            Ok(())
        }
        Stmt::Return(e, line) => {
            if !in_fn {
                return Err(err(*line, "r/ 返回只能出现在函数体内"));
            }
            if let Some(expr) = e {
                let t = check_expr(expr, scope, funcs, structs, *line)?;
                // v2.1：借用不可逃逸函数（规范 11.6.5）
                if t == Ty::BorrowStr {
                    return Err(err(
                        *line,
                        "借用不能逃逸函数（&str 形参不可返回；需要所有权请用 copy() 产生新所有者）",
                    ));
                }
                // v0.3：返回类型须与标注一致（提升规则见 value_assignable，规范 5.3）
                if !value_assignable(fn_ret, t, expr) {
                    return Err(err(
                        *line,
                        format!(
                            "r/ 返回值类型 {} 与声明的返回类型 {} 不符",
                            ty_label(t, structs),
                            ty_label(fn_ret, structs)
                        ),
                    ));
                }
                // 天权：返回 str / 结构体 Var = 移动所有权交给调用方（规范 11.2 / 14.4）
                if is_owned(fn_ret) {
                    if matches!(expr, Expr::Field(..)) {
                        return Err(err(
                            *line,
                            "不能把结构体字段移出作为返回值（请用 copy(p.x) 返回副本）",
                        ));
                    }
                    mark_top_move(expr, scope);
                }
            }
            Ok(())
        }
        Stmt::Expr(e, line) => match e {
            Expr::Call { .. } => {
                check_expr(e, scope, funcs, structs, *line)?;
                Ok(())
            }
            _ => Err(err(*line, "语句级表达式只能是函数调用")),
        },
    }
}

/// 传参 / 返回值兼容规则（规范 5.3）：同型；I32→I64 提升；整数→F64 提升；
/// I32 目标仅接受范围内整数字面量
fn ty_assignable(expected: Ty, actual: Ty) -> bool {
    if expected == actual {
        return true;
    }
    match (expected, actual) {
        (Ty::I64, Ty::I32) => true,
        (Ty::F64, Ty::I32) | (Ty::F64, Ty::I64) => true,
        (Ty::I32, Ty::I64) => false, // 收窄仅限字面量，由调用方特判
        _ => false,
    }
}

/// 带字面量特判的兼容判定（形参/返回值共用）
fn value_assignable(expected: Ty, actual: Ty, value: &Expr) -> bool {
    if ty_assignable(expected, actual) {
        return true;
    }
    if expected == Ty::I32 && actual == Ty::I64 {
        if let Expr::Int(v) = value {
            return *v >= i32::MIN as i64 && *v <= i32::MAX as i64;
        }
    }
    // v2.1：字符串字面量为静态存储，可直接作为 &str 形参实参（只读借用）
    if expected == Ty::BorrowStr && actual == Ty::Str && matches!(value, Expr::Str(_)) {
        return true;
    }
    false
}

fn assign_compatible(var_ty: Ty, val_ty: Ty, value: &Expr) -> bool {
    value_assignable(var_ty, val_ty, value)
}

/// v2.1：借用值在"只读位"（比较、拼接、打印）视同 str（规范 11.6.2）
pub fn norm(t: Ty) -> Ty {
    if t == Ty::BorrowStr {
        Ty::Str
    } else {
        t
    }
}

/// v3.0：受天权所有权约束的堆类型——str 与结构体（规范 11.2 / 14.4）。
/// 赋值、传参、返回均移动所有权；数值与 bool 为纯值类型，不参与。
pub fn is_owned(t: Ty) -> bool {
    t == Ty::Str || t.is_struct()
}

// ── v3.0 结构体表（代码生成侧共享）──────────────────────────────
// 两个后端的 emit_* 函数签名里已贯穿 sigs；再逐一加 structs 参数会改动近百处
// 调用点且无收益。这里用线程局部表：generate()/generate_object() 入口设置一次，
// crate 内部单线程，单测并行时各线程互不影响。类型检查侧仍是显式传参。
thread_local! {
    static STRUCTS: std::cell::RefCell<Vec<StructDef>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// 设置当前线程的结构体表（后端入口调用）
pub fn set_structs(structs: &[StructDef]) {
    STRUCTS.with(|s| *s.borrow_mut() = structs.to_vec());
}

/// 读取当前线程的结构体表快照
pub fn structs() -> Vec<StructDef> {
    STRUCTS.with(|s| s.borrow().clone())
}

/// v3.0：按 Ty::Struct 索引取结构体定义
pub fn struct_by_id<'a>(structs: &'a [StructDef], t: Ty) -> Option<&'a StructDef> {
    match t {
        Ty::Struct(i) => structs.get(i as usize),
        _ => None,
    }
}

/// v3.0：按名查结构体，返回 (索引, 定义)
pub fn struct_by_name<'a>(structs: &'a [StructDef], name: &str) -> Option<(u32, &'a StructDef)> {
    structs
        .iter()
        .position(|s| s.name == name)
        .map(|i| (i as u32, &structs[i]))
}

/// v3.0：字段类型查询（找不到字段返回 None）
pub fn field_ty(sd: &StructDef, fname: &str) -> Option<Ty> {
    sd.fields.iter().find(|f| f.name == fname).map(|f| f.ty)
}

/// v3.0：含结构体名的类型标签（报错信息用）
pub fn ty_label(t: Ty, structs: &[StructDef]) -> String {
    match t {
        Ty::Struct(i) => structs
            .get(i as usize)
            .map(|s| s.name.clone())
            .unwrap_or_else(|| "struct".into()),
        other => other.label().to_string(),
    }
}

/// 变量类型查询抽象：让 ty_of 同时服务于检查器（VarInfo 表）
/// 与天权代码生成的 str 声明收集（纯 Ty 表）
pub trait TyLookup {
    fn lookup_ty(&self, name: &str) -> Option<Ty>;
}

impl TyLookup for HashMap<String, Ty> {
    fn lookup_ty(&self, name: &str) -> Option<Ty> {
        self.get(name).copied()
    }
}

impl TyLookup for Scope {
    fn lookup_ty(&self, name: &str) -> Option<Ty> {
        self.get(name).map(|v| v.ty)
    }
}

/// 表达式类型推导（gen_c / gen_native 复用）
/// v3.0：structs 为结构体表，供字段访问与结构体字面量解析类型
pub fn ty_of<S: TyLookup>(
    expr: &Expr,
    scope: &S,
    funcs: &HashMap<String, FuncSig>,
    structs: &[StructDef],
) -> Result<Ty, CheckError> {
    match expr {
        Expr::Int(_) => Ok(Ty::I64),
        Expr::Float(_) => Ok(Ty::F64),
        Expr::Str(_) => Ok(Ty::Str),
        Expr::Bool(_) => Ok(Ty::Bool),
        Expr::Var(name) => scope
            .lookup_ty(name)
            .ok_or_else(|| err(0, format!("变量 '{}' 未声明", name))),
        // v3.0：p.x —— 基表达式必须是结构体，字段必须存在（规范 14.3）
        Expr::Field(base, fname) => {
            let bt = norm(ty_of(base, scope, funcs, structs)?);
            let sd = struct_by_id(structs, bt)
                .ok_or_else(|| err(0, format!("类型 {} 没有字段（只有结构体能取字段）", ty_label(bt, structs))))?;
            field_ty(sd, fname).ok_or_else(|| {
                let mut e = err(0, format!("结构体 '{}' 没有字段 '{}'", sd.name, fname));
                let names: Vec<String> = sd.fields.iter().map(|f| f.name.clone()).collect();
                e.candidates = similar_candidates(fname, &names);
                if !e.candidates.is_empty() {
                    e.hint = format!("是否想用 '{}'？", e.candidates[0]);
                }
                e
            })
        }
        // v3.0：Point{...} —— 类型为该结构体（字段校验在 check_expr 完成）
        Expr::StructLit { name, .. } => struct_by_name(structs, name)
            .map(|(i, _)| Ty::Struct(i))
            .ok_or_else(|| err(0, format!("未知结构体 '{}'", name))),
        Expr::Neg(e) => {
            let t = ty_of(e, scope, funcs, structs)?;
            if t.is_numeric() {
                Ok(t)
            } else {
                Err(err(0, format!("类型 {} 不能取负号", t.label())))
            }
        }
        Expr::Bin { op, lhs, rhs } => {
            let lt = norm(ty_of(lhs, scope, funcs, structs)?);
            let rt = norm(ty_of(rhs, scope, funcs, structs)?);
            if matches!(
                op,
                BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge | BinOp::Eq | BinOp::Ne
            ) {
                // 比较
                let ok = match (lt, rt) {
                    (a, b) if a.is_numeric() && b.is_numeric() => true,
                    (Ty::Str, Ty::Str) => matches!(op, BinOp::Eq | BinOp::Ne),
                    (Ty::Bool, Ty::Bool) => matches!(op, BinOp::Eq | BinOp::Ne),
                    _ => false,
                };
                if ok {
                    Ok(Ty::Bool)
                } else {
                    Err(err(0, format!("类型 {} 与 {} 不能用 {} 比较", lt.label(), rt.label(), op.c_str())))
                }
            } else {
                // 算术：数字之间；int 混算 → i64，见 float → f64
                if lt.is_numeric() && rt.is_numeric() {
                    Ok(if lt == Ty::F64 || rt == Ty::F64 { Ty::F64 } else { Ty::I64 })
                } else if *op == BinOp::Add && lt == Ty::Str && rt == Ty::Str {
                    // v0.3：字符串拼接（规范第 6 节）
                    Ok(Ty::Str)
                } else if lt == Ty::Str || rt == Ty::Str {
                    // 此处只剩 str 与数字/bool 混算
                    Err(err(0, format!("str 与数字不能混算 {}，请用 tos() 显式转换", op.c_str())))
                } else {
                    Err(err(0, format!("类型 {} 与 {} 不参与算术运算 {}", lt.label(), rt.label(), op.c_str())))
                }
            }
        }
        Expr::Call { name, args } => {
            // v0.3：调用类型 = 声明的返回类型，实参按形参类型校验（规范 5.3）
            let sig = funcs
                .get(name)
                .ok_or_else(|| err(0, format!("函数 '{}' 未定义", name)))?;
            if sig.params.len() != args.len() {
                return Err(err(
                    0,
                    format!("函数 '{}' 期望 {} 个参数，实际 {} 个", name, sig.params.len(), args.len()),
                ));
            }
            for (a, pt) in args.iter().zip(&sig.params) {
                let at = ty_of(a, scope, funcs, structs)?;
                if !value_assignable(*pt, at, a) {
                    return Err(err(
                        0,
                        format!("函数 '{}' 的参数期望 {}，实际 {}", name, pt.label(), at.label()),
                    ));
                }
            }
            Ok(sig.ret)
        }
        Expr::Convert { name, arg } => {
            let t = ty_of(arg, scope, funcs, structs)?;
            if name == "toi" {
                match t {
                    Ty::I32 | Ty::I64 | Ty::F64 => Ok(Ty::I64),
                    Ty::Str => match arg.as_ref() {
                        Expr::Str(s) => s
                            .parse::<i64>()
                            .map(|_| Ty::I64)
                            .map_err(|_| err(0, format!("字符串 '{}' 无法转换为整数", s))),
                        _ => Err(err(0, "toi 不能用于非字面量字符串（v0.1）")),
                    },
                    other => Err(err(0, format!("toi 不能用于 {}", other.label()))),
                }
            } else {
                // tos：数字/bool → str；str 原样
                Ok(Ty::Str)
            }
        }
        // v2.1：借用表达式类型为 &str（仅出现在 &str 形参实参位，规范 11.6）
        Expr::Borrow(inner) => {
            let t = ty_of(inner, scope, funcs, structs)?;
            if t == Ty::Str {
                Ok(Ty::BorrowStr)
            } else {
                Err(err(0, format!("只能借用 str，实际 {}", t.label())))
            }
        }
    }
}

/// 天权 v2.0 / v3.0：递归收集语句块内声明的全部**受所有权约束**的变量
/// （str 与结构体，按首次出现顺序去重）。
/// 供两个后端做"槽位提升 + 作用域出口释放"（规范 11.3 / 14.4）。
pub fn collect_str_decls(
    stmts: &[Stmt],
    scope: &mut HashMap<String, Ty>,
    sigs: &HashMap<String, FuncSig>,
    structs: &[StructDef],
    out: &mut Vec<String>,
) {
    for s in stmts {
        match s {
            Stmt::Decl { name, ty, value, .. } => {
                let t = (*ty)
                    .unwrap_or_else(|| ty_of(value, scope, sigs, structs).unwrap_or(Ty::I64));
                scope.insert(name.clone(), t);
                if is_owned(t) && !out.contains(name) {
                    out.push(name.clone());
                }
            }
            Stmt::While { cond, body, .. } => {
                let _ = ty_of(cond, scope, sigs, structs);
                collect_str_decls(body, scope, sigs, structs, out);
            }
            Stmt::If { body, else_body, .. } => {
                collect_str_decls(body, scope, sigs, structs, out);
                if let Some(eb) = else_body {
                    collect_str_decls(eb, scope, sigs, structs, out);
                }
            }
            _ => {}
        }
    }
}

/// 天权 v2.0：判断 expr 是否在"移动位"消费了变量 name 的所有权
/// （如 `a = g(a)` 中 g 以 str 形参接管 a）——此时重新绑定前不可再释放旧值，
/// 否则与被调函数出口的释放构成双重释放。
pub fn consumes_var(
    e: &Expr,
    name: &str,
    is_top: bool,
    sigs: &HashMap<String, FuncSig>,
) -> bool {
    match e {
        Expr::Var(n) => is_top && n == name,
        Expr::Neg(x) => consumes_var(x, name, false, sigs),
        Expr::Bin { lhs, rhs, .. } => {
            consumes_var(lhs, name, false, sigs) || consumes_var(rhs, name, false, sigs)
        }
        Expr::Call { name: fname, args } => sigs
            .get(fname)
            .map(|sig| {
                args.iter()
                    .zip(&sig.params)
                    // v3.0：str 与结构体形参都接管所有权（规范 14.4）
                    .any(|(a, pt)| is_owned(*pt) && consumes_var(a, name, true, sigs))
            })
            .unwrap_or(false),
        // copy()/tos() 参数只借用（规范 11.2.4）
        Expr::Convert { arg, .. } => consumes_var(arg, name, false, sigs),
        _ => false,
    }
}

/// 语句/表达式级检查（带天权移动追踪）：类型 + use-after-move 一次性完成。
/// ty_of（纯函数）仅供代码生成复用；此处逻辑与其保持一致并叠加天权规则。
fn check_expr(
    expr: &Expr,
    scope: &mut Scope,
    funcs: &HashMap<String, FuncSig>,
    structs: &[StructDef],
    line: usize,
) -> Result<Ty, CheckError> {
    let check = |e: &Expr, s: &mut Scope| check_expr(e, s, funcs, structs, line);
    match expr {
        Expr::Int(_) => Ok(Ty::I64),
        Expr::Float(_) => Ok(Ty::F64),
        Expr::Str(_) => Ok(Ty::Str),
        Expr::Bool(_) => Ok(Ty::Bool),
        Expr::Var(name) => match scope.get(name) {
            None => {
                let mut e = err(line, format!("变量 '{}' 未声明", name));
                let names: Vec<String> = scope.keys().cloned().collect();
                e.candidates = similar_candidates(name, &names);
                if !e.candidates.is_empty() {
                    e.hint = format!("是否想用 '{}'？", e.candidates[0]);
                }
                Err(e)
            }
            // v3.0：结构体同样是移动语义（规范 14.4）
            Some(v) if is_owned(v.ty) && v.moved => Err(err(
                line,
                format!("变量 '{}' 的值已被移动，不能再使用（天权 11.2；需要副本请用 copy()）", name),
            )),
            Some(v) => Ok(v.ty),
        },
        Expr::Neg(e) => {
            let t = check(e, scope)?;
            if t.is_numeric() {
                Ok(t)
            } else {
                Err(err(line, format!("类型 {} 不能取负号", t.label())))
            }
        }
        Expr::Bin { op, lhs, rhs } => {
            let lt = norm(check(lhs, scope)?);
            let rt = norm(check(rhs, scope)?);
            if matches!(
                op,
                BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge | BinOp::Eq | BinOp::Ne
            ) {
                let ok = match (lt, rt) {
                    (a, b) if a.is_numeric() && b.is_numeric() => true,
                    (Ty::Str, Ty::Str) => matches!(op, BinOp::Eq | BinOp::Ne),
                    (Ty::Bool, Ty::Bool) => matches!(op, BinOp::Eq | BinOp::Ne),
                    _ => false,
                };
                if ok {
                    Ok(Ty::Bool)
                } else {
                    Err(err(line, format!("类型 {} 与 {} 不能用 {} 比较", lt.label(), rt.label(), op.c_str())))
                }
            } else if lt.is_numeric() && rt.is_numeric() {
                Ok(if lt == Ty::F64 || rt == Ty::F64 { Ty::F64 } else { Ty::I64 })
            } else if *op == BinOp::Add && lt == Ty::Str && rt == Ty::Str {
                Ok(Ty::Str)
            } else if lt == Ty::Str || rt == Ty::Str {
                Err(err(line, format!("str 与数字不能混算 {}，请用 tos() 显式转换", op.c_str())))
            } else {
                Err(err(line, format!("类型 {} 与 {} 不参与算术运算 {}", lt.label(), rt.label(), op.c_str())))
            }
        }
        Expr::Borrow(_) => {
            // 借用只允许出现在 &str 形参实参位置（Call 分支已特判），其余位置非法（规范 11.6.3）
            Err(err(line, "借用 & 只能作为 &str 形参的实参使用（v2.1）"))
        }
        Expr::Call { name, args } => {
            let sig = match funcs.get(name) {
                Some(s) => s.clone(),
                None => {
                    let mut e = err(line, format!("函数 '{}' 未定义", name));
                    let names: Vec<String> = funcs.keys().cloned().collect();
                    e.candidates = similar_candidates(name, &names);
                    if !e.candidates.is_empty() {
                        e.hint = format!("是否想调用 '{}'？", e.candidates[0]);
                    }
                    return Err(e);
                }
            };
            if sig.params.len() != args.len() {
                return Err(err(
                    line,
                    format!("函数 '{}' 期望 {} 个参数，实际 {} 个", name, sig.params.len(), args.len()),
                ));
            }
            // 天权：str 形参按序移动实参所有权（规范 11.2）；&str 形参只借用（规范 11.6）
            for (a, pt) in args.iter().zip(&sig.params) {
                if *pt == Ty::BorrowStr {
                    // 借用形参：实参必须是 &变量（不移动源，规范 11.6.1/11.6.2），
                    // 或字符串字面量（静态存储，天然只读出借）
                    match a {
                        Expr::Str(_) => continue,
                        Expr::Borrow(inner) => {
                            if !matches!(inner.as_ref(), Expr::Var(_)) {
                                return Err(err(line, "借用 & 只能作用于 str 变量"));
                            }
                            let t = check(inner, scope)?;
                            if t != Ty::Str {
                                return Err(err(line, format!("只能借用 str，实际 {}", t.label())));
                            }
                            continue;
                        }
                        _ => {
                            return Err(err(
                                line,
                                format!("函数 '{}' 的形参是借用 &str，实参需加 & 前缀（借用不移动源变量）", name),
                            ))
                        }
                    }
                }
                if *pt == Ty::Str && matches!(a, Expr::Borrow(_)) {
                    return Err(err(
                        line,
                        format!("函数 '{}' 的形参 str 接管所有权，实参不能是借用；去掉 & 传值，或将形参改为 &str", name),
                    ));
                }
                let at = check(a, scope)?;
                if !value_assignable(*pt, at, a) {
                    return Err(err(
                        line,
                        format!("函数 '{}' 的参数期望 {}，实际 {}", name, pt.label(), at.label()),
                    ));
                }
                // v3.0：str 与结构体形参都接管所有权（规范 14.4）
                if is_owned(*pt) {
                    if matches!(a, Expr::Field(..)) {
                        return Err(err(
                            line,
                            format!(
                                "不能把结构体字段移出传给 '{}'（字段只能借用：调用 copy(p.x) 取副本）",
                                name
                            ),
                        ));
                    }
                    mark_top_move(a, scope);
                }
            }
            Ok(sig.ret)
        }
        // v3.0：p.x —— 字段读取（规范 14.3）。读取字段只读借用，不移动结构体本身
        Expr::Field(base, fname) => {
            let bt = norm(check(base, scope)?);
            let sd = struct_by_id(structs, bt).ok_or_else(|| {
                err(
                    line,
                    format!("类型 {} 没有字段（只有结构体能用 . 取字段）", ty_label(bt, structs)),
                )
            })?;
            field_ty(sd, fname).ok_or_else(|| {
                let mut e = err(line, format!("结构体 '{}' 没有字段 '{}'", sd.name, fname));
                e.candidates =
                    similar_candidates(fname, &sd.fields.iter().map(|f| f.name.clone()).collect::<Vec<_>>());
                if !e.candidates.is_empty() {
                    e.hint = format!("是否想用 '{}'？", e.candidates[0]);
                }
                e
            })
        }
        // v3.0：Point{...} —— 构造结构体值（规范 14.2）
        Expr::StructLit { name, fields, .. } => {
            let (sid, sd) = struct_by_name(structs, name)
                .ok_or_else(|| err(line, format!("未知结构体 '{}'", name)))?;
            let ty = Ty::Struct(sid);
            let named = fields.iter().any(|(n, _)| n.is_some());
            if named && fields.iter().any(|(n, _)| n.is_none()) {
                return Err(err(
                    line,
                    format!("结构体 '{}' 的字面量不能混用位置式与命名式字段", name),
                ));
            }
            if fields.len() != sd.fields.len() {
                return Err(err(
                    line,
                    format!(
                        "结构体 '{}' 期望 {} 个字段，实际 {} 个",
                        name,
                        sd.fields.len(),
                        fields.len()
                    ),
                ));
            }
            for (i, (fname_opt, v)) in fields.iter().enumerate() {
                let (fd_name, fd_ty) = if named {
                    let n = fname_opt.as_ref().unwrap();
                    let ft = field_ty(sd, n).ok_or_else(|| {
                        let mut e = err(line, format!("结构体 '{}' 没有字段 '{}'", name, n));
                        e.candidates = similar_candidates(
                            n,
                            &sd.fields.iter().map(|f| f.name.clone()).collect::<Vec<_>>(),
                        );
                        e
                    })?;
                    (n.clone(), ft)
                } else {
                    (sd.fields[i].name.clone(), sd.fields[i].ty)
                };
                let vt = check(v, scope)?;
                if !value_assignable(fd_ty, vt, v) {
                    return Err(err(
                        line,
                        format!(
                            "结构体 '{}' 的字段 '{}' 期望 {}，实际 {}",
                            name,
                            fd_name,
                            ty_label(fd_ty, structs),
                            ty_label(vt, structs)
                        ),
                    ));
                }
                // str 字段从变量接收所有权（变量失效，结构体成为新所有者）
                if fd_ty == Ty::Str {
                    mark_top_move(v, scope);
                }
            }
            Ok(ty)
        }
        Expr::Convert { name, arg } => {
            if name == "copy" {
                // copy(s)/copy(&s)：只借用源变量，产生新的独立所有者（规范 11.2.4/11.2.5/11.6.4）
                let t = check(arg, scope)?;
                if t != Ty::Str && t != Ty::BorrowStr {
                    return Err(err(line, format!("copy() 只能用于 str，实际 {}", t.label())));
                }
                return Ok(Ty::Str);
            }
            let t = check(arg, scope)?;
            if name == "toi" {
                match t {
                    Ty::I32 | Ty::I64 | Ty::F64 => Ok(Ty::I64),
                    Ty::Str => match arg.as_ref() {
                        Expr::Str(s) => s
                            .parse::<i64>()
                            .map(|_| Ty::I64)
                            .map_err(|_| err(line, format!("字符串 '{}' 无法转换为整数", s))),
                        _ => Err(err(line, "toi 不能用于非字面量字符串（v0.1）")),
                    },
                    other => Err(err(line, format!("toi 不能用于 {}", other.label()))),
                }
            } else {
                // tos：数字/bool → str；str 原样（只借用）
                Ok(Ty::Str)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex_spanned;
    use crate::parser::parse;

    fn check_src(src: &str) -> Result<(), String> {
        let prog = parse(lex_spanned(src).unwrap()).map_err(|e| e.to_string())?;
        check(&prog).map_err(|e| e.to_string())
    }

    #[test]
    fn test_valid_main_t() {
        let src = "# main.t\n/a=20\n`a/2\n//cnt=1\nw/cnt<=3{\n    cnt=cnt+1\n    `cnt/a\n}\nf/add(x,y){\n    /res=x+y\n    `res\n}\nadd(10,90)\n";
        assert!(check_src(src).is_ok());
    }

    #[test]
    fn test_readonly_assign_rejected() {
        let e = check_src("/a=1\na=2\n").unwrap_err();
        assert!(e.contains("只读"), "实际报错：{}", e);
    }

    #[test]
    fn test_undeclared_var() {
        let e = check_src("`b\n").unwrap_err();
        assert!(e.contains("未声明"));
    }

    #[test]
    fn test_str_number_mix() {
        assert!(check_src("`\"a\"+1\n").is_err());
        assert!(check_src("/a=\"hi\"\n`a*2\n").is_err());
    }

    #[test]
    fn test_cond_must_be_bool() {
        let e = check_src("//x=1\nw/x{\n`x\n}\n").unwrap_err();
        assert!(e.contains("bool"), "实际报错：{}", e);
    }

    #[test]
    fn test_v03_type_annotations() {
        // 全量类型标注：str 参数与返回值
        assert!(check_src("f/join(a:str, b:str):str{\nr/a+b\n}\n`join(\"天\", \"道\")\n").is_ok());
        // 实参类型不匹配
        assert!(check_src("f/g(n:str){`n\ng(1)\n}\n").is_err());
        assert!(check_src("f/g(n:i64){`n\ng(\"x\")\n}\n").is_err());
        // 返回类型不匹配
        assert!(check_src("f/g():i64{\nr/\"a\"\n}\n").is_err());
        assert!(check_src("f/g():str{\nr/\"a\"\n}\n").is_ok());
        // 整数隐式提升为 f64
        assert!(check_src("f/g():f64{\nr/3\n}\n`g()\n").is_ok());
        // 调用表达式类型 = 返回类型
        assert!(check_src("f/g():f64{\nr/1.5\n}\n`g()*2\n").is_ok());
        // 裸 r/ 在任意返回类型下合法
        assert!(check_src("f/g():str{\nr/\n}\n").is_ok());
    }

    #[test]
    fn test_v03_str_concat() {
        assert!(check_src("`\"天\"+\"道\"\n").is_ok());
        assert!(check_src("/a=\"x\"\n//b=\"y\"\n`a+b+a\n").is_ok());
        // str 与数字仍禁混算
        assert!(check_src("`\"a\"+1\n").is_err());
        assert!(check_src("`tos(1)+\"a\"\n").is_ok());
        // str 不支持减乘除
        assert!(check_src("`\"a\"-\"b\"\n").is_err());
    }

    #[test]
    fn test_function_checks() {
        assert!(check_src("f/add(x,y){`x}\nadd(1)\n").is_err()); // 参数个数
        assert!(check_src("sub(1,2)\n").is_err()); // 未定义
        // v0.2：调用可作表达式与返回值
        assert!(check_src("f/g(x){r/x\n}\n/a=g(1)\n`a+g(2)\n").is_ok());
        // 返回值须为整数
        assert!(check_src("f/g(x){r/1.5\n}\n").is_err());
        // r/ 不能出现在顶层
        assert!(check_src("r/1\n").is_err());
    }

    #[test]
    fn test_type_annotation() {
        assert!(check_src("/num:i32=10\n").is_ok());
        assert!(check_src("/num:i32=99999999999\n").is_err()); // 超出 i32
        assert!(check_src("/s:i32=\"hi\"\n").is_err());
    }

    #[test]
    fn test_conversions() {
        assert!(check_src("`toi(\"42\")\n").is_ok());
        assert!(check_src("`toi(\"abc\")\n").is_err());
        assert!(check_src("/x=3.7\n`toi(x)\n").is_ok());
        assert!(check_src("`tos(42)\n").is_ok());
    }

    // ---------- 天权 v2.0：所有权与移动 ----------

    #[test]
    fn test_move_then_use_rejected() {
        // 移动后使用 = 编译期错误（规范 11.2.2）
        let e = check_src("//s=\"a\"\n//t=s\n`s\n").unwrap_err();
        assert!(e.contains("已被移动"), "实际报错：{}", e);
        // 打印也使用
        assert!(check_src("//s=\"a\"\n//t=s\n`s==\"a\"\n").is_err());
        // 传参也是使用
        let e = check_src("f/g(x:str){`x\n}\n//s=\"a\"\ng(s)\ng(s)\n").unwrap_err();
        assert!(e.contains("已被移动"), "实际报错：{}", e);
    }

    #[test]
    fn test_move_and_revive() {
        // 失效变量可通过 // 重新赋值复活（规范 11.2.3）
        assert!(check_src("//s=\"a\"\n//t=s\ns=\"b\"\n`s\n").is_ok());
        // 只读变量不能复活
        let e = check_src("/s=\"a\"\n//t=s\ns=\"b\"\n").unwrap_err();
        assert!(e.contains("只读"), "实际报错：{}", e);
    }

    #[test]
    fn test_copy_keeps_source() {
        // copy() 只借用：源变量继续可用（规范 11.2.4/11.2.5）
        assert!(check_src("//s=\"a\"\n//t=copy(s)\n`s\n`t\n").is_ok());
        // copy 只能用于 str
        assert!(check_src("`copy(1)\n").is_err());
    }

    #[test]
    fn test_borrow_positions_do_not_move() {
        // 比较、拼接、打印只借用（规范 11.2.4）
        assert!(check_src("//s=\"a\"\n`s==\"a\"\n`s!=\"b\"\n`s\n").is_ok());
        assert!(check_src("//s=\"a\"\n//t=s+\"b\"\n`s\n").is_ok());
        assert!(check_src("//s=\"a\"\n`copy(s)\n`s\n").is_ok());
    }

    #[test]
    fn test_str_call_moves_arg() {
        // 传参 = 移动（规范 11.2.1）
        let e = check_src("f/g(x:str){`x\n}\n//s=\"a\"\n//t=g(s)\n`s\n").unwrap_err();
        assert!(e.contains("已被移动"), "实际报错：{}", e);
        // 返回 str = 移动到调用方
        assert!(check_src("f/g():str{\n//x=\"a\"\nr/x\n}\n//s=g()\n`s\n").is_ok());
        // 表达式内的调用同样移动 str 实参
        let e = check_src("f/g(x:str):i64{\nr/1\n}\n//s=\"a\"\n/g=g(s)+1\n`s\n").unwrap_err();
        assert!(e.contains("已被移动"), "实际报错：{}", e);
    }

    #[test]
    fn test_move_in_branch_is_conservative() {
        // 任一分支中移动即视为已移动（规范 11.4.3）
        let src = "//s=\"a\"\n//c=1\ni/c==1{\n//t=s\n}\ne/{\n`s\n}\n`s\n";
        assert!(check_src(src).is_err());
        let e = check_src(src).unwrap_err();
        assert!(e.contains("已被移动"), "实际报错：{}", e);
    }

    // ---------- 天权 v2.1：借用 &s / &str（规范 11.6） ----------

    #[test]
    fn test_borrow_ok() {
        // 借用传参不移动源：调用后 s 仍可用、仍可移动（规范 11.6.1/11.6.2）
        assert!(check_src("f/len(x:&str):i64{\nr/1\n}\n//s=\"a\"\n/g=len(&s)\n`s\n").is_ok());
        assert!(check_src("f/len(x:&str):i64{\nr/1\n}\n//s=\"a\"\n/g=len(&s)\n//t=s\n").is_ok());
        // 借用形参可拼接、可比较、可打印（只读位视同 str）
        assert!(check_src("f/shout(x:&str):str{\nr/x+\"!\"\n}\n").is_ok());
        assert!(check_src("f/eq(x:&str):bool{\nr/x==\"a\"\n}\n").is_ok());
        // copy(&s)：从借用产生独立所有者（逃逸的合法路径）
        assert!(check_src("f/dup(x:&str):str{\nr/copy(x)\n}\n").is_ok());
    }

    #[test]
    fn test_borrow_rejects() {
        // 借用形参的实参缺 & 前缀
        let e = check_src("f/len(x:&str):i64{\nr/1\n}\n//s=\"a\"\nlen(s)\n").unwrap_err();
        assert!(e.contains("& 前缀"), "实际报错：{}", e);
        // 值形参（接管所有权）收到借用
        let e = check_src("f/g(x:str){`x\n}\n//s=\"a\"\ng(&s)\n").unwrap_err();
        assert!(e.contains("不能是借用"), "实际报错：{}", e);
        // 借用逃逸：返回借用形参
        let e = check_src("f/g(x:&str):str{\nr/x\n}\n").unwrap_err();
        assert!(e.contains("逃逸"), "实际报错：{}", e);
        // 返回类型不能是借用
        let e = check_src("f/g():&str{\nr/\"a\"\n}\n").unwrap_err();
        assert!(e.contains("借用"), "实际报错：{}", e);
        // 借用不能绑定给 str 变量（取得所有权）
        let e = check_src("f/g(x:&str):i64{\n//t=x\nr/1\n}\n").unwrap_err();
        assert!(e.contains("所有权"), "实际报错：{}", e);
        // 借用在其他表达式位置非法
        assert!(check_src("//s=\"a\"\n//t=&s\n").is_err());
        // 对已移动变量借用 = use-after-move
        let e = check_src("f/len(x:&str):i64{\nr/1\n}\n//s=\"a\"\n//t=s\nlen(&s)\n").unwrap_err();
        assert!(e.contains("已被移动"), "实际报错：{}", e);
    }

    #[test]
    fn test_contract_ok() {
        // v2.2 语义锚点：合法 @pre/@post/@example 通过检查（规范第 12 节）
        assert!(check_src(
            "f/wrap(s:&str):str{\n@pre: s != \"\"\n@post: ret != \"\"\n@example: wrap(\"hi\") -> \"hi!\"\nr/ s+\"!\"\n}\n"
        )
        .is_ok());
        // 契约可与借用字面量实参共存（@example 内 wrap("hi") 走静态借用）
        assert!(check_src(
            "f/w(s:&str):i64{\n@pre: s != \"\"\n@example: w(\"x\") -> 1\nr/ 1\n}\n"
        )
        .is_ok());
    }

    #[test]
    fn test_contract_rejects() {
        // @pre 非 bool
        let e = check_src("f/g(x:i64):i64{\n@pre: x+1\nr/ x\n}\n").unwrap_err();
        assert!(e.contains("@pre 必须"), "实际报错：{}", e);
        // @post 非 bool（ret 伪变量参与算术 → i64）
        let e = check_src("f/g(x:i64):i64{\n@post: ret+1\nr/ x\n}\n").unwrap_err();
        assert!(e.contains("@post 必须"), "实际报错：{}", e);
        // @example 必须调用本函数自身
        let e = check_src("f/g():i64{\nr/ 1\n}\nf/h():i64{\n@example: g() -> 1\nr/ 2\n}\n").unwrap_err();
        assert!(e.contains("@example 必须"), "实际报错：{}", e);
        // @example 期望值类型与返回类型不符
        let e = check_src("f/g():i64{\n@example: g() -> \"a\"\nr/ 1\n}\n").unwrap_err();
        assert!(e.contains("不符"), "实际报错：{}", e);
        // 未知锚点（解析期拦截）
        assert!(check_src("f/g():i64{\n@foo: 1\nr/ 1\n}\n").is_err());
    }
}
