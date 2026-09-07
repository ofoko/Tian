//! Cranelift 原生后端（v1.1）：AST → 机器码目标文件
//!
//! 不再翻译为 C；本模块直接产出 .o（宿主架构机器码），
//! 与 tian_rt.c 的编译产物由系统链接器（cc）合成可执行文件——与 rustc 的链接策略一致。
//! 整数除零走 __t_panic 快速失败（v3.2，与 C 后端一致，规范第 15 节）。

use std::collections::HashMap;

use cranelift_codegen::ir::types;
use cranelift_codegen::ir::{
    AbiParam, Block, Function, InstBuilder, MemFlags, Signature, StackSlotData, StackSlotKind,
    Type, UserFuncName, Value,
};
use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_frontend::Variable;
use cranelift_module::{default_libcall_names, DataDescription, DataId, FuncId, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};

use crate::ast::*;
use crate::type_check::{arrs, collect_str_decls, consumes_var, darr_by_id, darrs, is_owned, norm, set_arrs, set_darrs, set_structs, set_tuples, structs, FuncSig};

/// 指针类型：64 位宿主（aarch64/x86_64）为 I64
fn ptr_ty() -> Type {
    types::I64
}

/// Tian 类型 → Cranelift 类型（str/结构体为指针，bool 为 i8）
fn cl_ty(t: Ty) -> Type {
    match t {
        Ty::I32 => types::I32,
        Ty::I64 => types::I64,
        Ty::F64 => types::F64,
        Ty::Bool => types::I8,
        // v2.1：借用 &str 与 str 同为只读指针，但非所有者（规范 11.6）
        // v3.0：结构体与 str 同为堆指针（规范第 14 节）
        Ty::Str | Ty::BorrowStr | Ty::Struct(_) => ptr_ty(),
        // v3.3：数组变量持有指向栈槽的指针（规范第 16 节）
        Ty::Arr(_) => ptr_ty(),
        // v4.0：动态数组 = 堆指针（规范第 22 节）
        Ty::DArr(_) => ptr_ty(),
        // v4.5：元组 = 指向 malloc 块的堆指针（规范第 24 节；仅存在于返回边界）
        Ty::Tuple(_) => ptr_ty(),
    }
}

/// 编译整个程序，输出目标文件字节
pub fn generate_object(prog: &Program) -> Result<Vec<u8>, String> {
    let mut flag_builder = settings::builder();
    flag_builder
        .enable("is_pic")
        .map_err(|e| format!("Cranelift 配置错误: {}", e))?;
    let isa_builder = cranelift_native::builder().map_err(|_| "无法识别宿主机架构")?;
    let isa = isa_builder
        .finish(settings::Flags::new(flag_builder))
        .map_err(|e| format!("Cranelift ISA 错误: {}", e))?;
    let builder = ObjectBuilder::new(isa, "tian", default_libcall_names())
        .map_err(|e| format!("ObjectBuilder 创建失败: {}", e))?;
    let mut module = ObjectModule::new(builder);

    let rt = Runtime::declare(&mut module);

    // v3.0：结构体表注入线程局部，供后续 emit_* 查询布局（与 gen_c 同策略）
    set_structs(&prog.structs);
    set_arrs(&prog.arrs);
    set_darrs(&prog.darrs);
    set_tuples(&prog.tuples);

    // 模块级字符串字面量池：全局去重，命名全局唯一（跨函数共享）
    let mut str_pool: HashMap<String, DataId> = HashMap::new();
    let mut str_count = 0usize;

    // 校验后的函数签名表
    let sigs: HashMap<String, FuncSig> = prog
        .funcs
        .iter()
        .map(|f| {
            (
                f.name.clone(),
                FuncSig {
                    params: f.params.iter().map(|p| p.ty.unwrap_or(Ty::I64)).collect(),
                    borrows: f.params.iter().map(|p| p.borrow && p.ty.map(|t| t.is_darr()).unwrap_or(false)).collect(),
                    ret: f.ret.unwrap_or(Ty::I64),
                },
            )
        })
        .collect();

    // 声明所有天函数（定义顺序无关）
    let mut decls: HashMap<String, (FuncId, Signature)> = HashMap::new();
    for f in &prog.funcs {
        let fsig = &sigs[&f.name];
        let mut csig = module.make_signature();
        for pt in &fsig.params {
            csig.params.push(AbiParam::new(cl_ty(*pt)));
        }
        csig.returns.push(AbiParam::new(cl_ty(fsig.ret)));
        let id = module
            .declare_function(&cname(&f.name), Linkage::Local, &csig)
            .map_err(|e| format!("声明函数 '{}' 失败: {}", f.name, e))?;
        decls.insert(f.name.clone(), (id, csig));
    }

    let mut fb_ctx = FunctionBuilderContext::new();
    for (idx, f) in prog.funcs.iter().enumerate() {
        let fsig = sigs[&f.name].clone();
        let (id, _) = decls[&f.name];
        let mut func = Function::with_name_signature(UserFuncName::user(0, idx as u32), decls[&f.name].1.clone());
        compile_fn(
            &mut module, &mut fb_ctx, &mut func, f, &fsig, &sigs, &rt, id, &decls,
            &mut str_pool, &mut str_count,
        )?;
        let mut ctx = module.make_context();
        ctx.func = std::mem::replace(&mut func, Function::new());
        module
            .define_function(id, &mut ctx)
            .map_err(|e| format!("定义函数 '{}' 失败: {}", f.name, e))?;
        module.clear_context(&mut ctx);
    }

    // 顶层语句 → __tian_top（运行时 main 调用；无顶层语句时为空函数）
    compile_top(&mut module, &mut fb_ctx, prog, &sigs, &rt, &decls, &mut str_pool, &mut str_count)?;

    let product = module.finish();
    product.emit().map_err(|e| format!("输出目标文件失败: {}", e))
}

/// 运行时导入函数（实现见 tian_rt.c）
struct Runtime {
    print_ll: FuncId,
    print_f: FuncId,
    print_s: FuncId,
    print_b: FuncId,
    tos_ll: FuncId,
    tos_f: FuncId,
    tos_b: FuncId,
    cat: FuncId,
    seq: FuncId,
    sne: FuncId,
    /// 天权 v2.0：显式复制 / 释放
    dup: FuncId,
    free: FuncId,
    /// v3.0：堆分配（结构体构造用；C 后端直接调 malloc，Cranelift 需经运行时）
    malloc: FuncId,
    /// v2.2 语义锚点：契约断言（规范第 12 节）
    check: FuncId,
    panic: FuncId,
    dpop: FuncId,
    dpush_s: FuncId,
    dget_s: FuncId,
    dfree_s: FuncId,
    sget: FuncId,
    sub: FuncId,
    tof: FuncId,
    strlen: FuncId,
    dnew: FuncId,
    dlen: FuncId,
    dget_i: FuncId,
    dget_f: FuncId,
    dset_i: FuncId,
    dset_f: FuncId,
    dpush_i: FuncId,
    dpush_f: FuncId,
}

impl Runtime {
    fn declare(module: &mut ObjectModule) -> Runtime {
        let mut mk = |name: &str, params: Vec<Type>, ret: Option<Type>| {
            let mut sig = module.make_signature();
            for p in params {
                sig.params.push(AbiParam::new(p));
            }
            if let Some(r) = ret {
                sig.returns.push(AbiParam::new(r));
            }
            match module.declare_function(name, Linkage::Import, &sig) {
                Ok(id) => id,
                Err(e) => panic!("声明运行时函数 {} 失败: {}", name, e),
            }
        };
        Runtime {
            print_ll: mk("__t_print_ll", vec![types::I64], None),
            print_f: mk("__t_print_f", vec![types::F64], None),
            print_s: mk("__t_print_s", vec![ptr_ty()], None),
            print_b: mk("__t_print_b", vec![types::I32], None),
            tos_ll: mk("__t_tos_ll", vec![types::I64], Some(ptr_ty())),
            tos_f: mk("__t_tos_f", vec![types::F64], Some(ptr_ty())),
            tos_b: mk("__t_tos_b", vec![types::I32], Some(ptr_ty())),
            cat: mk("__t_cat", vec![ptr_ty(), ptr_ty()], Some(ptr_ty())),
            seq: mk("__t_seq", vec![ptr_ty(), ptr_ty()], Some(types::I32)),
            sne: mk("__t_sne", vec![ptr_ty(), ptr_ty()], Some(types::I32)),
            dup: mk("__t_dup", vec![ptr_ty()], Some(ptr_ty())),
            free: mk("__t_free", vec![ptr_ty()], None),
            malloc: mk("__t_malloc", vec![types::I64], Some(ptr_ty())),
            check: mk("__t_check", vec![types::I32, ptr_ty()], None),
            panic: mk("__t_panic", vec![ptr_ty()], None),
            dpop: mk("__t_dpop", vec![ptr_ty()], None),
            dpush_s: mk("__t_dpush_s", vec![ptr_ty(), ptr_ty()], Some(ptr_ty())),
            dget_s: mk("__t_dget_s", vec![ptr_ty(), types::I64], Some(ptr_ty())),
            dfree_s: mk("__t_dfree_s", vec![ptr_ty()], None),
            sget: mk("__t_sget", vec![ptr_ty(), types::I64], Some(types::I64)),
            sub: mk("__t_sub", vec![ptr_ty(), types::I64, types::I64], Some(ptr_ty())),
            tof: mk("__t_tof", vec![ptr_ty()], Some(types::F64)),
            strlen: mk("strlen", vec![ptr_ty()], Some(types::I64)),
            dnew: mk("__t_dnew", vec![types::I64], Some(ptr_ty())),
            dlen: mk("__t_dlen", vec![ptr_ty()], Some(types::I64)),
            dget_i: mk("__t_dget_i", vec![ptr_ty(), types::I64], Some(types::I64)),
            dget_f: mk("__t_dget_f", vec![ptr_ty(), types::I64], Some(types::F64)),
            dset_i: mk("__t_dset_i", vec![ptr_ty(), types::I64, types::I64], None),
            dset_f: mk("__t_dset_f", vec![ptr_ty(), types::I64, types::F64], None),
            dpush_i: mk("__t_dpush_i", vec![ptr_ty(), types::I64], Some(ptr_ty())),
            dpush_f: mk("__t_dpush_f", vec![ptr_ty(), types::F64], Some(ptr_ty())),
        }
    }
}

/// 标识符 mangle（与 C 后端一致）
fn cname(s: &str) -> String {
    let mut r = String::from("t_");
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            r.push(c);
        } else {
            r.push_str(&format!("x{:x}", c as u32));
        }
    }
    r
}

struct FnCtx<'a> {
    module: &'a mut ObjectModule,
    sigs: &'a HashMap<String, FuncSig>,
    rt: &'a Runtime,
    decls: &'a HashMap<String, (FuncId, Signature)>,
    vars: HashMap<String, (Variable, Ty)>,
    var_count: u32,
    /// 模块级字符串池（跨函数共享，去重）
    str_pool: &'a mut HashMap<String, DataId>,
    str_count: &'a mut usize,
    /// 当前块是否已发射终止符（return）；其余语句恒为 false
    block_terminated: bool,
    /// v2.2：当前函数的 @post 契约（表达式 + 锚点行号）；无则 None
    post: Option<(&'a Expr, usize)>,
    /// v2.2：当前函数名（契约失败信息用）
    fname: &'a str,
    /// v3.8：循环栈（header=continue 目标，exit=break 目标），规范第 19 节
    loop_stack: Vec<(Block, Block)>,
}

