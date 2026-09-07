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
    /// v4.1：是否为函数形参（动态数组 push/pop 允许在形参上原地修改后交还）
    pub is_param: bool,
    /// v4.3：是否为动态数组借用形参 &[]T（只读视图，规范 22.8）
    pub is_borrow: bool,
}

impl VarInfo {
    pub fn new(ty: Ty, mutable: bool) -> Self {
        VarInfo {
            ty,
            mutable,
            moved: false,
            is_param: false,
            is_borrow: false,
        }
    }

    /// v4.1：形参绑定（只读，但动态数组允许 push/pop 原地修改后交还）
    pub fn new_param(ty: Ty, is_borrow: bool) -> Self {
        VarInfo {
            ty,
            mutable: false,
            moved: false,
            is_param: true,
            is_borrow,
        }
    }
}

pub type Scope = HashMap<String, VarInfo>;

/// 函数签名（v0.3：全量类型标注，省略默认 i64）
#[derive(Debug, Clone)]
pub struct FuncSig {
    pub params: Vec<Ty>,
    /// v4.3：与 params 对齐——动态数组借用形参标记（不移动、不置空源）
    pub borrows: Vec<bool>,
    pub ret: Ty,
}

/// 入口：校验整个程序
pub fn check(prog: &Program) -> Result<(), CheckError> {
    // v3.3/v4.0：数组类型表供 ty_label 报错使用
    set_arrs(&prog.arrs);
    set_darrs(&prog.darrs);
    set_tuples(&prog.tuples);
    set_maps(&prog.maps);
    // 函数表（定义顺序无关）
    let mut funcs: HashMap<String, FuncSig> = HashMap::new();
    for f in &prog.funcs {
        let sig = FuncSig {
            params: f.params.iter().map(|p| p.ty.unwrap_or(Ty::I64)).collect(),
            borrows: f.params.iter()
                .map(|p| p.borrow && p.ty.map(|t| t.is_darr() || t.is_map()).unwrap_or(false))
                .collect(),
            ret: f.ret.unwrap_or(Ty::I64),
        };
        if funcs.insert(f.name.clone(), sig).is_some() {
            return Err(err(0, format!("函数 '{}' 重复定义", f.name)));
        }
        // v2.1：返回类型不能是借用（借用不可逃逸，规范 11.6.5）
        if f.ret == Some(Ty::BorrowStr) {
            return Err(err(0, format!("函数 '{}' 的返回类型不能是借用 &str", f.name)));
        }
        // v3.4：数组可作为参数（只读视图传递，参数本就不可写，规范 16.6）；返回值仍不支持
        if f.ret.map(|t| t.is_arr()).unwrap_or(false) {
            return Err(err(
                0,
                format!("函数 '{}' 的返回类型不能是数组（规范 16.5）", f.name),
            ));
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
            // v3.3/v4.0：数组不能作为结构体字段
            if f.ty.is_arr() || f.ty.is_darr() {
                return Err(err(
                    f.line,
                    format!("结构体 '{}' 的字段 '{}' 不能是数组类型（v3.3 暂不支持）", sd.name, f.name),
                ));
            }
            // v4.7：map 不能作为结构体字段（本轮不支持含容器的结构体，规范 25.5）
            if f.ty.is_map() {
                return Err(err(
                    f.line,
                    format!(
                        "结构体 '{}' 的字段 '{}' 不能是 map 类型（v4.7 暂不支持嵌套容器）",
                        sd.name, f.name
                    ),
                ));
            }
        }
    }

    // 顶层语句（全局作用域，r/ 禁止出现在顶层）
    let mut top_scope: Scope = HashMap::new();
    for s in &prog.top {
        check_stmt(s, &mut top_scope, &funcs, &prog.structs, false, Ty::I64, false)?;
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
                    VarInfo::new_param(p.ty.unwrap_or(Ty::I64), p.borrow),
                )
            })
            .collect();
        for s in &f.body {
            check_stmt(s, &mut scope, &funcs, &prog.structs, true, ret, false)?;
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
        .map(|p| (p.name.clone(), VarInfo::new_param(p.ty.unwrap_or(Ty::I64), p.borrow)))
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
                // v4.6：@post 不支持元组返回值（元组不可比较/不可访问单个元素，规范 24.6）
                // 从编译期拦截，杜绝后端静默跳过
                if ret.is_tuple() {
                    return Err(err(
                        *line,
                        "@post 不支持元组返回值（元组不可比较/不可访问单个元素，规范 24.6）",
                    ));
                }
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
    in_loop: bool,
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
                // v4.2：str 动态数组元素是容器的借用视图，不可取得所有权（容器负责释放，规范 22.6）
                if let Expr::Index(base, _) = value {
                    let bty = ty_of(base, scope, funcs, structs).ok();
                    if let Some(Ty::DArr(_)) = bty {
                        let et = ty_of(value, scope, funcs, structs).ok();
                        if et == Some(Ty::Str) {
                            return Err(err(
                                *line,
                                "不能把动态数组的 str 元素移出为所有者（容器负责释放；需要副本用 copy(a[i])）",
                            ));
                        }
                    }
                }
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
        // v4.5：多声明解构 //a, b = f(...)（规范 24.3）
        // 元组仅存在于函数返回边界，因此右侧必须是返回元组的调用；
        // 每个元素按其类型接管所有权（天权按元素接管，规范 24.4）
        Stmt::MultiDecl {
            mutable,
            names,
            value,
            line,
        } => {
            let vt = check_expr(value, scope, funcs, structs, *line)?;
            let table = tuples();
            let td = match tuple_by_id(&table, vt) {
                Some(td) => td,
                None => {
                    return Err(err(
                        *line,
                        "解构赋值的右侧必须是返回元组的函数调用（元组仅存在于返回边界，不可作为值出现）",
                    ))
                }
            };
            if names.len() != td.elems.len() {
                return Err(err(
                    *line,
                    format!(
                        "解构数量不符：左侧 {} 个，元组 {} 个元素",
                        names.len(),
                        td.elems.len()
                    ),
                ));
            }
            for n in names {
                if scope.contains_key(n) {
                    return Err(err(*line, format!("变量 '{}' 重复声明", n)));
                }
            }
            for (i, n) in names.iter().enumerate() {
                let et = td.elems[i];
                // 借用不持有所有权，无法接管为独立变量（规范 11.6.5）
                if et == Ty::BorrowStr {
                    return Err(err(
                        *line,
                        format!("解构的第 {} 个元素是借用 &str，不能取得所有权", i + 1),
                    ));
                }
                scope.insert(n.clone(), VarInfo::new(et, *mutable));
            }
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
                Expr::Index(base, idx) => {
                    // v3.3：数组下标赋值 a[i] = e（规范 16.3）
                    let bn = match base.as_ref() {
                        Expr::Var(bn) => bn,
                        _ => return Err(err(*line, "数组下标赋值的左侧只能是数组变量（如 a[i]=1）")),
                    };
                    let info = match scope.get(bn) {
                        Some(v) => *v,
                        None => return Err(err(*line, format!("变量 '{}' 未声明", bn))),
                    };
                    let darr_param = info.is_param && info.ty.is_darr();
                    // v4.7：map 形参同 darr 形参——元素写入即对局部句柄重绑（扩容换指针，见下 map 分支）
                    let map_param = info.is_param && info.ty.is_map();
                    if !info.mutable && !darr_param && !map_param {
                        return Err(err(
                            *line,
                            format!("变量 '{}' 是只读的（/ 声明），不能修改其元素", bn),
                        ));
                    }
                    let it = check_expr(idx, scope, funcs, structs, *line)?;
                    // v4.7：m[k] = v —— 整体移动/重绑语义（扩容可能换指针）
                    if let Some(m) = map_by_id(&maps(), info.ty) {
                        if info.is_borrow {
                            return Err(err(
                                *line,
                                format!("变量 '{}' 是借用 &map[..]（只读视图），不能写元素", bn),
                            ));
                        }
                        if !info.mutable && !map_param {
                            return Err(err(
                                *line,
                                format!("变量 '{}' 是只读的（/ 声明），map 元素写入需重绑变量，请用 // 声明", bn),
                            ));
                        }
                        if !value_assignable(m.key, it, idx) {
                            return Err(err(
                                *line,
                                format!("map 键期望 {}，实际 {}", m.key.label(), ty_label(it, structs)),
                            ));
                        }
                        (Some(bn.clone()), m.val)
                    } else if !matches!(it, Ty::I32 | Ty::I64) {
                        return Err(err(
                            *line,
                            format!("数组下标必须是整数，实际 {}", ty_label(it, structs)),
                        ));
                    } else if norm(info.ty) == Ty::Str {
                        // v4.2：str 不可按下标赋值（str 不可变，规范第 23 节）
                        return Err(err(*line, "str 不可按下标赋值（str 不可变；用拼接构造新串）"));
                    } else if info.is_borrow {
                        return Err(err(
                            *line,
                            format!("变量 '{}' 是借用 &[]T（只读视图），不能修改其元素", bn),
                        ));
                    } else {
                        // v4.0：动态数组下标赋值——长度运行时决定，仅静态检查（规范第 22 节）
                        let dtable = darrs();
                        if let Some(d) = darr_by_id(&dtable, info.ty) {
                            if let Expr::Neg(x) = &**idx {
                                if matches!(x.as_ref(), Expr::Int(_)) {
                                    return Err(err(*line, "数组下标不能为负"));
                                }
                            }
                            (None, d.elem)
                        } else {
                            let table = arrs();
                            let a = arr_by_id(&table, info.ty).ok_or_else(|| {
                                err(
                                    *line,
                                    format!("类型 {} 不能按下标赋值（左侧必须是数组或 map）", ty_label(info.ty, structs)),
                                )
                            })?;
                            if let Expr::Int(n) = &**idx {
                                if *n < 0 || *n as u64 >= a.len {
                                    return Err(err(
                                        *line,
                                        format!("下标 {} 越界：数组长度为 {}（合法范围 0..{}）", n, a.len, a.len - 1),
                                    ));
                                }
                            }
                            if let Expr::Neg(x) = &**idx {
                                if let Expr::Int(n) = x.as_ref() {
                                    return Err(err(
                                        *line,
                                        format!("下标 -{} 越界：数组下标不能为负（合法范围 0..{}）", n, a.len - 1),
                                    ));
                                }
                            }
                            (None, a.elem)
                        }
                    }
                }
                _ => return Err(err(*line, "赋值的左侧只能是变量、变量.字段 或 数组元素 a[i]")),
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
                // v4.2：str 动态数组元素是借用视图，不可取得所有权（规范 22.6）
                if let Expr::Index(base, _) = value {
                    let bty = ty_of(base, scope, funcs, structs).ok();
                    if let Some(Ty::DArr(_)) = bty {
                        let et = ty_of(value, scope, funcs, structs).ok();
                        if et == Some(Ty::Str) {
                            return Err(err(
                                *line,
                                "不能把动态数组的 str 元素移出为所有者（容器负责释放；需要副本用 copy(a[i])）",
                            ));
                        }
                    }
                }
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
            // v3.3/v4.0：数组整体不可打印
            if t.is_arr() || t.is_darr() {
                return Err(err(
                    *line,
                    format!(
                        "不能直接打印数组 '{}'（请打印元素如 `a[0]，或长度 `len(a)）",
                        ty_label(t, structs)
                    ),
                ));
            }
            // v4.6：元组整体不可直接打印（元组仅存在于返回边界与解构，规范 24.2/24.5）
            if t.is_tuple() {
                return Err(err(
                    *line,
                    format!(
                        "不能直接打印元组 '{}'（请用 destructuring `//a, b = f()` 后分别打印 a、b）",
                        ty_label(t, structs)
                    ),
                ));
            }
            // v4.7：map 整体不可直接打印（规范 25.5）
            if t.is_map() {
                return Err(err(
                    *line,
                    format!(
                        "不能直接打印 map '{}'（请打印具体键值，如 `has(m, \"x\")、`len(m) 或逐一 `m[k]）",
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
                    check_stmt(st, s, funcs, structs, in_fn, fn_ret, true)?;
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
                    check_stmt(st, s, funcs, structs, in_fn, fn_ret, in_loop)?;
                }
                Ok(())
            })?;
            if let Some(eb) = else_body {
                with_block_scope(scope, |s| {
                    for st in eb {
                        check_stmt(st, s, funcs, structs, in_fn, fn_ret, in_loop)?;
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
                // v4.3：借用动态数组形参不可作为返回值（借用不逃逸，规范 22.8）；
                // v4.7：借用 map 形参同理（规范 25.6）
                if fn_ret.is_darr() || fn_ret.is_map() {
                    if let Expr::Var(n) = expr {
                        if scope.get(n).map(|v| v.is_borrow).unwrap_or(false) {
                            return Err(err(
                                *line,
                                "借用 &[]T/&map 不可逃逸函数（借用形参不可返回；需要所有权请让调用方持有）",
                            ));
                        }
                    }
                }
                // v4.0：动态数组返回值只能是变量或函数调用（字面量无法在表达式位构造，规范 22.4）；
                // v4.7：map 同理（map[K]V 字面量必须先赋给变量）
                if (fn_ret.is_darr() || fn_ret.is_map()) && !matches!(expr, Expr::Var(_) | Expr::Call { .. }) {
                    return Err(err(
                        *line,
                        "动态数组/map 返回值只能是变量或函数调用（字面量请先赋给变量）",
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
        // v4.0：panic(msg) / check(cond, msg) 快速失败（规范第 21 节）
        Stmt::Panic(msg, line) => {
            let t = check_expr(msg, scope, funcs, structs, *line)?;
            if t != Ty::Str {
                return Err(err(*line, format!("panic 消息必须是 str，实际 {}", ty_label(t, structs))));
            }
            Ok(())
        }
        Stmt::Check(cond, msg, line) => {
            let ct = check_expr(cond, scope, funcs, structs, *line)?;
            if ct != Ty::Bool {
                return Err(err(*line, format!("check 条件必须是 bool，实际 {}", ty_label(ct, structs))));
            }
            let mt = check_expr(msg, scope, funcs, structs, *line)?;
            if mt != Ty::Str {
                return Err(err(*line, format!("check 消息必须是 str，实际 {}", ty_label(mt, structs))));
            }
            Ok(())
        }
        // v3.8：break / continue 仅限 w/ 循环体内（规范第 19 节）
        Stmt::Break(line) => {
            if !in_loop {
                return Err(err(*line, "break 只能出现在 w/ 循环体内"));
            }
            Ok(())
        }
        Stmt::Continue(line) => {
            if !in_loop {
                return Err(err(*line, "continue 只能出现在 w/ 循环体内"));
            }
            Ok(())
        }
        Stmt::Expr(e, line) => match e {
            Expr::Call { .. } => {
                check_expr(e, scope, funcs, structs, *line)?;
                Ok(())
            }
            // v4.0：push(a, v) 语句 —— a 必须是可变的动态数组变量（规范第 22 节）
            Expr::Push { arr, value } => {
                let bn = match arr.as_ref() {
                    Expr::Var(bn) => bn,
                    _ => return Err(err(*line, "push 的实参必须是动态数组变量")),
                };
                let info = match scope.get(bn) {
                    Some(v) => *v,
                    None => return Err(err(*line, format!("变量 '{}' 未声明", bn))),
                };
                if info.is_borrow {
                    return Err(err(
                        *line,
                        format!("变量 '{}' 是借用 &[]T（只读视图），不能 push", bn),
                    ));
                }
                if !info.mutable && !info.is_param {
                    return Err(err(
                        *line,
                        format!("变量 '{}' 是只读的（/ 声明），不能 push", bn),
                    ));
                }
                let dtable = darrs();
                let d = darr_by_id(&dtable, info.ty).ok_or_else(|| {
                    err(*line, format!("类型 {} 不能 push（只有动态数组可以）", ty_label(info.ty, structs)))
                })?;
                let vt = check_expr(value, scope, funcs, structs, *line)?;
                if !value_assignable(d.elem, vt, value) {
                    return Err(err(
                        *line,
                        format!("push 元素期望 {}，实际 {}", d.elem.label(), ty_label(vt, structs)),
                    ));
                }
                // v4.2：str 元素移交所有权进容器（字面量由编译器 dup，规范 22.6）
                if d.elem == Ty::Str {
                    mark_top_move(value, scope);
                }
                Ok(())
            }
            // v4.1：pop(a) 语句 —— a 必须是可变的动态数组变量
            Expr::Pop { arr } => {
                let bn = match arr.as_ref() {
                    Expr::Var(bn) => bn,
                    _ => return Err(err(*line, "pop 的实参必须是动态数组变量")),
                };
                let info = match scope.get(bn) {
                    Some(v) => *v,
                    None => return Err(err(*line, format!("变量 '{}' 未声明", bn))),
                };
                if info.is_borrow {
                    return Err(err(
                        *line,
                        format!("变量 '{}' 是借用 &[]T（只读视图），不能 pop", bn),
                    ));
                }
                if !info.mutable && !info.is_param {
                    return Err(err(
                        *line,
                        format!("变量 '{}' 是只读的（/ 声明），不能 pop", bn),
                    ));
                }
                if darr_by_id(&darrs(), info.ty).is_none() {
                    return Err(err(
                        *line,
                        format!("类型 {} 不能 pop（只有动态数组可以）", ty_label(info.ty, structs)),
                    ));
                }
                Ok(())
            }
            // v4.7：del(m, k) 语句 —— m 必须是可变的（非借用）map 变量；键类型匹配
            Expr::Del { map, key } => {
                let bn = match map.as_ref() {
                    Expr::Var(bn) => bn,
                    _ => return Err(err(*line, "del 的第一个实参必须是 map 变量")),
                };
                let info = match scope.get(bn) {
                    Some(v) => *v,
                    None => return Err(err(*line, format!("变量 '{}' 未声明", bn))),
                };
                if info.is_borrow {
                    return Err(err(
                        *line,
                        format!("变量 '{}' 是借用 &map[..]（只读视图），不能 del", bn),
                    ));
                }
                let table = maps();
                let m = map_by_id(&table, info.ty).ok_or_else(|| {
                    err(*line, format!("类型 {} 不能 del（只有 map 可以）", ty_label(info.ty, structs)))
                })?;
                let kt = check_expr(key, scope, funcs, structs, *line)?;
                if !value_assignable(m.key, kt, key) {
                    return Err(err(
                        *line,
                        format!("del 键期望 {}，实际 {}", m.key.label(), ty_label(kt, structs)),
                    ));
                }
                Ok(())
            }
            _ => Err(err(*line, "语句级表达式只能是函数调用、push、pop 或 del")),
        },
    }
}

/// v3.7：sel 分支类型统一（规范第 18 节）——同型或按提升规则统一到更大类型；
/// 仅数值与 bool（str/结构体/数组分支会造成堆值泄漏或语义复杂化，不支持）
fn sel_unify(at: Ty, bt: Ty, structs: &[StructDef], line: usize) -> Result<Ty, CheckError> {
    let ok_ty = |t: Ty| matches!(t, Ty::I32 | Ty::I64 | Ty::F64 | Ty::Bool);
    if !ok_ty(at) || !ok_ty(bt) {
        return Err(err(
            line,
            format!(
                "sel 分支只能是数值或 bool（实际 {} 与 {}）；str/结构体/数组不支持",
                ty_label(at, structs),
                ty_label(bt, structs)
            ),
        ));
    }
    if at == bt {
        return Ok(at);
    }
    match (at, bt) {
        (Ty::I64, Ty::I32) | (Ty::I32, Ty::I64) => Ok(Ty::I64),
        (Ty::F64, Ty::I32) | (Ty::I32, Ty::F64) | (Ty::F64, Ty::I64) | (Ty::I64, Ty::F64) => Ok(Ty::F64),
        _ => Err(err(
            line,
            format!("sel 两分支类型不符：{} 与 {}", ty_label(at, structs), ty_label(bt, structs)),
        )),
    }
}

/// 传参 / 返回值兼容规则（规范 5.3）：同型；I32→I64 提升；整数→F64 提升；
/// I32 目标仅接受范围内整数字面量
fn ty_assignable(expected: Ty, actual: Ty) -> bool {    if expected == actual {
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
    t == Ty::Str || t.is_struct() || t.is_darr() || t.is_map()
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
        Ty::Arr(i) => arrs()
            .get(i as usize)
            .map(|a| format!("{}[{}]", a.elem.label(), a.len))
            .unwrap_or_else(|| "array".into()),
        Ty::DArr(i) => darrs()
            .get(i as usize)
            .map(|d| format!("[]{}", d.elem.label()))
            .unwrap_or_else(|| "darr".into()),
        Ty::Map(i) => maps()
            .get(i as usize)
            .map(|m| format!("map[{}]{}", m.key.label(), m.val.label()))
            .unwrap_or_else(|| "map".into()),
        other => other.label().to_string(),
    }
}

// ── v3.3 数组类型表（代码生成侧共享，机制同 STRUCTS）────────────
thread_local! {
    static ARRS: std::cell::RefCell<Vec<ArrDef>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// 设置当前线程的数组类型表（后端入口调用）
pub fn set_arrs(arrs: &[ArrDef]) {
    ARRS.with(|a| *a.borrow_mut() = arrs.to_vec());
}

/// 读取当前线程的数组类型表快照
pub fn arrs() -> Vec<ArrDef> {
    ARRS.with(|a| a.borrow().clone())
}

/// v3.3：按 Ty::Arr 索引取数组定义
pub fn arr_by_id<'a>(arrs: &'a [ArrDef], t: Ty) -> Option<&'a ArrDef> {
    match t {
        Ty::Arr(i) => arrs.get(i as usize),
        _ => None,
    }
}

// ── v4.0 动态数组类型表（机制同 ARRS）────────────────────────────
thread_local! {
    static DARRS: std::cell::RefCell<Vec<DArrDef>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// 设置当前线程的动态数组类型表（后端入口调用）
pub fn set_darrs(darrs: &[DArrDef]) {
    DARRS.with(|d| *d.borrow_mut() = darrs.to_vec());
}

/// 读取当前线程的动态数组类型表快照
pub fn darrs() -> Vec<DArrDef> {
    DARRS.with(|d| d.borrow().clone())
}

/// v4.0：按 Ty::DArr 索引取定义
pub fn darr_by_id<'a>(darrs: &'a [DArrDef], t: Ty) -> Option<&'a DArrDef> {
    match t {
        Ty::DArr(i) => darrs.get(i as usize),
        _ => None,
    }
}

// ── v4.5 元组类型表（机制同 ARRS/DARRS；规范第 24 节）────────────────
thread_local! {
    static TUPLES: std::cell::RefCell<Vec<TupleDef>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// 设置当前线程的元组类型表（检查入口与后端入口均调用）
pub fn set_tuples(tuples: &[TupleDef]) {
    TUPLES.with(|t| *t.borrow_mut() = tuples.to_vec());
}

/// 读取当前线程的元组类型表快照
pub fn tuples() -> Vec<TupleDef> {
    TUPLES.with(|t| t.borrow().clone())
}

/// v4.5：按 Ty::Tuple 索引取元组定义
pub fn tuple_by_id<'a>(tuples: &'a [TupleDef], t: Ty) -> Option<&'a TupleDef> {
    match t {
        Ty::Tuple(i) => tuples.get(i as usize),
        _ => None,
    }
}

// ── v4.7 关联数组类型表（机制同 ARRS/DARRS；规范第 25 节）────────────
thread_local! {
    static MAPS: std::cell::RefCell<Vec<MapDef>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// 设置当前线程的关联数组类型表（检查入口与后端入口均调用）
pub fn set_maps(maps: &[MapDef]) {
    MAPS.with(|m| *m.borrow_mut() = maps.to_vec());
}

/// 读取当前线程的关联数组类型表快照
pub fn maps() -> Vec<MapDef> {
    MAPS.with(|m| m.borrow().clone())
}

/// v4.7：按 Ty::Map 索引取关联数组定义
pub fn map_by_id<'a>(maps: &'a [MapDef], t: Ty) -> Option<&'a MapDef> {
    match t {
        Ty::Map(i) => maps.get(i as usize),
        _ => None,
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
        // v3.3：数组字面量类型 = 对应数组类型（元素校验在 check_expr）
        Expr::ArrLit { arr, .. } => {
            if arrs().get(*arr as usize).is_none() {
                return Err(err(0, "内部错误：数组类型索引越界"));
            }
            Ok(Ty::Arr(*arr))
        }
        // v3.3：a[i] 类型 = 元素类型（下标校验在 check_expr）
        Expr::Index(base, idx) => {
            let bt = norm(ty_of(base, scope, funcs, structs)?);
            // v4.7：m[k] 类型 = 值类型 V（键类型校验在 check_expr；map 键可为 str 或 i64，规范第 25 节）
            if let Some(m) = map_by_id(&maps(), bt) {
                return Ok(m.val);
            }
            let it = norm(ty_of(idx, scope, funcs, structs)?);
            if !matches!(it, Ty::I32 | Ty::I64) {
                return Err(err(0, format!("数组下标必须是整数，实际 {}", ty_label(it, structs))));
            }
            // v4.2：str 下标读 → 字节 i64（规范第 23 节）
            if bt == Ty::Str {
                return Ok(Ty::I64);
            }
            let table = arrs();
            if let Some(a) = arr_by_id(&table, bt) {
                return Ok(a.elem);
            }
            let dtable = darrs();
            if let Some(d) = darr_by_id(&dtable, bt) {
                return Ok(d.elem);
            }
            Err(err(0, format!("类型 {} 不能取下标（只有数组、str 和 map 能用 [i]）", ty_label(bt, structs))))
        }
        Expr::Sub { .. } => Ok(Ty::Str),
        // v4.0：动态数组字面量类型
        Expr::DArrLit { darr, .. } => {
            if darrs().get(*darr as usize).is_none() {
                return Err(err(0, "内部错误：动态数组类型索引越界"));
            }
            Ok(Ty::DArr(*darr))
        }
        // v4.7：map 字面量类型（条目标键/值类型校验在 check_expr）
        Expr::MapLit { map, .. } => {
            if maps().get(*map as usize).is_none() {
                return Err(err(0, "内部错误：关联数组类型索引越界"));
            }
            Ok(Ty::Map(*map))
        }
        // v4.7：has(m, k) → bool
        Expr::Has { .. } => Ok(Ty::Bool),
        // v4.7：del(m, k) 无值（仅语句级，规范第 25 节）
        Expr::Del { .. } => Err(err(0, "del 不产生值（只能作为语句）")),
        // v4.0/v4.1：push/pop 无值（仅语句级，规范第 22 节）
        // v4.2：sub 产生新所有权的堆串
        Expr::Sub { .. } => Ok(Ty::Str),
        Expr::Push { .. } => Err(err(0, "push 不产生值（只能作为语句）")),
        Expr::Pop { .. } => Err(err(0, "pop 不产生值（只能作为语句）")),
        // v3.3：len(a) 为编译期常量 i64；len(s) 为 UTF-8 字节数；v4.7：len(m) 返回键值对数
        Expr::Len(inner) => {
            let t = norm(ty_of(inner, scope, funcs, structs)?);
            if t.is_arr() || t == Ty::Str || t.is_darr() || t.is_map() {
                return Ok(Ty::I64);
            }
            Err(err(0, format!("len() 只能用于数组、str 或 map，实际 {}", ty_label(t, structs))))
        }
        // v4.7：keys(m) → 新拥有的 []K 键快照（map 或 &map 均可；规范 25.4）
        Expr::Keys(map) => {
            let bt = norm(ty_of(map, scope, funcs, structs)?);
            let ms = maps();
            let mde = map_by_id(&ms, bt)
                .ok_or_else(|| err(0, format!("keys() 实参必须是 map，实际 {}", ty_label(bt, structs))))?;
            let darr = darrs()
                .iter()
                .position(|d| d.elem == mde.key)
                .ok_or_else(|| err(0, "内部错误：map 键类型无对应动态数组"))? as u32;
            Ok(Ty::DArr(darr))
        }
        // v3.7：sel(cond, a, b) —— 仅数值/bool 分支；类型按提升规则统一（规范第 18 节）
        Expr::Sel { cond, a, b } => {
            let ct = norm(ty_of(cond, scope, funcs, structs)?);
            if ct != Ty::Bool {
                return Err(err(0, format!("sel 条件必须是 bool，实际 {}", ty_label(ct, structs))));
            }
            let at = norm(ty_of(a, scope, funcs, structs)?);
            let bt = norm(ty_of(b, scope, funcs, structs)?);
            sel_unify(at, bt, structs, 0)
        }
        // v4.5：元组表达式（仅返回边界合法，规范 24.2）
        Expr::TupExpr { elems, tup } => {
            let table = tuples();
            let td = table
                .get(*tup as usize)
                .ok_or_else(|| err(0, "内部错误：元组类型索引越界"))?;
            if elems.len() != td.elems.len() {
                return Err(err(0, "元组元素个数与声明不符（内部错误）"));
            }
            Ok(Ty::Tuple(*tup))
        }
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
        // v4.3：宽松推导（借用合法性由 check_expr 严格校验，规范 11.6 / 22.8 / 25.6）
        Expr::Borrow(inner) => {
            let t = ty_of(inner, scope, funcs, structs)?;
            if t == Ty::Str {
                Ok(Ty::BorrowStr)
            } else if t.is_darr() || t.is_map() {
                Ok(t)
            } else {
                Err(err(0, format!("只能借用 str、动态数组或 map，实际 {}", t.label())))
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
            if name == "tof" {
                // v4.4：tof(s) str → f64（运行时 strtod 全量消费，规范第 7 节）
                let nt = norm(t);
                if nt != Ty::Str {
                    return Err(err(0, format!("tof 实参必须是 str，实际 {}", ty_label(t, structs))));
                }
                return Ok(Ty::F64);
            }
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
            // v4.5：多声明解构——owned 元素各自成为堆槽（规范 24.3）
            Stmt::MultiDecl { names, value, .. } => {
                let vt = ty_of(value, scope, sigs, structs).unwrap_or(Ty::I64);
                let table = tuples();
                if let Some(td) = tuple_by_id(&table, vt) {
                    for (i, n) in names.iter().enumerate() {
                        let t = td.elems.get(i).copied().unwrap_or(Ty::I64);
                        scope.insert(n.clone(), t);
                        if is_owned(t) && !out.contains(n) {
                            out.push(n.clone());
                        }
                    }
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
        Expr::Borrow(inner) => {
            // v4.3：宽松推导（借用合法性由 check_expr 严格校验，规范 11.6 / 22.8 / 25.6）
            let t = ty_of(inner, scope, funcs, structs)?;
            if t == Ty::Str || t.is_darr() || t.is_map() {
                Ok(if t == Ty::Str { Ty::BorrowStr } else { t })
            } else {
                Err(err(0, format!("只能借用 str、动态数组或 map，实际 {}", t.label())))
            }
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
            for (ai, (a, pt)) in args.iter().zip(&sig.params).enumerate() {
                if *pt == Ty::BorrowStr {
                    // 借用形参：实参必须是 &变量（不移动源，规范 11.6.1/11.6.2），
                    // 或字符串字面量（静态存储，天然只读出借）
                    match a {
                        Expr::Str(_) => continue,
                        // v4.2：借用形参可原样转借给下一个 &str 形参（视图无所有权，规范 23.4）
                        Expr::Var(n) if scope.get(n).map(|v| v.ty) == Some(Ty::BorrowStr) => continue,
                        Expr::Borrow(inner) => {
                            // v3.1：&p.f 借用 str 字段（只读视图，不移动不释放，规范 14.4.4）
                            if let Expr::Field(base, fname) = inner.as_ref() {
                                let bt = check(base, scope)?;
                                let sd = match bt {
                                    Ty::Struct(idx) => structs.get(idx as usize).ok_or_else(|| {
                                        err(line, "内部错误：结构体索引越界".to_string())
                                    })?,
                                    other => {
                                        return Err(err(
                                            line,
                                            format!("只能借用 str 变量或结构体的 str 字段，实际 {}", other.label()),
                                        ))
                                    }
                                };
                                let ft = field_ty(sd, fname).ok_or_else(|| {
                                    err(line, format!("结构体 '{}' 没有字段 '{}'", sd.name, fname))
                                })?;
                                if ft != Ty::Str {
                                    return Err(err(
                                        line,
                                        format!("字段 '{}' 不是 str 类型，只能借用 str 字段", fname),
                                    ));
                                }
                                continue;
                            }
                            if !matches!(inner.as_ref(), Expr::Var(_)) {
                                return Err(err(line, "借用 & 只能作用于 str 变量或 str 字段（&变量 或 &变量.字段）"));
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
                // v3.4：数组实参 = 只读视图传递；实参必须是数组变量（字面量无存储，规范 16.6）
                if pt.is_arr() {
                    if !matches!(a, Expr::Var(_)) {
                        return Err(err(
                            line,
                            format!("函数 '{}' 的数组形参只接受数组变量实参（只读视图传递）", name),
                        ));
                    }
                    if at != *pt {
                        return Err(err(
                            line,
                            format!(
                                "函数 '{}' 的数组形参期望 {}，实际 {}",
                                name,
                                ty_label(*pt, structs),
                                ty_label(at, structs)
                            ),
                        ));
                    }
                    continue;
                }
                if !value_assignable(*pt, at, a) {
                    return Err(err(
                        line,
                        format!("函数 '{}' 的参数期望 {}，实际 {}", name, pt.label(), at.label()),
                    ));
                }
                // v4.3：借用 &[]T 形参——实参必须是数组变量（只读视图，不移动），不可是借用变量转 owned
                let is_borrow_param = sig.borrows.get(ai).copied().unwrap_or(false);
                if is_borrow_param {
                    // v4.3：&h 与 h 均可（借用视图，& 只是语法糖，规范 22.8）
                    let arg_var = match a {
                        Expr::Var(n) => Some(n.clone()),
                        Expr::Borrow(inner) => match inner.as_ref() {
                            Expr::Var(n) => Some(n.clone()),
                            _ => None,
                        },
                        _ => None,
                    };
                    if let Some(n) = arg_var {
                        let info = scope.get(&n).copied();
                        match info {
                            None => return Err(err(line, format!("变量 '{}' 未声明", n))),
                            Some(v) => {
                                if v.is_borrow {
                                    continue; // 借用转借（视图无所有权）
                                }
                                if darr_by_id(&darrs(), v.ty).is_none() && map_by_id(&maps(), v.ty).is_none() {
                                    return Err(err(
                                        line,
                                        format!("函数 '{}' 的 &[]T/&map 形参期望动态数组或 map 变量", name),
                                    ));
                                }
                                continue;
                            }
                        }
                    }
                    if matches!(a, Expr::Call { .. }) {
                        continue;
                    }
                    return Err(err(
                        line,
                        format!("函数 '{}' 的 &[]T 形参只接受数组变量实参", name),
                    ));
                }
                // v4.5：元组不可作为实参（仅存在于返回边界，规范 24.5）
                if at.is_tuple() || pt.is_tuple() {
                    return Err(err(
                        line,
                        format!("函数 '{}' 的元组不可作为实参（元组仅用于返回边界）", name),
                    ));
                }
                // v4.0：动态数组形参移动所有权；实参只能是变量或函数调用
                if pt.is_darr() && !matches!(a, Expr::Var(_) | Expr::Call { .. }) {
                    // v4.3：借用语法 &h 不可传给接管所有权的形参（语义混淆，规范 22.8）
                    if let Expr::Borrow(inner) = a {
                        if let Expr::Var(n) = inner.as_ref() {
                            return Err(err(
                                line,
                                format!(
                                    "不能把借用 '&{}' 传给接管所有权的形参（去掉 & 传值，或把形参改为 &[]T）",
                                    n
                                ),
                            ));
                        }
                    }
                    // 借用变量不可作为 owned []T 实参（视图无所有权，规范 22.8）
                    if let Expr::Var(n) = a {
                        if scope.get(n).map(|v| v.is_borrow).unwrap_or(false) {
                            return Err(err(
                                line,
                                format!(
                                    "不能把借用变量 '{}' 传给接管所有权的形参（借用视图无所有权）",
                                    n
                                ),
                            ));
                        }
                    }
                    return Err(err(
                        line,
                        format!("函数 '{}' 的动态数组实参只能是数组变量或函数调用", name),
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
        // v3.3：数组字面量——元素个数与类型严格校验（规范 16.2）
        Expr::ArrLit { arr, elems } => {
            let table = arrs();
            let a = table
                .get(*arr as usize)
                .ok_or_else(|| err(line, "内部错误：数组类型索引越界"))?;
            if elems.len() as u64 != a.len {
                return Err(err(
                    line,
                    format!("数组 {}[{}] 期望 {} 个元素，实际 {} 个", a.elem.label(), a.len, a.len, elems.len()),
                ));
            }
            for (i, e) in elems.iter().enumerate() {
                let vt = check(e, scope)?;
                if !value_assignable(a.elem, vt, e) {
                    return Err(err(
                        line,
                        format!(
                            "数组元素 #{} 期望 {}，实际 {}",
                            i,
                            a.elem.label(),
                            ty_label(vt, structs)
                        ),
                    ));
                }
            }
            Ok(Ty::Arr(*arr))
        }
        // v3.3/v4.0：数组下标——基表达式必须是数组变量；定长数组做编译期越界检查
        Expr::Index(base, idx) => {
            if !matches!(base.as_ref(), Expr::Var(_)) {
                return Err(err(line, "数组下标的左侧只能是数组变量（如 a[i]）"));
            }
            let bt = check(base, scope)?;
            let nt = norm(bt);
            let it = check(idx, scope)?;
            // v4.7：m[k] —— 键类型须与 map 键类型一致（K ∈ {i64,str}）
            if let Some(m) = map_by_id(&maps(), nt) {
                if !value_assignable(m.key, it, idx) {
                    return Err(err(
                        line,
                        format!("map 键期望 {}，实际 {}", m.key.label(), ty_label(it, structs)),
                    ));
                }
                return Ok(m.val);
            }
            if !matches!(it, Ty::I32 | Ty::I64) {
                return Err(err(
                    line,
                    format!("数组下标必须是整数，实际 {}", ty_label(it, structs)),
                ));
            }
            // v4.2：str 下标读（字节）——运行时越界守卫
            if nt == Ty::Str {
                return Ok(Ty::I64);
            }
            let table = arrs();
            if let Some(a) = arr_by_id(&table, nt) {
                if let Expr::Int(n) = &**idx {
                    if *n < 0 || *n as u64 >= a.len {
                        return Err(err(
                            line,
                            format!("下标 {} 越界：数组长度为 {}（合法范围 0..{}）", n, a.len, a.len - 1),
                        ));
                    }
                }
                // 负号字面量 -1 被解析为 Neg(Int)，同样做编译期检查
                if let Expr::Neg(x) = &**idx {
                    if let Expr::Int(n) = x.as_ref() {
                        return Err(err(
                            line,
                            format!("下标 -{} 越界：数组下标不能为负（合法范围 0..{}）", n, a.len - 1),
                        ));
                    }
                }
                return Ok(a.elem);
            }
            // v4.0：动态数组——长度运行时决定，仅静态检查
            let dtable = darrs();
            let d = darr_by_id(&dtable, nt).ok_or_else(|| {
                err(
                    line,
                    format!("类型 {} 不能取下标（只有数组能用 [i]）", ty_label(nt, structs)),
                )
            })?;
            Ok(d.elem)
        }
        // v4.0：动态数组字面量——元素类型校验（个数不限，规范第 22 节）
        Expr::DArrLit { darr, elems } => {
            let table = darrs();
            let d = table
                .get(*darr as usize)
                .ok_or_else(|| err(line, "内部错误：动态数组类型索引越界"))?;
            for (i, e) in elems.iter().enumerate() {
                let vt = check(e, scope)?;
                if !value_assignable(d.elem, vt, e) {
                    return Err(err(
                        line,
                        format!(
                            "动态数组元素 #{} 期望 {}，实际 {}",
                            i,
                            d.elem.label(),
                            ty_label(vt, structs)
                        ),
                    ));
                }
            }
            Ok(Ty::DArr(*darr))
        }
        // v4.7：map 字面量——条目标键/值类型校验（规范第 25 节）
        Expr::MapLit { map, entries } => {
            let table = maps();
            let m = table
                .get(*map as usize)
                .ok_or_else(|| err(line, "内部错误：关联数组类型索引越界"))?;
            for (i, (k, v)) in entries.iter().enumerate() {
                let kt = norm(check(k, scope)?);
                if !value_assignable(m.key, kt, k) {
                    return Err(err(
                        line,
                        format!("map 条目 #{} 键期望 {}，实际 {}", i, m.key.label(), ty_label(kt, structs)),
                    ));
                }
                let vt = norm(check(v, scope)?);
                if !value_assignable(m.val, vt, v) {
                    return Err(err(
                        line,
                        format!("map 条目 #{} 值期望 {}，实际 {}", i, m.val.label(), ty_label(vt, structs)),
                    ));
                }
            }
            Ok(Ty::Map(*map))
        }
        // v4.7：has(m, k) → bool（表达式）
        Expr::Has { map, key } => {
            let mt = norm(check(map, scope)?);
            let ms = maps();
            let m = map_by_id(&ms, mt).ok_or_else(|| {
                err(line, format!("has() 第一个实参必须是 map，实际 {}", ty_label(mt, structs)))
            })?;
            let kt = norm(check(key, scope)?);
            if !value_assignable(m.key, kt, key) {
                return Err(err(
                    line,
                    format!("has() 键期望 {}，实际 {}", m.key.label(), ty_label(kt, structs)),
                ));
            }
            Ok(Ty::Bool)
        }
        // v4.7：del(m, k) 是语句级（作表达式取值报错，规范第 25 节）
                // v4.7：keys(m) → 新拥有的 []K 键快照（map 或 &map 均可；规范 25.4）
        Expr::Keys(map) => {
            let bt = norm(check(map, scope)?);
            let ms = maps();
            let m = map_by_id(&ms, bt).ok_or_else(|| {
                err(line, format!("keys() 实参必须是 map，实际 {}", ty_label(bt, structs)))
            })?;
            let id = darrs()
                .iter()
                .position(|d| d.elem == m.key)
                .ok_or_else(|| err(line, "内部错误：map 键类型无对应动态数组"))? as u32;
            Ok(Ty::DArr(id))
        }
Expr::Del { .. } => Err(err(line, "del 只能作为语句使用")),
        // v3.5/v4.0：len —— 定长数组 / str / 动态数组
        Expr::Len(inner) => {
            let t = check(inner, scope)?;
            let nt = norm(t);
            if !nt.is_arr() && nt != Ty::Str && !nt.is_darr() && !nt.is_map() {
                return Err(err(
                    line,
                    format!("len() 只能用于数组、str 或 map，实际 {}", ty_label(nt, structs)),
                ));
            }
            Ok(Ty::I64)
        }
        // v4.0/v4.1：push/pop 不是表达式（语句级，规范第 22 节）
        // v4.2：sub(s, start, n) —— s 为 str，start/n 为整数（规范第 23 节）
        Expr::Sub { s, start, n } => {
            let st = check(s, scope)?;
            if norm(st) != Ty::Str {
                return Err(err(line, format!("sub 实参必须是 str，实际 {}", ty_label(norm(st), structs))));
            }
            for (label, e) in [("start", start), ("n", n)] {
                let t = check(e, scope)?;
                if !matches!(t, Ty::I32 | Ty::I64) {
                    return Err(err(
                        line,
                        format!("sub 的 {} 必须是整数，实际 {}", label, ty_label(t, structs)),
                    ));
                }
            }
            Ok(Ty::Str)
        }
        Expr::Push { .. } => Err(err(line, "push 只能作为语句使用")),
        Expr::Pop { .. } => Err(err(line, "pop 只能作为语句使用")),
        // v3.7：sel 条件表达式严格校验（带行号；规范第 18 节）
        Expr::Sel { cond, a, b } => {
            let ct = check(cond, scope)?;
            if ct != Ty::Bool {
                return Err(err(line, format!("sel 条件必须是 bool，实际 {}", ty_label(ct, structs))));
            }
            let at = check(a, scope)?;
            let bt = check(b, scope)?;
            sel_unify(at, bt, structs, line)
        }
        // v4.5：元组构造表达式（规范第 24 节）
        Expr::TupExpr { elems, tup } => {
            let table = tuples();
            let td = table
                .get(*tup as usize)
                .ok_or_else(|| err(line, "内部错误：元组类型索引越界"))?;
            if elems.len() != td.elems.len() {
                return Err(err(line, "元组元素个数与声明不符（内部错误）"));
            }
            for e in elems.iter() {
                check(e, scope)?;
            }
            Ok(Ty::Tuple(*tup))
        }
        Expr::Convert { name, arg } => {
            if name == "copy" {
                // copy(s)/copy(&s)：只借用源变量，产生新的独立所有者（规范 11.2.4/11.2.5/11.6.4）
                let t = match arg.as_ref() {
                    // copy(&s)：借用 → 所有权（规范 11.6.4 逃逸合法路径）
                    Expr::Borrow(inner) => {
                        if !matches!(inner.as_ref(), Expr::Var(_)) {
                            return Err(err(line, "借用 & 只能作用于 str 变量"));
                        }
                        let inner_t = check(inner, scope)?;
                        if inner_t != Ty::Str {
                            return Err(err(line, format!("只能借用 str，实际 {}", inner_t.label())));
                        }
                        Ty::BorrowStr
                    }
                    _ => check(arg, scope)?,
                };
                if t != Ty::Str && t != Ty::BorrowStr {
                    return Err(err(line, format!("copy() 只能用于 str，实际 {}", t.label())));
                }
                return Ok(Ty::Str);
            }
            let t = check(arg, scope)?;
            if name == "tof" {
                // v4.4：tof(s) —— 实参必须是 str（运行时快速失败，规范第 7 节）
                let nt = norm(t);
                if nt != Ty::Str {
                    return Err(err(line, format!("tof 实参必须是 str，实际 {}", ty_label(nt, structs))));
                }
                return Ok(Ty::F64);
            }
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
        assert!(check_src("f/lenstr(x:&str):i64{\nr/1\n}\n//s=\"a\"\n/g=lenstr(&s)\n`s\n").is_ok());
        assert!(check_src("f/lenstr(x:&str):i64{\nr/1\n}\n//s=\"a\"\n/g=lenstr(&s)\n//t=s\n").is_ok());
        // 借用形参可拼接、可比较、可打印（只读位视同 str）
        assert!(check_src("f/shout(x:&str):str{\nr/x+\"!\"\n}\n").is_ok());
        assert!(check_src("f/eq(x:&str):bool{\nr/x==\"a\"\n}\n").is_ok());
        // copy(&s)：从借用产生独立所有者（逃逸的合法路径）
        assert!(check_src("f/dup(x:&str):str{\nr/copy(x)\n}\n").is_ok());
    }

    #[test]
    fn test_borrow_rejects() {
        // 借用形参的实参缺 & 前缀
        let e = check_src("f/lenstr(x:&str):i64{\nr/1\n}\n//s=\"a\"\nlenstr(s)\n").unwrap_err();
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
        let e = check_src("f/lenstr(x:&str):i64{\nr/1\n}\n//s=\"a\"\n//t=s\nlenstr(&s)\n").unwrap_err();
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

    #[test]
    fn test_tuple_contract() {
        // v4.6：元组 ret 的 @example 支持 (e1, e2) 字面量逐元素结构比较（规范 24.1 例外）
        // 注意：语义锚点必须位于函数体首部（规范 12 节）；变量声明放在锚点之后。
        assert!(
            check_src(
                "f/range_stats(n:i64):(i64, i64){\n@example: range_stats(3) -> (3, 6)\n//c=0\nr/n, n/2\n}\n"
            )
            .is_ok()
        );
        // 含 f64 元素
        assert!(
            check_src(
                "f/div(a:i64,b:i64):(i64, f64){\n@example: div(5, 2) -> (2, 2.5)\nr/a, a/2.0\n}\n"
            )
            .is_ok()
        );
        // 含 str 元素
        assert!(
            check_src(
                "f/pair(n:i64):(i64, str){\n@example: pair(5) -> (5, \"hi\")\nr/n, \"hi\"\n}\n"
            )
            .is_ok()
        );
        // @post 不支持元组返回（编译期拦截，规范 24.6）
        let e = check_src(
            "f/g(x:i64):(i64, i64){\n@post: true\nr/x, x\n}\n",
        )
        .unwrap_err();
        assert!(e.contains("@post 不支持元组"), "实际报错：{}", e);
    }

    #[test]
    fn test_tuple_rejected_positions() {
        // v4.6 补漏：元组作实参
        let e = check_src("f/g(a:i64){`a}\nf/h(a:i64,b:i64):(i64,i64){r/a,b}\ng(h(1,2))\n").unwrap_err();
        assert!(e.contains("参数期望"), "tuple-as-arg 实际报错：{}", e);
        // 元组直接打印
        let e = check_src("f/h(a:i64,b:i64):(i64,i64){r/a,b}\n`h(1,2)\n").unwrap_err();
        assert!(e.contains("不能"), "tuple-print 实际报错：{}", e);
        // 元组作 sel 分支
        let e = check_src("f/h(a:i64,b:i64):(i64,i64){r/a,b}\n`sel(true, h(1,2), h(2,3))\n").unwrap_err();
        assert!(e.contains("sel"), "tuple-sel 实际报错：{}", e);
    }
}