fn compile_fn(
    module: &mut ObjectModule,
    fb_ctx: &mut FunctionBuilderContext,
    func: &mut Function,
    f: &FnDef,
    fsig: &FuncSig,
    sigs: &HashMap<String, FuncSig>,
    rt: &Runtime,
    _id: FuncId,
    decls: &HashMap<String, (FuncId, Signature)>,
    str_pool: &mut HashMap<String, DataId>,
    str_count: &mut usize,
) -> Result<(), String> {
    let mut ctx = FnCtx {
        module,
        sigs,
        rt,
        decls,
        vars: HashMap::new(),
        var_count: 0,
        str_pool,
        str_count,
        block_terminated: false,
        post: f.contracts.iter().find_map(|c| match c {
            Contract::Post(e, line) => Some((e, *line)),
            _ => None,
        }),
        fname: &f.name,
        loop_stack: Vec::new(),
    };
    let mut builder = FunctionBuilder::new(func, fb_ctx);
    let entry = builder.create_block();
    builder.append_block_params_for_function_params(entry);
    builder.switch_to_block(entry);
    builder.seal_block(entry);

    // 参数绑定（只读变量）
    for (i, p) in f.params.iter().enumerate() {
        let t = p.ty.unwrap_or(Ty::I64);
        let var = Variable::from_u32(ctx.var_count);
        ctx.var_count += 1;
        builder.declare_var(var, cl_ty(t));
        let val = builder.block_params(entry)[i];
        builder.def_var(var, val);
        ctx.vars.insert(p.name.clone(), (var, t));
    }

    // 天权 v2.0：收集函数内全部 str/结构体声明（is_owned），入口预置 NULL 槽（规范 11.3 / 14.4）；
    // 循环内重复声明时每次绑定前释放旧值，无泄漏
    let mut ty_scope: HashMap<String, Ty> = f
        .params
        .iter()
        .map(|p| (p.name.clone(), p.ty.unwrap_or(Ty::I64)))
        .collect();
    let mut heap_names: Vec<String> = Vec::new();
    collect_str_decls(&f.body, &mut ty_scope, &sigs, &structs(), &mut heap_names);
    let mut heap_vars: Vec<(Variable, Ty)> = Vec::new();
    for n in &heap_names {
        // 类型取自 ty_scope（collect_str_decls 已写入），str 与结构体同为指针槽
        let t = *ty_scope.get(n).unwrap_or(&Ty::Str);
        let var = Variable::from_u32(ctx.var_count);
        ctx.var_count += 1;
        builder.declare_var(var, cl_ty(t));
        let z = builder.ins().iconst(ptr_ty(), 0);
        builder.def_var(var, z);
        ctx.vars.insert(n.clone(), (var, t));
        heap_vars.push((var, t));
    }
    // 天权：str/结构体 形参也是所有者：纳入函数出口释放清单
    for p in &f.params {
        // v4.3：借用 &[]T 形参不是所有者，不释放（规范 22.8）
        if p.ty.map(is_owned).unwrap_or(false) && !p.borrow {
            heap_vars.push((ctx.vars[&p.name].0, p.ty.unwrap_or(Ty::I64)));
        }
    }

    // v2.2：@post 出口契约需要 ret 伪变量（仅在返回点定义，规范第 12 节）
    if ctx.post.is_some() {
        let var = Variable::from_u32(ctx.var_count);
        ctx.var_count += 1;
        builder.declare_var(var, cl_ty(fsig.ret));
        ctx.vars.insert("#ret".into(), (var, fsig.ret));
    }

    // v2.2：@pre 入口契约——假设不成立立即失败
    for c in &f.contracts {
        if let Contract::Pre(e, line) = c {
            let (pv, pt) = emit_expr(e, &mut ctx, &mut builder)?;
            if pt != Ty::Bool {
                return Err("@pre 必须是 bool 表达式（检查器已拦截）".into());
            }
            let pi = builder.ins().uextend(types::I32, pv);
            let what = str_ptr(
                &mut ctx,
                &mut builder,
                &format!("@pre 契约失败：{}（第 {} 行）", f.name, line),
            )?;
            let ck = ctx.rt.check;
            call_rt(&mut ctx, &mut builder, ck, types::I32, 2, None, &[pi, what])?;
        }
    }

    emit_block(&mut ctx, &mut builder, &f.body, fsig.ret)?;

    // 函数末尾隐式 r/（规范 5.3）；块已终止则跳过
    if !ctx.block_terminated {
        // 天权：落出路径释放全部堆槽（str/结构体，深释放；已移出的为 NULL，free 安全；
        // 早退路径按 11.4.2 跳过释放，仅影响内存占用）
        for (v, t) in &heap_vars {
            let p = builder.use_var(*v);
            if *t == Ty::Str {
                let fr = ctx.rt.free;
                call_rt(&mut ctx, &mut builder, fr, ptr_ty(), 1, None, &[p])?;
            } else {
                emit_deep_free(p, *t, &mut ctx, &mut builder)?;
            }
        }
        let val = empty_or_zero(&mut ctx, &mut builder, fsig.ret)?;
        if let Some((post, line)) = ctx.post {
            emit_post_check(&mut ctx, &mut builder, post, line, val)?;
        }
        builder.ins().return_(&[val]);
    }
    builder.finalize();
    Ok(())
}

/// v2.2：发射 @post 校验（ret 伪变量 → 契约表达式 → __t_check）
fn emit_post_check(
    ctx: &mut FnCtx,
    builder: &mut FunctionBuilder,
    post: &Expr,
    line: usize,
    val: Value,
) -> Result<(), String> {
    let rv = ctx.vars["#ret"].0;
    builder.def_var(rv, val);
    let rewritten = rewrite_ret(post);
    let (pv, pt) = emit_expr(&rewritten, ctx, builder)?;
    if pt != Ty::Bool {
        return Err("@post 必须是 bool 表达式（检查器已拦截）".into());
    }
    let pi = builder.ins().uextend(types::I32, pv);
    let what = str_ptr(ctx, builder, &format!("@post 契约失败（第 {} 行）", line))?;
    let ck = ctx.rt.check;
    call_rt(ctx, builder, ck, types::I32, 2, None, &[pi, what])?;
    Ok(())
}

/// v2.2：@post 表达式中的伪变量 ret → #ret（避免与用户变量同名冲突）
fn rewrite_ret(e: &Expr) -> Expr {
    match e {
        Expr::Var(n) if n == "ret" => Expr::Var("#ret".into()),
        Expr::Neg(x) => Expr::Neg(Box::new(rewrite_ret(x))),
        Expr::Bin { op, lhs, rhs } => Expr::Bin {
            op: *op,
            lhs: Box::new(rewrite_ret(lhs)),
            rhs: Box::new(rewrite_ret(rhs)),
        },
        Expr::Call { name, args } => Expr::Call {
            name: name.clone(),
            args: args.iter().map(|a| rewrite_ret(a)).collect(),
        },
        Expr::Convert { name, arg } => Expr::Convert {
            name: name.clone(),
            arg: Box::new(rewrite_ret(arg)),
        },
        other => other.clone(),
    }
}

/// 返回类型零值；str 为独立空串副本（规范 5.3 + 天权 11.1）
fn empty_or_zero(
    ctx: &mut FnCtx,
    builder: &mut FunctionBuilder,
    ret: Ty,
) -> Result<Value, String> {
    if ret == Ty::Str {
        let p = str_ptr(ctx, builder, "")?;
        let d = ctx.rt.dup;
        Ok(
            call_rt(ctx, builder, d, ptr_ty(), 1, Some(ptr_ty()), &[p])?
                .ok_or("__t_dup 无返回值")?,
        )
    } else {
        Ok(match ret {
            Ty::I32 => builder.ins().iconst(types::I32, 0),
            Ty::I64 => builder.ins().iconst(types::I64, 0),
            Ty::F64 => builder.ins().f64const(0.0),
            Ty::Bool => builder.ins().iconst(types::I8, 0),
            Ty::Arr(_) => unreachable!("定长数组不可作为返回类型（检查器已拦截）"),
            Ty::DArr(_) => builder.ins().iconst(ptr_ty(), 0),
            // v3.0：结构体零值 = 空指针（规范 14.5；裸 r/ 返回 NULL，未初始化的结构体不读取字段）
            Ty::Struct(_) => builder.ins().iconst(ptr_ty(), 0),
            // v4.5：元组零值 = 空指针（元组仅存在于返回边界，裸 r/ 返回 NULL 后立即被解构释放）
            Ty::Tuple(_) => builder.ins().iconst(ptr_ty(), 0),
            // 借用不可作为返回类型（检查器已拦截）
            Ty::Str | Ty::BorrowStr => unreachable!(),
        })
    }
}

/// 顶层语句编译为 __tian_top（运行时 main 调用）
fn compile_top(
    module: &mut ObjectModule,
    fb_ctx: &mut FunctionBuilderContext,
    prog: &Program,
    sigs: &HashMap<String, FuncSig>,
    rt: &Runtime,
    decls: &HashMap<String, (FuncId, Signature)>,
    str_pool: &mut HashMap<String, DataId>,
    str_count: &mut usize,
) -> Result<(), String> {
    let mut sig = module.make_signature();
    sig.returns.push(AbiParam::new(types::I32));
    let id = module
        .declare_function("__tian_top", Linkage::Export, &sig)
        .map_err(|e| e.to_string())?;

    let mut func = Function::with_name_signature(UserFuncName::user(0, 9999), sig.clone());
    {
        let mut ctx = FnCtx {
            module,
            sigs,
            rt,
            decls,
            vars: HashMap::new(),
            var_count: 0,
            str_pool,
            str_count,
            block_terminated: false,
            loop_stack: Vec::new(),
            post: None,
            fname: "__tian_top",
        };
        let mut builder = FunctionBuilder::new(&mut func, fb_ctx);
        let entry = builder.create_block();
        builder.switch_to_block(entry);
        builder.seal_block(entry);
        // 天权：顶层 str/结构体 槽提升为 NULL 初始化
        let mut ty_scope: HashMap<String, Ty> = HashMap::new();
        let mut heap_names: Vec<String> = Vec::new();
        collect_str_decls(&prog.top, &mut ty_scope, &sigs, &structs(), &mut heap_names);
        let mut heap_vars: Vec<(Variable, Ty)> = Vec::new();
        for n in &heap_names {
            let t = *ty_scope.get(n).unwrap_or(&Ty::Str);
            let var = Variable::from_u32(ctx.var_count);
            ctx.var_count += 1;
            builder.declare_var(var, cl_ty(t));
            let z = builder.ins().iconst(ptr_ty(), 0);
            builder.def_var(var, z);
            ctx.vars.insert(n.clone(), (var, t));
            heap_vars.push((var, t));
        }
        // v2.2：@example 自检样例——程序启动时运行，失败即非零退出（规范第 12 节）
        for f in &prog.funcs {
            for c in &f.contracts {
                if let Contract::Example { call, expected, line } = c {
                    // v4.6：元组返回 → 逐元素结构比较（规范 24.1 例外），避免退化为指针 icmp
                    if let Some(Ty::Tuple(tid)) = emit_ty_of(call, &ctx) {
                        let td = crate::type_check::tuples()[tid as usize].clone();
                        let exp_elems = match expected {
                            Expr::TupExpr { elems, .. } => elems,
                            _ => unreachable!("元组 @example 期望值必须是元组字面量"),
                        };
                        let (cv, _) = emit_expr(call, &mut ctx, &mut builder)?;
                        // 调用结果存入 FunctionBuilder 临时变量
                        let var = Variable::from_u32(ctx.var_count);
                        ctx.var_count += 1;
                        builder.declare_var(var, ptr_ty());
                        builder.def_var(var, cv);
                        let mut all: Option<Value> = None;
                        for (i, elem_ty) in td.elems.iter().enumerate() {
                            let base = builder.use_var(var);
                            let ev = load_field(&mut builder, base, (i as i32) * 8, *elem_ty)?;
                            let xv = emit_bind_native(&exp_elems[i], &mut ctx, &mut builder)?;
                            let cmp = match elem_ty {
                                Ty::Str => {
                                    let seq = ctx.rt.seq;
                                    let sq = call_rt(
                                        &mut ctx, &mut builder, seq, ptr_ty(), 2,
                                        Some(types::I32), &[ev, xv],
                                    )?
                                    .ok_or("__t_seq 无返回值")?;
                                    // __t_seq 返回 strcmp==0（相等为 1），故用 Ne 与 0 比较得到“是否相等”
                                    let z = builder.ins().iconst(types::I32, 0);
                                    builder.ins().icmp(IntCC::NotEqual, sq, z)
                                }
                                Ty::F64 => builder.ins().fcmp(FloatCC::Equal, ev, xv),
                                _ => {
                                    let a = coerce(&mut builder, ev, *elem_ty, Ty::I64)?;
                                    let b = coerce(&mut builder, xv, *elem_ty, Ty::I64)?;
                                    builder.ins().icmp(IntCC::Equal, a, b)
                                }
                            };
                            all = Some(match all {
                                None => cmp,
                                Some(p) => builder.ins().band(p, cmp),
                            });
                        }
                        let ok = all.ok_or("元组 @example 至少需要 1 个元素")?;
                        let ok32 = builder.ins().uextend(types::I32, ok);
                        let what = str_ptr(
                            &mut ctx,
                            &mut builder,
                            &format!("@example 契约失败：{}（第 {} 行）", f.name, line),
                        )?;
                        let ck = ctx.rt.check;
                        call_rt(&mut ctx, &mut builder, ck, types::I32, 2, None, &[ok32, what])?;
                    } else {
                    let (cv, ct) = emit_expr(call, &mut ctx, &mut builder)?;
                    let (ev, _) = emit_expr(expected, &mut ctx, &mut builder)?;
                    let ok = if ct == Ty::Str {
                        // __t_seq 已返回 I32，无需扩展
                        let sk = ctx.rt.seq;
                        call_rt(&mut ctx, &mut builder, sk, ptr_ty(), 2, Some(types::I32), &[cv, ev])?
                            .ok_or("__t_seq 无返回值")?
                    } else if ct == Ty::F64 {
                        builder.ins().fcmp(FloatCC::Equal, cv, ev)
                    } else {
                        builder.ins().icmp(IntCC::Equal, cv, ev)
                    };
                    let ok32 = if ct == Ty::Str {
                        ok
                    } else {
                        builder.ins().uextend(types::I32, ok)
                    };
                    let what = str_ptr(
                        &mut ctx,
                        &mut builder,
                        &format!("@example 契约失败：{}（第 {} 行）", f.name, line),
                    )?;
                    let ck = ctx.rt.check;
                    call_rt(&mut ctx, &mut builder, ck, types::I32, 2, None, &[ok32, what])?;
                    }
                }
            }
        }
        emit_block(&mut ctx, &mut builder, &prog.top, Ty::I64)?;
        if !ctx.block_terminated {
            // 天权：进程退出前释放顶层堆槽（str/结构体，深释放；已移出的为 NULL，安全）
            for (v, t) in &heap_vars {
                let p = builder.use_var(*v);
                if *t == Ty::Str {
                    let fr = ctx.rt.free;
                    call_rt(&mut ctx, &mut builder, fr, ptr_ty(), 1, None, &[p])?;
                } else {
                    emit_deep_free(p, *t, &mut ctx, &mut builder)?;
                }
            }
            let zero = builder.ins().iconst(types::I32, 0);
            builder.ins().return_(&[zero]);
        }
        builder.finalize();
    }

    let mut c2 = module.make_context();
    c2.func = func;
    module.define_function(id, &mut c2).map_err(|e| e.to_string())?;
    module.clear_context(&mut c2);
    Ok(())
}

/// 依序发射语句块；return 后的残余语句进入不可达死块（Cranelift 要求块单终止符）
fn emit_block(
    ctx: &mut FnCtx,
    builder: &mut FunctionBuilder,
    stmts: &[Stmt],
    fn_ret: Ty,
) -> Result<(), String> {
    for (i, s) in stmts.iter().enumerate() {
        emit_stmt(ctx, builder, s, fn_ret)?;
        // v3.8：break/continue 与 return 一样截断后续语句（换死块继续发射）
        if matches!(s, Stmt::Return(..) | Stmt::Break(..) | Stmt::Continue(..) | Stmt::Panic(..))
            && i + 1 < stmts.len()
        {
            let dead = builder.create_block();
            builder.seal_block(dead);
            builder.switch_to_block(dead);
            ctx.block_terminated = false;
        }
    }
    Ok(())
}

fn emit_stmt(
    ctx: &mut FnCtx,
    builder: &mut FunctionBuilder,
    s: &Stmt,
    fn_ret: Ty,
) -> Result<(), String> {
    ctx.block_terminated = false;
    match s {
        Stmt::Decl { name, ty, value, mutable, .. } => {
            let t = ty.unwrap_or_else(|| emit_ty_of(value, ctx).unwrap_or(Ty::I64));
            // v4.0：动态数组声明——堆指针 + __t_dnew/dset（天权移动语义，规范第 22 节）
            if t.is_darr() {
                let de = match t {
                    Ty::DArr(i) => darrs()[i as usize].clone(),
                    _ => unreachable!(),
                };
                let (var, _) = *ctx
                    .vars
                    .get(name)
                    .ok_or("堆值槽未预置（编译器内部错误）")?;
                match value {
                    Expr::DArrLit { elems, .. } => {
                        let cap = (elems.len() as i64).max(4);
                        let capv = builder.ins().iconst(types::I64, cap);
                        let mut h = call_rt(ctx, builder, ctx.rt.dnew, types::I64, 1, Some(ptr_ty()), &[capv])?
                            .ok_or("__t_dnew 无返回值")?;
                        // 字面量初始化按 push 语义（len 递增），push 返回可能 realloc 的新指针
                        for e in elems.iter() {
                            let (v, vt) = emit_expr(e, ctx, builder)?;
                            if de.elem == Ty::Str {
                                // v4.2：str 元素移交所有权（字面量 dup，规范 22.6）
                                let sv = emit_bind_native(e, ctx, builder)?;
                                h = call_rt(ctx, builder, ctx.rt.dpush_s, ptr_ty(), 2, Some(ptr_ty()), &[h, sv])?
                                    .ok_or("__t_dpush_s 无返回值")?;
                            } else if de.elem == Ty::F64 {
                                let v = coerce(builder, v, vt, Ty::F64)?;
                                h = call_rt(ctx, builder, ctx.rt.dpush_f, types::F64, 2, Some(ptr_ty()), &[h, v])?
                                    .ok_or("__t_dpush_f 无返回值")?;
                            } else {
                                // v4.6：Bool 元素桥接为 I64（ABI 与 C 后端一致）
                                let (v, abt) = coerce_darr_elem(builder, v, vt, de.elem)?;
                                h = call_rt(ctx, builder, ctx.rt.dpush_i, abt, 2, Some(ptr_ty()), &[h, v])?
                                    .ok_or("__t_dpush_i 无返回值")?;
                            }
                        }
                        let old = builder.use_var(var);
                        let fr = ctx.rt.free;
                        call_rt(ctx, builder, fr, ptr_ty(), 1, None, &[old])?;
                        builder.def_var(var, h);
                    }
                    _ => {
                        // Var/Call：先求值新指针（可能移动源），再释放旧值，最后接管
                        let (val, vt) = emit_expr(value, ctx, builder)?;
                        let val = coerce(builder, val, vt, t)?;
                        let old = builder.use_var(var);
                        let consumed = consumes_var(value, name, true, ctx.sigs);
                        if !consumed {
                            // v4.2：str 元素容器用深释放（规范 22.6）
                            let f = if de.elem == Ty::Str { ctx.rt.dfree_s } else { ctx.rt.free };
                            call_rt(ctx, builder, f, ptr_ty(), 1, None, &[old])?;
                        }
                        emit_move_nulls_native(value, true, ctx, builder)?;
                        builder.def_var(var, val);
                    }
                }
                let _ = mutable;
                return Ok(());
            }
            // v3.3：数组声明——栈槽 + 逐元素初始化/拷贝（纯值类型，不参与天权，规范 16.1）
            if let Ty::Arr(_) = t {
                let (addr, elem, len) = create_arr_slot(ctx, builder, t);
                let var = Variable::from_u32(ctx.var_count);
                ctx.var_count += 1;
                builder.declare_var(var, ptr_ty());
                builder.def_var(var, addr);
                ctx.vars.insert(name.clone(), (var, t));
                match value {
                    Expr::ArrLit { elems, .. } => {
                        for (i, e) in elems.iter().enumerate() {
                            let (v, vt) = emit_expr(e, ctx, builder)?;
                            let v = coerce(builder, v, vt, elem)?;
                            store_field(builder, addr, (i as i32) * 8, elem, v);
                        }
                    }
                    Expr::Var(src) => {
                        let (sv, _) = *ctx.vars.get(src).ok_or("源数组未声明（编译器内部错误）")?;
                        let svv = builder.use_var(sv);
                        emit_arr_copy(builder, addr, svv, elem, len)?;
                    }
                    _ => return Err("数组声明的 RHS 非法（应被 type_check 拦截）".into()),
                }
                emit_move_nulls_native(value, false, ctx, builder)?;
                let _ = mutable;
                return Ok(());
            }
            if is_owned(t) {
                // 天权：堆值槽（str/结构体）已在入口预置 NULL；释放旧值（首次为 NULL，深释放内部已做 NULL 守卫，
                // 规范 11.3.2 / 14.4），后接管新值（字面量/移动源）；移动来的值直接接管指针
                let (var, _) = *ctx
                    .vars
                    .get(name)
                    .ok_or("堆值槽未预置（编译器内部错误）")?;
                if t == Ty::Str {
                    let val = emit_bind_native(value, ctx, builder)?;
                    let old = builder.use_var(var);
                    let fr = ctx.rt.free;
                    call_rt(ctx, builder, fr, ptr_ty(), 1, None, &[old])?;
                    builder.def_var(var, val);
                } else {
                    let val = emit_struct_rhs(value, ctx, builder)?;
                    let old = builder.use_var(var);
                    emit_deep_free(old, t, ctx, builder)?;
                    builder.def_var(var, val);
                }
                emit_move_nulls_native(value, true, ctx, builder)?;
            } else {
                let (val, vt) = emit_expr(value, ctx, builder)?;
                let val = coerce(builder, val, vt, t)?;
                let var = Variable::from_u32(ctx.var_count);
                ctx.var_count += 1;
                builder.declare_var(var, cl_ty(t));
                builder.def_var(var, val);
                ctx.vars.insert(name.clone(), (var, t));
                // RHS 中的调用可能以 str/结构体 形参移动了实参
                emit_move_nulls_native(value, false, ctx, builder)?;
            }
            let _ = mutable;
            Ok(())
        }
        Stmt::Assign { target, value, .. } => {
            match target {
                Expr::Var(name) => {
                    let (var, t) = *ctx.vars.get(name).ok_or_else(|| {
                        format!("变量 '{}' 未声明（编译器内部错误，应被 type_check 拦截）", name)
                    })?;
                    // v4.0：动态数组整体赋值——移动语义（规范第 22 节）
                    if t.is_darr() {
                        let de = match t {
                            Ty::DArr(i) => darrs()[i as usize].clone(),
                            _ => unreachable!(),
                        };
                        let dst = builder.use_var(var);
                        let self_move = matches!(value, Expr::Var(n) if n == name);
                        if !self_move {
                            match value {
                                Expr::DArrLit { elems, .. } => {
                                    let cap = (elems.len() as i64).max(4);
                                    let capv = builder.ins().iconst(types::I64, cap);
                                    let mut h = call_rt(ctx, builder, ctx.rt.dnew, types::I64, 1, Some(ptr_ty()), &[capv])?
                                        .ok_or("__t_dnew 无返回值")?;
                                    for e in elems.iter() {
                                        let (v, vt) = emit_expr(e, ctx, builder)?;
                                        // 字面量初始化按 push 语义（len 递增）
                                        if de.elem == Ty::Str {
                                            let sv = emit_bind_native(e, ctx, builder)?;
                                            h = call_rt(ctx, builder, ctx.rt.dpush_s, ptr_ty(), 2, Some(ptr_ty()), &[h, sv])?
                                                .ok_or("__t_dpush_s 无返回值")?;
                                        } else if de.elem == Ty::F64 {
                                            let v = coerce(builder, v, vt, Ty::F64)?;
                                            h = call_rt(ctx, builder, ctx.rt.dpush_f, types::F64, 2, Some(ptr_ty()), &[h, v])?
                                                .ok_or("__t_dpush_f 无返回值")?;
                                        } else {
                                            // v4.6：Bool 元素桥接为 I64（ABI 与 C 后端一致）
                                            let (v, abt) = coerce_darr_elem(builder, v, vt, de.elem)?;
                                            h = call_rt(ctx, builder, ctx.rt.dpush_i, abt, 2, Some(ptr_ty()), &[h, v])?
                                                .ok_or("__t_dpush_i 无返回值")?;
                                        }
                                    }
                                    let consumed = consumes_var(value, name, true, ctx.sigs);
                                    if !consumed {
                                        let f = if de.elem == Ty::Str { ctx.rt.dfree_s } else { ctx.rt.free };
                                        call_rt(ctx, builder, f, ptr_ty(), 1, None, &[dst])?;
                                    }
                                    builder.def_var(var, h);
                                }
                                _ => {
                                    let (val, vt) = emit_expr(value, ctx, builder)?;
                                    let val = coerce(builder, val, vt, t)?;
                                    let consumed = consumes_var(value, name, true, ctx.sigs);
                                    if !consumed {
                                        let f = if de.elem == Ty::Str { ctx.rt.dfree_s } else { ctx.rt.free };
                                        call_rt(ctx, builder, f, ptr_ty(), 1, None, &[dst])?;
                                    }
                                    emit_move_nulls_native(value, true, ctx, builder)?;
                                    builder.def_var(var, val);
                                }
                            }
                        }
                        // v4.6 修复：self_move（h=h）即自移动恒等，槽位原样保留——
                        // 原先无条件 emit_move_nulls 把槽位清成 NULL，后续读取段错误
                        return Ok(());
                    }
                    // v3.3：数组整体赋值——拷贝语义，逐元素（规范 16.1）
                    if let Ty::Arr(_) = t {
                        let (elem, len) = arr_def(t);
                        let dst = builder.use_var(var);
                        match value {
                            Expr::ArrLit { elems, .. } => {
                                for (i, e) in elems.iter().enumerate() {
                                    let (v, vt) = emit_expr(e, ctx, builder)?;
                                    let v = coerce(builder, v, vt, elem)?;
                                    store_field(builder, dst, (i as i32) * 8, elem, v);
                                }
                            }
                            Expr::Var(src) => {
                                let (sv, _) = *ctx.vars.get(src).ok_or("源数组未声明（编译器内部错误）")?;
                                let svv = builder.use_var(sv);
                                emit_arr_copy(builder, dst, svv, elem, len)?;
                            }
                            _ => return Err("数组赋值的 RHS 非法（应被 type_check 拦截）".into()),
                        }
                        emit_move_nulls_native(value, false, ctx, builder)?;
                        return Ok(());
                    }
                    if is_owned(t) {
                        let self_move = matches!(value, Expr::Var(n) if n == name);
                        if !self_move {
                            if t == Ty::Str {
                                // 天权 11.3.2：先求值新值（可能读取自身旧值，如 a=a+"x"），
                                // 再释放旧值、置空被移动的源变量、最后接管新值
                                let val = emit_bind_native(value, ctx, builder)?;
                                let old = builder.use_var(var);
                                // 若 RHS 内部已消费自身（a=g(a)），旧值由被调函数释放，不可再 free
                                let consumed = consumes_var(value, name, true, ctx.sigs);
                                if !consumed {
                                    let fr = ctx.rt.free;
                                    call_rt(ctx, builder, fr, ptr_ty(), 1, None, &[old])?;
                                }
                                emit_move_nulls_native(value, true, ctx, builder)?;
                                builder.def_var(var, val);
                            } else {
                                // v3.0：结构体变量赋值——深释放旧值后接管（规范 14.4）
                                let val = emit_struct_rhs(value, ctx, builder)?;
                                emit_move_nulls_native(value, true, ctx, builder)?;
                                let old = builder.use_var(var);
                                emit_deep_free(old, t, ctx, builder)?;
                                builder.def_var(var, val);
                            }
                        }
                        // 自我移动 p=p：所有权不变（复活语义，规范 11.2.3）
                    } else {
                        let (val, vt) = emit_expr(value, ctx, builder)?;
                        let val = coerce(builder, val, vt, t)?;
                        builder.def_var(var, val);
                        emit_move_nulls_native(value, false, ctx, builder)?;
                    }
                    Ok(())
                }
                Expr::Field(base, fname) => {
                    // v3.0：字段赋值 p.x = e（规范 14.3）
                    let (bptr, _) = emit_expr(base, ctx, builder)?;
                    let bt = emit_ty_of(base, ctx)
                        .ok_or("字段赋值：左侧结构体类型未知（编译器内部错误）")?;
                    let (_sd, idx, ft) = struct_field(bt, fname)
                        .ok_or_else(|| format!("结构体无字段 '{}'（应被 type_check 拦截）", fname))?;
                    let offset = field_offset(idx);
                    if ft == Ty::Str {
                        // 释放旧字段字符串，再接管新值（移动源置空）
                        // 字面量/tos 结果需 dup 为独立堆内存（天权 11.1.1）
                        let oldp = builder.ins().load(ptr_ty(), MemFlags::new(), bptr, offset);
                        let fr = ctx.rt.free;
                        call_rt(ctx, builder, fr, ptr_ty(), 1, None, &[oldp])?;
                        let v = emit_bind_native(value, ctx, builder)?;
                        store_field(builder, bptr, offset, Ty::Str, v);
                        emit_move_nulls_native(value, true, ctx, builder)?;
                    } else {
                        let (val, vt) = emit_expr(value, ctx, builder)?;
                        let v = coerce(builder, val, vt, ft)?;
                        store_field(builder, bptr, offset, ft, v);
                    }
                    Ok(())
                }
                Expr::Index(base, idx) => {
                    // v4.0：动态数组 a[i] = e —— __t_dset_* 守卫（规范第 22 节）
                    let bt0 = emit_ty_of(base, ctx).ok_or("下标基类型未知（编译器内部错误）")?;
                    if let Some(de) = darr_by_id(&darrs(), bt0).cloned() {
                        let (bp, _) = emit_expr(base, ctx, builder)?;
                        let (iv, it) = emit_expr(idx, ctx, builder)?;
                        let i = coerce(builder, iv, it, Ty::I64)?;
                        let (v, vt) = emit_expr(value, ctx, builder)?;
                        if de.elem == Ty::F64 {
                            let v = coerce(builder, v, vt, Ty::F64)?;
                            call_rt(ctx, builder, ctx.rt.dset_f, types::I64, 3, None, &[bp, i, v])?;
                        } else {
                            // v4.6：Bool 元素桥接为 I64（ABI 与 C 后端一致）
                            let (v, abt) = coerce_darr_elem(builder, v, vt, de.elem)?;
                            call_rt(ctx, builder, ctx.rt.dset_i, types::I64, 3, None, &[bp, i, v])?;
                        }
                        emit_move_nulls_native(value, false, ctx, builder)?;
                        return Ok(());
                    }
                    // v3.3：a[i] = e —— 越界守卫 + 元素存储（规范 16.3）
                    let (bp, bt) = emit_expr(base, ctx, builder)?;
                    let (elem, len) = arr_def(bt);
                    let (iv, it) = emit_expr(idx, ctx, builder)?;
                    let i = emit_idx_guard(ctx, builder, iv, it, len)?;
                    let eight = builder.ins().iconst(types::I64, 8);
                    let off = builder.ins().imul(i, eight);
                    let addr = builder.ins().iadd(bp, off);
                    let (v, vt) = emit_expr(value, ctx, builder)?;
                    let v = coerce(builder, v, vt, elem)?;
                    store_field(builder, addr, 0, elem, v);
                    emit_move_nulls_native(value, false, ctx, builder)?;
                    Ok(())
                }
                _ => Err("非法赋值左侧（应被 type_check 拦截）".into()),
            }
        }
        Stmt::Print(e, _) => {
            let (val, t) = emit_expr(e, ctx, builder)?;
            let (rt_fn, arg_ty) = match t {
                Ty::I32 | Ty::I64 => (ctx.rt.print_ll, types::I64),
                Ty::F64 => (ctx.rt.print_f, types::F64),
                // v2.1：借用只读打印，视同 str（规范 11.6.2）
                Ty::Str | Ty::BorrowStr => (ctx.rt.print_s, ptr_ty()),
                Ty::Bool => (ctx.rt.print_b, types::I32),
                Ty::Struct(_) => unreachable!("结构体不可直接打印（检查器已拦截）"),
                Ty::Arr(_) | Ty::DArr(_) => unreachable!("数组不可直接打印（检查器已拦截）"),
                // v4.5：元组不可直接打印（仅存在于返回边界，需先解构）
                Ty::Tuple(_) => unreachable!("元组不可直接打印（检查器已拦截）"),
            };
            let arg = coerce_print(builder, val, t, arg_ty)?;
            call_rt(ctx, builder, rt_fn, arg_ty, 1, None, &[arg])?;
            // 打印表达式内的调用可能移动了 str/结构体实参（规范 11.2 / 14.4）
            emit_move_nulls_native(e, false, ctx, builder)
        }
        Stmt::While { cond, body, .. } => {
            let header = builder.create_block();
            let body_b = builder.create_block();
            let exit = builder.create_block();
            ctx.loop_stack.push((header, exit));
            builder.ins().jump(header, &[]);
            builder.switch_to_block(header);
            let (c, ct) = emit_expr(cond, ctx, builder)?;
            let c = coerce_cond(builder, c, ct)?;
            builder.ins().brif(c, body_b, &[], exit, &[]);
            builder.switch_to_block(body_b);
            builder.seal_block(body_b);
            emit_block(ctx, builder, body, fn_ret)?;
            // v3.8：循环体末尾可能已被 break 截断，不可再发 jump header
            if !ctx.block_terminated {
                builder.ins().jump(header, &[]);
            }
            ctx.loop_stack.pop();
            builder.seal_block(header);
            builder.switch_to_block(exit);
            builder.seal_block(exit);
            // v4.6：exit 是新块；循环体内 break/continue 置位的截断标志在此复位，
            // 否则后续 emit（含函数出口隐式 r/）误判当前块已终止
            ctx.block_terminated = false;
            Ok(())
        }
        Stmt::If { cond, body, else_body, .. } => {
            let (c, ct) = emit_expr(cond, ctx, builder)?;
            let c = coerce_cond(builder, c, ct)?;
            let then_b = builder.create_block();
            let merge = builder.create_block();
            match else_body {
                Some(eb) => {
                    let else_b = builder.create_block();
                    builder.ins().brif(c, then_b, &[], else_b, &[]);
                    builder.switch_to_block(then_b);
                    builder.seal_block(then_b);
                    emit_block(ctx, builder, body, fn_ret)?;
                    // v4.6：分支体可能以 break/continue/return/panic 截断（块已有终止符），
                    // 不可再发 jump（Verifier：终止符后不得有指令）
                    if !ctx.block_terminated {
                        builder.ins().jump(merge, &[]);
                    }
                    builder.switch_to_block(else_b);
                    builder.seal_block(else_b);
                    emit_block(ctx, builder, eb, fn_ret)?;
                    if !ctx.block_terminated {
                        builder.ins().jump(merge, &[]);
                    }
                }
                None => {
                    builder.ins().brif(c, then_b, &[], merge, &[]);
                    builder.switch_to_block(then_b);
                    builder.seal_block(then_b);
                    emit_block(ctx, builder, body, fn_ret)?;
                    if !ctx.block_terminated {
                        builder.ins().jump(merge, &[]);
                    }
                }
            }
            builder.seal_block(merge);
            builder.switch_to_block(merge);
            // v4.6：merge 是新块；分支体内的 break/continue/return 截断标志在此复位
            ctx.block_terminated = false;
            Ok(())
        }
        Stmt::Return(e, _) => {
            let val = match e {
                Some(expr) => {
                    if fn_ret.is_tuple() {
                        // v4.5：元组返回——emit_expr 直接构造/取回 malloc 块指针（规范第 24 节）
                        let (v, _) = emit_expr(expr, ctx, builder)?;
                        emit_move_nulls_native(expr, true, ctx, builder)?;
                        v
                    } else if fn_ret == Ty::Str {
                        // 天权：返回即移交所有权（规范 11.2.1）；
                        // 先求值（字面量/tos 结果复制），再接管
                        let v = emit_bind_native(expr, ctx, builder)?;
                        emit_move_nulls_native(expr, true, ctx, builder)?;
                        v
                    } else {
                        let (v, vt) = emit_expr(expr, ctx, builder)?;
                        let v = coerce(builder, v, vt, fn_ret)?;
                        // 置空被移动的源变量（v3.0：结构体返回值同样需在返回位将源变量置空，规范 14.4）
                        emit_move_nulls_native(expr, is_owned(fn_ret), ctx, builder)?;
                        v
                    }
                }
                None => empty_or_zero(ctx, builder, fn_ret)?,
            };
            // v2.2：@post 出口契约——保证不成立立即失败（规范第 12 节）
            if let Some((post, line)) = ctx.post {
                emit_post_check(ctx, builder, post, line, val)?;
            }
            builder.ins().return_(&[val]);
            ctx.block_terminated = true;
            Ok(())
        }
        // v4.0：push(a, v) —— 可能 realloc，重新绑定返回的新指针（规范第 22 节）
        Stmt::Expr(Expr::Push { arr, value }, _) => {
            let (var, t) = match arr.as_ref() {
                Expr::Var(n) => *ctx.vars.get(n).ok_or("push 目标未声明（编译器内部错误）")?,
                _ => return Err("push 实参必须是变量（应被 type_check 拦截）".into()),
            };
            let de = match t {
                Ty::DArr(i) => darrs()[i as usize].clone(),
                _ => return Err("push 目标不是动态数组（应被 type_check 拦截）".into()),
            };
            let dst = builder.use_var(var);
            if de.elem == Ty::Str {
                // v4.2：str 元素移交所有权（字面量 dup，规范 22.6）
                let v = emit_bind_native(value, ctx, builder)?;
                let h = call_rt(ctx, builder, ctx.rt.dpush_s, ptr_ty(), 2, Some(ptr_ty()), &[dst, v])?
                    .ok_or("__t_dpush_s 无返回值")?;
                builder.def_var(var, h);
                emit_move_nulls_native(value, true, ctx, builder)?;
                return Ok(());
            }
            let (v, vt) = emit_expr(value, ctx, builder)?;
            if de.elem == Ty::F64 {
                let v = coerce(builder, v, vt, Ty::F64)?;
                let h = call_rt(ctx, builder, ctx.rt.dpush_f, types::F64, 2, Some(ptr_ty()), &[dst, v])?
                    .ok_or("__t_dpush_f 无返回值")?;
                builder.def_var(var, h);
            } else {
                // v4.6：Bool 元素桥接为 I64（ABI 与 C 后端一致）
                let (v, abt) = coerce_darr_elem(builder, v, vt, de.elem)?;
                let h = call_rt(ctx, builder, ctx.rt.dpush_i, abt, 2, Some(ptr_ty()), &[dst, v])?
                    .ok_or("__t_dpush_i 无返回值")?;
                builder.def_var(var, h);
            }
            Ok(())
        }
        // v4.1：pop(a) —— 长度递减；空数组是运行时错误（规范第 22 节）
        Stmt::Expr(Expr::Pop { arr }, _) => {
            let (var, t) = match arr.as_ref() {
                Expr::Var(n) => *ctx.vars.get(n).ok_or("pop 目标未声明（编译器内部错误）")?,
                _ => return Err("pop 实参必须是变量（应被 type_check 拦截）".into()),
            };
            if darr_by_id(&darrs(), t).is_none() {
                return Err("pop 目标不是动态数组（应被 type_check 拦截）".into());
            }
            let dst = builder.use_var(var);
            call_rt(ctx, builder, ctx.rt.dpop, ptr_ty(), 1, None, &[dst])?;
            Ok(())
        }
        Stmt::Expr(e, _) => {
            emit_expr(e, ctx, builder)?;
            // 语句级调用可能移动了 str 实参
            emit_move_nulls_native(e, false, ctx, builder)
        }
        // v3.8：break / continue —— 跳到循环出口 / 头部（规范第 19 节）
        Stmt::Break(_) => {
            let (_, exit) = *ctx
                .loop_stack
                .last()
                .ok_or("break 在循环外（应被 type_check 拦截）")?;
            builder.ins().jump(exit, &[]);
            ctx.block_terminated = true;
            Ok(())
        }
        Stmt::Continue(_) => {
            let (header, _) = *ctx
                .loop_stack
                .last()
                .ok_or("continue 在循环外（应被 type_check 拦截）")?;
            builder.ins().jump(header, &[]);
            ctx.block_terminated = true;
            Ok(())
        }
        // v4.0：panic(msg) —— 动态消息调用 __t_panic 后 trap（规范第 21 节）
        Stmt::Panic(msg, _) => {
            let (mv, mt) = emit_expr(msg, ctx, builder)?;
            let m = coerce(builder, mv, mt, Ty::Str)?;
            let pfn = ctx.module.declare_func_in_func(ctx.rt.panic, &mut builder.func);
            builder.ins().call(pfn, &[m]);
            builder.ins().trap(cranelift_codegen::ir::TrapCode::unwrap_user(1));
            ctx.block_terminated = true;
            Ok(())
        }
        // v4.0：check(cond, msg) —— 条件不成立时以动态消息 panic
        Stmt::Check(cond, msg, _) => {
            let (cv, ct) = emit_expr(cond, ctx, builder)?;
            let c = coerce_cond(builder, cv, ct)?;
            let (mv, mt) = emit_expr(msg, ctx, builder)?;
            let m = coerce(builder, mv, mt, Ty::Str)?;
            let panic_blk = builder.create_block();
            let ok_blk = builder.create_block();
            // v4.6 修复：c 已是 I8（Bool），brif 直接接受——同宽 uextend 非法
            builder.ins().brif(c, ok_blk, &[], panic_blk, &[]);
            builder.seal_block(panic_blk);
            builder.seal_block(ok_blk);
            builder.switch_to_block(panic_blk);
            let pfn = ctx.module.declare_func_in_func(ctx.rt.panic, &mut builder.func);
            builder.ins().call(pfn, &[m]);
            builder.ins().trap(cranelift_codegen::ir::TrapCode::unwrap_user(1));
            builder.switch_to_block(ok_blk);
            Ok(())
        }
        // v4.5：多声明解构 //a, b = f(...) —— 元素按类型接管，块随即释放（规范第 24 节）
        Stmt::MultiDecl { names, value, .. } => {
            let vt = emit_ty_of(value, ctx).ok_or("MultiDecl RHS 类型未知（编译器内部错误）")?;
            let td = match vt {
                Ty::Tuple(i) => crate::type_check::tuples()[i as usize].clone(),
                _ => unreachable!("MultiDecl RHS 非元组（检查器已拦截）"),
            };
            // 预置 owned 堆槽（已在 prologue 收集），标量临时新建
            let mut slots: Vec<(Variable, Ty)> = Vec::with_capacity(names.len());
            for (i, n) in names.iter().enumerate() {
                let t = td.elems[i];
                let var = if is_owned(t) {
                    let (v, _) = *ctx.vars.get(n).ok_or("MultiDecl owned 堆槽未预置（编译器内部错误）")?;
                    v
                } else {
                    let v = Variable::from_u32(ctx.var_count);
                    ctx.var_count += 1;
                    builder.declare_var(v, cl_ty(t));
                    v
                };
                slots.push((var, t));
            }
            let (block, _) = emit_expr(value, ctx, builder)?;
            for (i, (var, t)) in slots.iter().enumerate() {
                let off = (i as i32) * 8;
                let v = load_field(builder, block, off, *t)?;
                builder.def_var(*var, v);
            }
            // 元组块随即释放（owned 元素已接管槽位，块本身仅承载指针/标量，free 安全）
            call_rt(ctx, builder, ctx.rt.free, ptr_ty(), 1, None, &[block])?;
            for (i, n) in names.iter().enumerate() {
                if !is_owned(td.elems[i]) {
                    ctx.vars.insert(n.clone(), (slots[i].0, td.elems[i]));
                }
            }
            Ok(())
        }
    }
}

/// 条件值统一为 bool（i8）
fn coerce_cond(_builder: &mut FunctionBuilder, v: Value, t: Ty) -> Result<Value, String> {
    match t {
        Ty::Bool => Ok(v),
        other => Err(format!("条件应为 bool，实际 {:?}（编译器内部错误）", other)),
    }
}

/// 打印参数整形（i32 → i64，bool → i32）
fn coerce_print(
    builder: &mut FunctionBuilder,
    v: Value,
    from: Ty,
    to: Type,
) -> Result<Value, String> {
    match from {
        Ty::I32 => {
            if to == types::I64 {
                Ok(builder.ins().sextend(types::I64, v))
            } else {
                Ok(v)
            }
        }
        // v2.1：借用只读打印，视同 str（规范 11.6.2）
        Ty::I64 | Ty::F64 | Ty::Str | Ty::BorrowStr => Ok(v),
        // 结构体不可直接打印（检查器已拦截）；此处仅为 match 完备性
        Ty::Struct(_) => Ok(v),
        Ty::Arr(_) | Ty::DArr(_) => unreachable!("数组不可直接打印（检查器已拦截）"),
        Ty::Bool => Ok(builder.ins().uextend(types::I32, v)),
        // v4.5：元组不可直接打印（检查器已拦截）
        Ty::Tuple(_) => unreachable!("元组不可直接打印（检查器已拦截）"),
    }
}

/// 赋值/传参/返回的类型收敛（与 type_check 的 value_assignable 对应）
fn coerce(builder: &mut FunctionBuilder, v: Value, from: Ty, to: Ty) -> Result<Value, String> {
    if from == to {
        return Ok(v);
    }
    match (from, to) {
        // v2.1：字符串字面量直接出借给借用形参（静态存储，无需转换）
        (Ty::Str, Ty::BorrowStr) => Ok(v),
        (Ty::I64, Ty::I32) => Ok(builder.ins().ireduce(types::I32, v)),
        (Ty::I32, Ty::I64) => Ok(builder.ins().sextend(types::I64, v)),
        (Ty::I32, Ty::F64) => Ok(builder.ins().fcvt_from_sint(types::F64, v)),
        (Ty::I64, Ty::F64) => Ok(builder.ins().fcvt_from_sint(types::F64, v)),
        // v4.6：darr Bool 槽位按 I64 存储（与 C 后端一致），读回截断到 I8
        (Ty::I64, Ty::Bool) => Ok(builder.ins().ireduce(types::I8, v)),
        _ => Err(format!("无法从 {:?} 转换到 {:?}（编译器内部错误）", from, to)),
    }
}

/// v4.6：动态数组元素 → 运行时 ABI 值。Bool 元素以 I64 槽存储（__t_dpush_i/dset_i，
/// 与 C 后端 long long 槽一致），I8 → I64 在调用点桥接（规范第 22 节）
fn coerce_darr_elem(
    builder: &mut FunctionBuilder,
    v: Value,
    vt: Ty,
    elem: Ty,
) -> Result<(Value, types::Type), String> {
    if elem == Ty::F64 {
        Ok((coerce(builder, v, vt, Ty::F64)?, types::F64))
    } else if elem == Ty::Bool {
        let b = coerce(builder, v, vt, Ty::Bool)?;
        Ok((builder.ins().uextend(types::I64, b), types::I64))
    } else {
        Ok((coerce(builder, v, vt, Ty::I64)?, types::I64))
    }
}

/// 简易类型推导（gen_native 内部；type_check 已通过，不应失败）
fn emit_ty_of(e: &Expr, ctx: &FnCtx) -> Option<Ty> {
    match e {
        Expr::Int(_) => Some(Ty::I64),
        Expr::Float(_) => Some(Ty::F64),
        Expr::Str(_) => Some(Ty::Str),
        Expr::Bool(_) => Some(Ty::Bool),
        Expr::Var(name) => ctx.vars.get(name).map(|(_, t)| *t),
        // v3.3/v4.0：a[i] → 元素类型（定长或动态）
        Expr::Index(base, _) => {
            // v4.6 修复：借用 str（&str 形参 / @pre 锚点）同样支持字节读
            let bt = norm(emit_ty_of(base, ctx)?);
            match bt {
                Ty::Arr(i) => crate::type_check::arrs().get(i as usize).map(|a| a.elem),
                Ty::DArr(i) => darrs().get(i as usize).map(|d| d.elem),
                Ty::Str => Some(Ty::I64),
                _ => None,
            }
        }
        Expr::Len(_) => Some(Ty::I64),
        Expr::ArrLit { arr, .. } => Some(Ty::Arr(*arr)),
        // v4.0：动态数组——字面量 → 类型；push 无值
        Expr::DArrLit { darr, .. } => Some(Ty::DArr(*darr)),
        Expr::Sub { .. } => Some(Ty::Str),
        // v4.5：元组表达式类型 = Ty::Tuple(tup)
        Expr::TupExpr { tup, .. } => Some(Ty::Tuple(*tup)),
        Expr::Push { .. } => None,
        Expr::Pop { .. } => None,
        // v3.7：sel 类型 = 分支统一结果
        Expr::Sel { cond, a, b } => {
            let at = emit_ty_of(a, ctx)?;
            let bt = emit_ty_of(b, ctx)?;
            let ct = emit_ty_of(cond, ctx)?;
            if ct != Ty::Bool {
                return None;
            }
            match (norm(at), norm(bt)) {
                (x, y) if x == y => Some(x),
                (Ty::I64, Ty::I32) | (Ty::I32, Ty::I64) => Some(Ty::I64),
                (Ty::F64, _) | (_, Ty::F64) => Some(Ty::F64),
                _ => None,
            }
        }
        Expr::Neg(x) => emit_ty_of(x, ctx),
        Expr::Bin { op, lhs, rhs } => {
            let lt = emit_ty_of(lhs, ctx)?;
            let rt = emit_ty_of(rhs, ctx)?;
            if matches!(op.c_str(), "<" | ">" | "<=" | ">=" | "==" | "!=") {
                Some(Ty::Bool)
            } else if lt == Ty::Str && rt == Ty::Str {
                Some(Ty::Str)
            } else if lt == Ty::F64 || rt == Ty::F64 {
                Some(Ty::F64)
            } else if lt.is_numeric() && rt.is_numeric() {
                Some(Ty::I64)
            } else {
                None
            }
        }
        Expr::Call { name, .. } => ctx.sigs.get(name).map(|s| s.ret),
        Expr::Convert { name, .. } if name == "tos" || name == "copy" => Some(Ty::Str),
        // v4.6 修复：tof 返回 F64（此前被兜底为 I64，声明槽类型错配）
        Expr::Convert { name, .. } if name == "tof" => Some(Ty::F64),
        Expr::Convert { .. } => Some(Ty::I64),
        Expr::Borrow(_) => Some(Ty::BorrowStr),
        // v3.0：结构体字面量 / 字段读取类型（规范第 14 节）
        Expr::StructLit { name, .. } => structs()
            .iter()
            .position(|s| s.name == *name)
            .map(|i| Ty::Struct(i as u32)),
        Expr::Field(base, fname) => {
            let bt = emit_ty_of(base, ctx)?;
            match bt {
                Ty::Struct(i) => structs()
                    .get(i as usize)
                    .and_then(|sd| sd.fields.iter().find(|f| f.name == *fname).map(|f| f.ty)),
                _ => None,
            }
        }
    }
}

fn call_rt(
    ctx: &mut FnCtx,
    builder: &mut FunctionBuilder,
    id: FuncId,
    arg_ty: Type,
    n_args: usize,
    ret: Option<Type>,
    args: &[Value],
) -> Result<Option<Value>, String> {
    let mut sig = ctx.module.make_signature();
    for _ in 0..n_args {
        sig.params.push(AbiParam::new(arg_ty));
    }
    if let Some(r) = ret {
        sig.returns.push(AbiParam::new(r));
    }
    let local = ctx.module.declare_func_in_func(id, &mut builder.func);
    let call = builder.ins().call(local, args);
    Ok(builder.inst_results(call).first().copied())
}

/// 天权：绑定 str 值到变量——字面量/tos 结果需复制成独立堆内存（规范 11.1.1）；
/// 其余（拼接、移动、copy 结果）已是可拥有的堆指针，直接接管
fn emit_bind_native(
    e: &Expr,
    ctx: &mut FnCtx,
    builder: &mut FunctionBuilder,
) -> Result<Value, String> {
    let (v, t) = emit_expr(e, ctx, builder)?;
    let needs_dup = t == Ty::Str
        && match e {
            Expr::Str(_) => true,
            Expr::Convert { name, .. } => name == "tos",
            _ => false,
        };
    if needs_dup {
        let d = ctx.rt.dup;
        call_rt(ctx, builder, d, ptr_ty(), 1, Some(ptr_ty()), &[v])?
            .ok_or("__t_dup 无返回值".into())
    } else {
        Ok(v)
    }
}

/// 天权：置空被移动的 str 变量（规范 11.2.1）。
/// is_top 表示当前位置是"移动位"（str 绑定的顶层 / str 形参的实参）；
/// 其余位置（拼接、比较、tos/copy 参数、打印）只借用（规范 11.2.4）。
fn emit_move_nulls_native(
    e: &Expr,
    is_top: bool,
    ctx: &FnCtx,
    builder: &mut FunctionBuilder,
) -> Result<(), String> {
    match e {
        Expr::Var(name) => {
            if is_top {
                if let Some(&(var, vt)) = ctx.vars.get(name) {
                    // v3.0：str 与结构体变量在移动位都要置空（规范 14.4）
                    if is_owned(vt) {
                        let z = builder.ins().iconst(ptr_ty(), 0);
                        builder.def_var(var, z);
                    }
                }
            }
            Ok(())
        }
        Expr::Neg(x) => emit_move_nulls_native(x, false, ctx, builder),
        Expr::Bin { lhs, rhs, .. } => {
            emit_move_nulls_native(lhs, false, ctx, builder)?;
            emit_move_nulls_native(rhs, false, ctx, builder)
        }
        Expr::Call { name, args } => {
            let sig = ctx
                .sigs
                .get(name)
                .cloned()
                .ok_or("函数签名缺失（编译器内部错误）")?;
            for (i, (a, pt)) in args.iter().zip(&sig.params).enumerate() {
                // v4.3：借用 &[]T 形参不置空源（视图无所有权，规范 22.8）
                let borrowed = sig.borrows.get(i).copied().unwrap_or(false);
                if is_owned(*pt) && !borrowed {
                    emit_move_nulls_native(a, true, ctx, builder)?;
                }
            }
            Ok(())
        }
        // copy()/tos() 的参数只借用，但其参数内部的调用仍可能移动
        Expr::Convert { arg, .. } => emit_move_nulls_native(arg, false, ctx, builder),
        // v4.2：动态数组字面量内的 str 元素在移动位（is_top）置空
        Expr::DArrLit { elems, .. } => {
            for e in elems {
                emit_move_nulls_native(e, true, ctx, builder)?;
            }
            Ok(())
        }
        Expr::Sub { s, .. } => emit_move_nulls_native(s, false, ctx, builder),
        // v4.5：元组元素在移动位（is_top）置空
        Expr::TupExpr { elems, .. } => {
            for e in elems {
                emit_move_nulls_native(e, true, ctx, builder)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

// ── v3.0 结构体原生支持（规范第 14 节）──────────────────────────────
// 内存布局：每个字段固定占 8 字节槽位（标量经扩展后存入，读取时再收窄），
// 结构体整体为堆指针（malloc），与 C 后端内存布局不同但程序输出一致（双后端验收）。

/// 字段索引 → 字节偏移（每个字段 8 字节对齐槽位）
fn field_offset(idx: usize) -> i32 {
    (idx * 8) as i32
}

/// 按结构体类型与字段名定位 (定义, 字段索引, 字段类型)
fn struct_field(bt: Ty, fname: &str) -> Option<(StructDef, usize, Ty)> {
    match bt {
        Ty::Struct(i) => {
            let sds = structs();
            let sd = sds.get(i as usize)?;
            let idx = sd.fields.iter().position(|f| f.name == fname)?;
            Some((sd.clone(), idx, sd.fields[idx].ty))
        }
        _ => None,
    }
}

/// 按字段类型把值存入结构体槽位（标量窄化后扩展到 8 字节再存）
// ── v3.3 数组支持（规范第 16 节）────────────────────────────────

/// v3.7：sel 分支类型统一（与 type_check::sel_unify 对应，代码生成侧）
fn sel_unify_ty(at: Ty, bt: Ty) -> Ty {
    if at == bt {
        return at;
    }
    match (at, bt) {
        (Ty::I64, Ty::I32) | (Ty::I32, Ty::I64) => Ty::I64,
        (Ty::F64, Ty::I32) | (Ty::I32, Ty::F64) | (Ty::F64, Ty::I64) | (Ty::I64, Ty::F64) => Ty::F64,
        _ => unreachable!("sel 分支类型不符（应被 type_check 拦截）"),
    }
}

/// v3.3：Ty::Arr 索引 → (元素类型, 长度)
fn arr_def(t: Ty) -> (Ty, u64) {    match t {
        Ty::Arr(i) => {
            let a = arrs()[i as usize].clone();
            (a.elem, a.len)
        }
        _ => unreachable!("arr_def 仅用于数组类型"),
    }
}

/// v3.2/v3.3：条件成立 → __t_panic(msg) 并 trap；否则继续当前块。
/// 创建 panic/ok 两块，返回后 builder 处于 ok 块。
fn emit_panic_branch(
    ctx: &mut FnCtx,
    builder: &mut FunctionBuilder,
    cond: Value,
    msg: &str,
) -> Result<(), String> {
    let panic_blk = builder.create_block();
    let ok_blk = builder.create_block();
    builder.ins().brif(cond, panic_blk, &[], ok_blk, &[]);
    builder.seal_block(panic_blk);
    builder.seal_block(ok_blk);
    builder.switch_to_block(panic_blk);
    let m = str_ptr(ctx, builder, msg)?;
    let pfn = ctx.module.declare_func_in_func(ctx.rt.panic, &mut builder.func);
    builder.ins().call(pfn, &[m]);
    builder.ins().trap(cranelift_codegen::ir::TrapCode::unwrap_user(1));
    builder.switch_to_block(ok_blk);
    Ok(())
}

/// v3.3：数组下标守卫——无符号比较 idx >= len 同时覆盖负数与越界
/// （u64 视角下负数是巨大的值）。返回 I64 下标值。
fn emit_idx_guard(
    ctx: &mut FnCtx,
    builder: &mut FunctionBuilder,
    idx: Value,
    it: Ty,
    len: u64,
) -> Result<Value, String> {
    let i64idx = coerce(builder, idx, it, Ty::I64)?;
    let l = builder.ins().iconst(types::I64, len as i64);
    let bad = builder.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, i64idx, l);
    emit_panic_branch(ctx, builder, bad, "数组越界")?;
    Ok(i64idx)
}

/// v3.3：为数组变量创建栈槽（每元素 8 字节，8 字节对齐），
/// 变量持有槽地址指针。返回 (槽地址, 元素类型, 长度)。
fn create_arr_slot(
    ctx: &mut FnCtx,
    builder: &mut FunctionBuilder,
    t: Ty,
) -> (Value, Ty, u64) {
    let (elem, len) = arr_def(t);
    let slot = builder.func.create_sized_stack_slot(StackSlotData::new(
        StackSlotKind::ExplicitSlot,
        (len * 8) as u32,
        3, // 2^3 = 8 字节对齐
    ));
    let addr = builder.ins().stack_addr(ptr_ty(), slot, 0);
    let _ = ctx;
    (addr, elem, len)
}

/// v3.3：整体拷贝数组（逐元素 load/store，值语义，规范 16.1）
fn emit_arr_copy(
    builder: &mut FunctionBuilder,
    dst: Value,
    src: Value,
    elem: Ty,
    len: u64,
) -> Result<(), String> {
    for i in 0..len {
        let v = load_field(builder, src, (i * 8) as i32, elem)?;
        store_field(builder, dst, (i * 8) as i32, elem, v);
    }
    Ok(())
}

fn store_field(builder: &mut FunctionBuilder, base: Value, offset: i32, ft: Ty, v: Value) {
    match ft {
        Ty::F64 => { builder.ins().store(MemFlags::new(), v, base, offset); }
        Ty::Str => { builder.ins().store(MemFlags::new(), v, base, offset); }
        Ty::Struct(_) => { builder.ins().store(MemFlags::new(), v, base, offset); }
        Ty::I64 => { builder.ins().store(MemFlags::new(), v, base, offset); }
        Ty::I32 => {
            let v = builder.ins().sextend(types::I64, v);
            builder.ins().store(MemFlags::new(), v, base, offset);
        }
        Ty::Bool => {
            let v = builder.ins().uextend(types::I64, v);
            builder.ins().store(MemFlags::new(), v, base, offset);
        }
        Ty::BorrowStr => unreachable!("借用不可作为结构体字段（检查器已拦截）"),
        Ty::Arr(_) | Ty::DArr(_) => unreachable!("数组不可作为结构体字段（检查器已拦截）"),
        // v4.5：元组仅存在于返回边界，不可作为结构体字段（规范 24.2）
        Ty::Tuple(_) => unreachable!("元组不可作为结构体字段（检查器已拦截）"),
    }
}

/// 按字段类型从结构体槽位读取值
fn load_field(builder: &mut FunctionBuilder, base: Value, offset: i32, ft: Ty) -> Result<Value, String> {
    match ft {
        Ty::F64 => Ok(builder.ins().load(types::F64, MemFlags::new(), base, offset)),
        Ty::Str => Ok(builder.ins().load(ptr_ty(), MemFlags::new(), base, offset)),
        Ty::Struct(_) => Ok(builder.ins().load(ptr_ty(), MemFlags::new(), base, offset)),
        Ty::I64 => Ok(builder.ins().load(types::I64, MemFlags::new(), base, offset)),
        Ty::I32 => {
            let v = builder.ins().load(types::I64, MemFlags::new(), base, offset);
            Ok(builder.ins().ireduce(types::I32, v))
        }
        Ty::Bool => {
            let v = builder.ins().load(types::I64, MemFlags::new(), base, offset);
            Ok(builder.ins().ireduce(types::I8, v))
        }
        Ty::BorrowStr => Err("借用不可作为结构体字段（应被 type_check 拦截）".into()),
        Ty::Arr(_) | Ty::DArr(_) => Err("数组不可作为结构体字段（应被 type_check 拦截）".into()),
        // v4.5：元组仅存在于返回边界，不可作为结构体字段（规范 24.2）
        Ty::Tuple(_) => Err("元组不可作为结构体字段（应被 type_check 拦截）".into()),
    }
}

/// 深释放一个结构体：先逐 str 字段释放，再释放结构体本身；NULL 安全（守卫跳过）
fn emit_deep_free(
    ptr: Value,
    t: Ty,
    ctx: &mut FnCtx,
    builder: &mut FunctionBuilder,
) -> Result<(), String> {
    if t.is_darr() {
        // v4.0/v4.2：动态数组释放——str 元素逐个深释放（规范 22.6）
        let de = match t {
            Ty::DArr(i) => darrs()[i as usize].clone(),
            _ => unreachable!(),
        };
        let f = if de.elem == Ty::Str { ctx.rt.dfree_s } else { ctx.rt.free };
        call_rt(ctx, builder, f, ptr_ty(), 1, None, &[ptr])?;
        return Ok(());
    }
    let id = match t {
        Ty::Struct(i) => i,
        _ => return Ok(()),
    };
    let sds = structs();
    let sd = match sds.get(id as usize) {
        Some(s) => s,
        None => return Ok(()),
    };
    // NULL 守卫：未初始化的结构体槽位为 NULL，不可对其字段取值
    let zero = builder.ins().iconst(ptr_ty(), 0);
    let is_null = builder.ins().icmp(IntCC::Equal, ptr, zero);
    let skip = builder.create_block();
    let body = builder.create_block();
    builder.ins().brif(is_null, skip, &[], body, &[]);
    builder.switch_to_block(body);
    builder.seal_block(body);
    for (i, f) in sd.fields.iter().enumerate() {
        if f.ty == Ty::Str {
            let off = field_offset(i);
            let sp = builder.ins().load(ptr_ty(), MemFlags::new(), ptr, off);
            let fr = ctx.rt.free;
            call_rt(ctx, builder, fr, ptr_ty(), 1, None, &[sp])?;
        }
    }
    let fr = ctx.rt.free;
    call_rt(ctx, builder, fr, ptr_ty(), 1, None, &[ptr])?;
    builder.ins().jump(skip, &[]);
    builder.switch_to_block(skip);
    builder.seal_block(skip);
    Ok(())
}

/// 结构体赋值/返回右侧求值：字面量 → 构造；其余（变量移动 / 函数调用）按表达式求值
fn emit_struct_rhs(
    value: &Expr,
    ctx: &mut FnCtx,
    builder: &mut FunctionBuilder,
) -> Result<Value, String> {
    match value {
        Expr::StructLit { name, fields, .. } => emit_struct_lit(name, fields, ctx, builder),
        _ => Ok(emit_expr(value, ctx, builder)?.0),
    }
}

/// 构造结构体字面量：malloc 一块内存，逐字段求值后存入对应槽位；
/// str 字段经由 emit_bind_native（字面量/tos 复制独立副本），其被移动的源变量随后置空
fn emit_struct_lit(
    name: &str,
    fields: &[(Option<String>, Expr)],
    ctx: &mut FnCtx,
    builder: &mut FunctionBuilder,
) -> Result<Value, String> {
    let sds = structs();
    let sd = sds
        .iter()
        .find(|s| s.name == name)
        .ok_or_else(|| format!("结构体 '{}' 未定义（应被 type_check 拦截）", name))?;
    let size = (sd.fields.len() * 8) as i64;
    let sz = builder.ins().iconst(types::I64, size);
    let m = ctx.rt.malloc;
    let ptr = call_rt(ctx, builder, m, types::I64, 1, Some(ptr_ty()), &[sz])?
        .ok_or("__t_malloc 无返回值")?;
    for (i, (optname, fexpr)) in fields.iter().enumerate() {
        let (idx, ft) = match optname {
            Some(nm) => {
                let pos = sd
                    .fields
                    .iter()
                    .position(|f| f.name == *nm)
                    .ok_or_else(|| format!("结构体 '{}' 无字段 '{}'", name, nm))?;
                (pos, sd.fields[pos].ty)
            }
            None => (i, sd.fields[i].ty),
        };
        let offset = field_offset(idx);
        if ft == Ty::Str {
            // 复用 str 绑定语义：字面量/tos 复制，其余（变量/拼接/调用）直接接管指针；
            // 随后由 emit_move_nulls_native 置空被移动的源变量（规范 14.2 / 11.2.1）
            let v = emit_bind_native(fexpr, ctx, builder)?;
            store_field(builder, ptr, offset, Ty::Str, v);
            emit_move_nulls_native(fexpr, true, ctx, builder)?;
        } else {
            let (v, vt) = emit_expr(fexpr, ctx, builder)?;
            let v = coerce(builder, v, vt, ft)?;
            store_field(builder, ptr, offset, ft, v);
        }
    }
    Ok(ptr)
}

/// 声明字符串字面量为只读数据段并返回指针值（模块级去重）
fn str_ptr(ctx: &mut FnCtx, builder: &mut FunctionBuilder, s: &str) -> Result<Value, String> {
    if let Some(&data_id) = ctx.str_pool.get(s) {
        let gv = ctx.module.declare_data_in_func(data_id, &mut builder.func);
        return Ok(builder.ins().symbol_value(ptr_ty(), gv));
    }
    let mut bytes = s.as_bytes().to_vec();
    bytes.push(0);
    let name = format!("__t_str{}", ctx.str_count);
    *ctx.str_count += 1;
    let data_id = ctx
        .module
        .declare_data(&name, Linkage::Local, false, false)
        .map_err(|e| e.to_string())?;
    let mut dd = DataDescription::new();
    dd.define(bytes.into_boxed_slice());
    ctx.module.define_data(data_id, &dd).map_err(|e| e.to_string())?;
    ctx.str_pool.insert(s.to_string(), data_id);
    let gv = ctx
        .module
        .declare_data_in_func(data_id, &mut builder.func);
    Ok(builder.ins().symbol_value(ptr_ty(), gv))
}

fn emit_expr(
    e: &Expr,
    ctx: &mut FnCtx,
    builder: &mut FunctionBuilder,
) -> Result<(Value, Ty), String> {
    match e {
        Expr::Int(v) => Ok((builder.ins().iconst(types::I64, *v), Ty::I64)),
        Expr::Float(v) => Ok((builder.ins().f64const(*v), Ty::F64)),
        Expr::Str(s) => {
            let p = str_ptr(ctx, builder, s)?;
            Ok((p, Ty::Str))
        }
        Expr::Bool(b) => Ok((builder.ins().iconst(types::I8, *b as i64), Ty::Bool)),
        Expr::Var(name) => {
            let (var, t) = *ctx
                .vars
                .get(name)
                .ok_or_else(|| format!("变量 '{}' 未声明（编译器内部错误）", name))?;
            Ok((builder.use_var(var), t))
        }
        // v2.1 借用：按只读指针传递，无复制、无移动、无置空（规范 11.6）
        Expr::Borrow(inner) => {
            let (v, _) = emit_expr(inner, ctx, builder)?;
            Ok((v, Ty::BorrowStr))
        }
        // v3.3/v4.0：a[i] 读 —— 定长守卫+栈槽加载；动态 __t_dget_*（规范 16.3/22）
        Expr::Index(base, idx) => {
            let (bp, bt) = emit_expr(base, ctx, builder)?;
            let (iv, it) = emit_expr(idx, ctx, builder)?;
            if let Some(de) = darr_by_id(&darrs(), bt).cloned() {
                let i = coerce(builder, iv, it, Ty::I64)?;
                if de.elem == Ty::F64 {
                    let r = call_rt(ctx, builder, ctx.rt.dget_f, types::I64, 2, Some(types::F64), &[bp, i])?
                        .ok_or("__t_dget_f 无返回值")?;
                    return Ok((r, Ty::F64));
                }
                if de.elem == Ty::Str {
                    let r = call_rt(ctx, builder, ctx.rt.dget_s, types::I64, 2, Some(ptr_ty()), &[bp, i])?
                        .ok_or("__t_dget_s 无返回值")?;
                    return Ok((r, Ty::Str));
                }
                let r = call_rt(ctx, builder, ctx.rt.dget_i, types::I64, 2, Some(types::I64), &[bp, i])?
                    .ok_or("__t_dget_i 无返回值")?;
                // v4.6：Bool 槽位按 I64 存储，读回截断为 I8（与 cl_ty(Bool) 一致）
                if de.elem == Ty::Bool {
                    let b = builder.ins().ireduce(types::I8, r);
                    return Ok((b, Ty::Bool));
                }
                return Ok((r, de.elem));
            }
            // v4.2：str 字节读（规范第 23 节）
            if norm(bt) == Ty::Str {
                let i = coerce(builder, iv, it, Ty::I64)?;
                let r = call_rt(ctx, builder, ctx.rt.sget, types::I64, 2, Some(types::I64), &[bp, i])?
                    .ok_or("__t_sget 无返回值")?;
                return Ok((r, Ty::I64));
            }
            let (elem, len) = arr_def(bt);
            let i = emit_idx_guard(ctx, builder, iv, it, len)?;
            let eight = builder.ins().iconst(types::I64, 8);
            let off = builder.ins().imul(i, eight);
            let addr = builder.ins().iadd(bp, off);
            let v = load_field(builder, addr, 0, elem)?;
            Ok((v, elem))
        }
        // v3.3/v3.5：len(a) 数组 → 编译期常量；len(s) str → strlen（UTF-8 字节数，规范 16.7）
        Expr::Len(inner) => {
            let t = emit_ty_of(inner, ctx).ok_or("len() 实参类型未知（编译器内部错误）")?;
            if norm(t).is_darr() {
                let (bp, _) = emit_expr(inner, ctx, builder)?;
                let r = call_rt(ctx, builder, ctx.rt.dlen, types::I64, 1, Some(types::I64), &[bp])?
                    .ok_or("__t_dlen 无返回值")?;
                Ok((r, Ty::I64))
            } else if norm(t) == Ty::Str {
                let (v, _) = emit_expr(inner, ctx, builder)?;
                let r = call_rt(ctx, builder, ctx.rt.strlen, types::I64, 1, Some(types::I64), &[v])?
                    .ok_or("strlen 无返回值")?;
                Ok((r, Ty::I64))
            } else {
                let (_, len) = arr_def(t);
                Ok((builder.ins().iconst(types::I64, len as i64), Ty::I64))
            }
        }
        // 数组字面量只出现在声明/赋值 RHS（已特判），不会走到通用表达式路径
        Expr::ArrLit { .. } => Err("数组字面量位置非法（应被 type_check 拦截）".into()),
        // v4.0：动态数组字面量/push 在声明与语句位置特判
        // v4.2：sub(s, start, n) —— 运行时拷贝出新所有权的堆串（规范第 23 节）
        Expr::Sub { s, start, n } => {
            let (sv, st) = emit_expr(s, ctx, builder)?;
            // v4.6 修复：借用 &str 在只读读取位视同 str（同 Bin 臂 norm 先例，规范 11.6.2）
            let sp = if st == Ty::BorrowStr { sv } else { coerce(builder, sv, st, Ty::Str)? };
            let (iv, it) = emit_expr(start, ctx, builder)?;
            let i = coerce(builder, iv, it, Ty::I64)?;
            let (nv, nt) = emit_expr(n, ctx, builder)?;
            let nn = coerce(builder, nv, nt, Ty::I64)?;
            let r = call_rt(ctx, builder, ctx.rt.sub, types::I64, 3, Some(ptr_ty()), &[sp, i, nn])?
                .ok_or("__t_sub 无返回值")?;
            Ok((r, Ty::Str))
        }
        Expr::DArrLit { .. } => Err("动态数组字面量位置非法（应被 type_check 拦截）".into()),
        Expr::Push { .. } => Err("push 只能作为语句（应被 type_check 拦截）".into()),
        Expr::Pop { .. } => Err("pop 只能作为语句（应被 type_check 拦截）".into()),
        // v4.5：元组表达式 → malloc 块 + 逐元素存储（每元素 8 字节槽，与 C 后端布局一致；规范第 24 节）
        Expr::TupExpr { elems, tup } => {
            let td = crate::type_check::tuples()[*tup as usize].clone();
            let size = builder.ins().iconst(types::I64, (td.elems.len() as i64) * 8);
            let block = call_rt(ctx, builder, ctx.rt.malloc, types::I64, 1, Some(ptr_ty()), &[size])?
                .ok_or("__t_malloc 无返回值")?;
            for (i, e) in elems.iter().enumerate() {
                let off = (i as i32) * 8;
                let t = td.elems[i];
                if t == Ty::Str {
                    let v = emit_bind_native(e, ctx, builder)?;
                    store_field(builder, block, off, Ty::Str, v);
                } else if t == Ty::F64 {
                    let (v, vt) = emit_expr(e, ctx, builder)?;
                    let v = coerce(builder, v, vt, Ty::F64)?;
                    store_field(builder, block, off, Ty::F64, v);
                } else {
                    let (v, vt) = emit_expr(e, ctx, builder)?;
                    let v = coerce(builder, v, vt, Ty::I64)?;
                    store_field(builder, block, off, t, v);
                }
            }
            Ok((block, Ty::Tuple(*tup)))
        }
        // v3.7：sel 条件表达式 —— 块参数实现惰性求值（与 C 三元语义一致，规范第 18 节）
        Expr::Sel { cond, a, b } => {
            let (cv, ct) = emit_expr(cond, ctx, builder)?;
            if ct != Ty::Bool {
                return Err("sel 条件必须是 bool（应被 type_check 拦截）".into());
            }
            // v4.6 修复：cv 已是 I8（Bool），brif 直接接受——同宽 uextend 非法
            // （此前 sel 在原生后端从未真正编译过，一直被 C 回退掩盖）
            let (at, bt) = match emit_ty_of(a, ctx).zip(emit_ty_of(b, ctx)) {
                Some((x, y)) => (norm(x), norm(y)),
                None => return Err("sel 分支类型未知（编译器内部错误）".into()),
            };
            let unified = sel_unify_ty(at, bt);
            let uty = cl_ty(unified);
            let merge = builder.create_block();
            builder.append_block_param(merge, uty);
            let then_blk = builder.create_block();
            let else_blk = builder.create_block();
            builder.ins().brif(cv, then_blk, &[], else_blk, &[]);
            builder.seal_block(then_blk);
            builder.seal_block(else_blk);
            builder.switch_to_block(then_blk);
            let (av, avt) = emit_expr(a, ctx, builder)?;
            let av = coerce(builder, av, avt, unified)?;
            builder.ins().jump(merge, &[av]);
            builder.switch_to_block(else_blk);
            let (bv, bvt) = emit_expr(b, ctx, builder)?;
            let bv = coerce(builder, bv, bvt, unified)?;
            builder.ins().jump(merge, &[bv]);
            builder.seal_block(merge);
            builder.switch_to_block(merge);
            Ok((builder.block_params(merge)[0], unified))
        }
        Expr::Neg(x) => {
            let (v, t) = emit_expr(x, ctx, builder)?;
            let r = match t {
                Ty::F64 => builder.ins().fneg(v),
                Ty::I32 | Ty::I64 => builder.ins().ineg(v),
                other => return Err(format!("类型 {:?} 不能取负", other)),
            };
            Ok((r, t))
        }
        Expr::Bin { op, lhs, rhs } => {
            // v2.1：借用值在只读运算位视同 str（规范 11.6.2）
            let norm = |t: Ty| if t == Ty::BorrowStr { Ty::Str } else { t };
            let lt = norm(emit_ty_of(lhs, ctx).ok_or("内部错误：左侧类型未知")?);
            let rt = norm(emit_ty_of(rhs, ctx).ok_or("内部错误：右侧类型未知")?);
            // str 比较 / 拼接走运行时
            if lt == Ty::Str && rt == Ty::Str {
                let (a, _) = emit_expr(lhs, ctx, builder)?;
                let (b, _) = emit_expr(rhs, ctx, builder)?;
                return match op {
                    BinOp::Add => {
                        let r = call_rt(ctx, builder, ctx.rt.cat, ptr_ty(), 2, Some(ptr_ty()), &[a, b])?
                            .ok_or("__t_cat 无返回值")?;
                        Ok((r, Ty::Str))
                    }
                    BinOp::Eq => {
                        let r = call_rt(ctx, builder, ctx.rt.seq, ptr_ty(), 2, Some(types::I32), &[a, b])?
                            .ok_or("__t_seq 无返回值")?;
                        let r8 = builder.ins().ireduce(types::I8, r);
                        Ok((r8, Ty::Bool))
                    }
                    BinOp::Ne => {
                        let r = call_rt(ctx, builder, ctx.rt.sne, ptr_ty(), 2, Some(types::I32), &[a, b])?
                            .ok_or("__t_sne 无返回值")?;
                        let r8 = builder.ins().ireduce(types::I8, r);
                        Ok((r8, Ty::Bool))
                    }
                    _ => Err("str 仅支持 + 与 ==/!=（应被 type_check 拦截）".into()),
                };
            }
            let (a, _) = emit_expr(lhs, ctx, builder)?;
            let (b, _) = emit_expr(rhs, ctx, builder)?;
            match op.c_str() {
                "<" | ">" | "<=" | ">=" | "==" | "!=" => {
                    if lt == Ty::F64 || rt == Ty::F64 {
                        let a = coerce(builder, a, lt, Ty::F64)?;
                        let b = coerce(builder, b, rt, Ty::F64)?;
                        let fcc = match op {
                            BinOp::Lt => FloatCC::LessThan,
                            BinOp::Gt => FloatCC::GreaterThan,
                            BinOp::Le => FloatCC::LessThanOrEqual,
                            BinOp::Ge => FloatCC::GreaterThanOrEqual,
                            BinOp::Eq => FloatCC::Equal,
                            _ => FloatCC::NotEqual,
                        };
                        Ok((builder.ins().fcmp(fcc, a, b), Ty::Bool))
                    } else {
                        let wide = if lt == Ty::I64 || rt == Ty::I64 { Ty::I64 } else { Ty::I32 };
                        let a = coerce(builder, a, lt, wide)?;
                        let b = coerce(builder, b, rt, wide)?;
                        let icc = match op {
                            BinOp::Lt => IntCC::SignedLessThan,
                            BinOp::Gt => IntCC::SignedGreaterThan,
                            BinOp::Le => IntCC::SignedLessThanOrEqual,
                            BinOp::Ge => IntCC::SignedGreaterThanOrEqual,
                            BinOp::Eq => IntCC::Equal,
                            _ => IntCC::NotEqual,
                        };
                        Ok((builder.ins().icmp(icc, a, b), Ty::Bool))
                    }
                }
                _ => {
                    if lt == Ty::F64 || rt == Ty::F64 {
                        let a = coerce(builder, a, lt, Ty::F64)?;
                        let b = coerce(builder, b, rt, Ty::F64)?;
                        let r = match op {
                            BinOp::Add => builder.ins().fadd(a, b),
                            BinOp::Sub => builder.ins().fsub(a, b),
                            BinOp::Mul => builder.ins().fmul(a, b),
                            BinOp::Div => builder.ins().fdiv(a, b),
                            _ => return Err("内部错误".into()),
                        };
                        Ok((r, Ty::F64))
                    } else {
                        let wide = if lt == Ty::I64 || rt == Ty::I64 { Ty::I64 } else { Ty::I32 };
                        let a = coerce(builder, a, lt, wide)?;
                        let b = coerce(builder, b, rt, wide)?;
                        let r = match op {
                            BinOp::Add => builder.ins().iadd(a, b),
                            BinOp::Sub => builder.ins().isub(a, b),
                            BinOp::Mul => builder.ins().imul(a, b),
                            BinOp::Div => {
                                // v3.2：除零快速失败——b==0 时调 __t_panic 退出，
                                // 与 C 后端行为一致（规范 15.1），替代裸 sdiv 陷阱
                                let z = builder.ins().iconst(cl_ty(wide), 0);
                                let isz = builder.ins().icmp(IntCC::Equal, b, z);
                                emit_panic_branch(ctx, builder, isz, "除数为零")?;
                                builder.ins().sdiv(a, b)
                            }
                            _ => return Err("内部错误".into()),
                        };
                        Ok((r, wide))
                    }
                }
            }
        }
        Expr::Call { name, args } => {
            let (id, _) = *ctx
                .decls
                .get(name)
                .ok_or_else(|| format!("函数 '{}' 未定义（编译器内部错误）", name))?;
            let sig = ctx.sigs.get(name).ok_or("内部错误")?.clone();
            if args.len() != sig.params.len() {
                return Err(format!("函数 '{}' 参数个数不符（编译器内部错误）", name));
            }
            let mut vals = Vec::new();
            for (a, pt) in args.iter().zip(&sig.params) {
                let (v, vt) = emit_expr(a, ctx, builder)?;
                vals.push(coerce(builder, v, vt, *pt)?);
            }
            let local = ctx.module.declare_func_in_func(id, &mut builder.func);
            let call = builder.ins().call(local, &vals);
            let ret_ty = sig.ret;
            match builder.inst_results(call).first().copied() {
                Some(v) => Ok((v, ret_ty)),
                // 无返回值调用不应出现在表达式位置（type_check 保证）
                None => Ok((builder.ins().iconst(types::I64, 0), Ty::I64)),
            }
        }
        Expr::Convert { name, arg } => {
            let (v, t) = emit_expr(arg, ctx, builder)?;
            if name == "tof" {
                // v4.4：tof(s)（规范第 7 节）；v4.6：借用 &str 只读位视同 str
                let sp = if t == Ty::BorrowStr { v } else { coerce(builder, v, t, Ty::Str)? };
                let r = call_rt(ctx, builder, ctx.rt.tof, ptr_ty(), 1, Some(types::F64), &[sp])?
                    .ok_or("__t_tof 无返回值")?;
                return Ok((r, Ty::F64));
            }
            if name == "copy" {
                // copy(s)/copy(&s)：显式复制为独立所有者（规范 11.2.5）
                // 无论源是 str 还是借用 &str，都调 __t_dup 产生独立堆副本
                let d = ctx.rt.dup;
                let r = call_rt(ctx, builder, d, ptr_ty(), 1, Some(ptr_ty()), &[v])?
                    .ok_or("__t_dup 无返回值")?;
                return Ok((r, Ty::Str));
            }
            if name == "toi" {
                match t {
                    Ty::F64 => {
                        let r = builder.ins().fcvt_to_sint_sat(types::I64, v);
                        Ok((r, Ty::I64))
                    }
                    Ty::I32 => Ok((builder.ins().sextend(types::I64, v), Ty::I64)),
                    Ty::I64 => Ok((v, Ty::I64)),
                    Ty::Str => match arg.as_ref() {
                        Expr::Str(s) => {
                            let n: i64 = s
                                .parse()
                                .map_err(|_| format!("字符串 '{}' 无法转换为整数", s))?;
                            Ok((builder.ins().iconst(types::I64, n), Ty::I64))
                        }
                        _ => Err("toi 不能用于非字面量字符串（应被 type_check 拦截）".into()),
                    },
                    other => Err(format!("toi 不能用于 {:?}", other)),
                }
            } else {
                // tos：数字/bool → str；str 原样（v2.1：借用视同 str，规范 11.6.2）
                match t {
                    Ty::Str | Ty::BorrowStr => Ok((v, Ty::Str)),
                    Ty::F64 => {
                        let r = call_rt(ctx, builder, ctx.rt.tos_f, types::F64, 1, Some(ptr_ty()), &[v])?
                            .ok_or("__t_tos_f 无返回值")?;
                        Ok((r, Ty::Str))
                    }
                    Ty::Bool => {
                        let a = builder.ins().uextend(types::I32, v);
                        let r = call_rt(ctx, builder, ctx.rt.tos_b, types::I32, 1, Some(ptr_ty()), &[a])?
                            .ok_or("__t_tos_b 无返回值")?;
                        Ok((r, Ty::Str))
                    }
                    Ty::I32 => {
                        let a = builder.ins().sextend(types::I64, v);
                        let r = call_rt(ctx, builder, ctx.rt.tos_ll, types::I64, 1, Some(ptr_ty()), &[a])?
                            .ok_or("__t_tos_ll 无返回值")?;
                        Ok((r, Ty::Str))
                    }
                    Ty::I64 => {
                        let r = call_rt(ctx, builder, ctx.rt.tos_ll, types::I64, 1, Some(ptr_ty()), &[v])?
                            .ok_or("__t_tos_ll 无返回值")?;
                        Ok((r, Ty::Str))
                    }
                    // 结构体不可转换（检查器已拦截）；此处仅为 match 完备性
                    Ty::Struct(_) => Err("结构体不支持 tos/copy（应被 type_check 拦截）".into()),
                    Ty::Arr(_) | Ty::DArr(_) => Err("数组不支持 tos/copy（应被 type_check 拦截）".into()),
                    // v4.5：元组不支持 tos/copy（仅存在于返回边界，规范 24.2）
                    Ty::Tuple(_) => Err("元组不支持 tos/copy（应被 type_check 拦截）".into()),
                }
            }
        }
        // v3.0：结构体字面量 → malloc + 逐字段存储（规范 14.2）
        Expr::StructLit { name, fields, .. } => {
            let ptr = emit_struct_lit(name, fields, ctx, builder)?;
            let id = structs()
                .iter()
                .position(|s| s.name == *name)
                .ok_or("结构体未定义（编译器内部错误）")?;
            Ok((ptr, Ty::Struct(id as u32)))
        }
        // v3.0：字段读取 p.x —— base 是结构体指针，按偏移加载（规范 14.3）；
        // str 字段返回其指针（只读借用，不复制）；字段不可移动（借用语义）
        Expr::Field(base, fname) => {
            let (bptr, _) = emit_expr(base, ctx, builder)?;
            let bt = emit_ty_of(base, ctx)
                .ok_or("字段读取：左侧结构体类型未知（编译器内部错误）")?;
            let (_sd, idx, ft) = struct_field(bt, fname)
                .ok_or_else(|| format!("结构体无字段 '{}'（应被 type_check 拦截）", fname))?;
            let offset = field_offset(idx);
            let v = load_field(builder, bptr, offset, ft)?;
            Ok((v, ft))
        }
    }
}
