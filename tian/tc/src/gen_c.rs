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
        // v3.4：数组参数 = 只读视图（const 指针，零拷贝；参数不可写，规范 16.6）
        Ty::Arr(i) => {
            let a = crate::type_check::arrs()[i as usize].clone();
            format!("const {} *", a.elem.c_name())
        }
        // v4.0：动态数组 = 堆指针（参与天权，规范第 22 节）
        Ty::DArr(_) => "void*".to_string(),
        // v4.7：map = 堆指针（参与天权，规范第 25 节）
        Ty::Map(_) => "void*".to_string(),
        // v4.5：元组 = 堆块指针（仅返回边界，规范第 24 节）
        Ty::Tuple(_) => "void*".to_string(),
        other => other.c_name().to_string(),
    }
}

/// v3.3：数组定义查询（Ty::Arr 索引 → (元素 C 类型, 长度)）
fn arr_of(t: Ty) -> (Ty, u64) {
    match t {
        Ty::Arr(i) => {
            let a = crate::type_check::arrs()[i as usize].clone();
            (a.elem, a.len)
        }
        _ => unreachable!("arr_of 仅用于数组类型"),
    }
}

/// v4.7：map 键类型 → wire 前缀（i64→i、str→s；规范第 25 节）
fn map_key_wire(t: Ty) -> &'static str {
    if t == Ty::Str {
        "s"
    } else {
        "i"
    }
}

/// v4.7：map 值类型 → wire 后缀（i32/i64/bool→i、f64→f、str→s；规范第 25 节）
fn map_val_wire(t: Ty) -> &'static str {
    match t {
        Ty::Str => "s",
        Ty::F64 => "f",
        _ => "i",
    }
}

/// v4.7：map 运行时函数后缀（mnew/mget/mhas/mset/mdel/mfree 共用），如 "ii"/"ss"/"if"
fn map_wire(m: &MapDef) -> String {
    format!("{}{}", map_key_wire(m.key), map_val_wire(m.val))
}

/// v4.7：释放一个 map 堆槽（深释放，__t_mfree_{suffix}；NULL 安全）
fn free_slot(var_c: &str, t: Ty) -> String {
    match t {
        Ty::Struct(i) => {
            let sd = structs()[i as usize].clone();
            format!("{}({});", free_fn(&sd), var_c)
        }
        // v4.2：str 动态数组逐元素深释放（规范 22.6）
        Ty::DArr(i) => {
            let d = crate::type_check::darrs()[i as usize].clone();
            if d.elem == Ty::Str {
                format!("__t_dfree_s({});", var_c)
            } else {
                format!("__t_free({});", var_c)
            }
        }
        // v4.7：map 整体深释放（NULL 安全，规范 25.7）
        Ty::Map(i) => {
            let m = crate::type_check::maps()[i as usize].clone();
            format!("__t_mfree_{}({});", map_wire(&m), var_c)
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

/// v4.7：map 字面量构造代码块（规范第 25 节）。
/// 发出 `{ void* __t_h = __t_mnew(8); 逐条目 __t_mset_{wire}(__t_h, k, v); tail }`。
/// 每个条目：str 键/值走 emit_bind（字面量 dup / 变量移动），标量走 emit_expr。
/// 块末 `tail` 由调用方给出（含换行前的两个空格缩进），通常为 `"{cn} = __t_h; }}"`。
fn emit_map_ctor(
    t: Ty,
    entries: &[(Expr, Expr)],
    scope: &Scope,
    sigs: &HashMap<String, FuncSig>,
    out: &mut String,
    level: usize,
    tail: &str,
) {
    let table = crate::type_check::maps();
    let m = match t {
        Ty::Map(i) => table[i as usize].clone(),
        _ => unreachable!("emit_map_ctor 仅用于 map 类型"),
    };
    let kwire = map_key_wire(m.key);
    let vwire = map_val_wire(m.val);
    let suffix = map_wire(&m);
    indent(out, level);
    let _ = writeln!(out, "{{ void* __t_h = __t_mnew(8);");
    for (k, v) in entries.iter() {
        let mut ks = String::new();
        let mut vs = String::new();
        if kwire == "s" {
            emit_bind(k, scope, sigs, &mut ks);
        } else {
            emit_expr(k, scope, sigs, &mut ks);
        }
        if vwire == "s" {
            emit_bind(v, scope, sigs, &mut vs);
        } else {
            emit_expr(v, scope, sigs, &mut vs);
        }
        let key_arg = if kwire == "s" {
            ks
        } else {
            format!("(long long)({})", ks)
        };
        let val_arg = if vwire == "s" || vwire == "f" {
            vs
        } else {
            format!("(long long)({})", vs)
        };
        indent(out, level);
        let _ = writeln!(out, "  __t_h = __t_mset_{}(__t_h, {}, {});", suffix, key_arg, val_arg);
    }
    indent(out, level);
    let _ = writeln!(out, "  {}", tail);
}

/// 生成完整 C 源码
pub fn generate(prog: &Program) -> String {
    let mut out = String::new();
    // v3.0：供 emit_* 内部查询结构体布局（线程局部表，见 type_check::set_structs）
    crate::type_check::set_structs(&prog.structs);
    crate::type_check::set_arrs(&prog.arrs);
    crate::type_check::set_darrs(&prog.darrs);
    crate::type_check::set_tuples(&prog.tuples);
    crate::type_check::set_maps(&prog.maps);

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
         static void __t_free(const char* p){ free((void*)p); }\n\
         static void __t_panic(const char* w){ fprintf(stderr, \"运行时错误：%s\\n\", w?w:\"\"); exit(1); }\n\
         static long long __t_idx(long long len, long long i){ if(i<0||i>=len) __t_panic(\"数组越界\"); return i; }\n\
         typedef struct { long long len, cap; } __t_darr_hdr;\n\
         static char* __t_darr_data(void* h){ return (char*)h + 16; }\n\
         static void* __t_dgrow(void* h){ __t_darr_hdr* x=(__t_darr_hdr*)h; x->cap*=2; void* n=realloc(h,16+(size_t)x->cap*8); if(!n){fprintf(stderr,\"天运行时：内存分配失败\\n\");exit(1);} return n; }\n\
         static void* __t_dnew(long long cap){ if(cap<4)cap=4; void* h=malloc(16+(size_t)cap*8); if(!h){fprintf(stderr,\"天运行时：内存分配失败\\n\");exit(1);} ((__t_darr_hdr*)h)->len=0; ((__t_darr_hdr*)h)->cap=cap; return h; }\n\
         static long long __t_dlen(void* h){ return ((__t_darr_hdr*)h)->len; }\n\
         static void __t_dbound(void* h, long long i){ long long len=((__t_darr_hdr*)h)->len; if(i<0||i>=len) __t_panic(\"数组越界\"); }\n\
         static long long __t_dget_i(void* h, long long i){ __t_dbound(h,i); return ((long long*)__t_darr_data(h))[i]; }\n\
         static double __t_dget_f(void* h, long long i){ __t_dbound(h,i); return ((double*)__t_darr_data(h))[i]; }\n\
         static void __t_dset_i(void* h, long long i, long long v){ __t_dbound(h,i); ((long long*)__t_darr_data(h))[i]=v; }\n\
         static void __t_dset_f(void* h, long long i, double v){ __t_dbound(h,i); ((double*)__t_darr_data(h))[i]=v; }\n\
         static void* __t_dpush_i(void* h, long long v){ __t_darr_hdr* x=(__t_darr_hdr*)h; if(x->len==x->cap) h=__t_dgrow(h); ((long long*)__t_darr_data(h))[((__t_darr_hdr*)h)->len]=v; ((__t_darr_hdr*)h)->len++; return h; }\n\
         static void* __t_dpush_f(void* h, double v){ __t_darr_hdr* x=(__t_darr_hdr*)h; if(x->len==x->cap) h=__t_dgrow(h); ((double*)__t_darr_data(h))[((__t_darr_hdr*)h)->len]=v; ((__t_darr_hdr*)h)->len++; return h; }\n\
         static void __t_dpop(void* h){ __t_darr_hdr* x=(__t_darr_hdr*)h; if(x->len==0) __t_panic(\"pop 空数组\"); x->len--; }\n\
         static long long __t_sget(const char* s, long long i){ long long n=(long long)strlen(s); if(i<0||i>=n) __t_panic(\"字符串下标越界\"); return (long long)(unsigned char)s[i]; }\n\
         static void* __t_dpush_s(void* h, char* v){ __t_darr_hdr* x=(__t_darr_hdr*)h; if(x->len==x->cap) h=__t_dgrow(h); ((char**)__t_darr_data(h))[((__t_darr_hdr*)h)->len]=v; ((__t_darr_hdr*)h)->len++; return h; }\n\
         static char* __t_dget_s(void* h, long long i){ __t_dbound(h,i); return ((char**)__t_darr_data(h))[i]; }\n\
         static void __t_dfree_s(void* h){ __t_darr_hdr* x=(__t_darr_hdr*)h; for(long long i=0;i<x->len;i++) free(((char**)__t_darr_data(h))[i]); free(h); }\n\
         static double __t_tof(const char* s){ char* end=0; double v=strtod(s,&end); if(end==s||*end!=0) __t_panic(\"无效数字\"); return v; }\n\
         static char* __t_sub(const char* s, long long start, long long n){ long long len=(long long)strlen(s); if(start<0||n<0||start+n>len) __t_panic(\"子串越界\"); char* r=(char*)malloc((size_t)n+1); if(!r){fprintf(stderr,\"天运行时：内存分配失败\\n\");exit(1);} memcpy(r,s+start,(size_t)n); r[n]=0; return r; }\n\
         static int __t_idiv_i(int a, int b){ if(b==0) __t_panic(\"除数为零\"); return a/b; }\n\
         static long long __t_idiv_l(long long a, long long b){ if(b==0) __t_panic(\"除数为零\"); return a/b; }\n\
         static void* __t_malloc(long long size){ void* p=malloc((size_t)size); if(!p){fprintf(stderr,\"天运行时：内存分配失败\\n\");exit(1);} return p; }\n\
         typedef struct __t_mnode{ long long hash; struct __t_mnode* next; long long key; long long val; } __t_mnode;\n\
         typedef struct { long long len, cap; } __t_map_hdr;\n\
         static __t_mnode** __t_mbuckets(void* h){ return (__t_mnode**)((char*)h + 16); }\n\
         static long long __t_mhash_s(const char* s){ long long h=0xcbf29ce484222325; while(*s) h=(h^(unsigned char)*s++)*0x100000001b3ull; return h; }\n\
         static long long __t_mhash_i(long long x){ x^=x>>33; x*=0xff51afd7ed558ccdull; x^=x>>33; x*=0xc4ceb9fe1a85ec53ull; x^=x>>33; return x; }\n\
         static __t_mnode* __t_mfind_i(void* h, long long k){ __t_map_hdr* x=(__t_map_hdr*)h; __t_mnode* n=__t_mbuckets(h)[__t_mhash_i(k)&(x->cap-1)]; while(n&&n->key!=k)n=n->next; return n; }\n\
         static __t_mnode* __t_mfind_s(void* h, const char* k){ __t_map_hdr* x=(__t_map_hdr*)h; __t_mnode* n=__t_mbuckets(h)[__t_mhash_s(k)&(x->cap-1)]; while(n&&strcmp((char*)n->key,k))n=n->next; return n; }\n\
         static void* __t_mgrow(void* h){ __t_map_hdr* x=(__t_map_hdr*)h; long long oldcap=x->cap, newcap=oldcap*2; void* n=realloc(h,16+(size_t)newcap*8); if(!n){fprintf(stderr,\"天运行时：内存分配失败\\n\");exit(1);} ((__t_map_hdr*)n)->cap=newcap; __t_mnode** b=__t_mbuckets(n); memset(b+oldcap,0,(size_t)oldcap*8); for(long long i=0;i<oldcap;i++){ __t_mnode* cur=b[i]; b[i]=NULL; while(cur){ __t_mnode* nx=cur->next; long long j=cur->hash&(newcap-1); cur->next=b[j]; b[j]=cur; cur=nx; } } return n; }\n\
         static __t_mnode* __t_mnode_new(void){ __t_mnode* n=(__t_mnode*)malloc(sizeof(__t_mnode)); if(!n){fprintf(stderr,\"天运行时：内存分配失败\\n\");exit(1);} return n; }\n\
         static void* __t_mnew(long long cap){ if(cap<8)cap=8; void* h=malloc(16+(size_t)cap*8); if(!h){fprintf(stderr,\"天运行时：内存分配失败\\n\");exit(1);} ((__t_map_hdr*)h)->len=0; ((__t_map_hdr*)h)->cap=cap; memset(__t_mbuckets(h),0,(size_t)cap*8); return h; }\n\
         static long long __t_mlen(void* h){ return ((__t_map_hdr*)h)->len; }\n\
         static long long __t_mhas_i(void* h, long long k){ return __t_mfind_i(h,k)!=NULL; }\n\
         static long long __t_mhas_s(void* h, const char* k){ return __t_mfind_s(h,k)!=NULL; }\n\
         static long long __t_mget_ii(void* h, long long k){ __t_mnode* n=__t_mfind_i(h,k); if(!n)__t_panic(\"map 键不存在\"); return n->val; }\n\
         static double __t_mget_if(void* h, long long k){ __t_mnode* n=__t_mfind_i(h,k); if(!n)__t_panic(\"map 键不存在\"); double d; memcpy(&d,&n->val,8); return d; }\n\
         static char* __t_mget_is(void* h, long long k){ __t_mnode* n=__t_mfind_i(h,k); if(!n)__t_panic(\"map 键不存在\"); return (char*)n->val; }\n\
         static long long __t_mget_si(void* h, const char* k){ __t_mnode* n=__t_mfind_s(h,k); if(!n)__t_panic(\"map 键不存在\"); return n->val; }\n\
         static double __t_mget_sf(void* h, const char* k){ __t_mnode* n=__t_mfind_s(h,k); if(!n)__t_panic(\"map 键不存在\"); double d; memcpy(&d,&n->val,8); return d; }\n\
         static char* __t_mget_ss(void* h, const char* k){ __t_mnode* n=__t_mfind_s(h,k); if(!n)__t_panic(\"map 键不存在\"); return (char*)n->val; }\n\
         static void* __t_mset_ii(void* h, long long k, long long v){ __t_map_hdr* x=(__t_map_hdr*)h; __t_mnode* n=__t_mfind_i(h,k); if(n){n->val=v; return h;} if((x->len+1)*4>x->cap*3){h=__t_mgrow(h);x=(__t_map_hdr*)h;} long long hh=__t_mhash_i(k); __t_mnode* nn=__t_mnode_new(); nn->hash=hh; nn->key=k; nn->val=v; nn->next=__t_mbuckets(h)[hh&(x->cap-1)]; __t_mbuckets(h)[hh&(x->cap-1)]=nn; x->len++; return h; }\n\
         static void* __t_mset_if(void* h, long long k, double v){ long long bits; memcpy(&bits,&v,8); return __t_mset_ii(h,k,bits); }\n\
         static void* __t_mset_is(void* h, long long k, char* v){ __t_map_hdr* x=(__t_map_hdr*)h; __t_mnode* n=__t_mfind_i(h,k); if(n){free((void*)n->val);n->val=(long long)v;return h;} if((x->len+1)*4>x->cap*3){h=__t_mgrow(h);x=(__t_map_hdr*)h;} long long hh=__t_mhash_i(k); __t_mnode* nn=__t_mnode_new(); nn->hash=hh; nn->key=k; nn->val=(long long)v; nn->next=__t_mbuckets(h)[hh&(x->cap-1)]; __t_mbuckets(h)[hh&(x->cap-1)]=nn; x->len++; return h; }\n\
         static void* __t_mset_si(void* h, char* k, long long v){ __t_map_hdr* x=(__t_map_hdr*)h; __t_mnode* n=__t_mfind_s(h,k); if(n){free(k);n->val=v;return h;} if((x->len+1)*4>x->cap*3){h=__t_mgrow(h);x=(__t_map_hdr*)h;} long long hh=__t_mhash_s(k); __t_mnode* nn=__t_mnode_new(); nn->hash=hh; nn->key=(long long)k; nn->val=v; nn->next=__t_mbuckets(h)[hh&(x->cap-1)]; __t_mbuckets(h)[hh&(x->cap-1)]=nn; x->len++; return h; }\n\
         static void* __t_mset_sf(void* h, char* k, double v){ long long bits; memcpy(&bits,&v,8); return __t_mset_si(h,k,bits); }\n\
         static void* __t_mset_ss(void* h, char* k, char* v){ __t_map_hdr* x=(__t_map_hdr*)h; __t_mnode* n=__t_mfind_s(h,k); if(n){free(k);free((void*)n->val);n->val=(long long)v;return h;} if((x->len+1)*4>x->cap*3){h=__t_mgrow(h);x=(__t_map_hdr*)h;} long long hh=__t_mhash_s(k); __t_mnode* nn=__t_mnode_new(); nn->hash=hh; nn->key=(long long)k; nn->val=(long long)v; nn->next=__t_mbuckets(h)[hh&(x->cap-1)]; __t_mbuckets(h)[hh&(x->cap-1)]=nn; x->len++; return h; }\n\
         static void __t_mdel_ii(void* h, long long k){ __t_map_hdr* x=(__t_map_hdr*)h; long long idx=__t_mhash_i(k)&(x->cap-1); __t_mnode** pp=&__t_mbuckets(h)[idx]; while(*pp){ if((*pp)->key==k){ __t_mnode* t=*pp; *pp=t->next; free(t); x->len--; return; } pp=&(*pp)->next; } }\n\
         static void __t_mdel_if(void* h, long long k){ __t_mdel_ii(h,k); }\n\
         static void __t_mdel_is(void* h, long long k){ __t_map_hdr* x=(__t_map_hdr*)h; long long idx=__t_mhash_i(k)&(x->cap-1); __t_mnode** pp=&__t_mbuckets(h)[idx]; while(*pp){ if((*pp)->key==k){ __t_mnode* t=*pp; *pp=t->next; free((void*)t->val); free(t); x->len--; return; } pp=&(*pp)->next; } }\n\
         static void __t_mdel_si(void* h, const char* k){ __t_map_hdr* x=(__t_map_hdr*)h; long long idx=__t_mhash_s(k)&(x->cap-1); __t_mnode** pp=&__t_mbuckets(h)[idx]; while(*pp){ if(!strcmp((char*)(*pp)->key,k)){ __t_mnode* t=*pp; *pp=t->next; free((void*)t->key); free(t); x->len--; return; } pp=&(*pp)->next; } }\n\
         static void __t_mdel_sf(void* h, const char* k){ __t_mdel_si(h,k); }\n\
         static void __t_mdel_ss(void* h, const char* k){ __t_map_hdr* x=(__t_map_hdr*)h; long long idx=__t_mhash_s(k)&(x->cap-1); __t_mnode** pp=&__t_mbuckets(h)[idx]; while(*pp){ if(!strcmp((char*)(*pp)->key,k)){ __t_mnode* t=*pp; *pp=t->next; free((void*)t->key); free((void*)t->val); free(t); x->len--; return; } pp=&(*pp)->next; } }\n\
         static void* __t_mkeys_i(void* h){ __t_map_hdr* x=(__t_map_hdr*)h; void* d=__t_dnew(x->cap); for(long long i=0;i<x->cap;i++){ __t_mnode* n=__t_mbuckets(h)[i]; while(n){ __t_dpush_i(d,n->key); n=n->next; } } return d; }\n\
         static void* __t_mkeys_s(void* h){ __t_map_hdr* x=(__t_map_hdr*)h; void* d=__t_dnew(x->cap); for(long long i=0;i<x->cap;i++){ __t_mnode* n=__t_mbuckets(h)[i]; while(n){ __t_dpush_s(d,__t_dup((char*)n->key)); n=n->next; } } return d; }\n\
         static void* __t_mvalues_i(void* h){ __t_map_hdr* x=(__t_map_hdr*)h; void* d=__t_dnew(x->cap); for(long long i=0;i<x->cap;i++){ __t_mnode* n=__t_mbuckets(h)[i]; while(n){ __t_dpush_i(d,n->val); n=n->next; } } return d; }\n\
         static void* __t_mvalues_f(void* h){ __t_map_hdr* x=(__t_map_hdr*)h; void* d=__t_dnew(x->cap); for(long long i=0;i<x->cap;i++){ __t_mnode* n=__t_mbuckets(h)[i]; while(n){ double v; memcpy(&v,&n->val,8); __t_dpush_f(d,v); n=n->next; } } return d; }\n\
         static void* __t_mvalues_s(void* h){ __t_map_hdr* x=(__t_map_hdr*)h; void* d=__t_dnew(x->cap); for(long long i=0;i<x->cap;i++){ __t_mnode* n=__t_mbuckets(h)[i]; while(n){ __t_dpush_s(d,__t_dup((char*)n->val)); n=n->next; } } return d; }\n\
         static void __t_sort_i(void* h){ __t_darr_hdr* x=(__t_darr_hdr*)h; long long* b=(long long*)__t_darr_data(h); long long i,j; for(i=1;i<x->len;i++){ long long t=b[i]; j=i-1; while(j>=0 && b[j]>t){ b[j+1]=b[j]; j--; } b[j+1]=t; } }\n\
         static void __t_sort_f(void* h){ __t_darr_hdr* x=(__t_darr_hdr*)h; double* b=(double*)__t_darr_data(h); long long i,j; for(i=1;i<x->len;i++){ double t=b[i]; j=(long long)i-1; while(j>=0 && b[j]>t){ b[j+1]=b[j]; j--; } b[j+1]=t; } }\n\
         static void __t_sort_s(void* h){ __t_darr_hdr* x=(__t_darr_hdr*)h; char** b=(char**)__t_darr_data(h); long long i,j; for(i=1;i<x->len;i++){ char* t=b[i]; j=(long long)i-1; while(j>=0 && strcmp(b[j],t)>0){ b[j+1]=b[j]; j--; } b[j+1]=t; } }\n\
         static void __t_mfree_ii(void* h){ if(!h)return; __t_map_hdr* x=(__t_map_hdr*)h; for(long long i=0;i<x->cap;i++){ __t_mnode* n=__t_mbuckets(h)[i]; while(n){ __t_mnode* nx=n->next; free(n); n=nx; } } free(h); }\n\
         static void __t_mfree_if(void* h){ __t_mfree_ii(h); }\n\
         static void __t_mfree_is(void* h){ if(!h)return; __t_map_hdr* x=(__t_map_hdr*)h; for(long long i=0;i<x->cap;i++){ __t_mnode* n=__t_mbuckets(h)[i]; while(n){ __t_mnode* nx=n->next; free((void*)n->val); free(n); n=nx; } } free(h); }\n\
         static void __t_mfree_si(void* h){ if(!h)return; __t_map_hdr* x=(__t_map_hdr*)h; for(long long i=0;i<x->cap;i++){ __t_mnode* n=__t_mbuckets(h)[i]; while(n){ __t_mnode* nx=n->next; free((void*)n->key); free(n); n=nx; } } free(h); }\n\
         static void __t_mfree_sf(void* h){ __t_mfree_si(h); }\n\
         static void __t_mfree_ss(void* h){ if(!h)return; __t_map_hdr* x=(__t_map_hdr*)h; for(long long i=0;i<x->cap;i++){ __t_mnode* n=__t_mbuckets(h)[i]; while(n){ __t_mnode* nx=n->next; free((void*)n->key); free((void*)n->val); free(n); n=nx; } } free(h); }\n\n",
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
                borrows: f.params.iter().map(|p| p.borrow && p.ty.map(|t| t.is_darr()).unwrap_or(false)).collect(),
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
            .map(|p| format!("{} {}", c_ty(p.ty.unwrap_or(Ty::I64)), cname(&p.name)))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(
            out,
            "static {} {}({});",
            c_ty(ret),
            cname(&f.name),
            if params.is_empty() { "void".into() } else { params }
        );
    }
    out.push('\n');

    // v2.2：@post 出口契约——在每个返回点内联校验（规范第 12 节）
    // 不再使用独立 helper 函数，因为 @post 可引用函数参数，内联可自然访问作用域
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
                // v4.6：元组返回 → 逐元素结构比较（规范 24.1 例外），避免退化为指针比较
                if frt.is_tuple() {
                    let tid = match frt {
                        Ty::Tuple(i) => i,
                        _ => unreachable!(),
                    };
                    let td = crate::type_check::tuples()[tid as usize].clone();
                    let exp_elems = match expected {
                        Expr::TupExpr { elems, .. } => elems,
                        _ => unreachable!("元组 @example 期望值必须是元组字面量（检查器已保证）"),
                    };
                    let mut call_s = String::new();
                    emit_expr(call, &empty_scope, &sigs, &mut call_s);
                    // 调用结果写入一次性临时指针，避免 call 求值两次
                    // 用块作用域包裹 __t_res 临时指针，多个元组 @example 不冲突（v4.6 修复）
                    let _ = writeln!(out, "{{");
                    let _ = writeln!(out, "    void* __t_res = {};", call_s);
                    let mut parts = Vec::new();
                    for (i, elem_ty) in td.elems.iter().enumerate() {
                        let mut exp_s = String::new();
                        emit_bind(&exp_elems[i], &empty_scope, &sigs, &mut exp_s);
                        let part = match elem_ty {
                            // v4.6 修复：str 槽位以 long long 存储，读出转 const char* 作值比较
                            Ty::Str => format!(
                                "__t_seq(((const char*)((long long*)__t_res)[{}]), {})",
                                i, exp_s
                            ),
                            Ty::F64 => format!("((double*)__t_res)[{}] == ({})", i, exp_s),
                            _ => format!("((long long*)__t_res)[{}] == (long long)({})", i, exp_s),
                        };
                        parts.push(part);
                    }
                    let cond = parts.join(" && ");
                    let _ = writeln!(
                        out,
                        "    if (!({})) {{ fprintf(stderr, \"@example 契约失败：{}（第 {} 行）\\n\"); exit(1); }}",
                        cond, f.name, line
                    );
                    let _ = writeln!(out, "}}");
                } else {
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
                    "    if (!({})) {{ fprintf(stderr, \"@example 契约失败：{}（第 {} 行）\\n\"); exit(1); }}",
                    cond,
                    f.name,
                    line
                );
                }
            }
        }
    }
    for s in &prog.top {
        emit_stmt(s, &mut scope, &sigs, Ty::I64, None, &mut out, 1);
    }
    for n in &top_strs {
        let t = *ty_scope.get(n).unwrap_or(&Ty::Str);
        let _ = writeln!(out, "    {} {} = NULL;", free_slot(&cname(n), t), cname(n));
    }
    out.push_str("    return 0;\n}\n\n");

    // 函数定义（规范 5.3）
    for f in &prog.funcs {
        let ret = f.ret.unwrap_or(Ty::I64);
        let params = f
            .params
            .iter()
            .map(|p| format!("{} {}", c_ty(p.ty.unwrap_or(Ty::I64)), cname(&p.name)))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(
            out,
            "static {} {}({}) {{",
            c_ty(ret),
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
        // v2.2：@post 出口契约（表达式引用 + 锚点行号，供每次返回前内联校验）
        let post: Option<(&Expr, usize)> = f.contracts.iter().find_map(|c| match c {
            Contract::Post(e, line) => Some((e, *line)),
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
                // v4.3：借用 &[]T 形参不是所有者，不释放（规范 22.8）
                .filter(|p| p.ty.map_or(false, is_owned) && !p.borrow)
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
            Ty::Arr(_) => unreachable!("定长数组不可作为返回类型（检查器已拦截）"),
            Ty::DArr(_) | Ty::Tuple(_) | Ty::Map(_) => "NULL",
        };
        match &post {
            Some((pexpr, cline)) => {
                let _ = writeln!(out, "    {{ {} t_ret = {};", c_ty(ret), zero);
                let mut ps = scope.clone();
                ps.insert("ret".into(), VarInfo::new(ret, false));
                let mut cond = String::new();
                emit_expr(pexpr, &ps, &sigs, &mut cond);
                let _ = writeln!(
                    out,
                    "    if (!({})) {{ fprintf(stderr, \"@post 契约失败（第 {} 行）\\n\"); exit(1); }} return t_ret; }}\n}}\n",
                    cond, cline
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
                for (i, (a, pt)) in args.iter().zip(&sig.params).enumerate() {
                    // v4.3：借用 &[]T 形参不置空源（视图无所有权，规范 22.8）
                    let borrowed = sig.borrows.get(i).copied().unwrap_or(false);
                    if is_owned(*pt) && !borrowed {
                        emit_move_nulls(a, true, scope, sigs, out, level);
                    }
                }
            }
        }
        // copy()/tos() 的参数只借用，但其参数内部的调用仍可能移动
        Expr::Convert { arg, .. } => emit_move_nulls(arg, false, scope, sigs, out, level),
        // v4.2：动态数组字面量内的 str 元素在移动位（is_top）置空
        Expr::DArrLit { elems, .. } => {
            for e in elems {
                emit_move_nulls(e, true, scope, sigs, out, level);
            }
        }
        Expr::Sub { s, .. } => emit_move_nulls(s, false, scope, sigs, out, level),
        // v4.5：元组元素在移动位（is_top）置空
        Expr::TupExpr { elems, .. } => {
            for e in elems {
                emit_move_nulls(e, true, scope, sigs, out, level);
            }
        }
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
                    .enumerate()
                    .zip(&sig.params)
                    .any(|((i, a), pt)| {
                        // v4.3：借用形参不置空源
                        let borrowed = sig.borrows.get(i).copied().unwrap_or(false);
                        is_owned(*pt) && !borrowed && has_move_nulls(a, true, sigs)
                    })
            })
            .unwrap_or(false),
        Expr::Convert { arg, .. } => has_move_nulls(arg, false, sigs),
        Expr::DArrLit { elems, .. } => elems.iter().any(|e| has_move_nulls(e, true, sigs)),
        Expr::Sub { s, .. } => has_move_nulls(s, false, sigs),
        Expr::TupExpr { elems, .. } => elems.iter().any(|e| has_move_nulls(e, true, sigs)),
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
    post: Option<&(&Expr, usize)>,
    out: &mut String,
    level: usize,
) {
    match s {
        Stmt::Decl { name, ty, value, mutable, .. } => {
            let t = ty.unwrap_or_else(|| ty_of(value, scope, sigs, &structs()).unwrap());
            scope.insert(name.clone(), VarInfo::new(t, *mutable));
            // v3.3：数组声明——C 数组 + 逐元素初始化/拷贝（纯值类型，不参与天权，规范 16.1）
            if let Ty::Arr(ai) = t {
                let (elem, len) = arr_of(t);
                let _ = ai;
                let cn = cname(name);
                indent(out, level);
                match value {
                    Expr::ArrLit { elems, .. } => {
                        let mut parts = Vec::new();
                        for e in elems {
                            let mut s = String::new();
                            emit_expr(e, scope, sigs, &mut s);
                            parts.push(s);
                        }
                        let _ = writeln!(
                            out,
                            "{} {}[{}] = {{{}}};",
                            elem.c_name(),
                            cn,
                            len,
                            parts.join(", ")
                        );
                    }
                    Expr::Var(src) => {
                        let _ = writeln!(out, "{} {}[{}];", elem.c_name(), cn, len);
                        let sc = cname(src);
                        for i in 0..len {
                            indent(out, level);
                            let _ = writeln!(out, "{}[{}] = {}[{}];", cn, i, sc, i);
                        }
                    }
                    _ => unreachable!("数组声明的 RHS 只能是字面量或同型变量（检查器已拦截）"),
                }
                emit_move_nulls(value, false, scope, sigs, out, level);
                return;
            }
            // v4.0：动态数组声明——堆指针，天权移动语义（规范第 22 节）
            if t.is_darr() {
                let de = match t {
                    Ty::DArr(i) => crate::type_check::darrs()[i as usize].clone(),
                    _ => unreachable!(),
                };
                let cn = cname(name);
                match value {
                    Expr::DArrLit { elems, .. } => {
                        indent(out, level);
                        let _ = writeln!(out, "{{ void* __t_h = __t_dnew({});", elems.len());
                        for e in elems.iter() {
                            let mut s = String::new();
                            if de.elem == Ty::Str {
                                // v4.2：str 字面量 dup / 变量移动（规范 22.6）
                                emit_bind(e, scope, sigs, &mut s);
                            } else {
                                emit_expr(e, scope, sigs, &mut s);
                            }
                            indent(out, level);
                            let helper = if de.elem == Ty::F64 {
                                "__t_dpush_f"
                            } else if de.elem == Ty::Str {
                                "__t_dpush_s"
                            } else {
                                "__t_dpush_i"
                            };
                            let _ = writeln!(out, "  {}(__t_h, {});", helper, s);
                        }
                        indent(out, level);
                        let _ = writeln!(out, "  {} = __t_h; }}", cn);
                        emit_move_nulls(value, true, scope, sigs, out, level);
                    }
                    _ => {
                        let mut e = String::new();
                        emit_expr(value, scope, sigs, &mut e);
                        indent(out, level);
                        let _ = writeln!(out, "{} = {};", cn, e);
                        // v4.2：Var RHS 是移动位——源变量置空（与 str 声明一致，规范 22.3）
                        emit_move_nulls(value, true, scope, sigs, out, level);
                    }
                }
                return;
            }
            // v4.7：map 声明——堆指针，天权移动语义（规范第 25 节）
            if t.is_map() {
                let cn = cname(name);
                match value {
                    Expr::MapLit { entries, .. } => {
                        emit_map_ctor(
                            t,
                            entries,
                            scope,
                            sigs,
                            out,
                            level,
                            &format!("{} = __t_h; }}", cn),
                        );
                        emit_move_nulls(value, true, scope, sigs, out, level);
                    }
                    _ => {
                        // Var 移动或函数返回：直接接管新指针（槽已 NULL 化，规范 25.7）
                        let mut e = String::new();
                        emit_expr(value, scope, sigs, &mut e);
                        indent(out, level);
                        let _ = writeln!(out, "{} = {};", cn, e);
                        emit_move_nulls(value, true, scope, sigs, out, level);
                    }
                }
                return;
            }
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
            // v4.0：动态数组下标赋值 a[i] = e —— __t_dset_* 守卫（规范第 22 节）
            if let Expr::Index(base, idx) = target {
                let bt = norm(ty_of(base, scope, sigs, &structs()).unwrap_or(Ty::I64));
                // v4.7：map 下标赋值 m[k] = e —— __t_mset_* 可能 realloc，变量重绑（规范第 25 节）
                if let Some(mde) = crate::type_check::map_by_id(&crate::type_check::maps(), bt) {
                    let be = match base.as_ref() {
                        Expr::Var(n) => cname(n),
                        _ => String::new(),
                    };
                    let mut ks = String::new();
                    emit_expr(idx, scope, sigs, &mut ks);
                    let mut vs = String::new();
                    if map_val_wire(mde.val) == "s" {
                        emit_bind(value, scope, sigs, &mut vs);
                    } else {
                        emit_expr(value, scope, sigs, &mut vs);
                    }
                    let key_arg = if map_key_wire(mde.key) == "s" {
                        // v4.7：mset 接管键所有权——字面量/变量统一 dup 出 map 自有副本（规范第 25 节）
                        format!("__t_dup({})", ks)
                    } else {
                        format!("(long long)({})", ks)
                    };
                    let val_arg = if matches!(map_val_wire(mde.val), "s" | "f") {
                        vs
                    } else {
                        format!("(long long)({})", vs)
                    };
                    let suffix = map_wire(&mde.clone());
                    indent(out, level);
                    let _ = writeln!(
                        out,
                        "{} = __t_mset_{}({}, {}, {});",
                        be, suffix, be, key_arg, val_arg
                    );
                    // str 值：@set 整体移动语义，源 str 被消费（字面量 dup / 变量移动）
                    if map_val_wire(mde.val) == "s" {
                        emit_move_nulls(value, true, scope, sigs, out, level);
                    } else {
                        emit_move_nulls(value, false, scope, sigs, out, level);
                    }
                    return;
                }
                if let Some(de) = crate::type_check::darr_by_id(&crate::type_check::darrs(), bt) {
                    let be = match base.as_ref() {
                        Expr::Var(n) => cname(n),
                        _ => String::new(),
                    };
                    let mut is = String::new();
                    emit_expr(idx, scope, sigs, &mut is);
                    let mut vs = String::new();
                    emit_expr(value, scope, sigs, &mut vs);
                    indent(out, level);
                    let helper = if de.elem == Ty::F64 { "__t_dset_f" } else { "__t_dset_i" };
                    let _ = writeln!(out, "{}({}, {}, {});", helper, be, is, vs);
                    emit_move_nulls(value, false, scope, sigs, out, level);
                    return;
                }
            }
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
                Expr::Index(base, idx) => {
                    // v3.3：a[i] = e —— 经 __t_idx 越界守卫（规范 16.3）
                    let bt = norm(ty_of(base, scope, sigs, &structs()).unwrap_or(Ty::I64));
                    let (elem, len) = arr_of(bt);
                    let be = match base.as_ref() {
                        Expr::Var(n) => cname(n),
                        _ => String::new(),
                    };
                    let mut is = String::new();
                    emit_expr(idx, scope, sigs, &mut is);
                    (
                        format!("{}[__t_idx({}, (long long)({}))]", be, len, is),
                        elem,
                        None,
                    )
                }
                _ => unreachable!("类型检查已拦截非法赋值左侧"),
            };
            // v4.7：map 整体赋值——移动语义（释放旧值除非自我消费，规范 25.7）
            if t.is_map() {
                let cn = cn;
                let consumed = target_var
                    .as_ref()
                    .map_or(false, |n| consumes_var(value, n, true, sigs));
                match value {
                    Expr::MapLit { entries, .. } => {
                        let tail = if !consumed {
                            format!("{}\n  {} = __t_h; }}", free_slot(&cn, t), cn)
                        } else {
                            format!("{} = __t_h; }}", cn)
                        };
                        emit_map_ctor(t, entries, scope, sigs, out, level, &tail);
                        emit_move_nulls(value, true, scope, sigs, out, level);
                    }
                    _ => {
                        let mut e = String::new();
                        emit_expr(value, scope, sigs, &mut e);
                        indent(out, level);
                        let _ = writeln!(out, "{{ void* __t_h = {};", e);
                        emit_move_nulls(value, true, scope, sigs, out, level);
                        indent(out, level);
                        if !consumed {
                            let _ = writeln!(out, "  {}", free_slot(&cn, t));
                        }
                        indent(out, level);
                        let _ = writeln!(out, "  {} = __t_h; }}", cn);
                    }
                }
                return;
            }
            // v4.0：动态数组整体赋值——移动语义（释放旧值除非自我消费，规范 22.3）
            if t.is_darr() {
                let de = match t {
                    Ty::DArr(i) => crate::type_check::darrs()[i as usize].clone(),
                    _ => unreachable!(),
                };
                let cn = cn;
                let consumed = target_var
                    .as_ref()
                    .map_or(false, |n| consumes_var(value, n, true, sigs));
                match value {
                    Expr::DArrLit { elems, .. } => {
                        indent(out, level);
                        let _ = writeln!(out, "{{ void* __t_h = __t_dnew({});", elems.len().max(1));
                        for e in elems.iter() {
                            let mut s = String::new();
                            if de.elem == Ty::Str {
                                emit_bind(e, scope, sigs, &mut s);
                            } else {
                                emit_expr(e, scope, sigs, &mut s);
                            }
                            indent(out, level);
                            let helper = if de.elem == Ty::F64 {
                                "__t_dpush_f"
                            } else if de.elem == Ty::Str {
                                "__t_dpush_s"
                            } else {
                                "__t_dpush_i"
                            };
                            let _ = writeln!(out, "  {}(__t_h, {});", helper, s);
                        }
                        indent(out, level);
                        if !consumed {
                            let _ = writeln!(out, "  {}", free_slot(&cn, t));
                        }
                        let _ = writeln!(out, "  {} = __t_h; }}", cn);
                        emit_move_nulls(value, true, scope, sigs, out, level);
                    }
                    _ => {
                        let mut e = String::new();
                        emit_expr(value, scope, sigs, &mut e);
                        indent(out, level);
                        let _ = writeln!(out, "{{ void* __t_h = {};", e);
                        emit_move_nulls(value, true, scope, sigs, out, level);
                        indent(out, level);
                        if !consumed {
                            let _ = writeln!(out, "  {}", free_slot(&cn, t));
                        }
                        indent(out, level);
                        let _ = writeln!(out, "  {} = __t_h; }}", cn);
                    }
                }
                return;
            }
            // v3.3：数组整体赋值（拷贝语义，逐元素，规范 16.1）
            if let Ty::Arr(_) = t {
                let (_, len) = arr_of(t);
                let cn = cn;
                match value {
                    Expr::ArrLit { elems, .. } => {
                        for (i, e) in elems.iter().enumerate() {
                            let mut s = String::new();
                            emit_expr(e, scope, sigs, &mut s);
                            indent(out, level);
                            let _ = writeln!(out, "{}[{}] = {};", cn, i, s);
                        }
                    }
                    Expr::Var(src) => {
                        let sc = cname(src);
                        for i in 0..len {
                            indent(out, level);
                            let _ = writeln!(out, "{}[{}] = {}[{}];", cn, i, sc, i);
                        }
                    }
                    _ => unreachable!("数组赋值的 RHS 只能是字面量或同型变量（检查器已拦截）"),
                }
                emit_move_nulls(value, false, scope, sigs, out, level);
                return;
            }
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
                Ty::Struct(_) => unreachable!("Print 类型已归一化（结构体不可直接打印，检查器已拦截）"),
                Ty::Arr(_) => unreachable!("Print 类型已归一化（数组不可直接打印，检查器已拦截）"),
                Ty::DArr(_) | Ty::Tuple(_) => unreachable!("Print 类型已归一化（不可直接打印，检查器已拦截）"),
                Ty::Map(_) => unreachable!("Print 类型已归一化（map 不可直接打印，检查器已拦截）"),
            };
            let cast = match t {
                Ty::I32 | Ty::I64 => "(long long)",
                Ty::F64 => "(double)",
                // v1.1：bool 统一打印为 true/false（规范第 4 节）
                Ty::Str => "",
                Ty::Bool => "",
                Ty::BorrowStr => unreachable!("Print 类型已归一化"),
                Ty::Struct(_) => unreachable!("Print 类型已归一化（结构体不可直接打印，检查器已拦截）"),
                Ty::Arr(_) => unreachable!("Print 类型已归一化（数组不可直接打印，检查器已拦截）"),
                Ty::DArr(_) => unreachable!("Print 类型已归一化（数组不可直接打印，检查器已拦截）"),
                Ty::Tuple(_) => unreachable!("Print 类型已归一化（元组不可直接打印，检查器已拦截）"),
                Ty::Map(_) => unreachable!("Print 类型已归一化（map 不可直接打印，检查器已拦截）"),
            };
            let val = if t == Ty::Bool {
                format!("__t_tos_b((int)({}))", x)
            } else {
                format!("{}({})", cast, x)
            };
            let _ = writeln!(out, "printf(\"{}\", {});", fmt, val);
            // 打印表达式内的调用可能移动了 str/结构体实参（规范 11.2 / 14.4）
            emit_move_nulls(e, false, scope, sigs, out, level);
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
                    // v4.6：元组返回走内部 malloc + 逐槽存储的 owned 分支（规范 24.3）；
                    // is_owned(Tuple) 为 false，需显式让元组恒进该分支，避免落入非 owned 分支的 unreachable。
                    if is_owned(fn_ret) || fn_ret.is_tuple() || needs_nulls || post.is_some() {
                        // 天权：返回前先求值到临时、置空被移动的源变量，再返回；
                        // v2.2：返回前内联校验 @post 出口契约
                        // v4.5：元组返回——malloc 块 + 逐元素存储（规范第 24 节）
                        if fn_ret.is_tuple() {
                            let tid = match fn_ret {
                                Ty::Tuple(i) => i,
                                _ => unreachable!(),
                            };
                            let td = crate::type_check::tuples()[tid as usize].clone();
                            let elems = match expr {
                                Expr::TupExpr { elems, .. } => elems.clone(),
                                other => vec![other.clone()],
                            };
                            let _ = writeln!(
                                out,
                                "{{ void* t_ret = __t_malloc({});",
                                td.elems.len() * 8
                            );
                            for (i, e) in elems.iter().enumerate() {
                                let mut s = String::new();
                                if td.elems[i] == Ty::Str {
                                    emit_bind(e, scope, sigs, &mut s);
                                } else {
                                    emit_expr(e, scope, sigs, &mut s);
                                }
                                indent(out, level);
                                // 槽位物理上是 8 字节：f64 走 double*，其余（含 str 指针）走 long long*
                                let pty = if td.elems[i] == Ty::F64 { "double" } else { "long long" };
                                let vty = if td.elems[i] == Ty::F64 { "double" } else { "long long" };
                                // 写第 i 个 8 字节槽：str 元素以 long long 重新解释存储（读取侧转回 const char*）
                                let _ = writeln!(out, "  (({}*)t_ret)[{}] = ({})({});", pty, i, vty, s);
                            }
                            emit_move_nulls(expr, true, scope, sigs, out, level);
                            if let Some((pexpr, cline)) = post {
                                let mut ps = scope.clone();
                                ps.insert("ret".into(), VarInfo::new(fn_ret, false));
                                // v4.5：元组的 @post 检查器不支持（元组不可比较/访问元素表达式），跳过
                                let _ = (pexpr, cline, &ps);
                            }
                            indent(out, level);
                            let _ = writeln!(out, "return t_ret; }}");
                            return;
                        }
                        let _ = writeln!(out, "{{ {} t_ret = {};", c_ty(fn_ret), x);
                        emit_move_nulls(expr, is_owned(fn_ret), scope, sigs, out, level);
                        if let Some((pexpr, cline)) = post {
                            let mut ps = scope.clone();
                            ps.insert("ret".into(), VarInfo::new(fn_ret, false));
                            let mut cond = String::new();
                            emit_expr(pexpr, &ps, sigs, &mut cond);
                            indent(out, level);
                            let _ = writeln!(
                                out,
                                "if (!({})) {{ fprintf(stderr, \"@post 契约失败（第 {} 行）\\n\"); exit(1); }}",
                                cond, cline
                            );
                        }
                        indent(out, level);
                        let _ = writeln!(out, "return t_ret; }}");
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
            Ty::Arr(_) => unreachable!("定长数组不可作为返回类型（检查器已拦截）"),
            Ty::DArr(_) => unreachable!("动态数组必走 owned 返回分支（检查器已拦截）"),
            Ty::Tuple(_) => unreachable!("元组必走 owned 返回分支（检查器已拦截）"),
            Ty::Map(_) => unreachable!("map 必走 owned 返回分支（检查器已拦截）"),
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
                        Ty::DArr(_) | Ty::Map(_) => "NULL",
                        Ty::BorrowStr => unreachable!("借用不可作为返回类型（检查器已拦截）"),
            Ty::Arr(_) => unreachable!("数组不可作为返回类型（检查器已拦截）"),
            Ty::Tuple(_) => unreachable!("元组返回不可省略值（检查器已拦截）"),
                    };
                    match post {
                        Some((pexpr, cline)) => {
                            let _ = writeln!(
                                out,
                                "{{ {} t_ret = {};",
                                c_ty(fn_ret),
                                zero
                            );
                            let mut ps = scope.clone();
                            ps.insert("ret".into(), VarInfo::new(fn_ret, false));
                            let mut cond = String::new();
                            emit_expr(pexpr, &ps, sigs, &mut cond);
                            indent(out, level);
                            let _ = writeln!(
                                out,
                                "if (!({})) {{ fprintf(stderr, \"@post 契约失败（第 {} 行）\\n\"); exit(1); }} return t_ret; }}",
                                cond, cline
                            );
                        }
                        None => {
                            let _ = writeln!(out, "return {};", zero);
                        }
                    }
                }
            }
        }
        // v4.5：多声明解构 //a, b = f(...) —— 元素按类型接管，块随即释放（规范第 24 节）
        Stmt::MultiDecl { names, value, mutable, .. } => {
            let vt = ty_of(value, scope, sigs, &structs()).unwrap_or(Ty::I64);
            let td = match vt {
                Ty::Tuple(i) => crate::type_check::tuples()[i as usize].clone(),
                _ => unreachable!("MultiDecl RHS 非元组（检查器已拦截）"),
            };
            // 堆槽由 prologue（main 顶层 / 函数 str_names）统一预声明（owned=NULL）；
            // 此处仅补声明标量（非 owned）元素
            for (i, n) in names.iter().enumerate() {
                if !is_owned(td.elems[i]) {
                    indent(out, level);
                    let _ = writeln!(out, "{} {};", c_ty(td.elems[i]), cname(n));
                }
            }
            let mut e = String::new();
            emit_expr(value, scope, sigs, &mut e);
            indent(out, level);
            let _ = writeln!(out, "{{ void* __t_t = {};", e);
            for (i, n) in names.iter().enumerate() {
                indent(out, level);
                let pty = if td.elems[i] == Ty::F64 { "double" } else { "long long" };
                let vty = if td.elems[i] == Ty::Str {
                    "const char*"
                } else if td.elems[i] == Ty::F64 {
                    "double"
                } else {
                    "long long"
                };
                // 读第 i 个 8 字节槽：((pty*)__t_t)[i]，再按元素类型取值
                let _ = writeln!(out, "  {} = ({}) (({}*)__t_t)[{}];", cname(n), vty, pty, i);
            }
            indent(out, level);
            let _ = writeln!(out, "  __t_free(__t_t); }}");
            // owned 元素所在槽由作用域出口释放；RHS 内部的移动已由 emit_move_nulls 处理
            for (i, n) in names.iter().enumerate() {
                scope.insert(n.clone(), VarInfo::new(td.elems[i], *mutable));
            }
        }
        // v4.1：pop(a) —— 长度递减；空数组是运行时错误（规范第 22 节）
        Stmt::Expr(Expr::Pop { arr }, _) => {
            let base_name = match arr.as_ref() {
                Expr::Var(n) => cname(n),
                _ => String::new(),
            };
            indent(out, level);
            let _ = writeln!(out, "__t_dpop({});", base_name);
        }
        // v4.8：sort(a) —— 原地升序排序，空/单元素数组为幂等空操作（规范第 26 节）
        Stmt::Expr(Expr::Sort(arr), _) => {
            let base_name = match arr.as_ref() {
                Expr::Var(n) => cname(n),
                _ => String::new(),
            };
            let dt = match arr.as_ref() {
                Expr::Var(n) => scope.get(n).map(|v| v.ty).unwrap_or(Ty::I64),
                _ => Ty::I64,
            };
            let helper = match crate::type_check::darr_by_id(&crate::type_check::darrs(), dt)
                .map(|d| d.elem)
            {
                Some(Ty::F64) => "__t_sort_f",
                Some(Ty::Str) => "__t_sort_s",
                _ => "__t_sort_i",
            };
            indent(out, level);
            let _ = writeln!(out, "{}({});", helper, base_name);
        }
        // v4.0：push(a, v) —— 可能 realloc，变量重新绑定返回的新指针（规范第 22 节）
        Stmt::Expr(Expr::Push { arr, value }, _) => {
            let (base_name, de) = match arr.as_ref() {
                Expr::Var(n) => (
                    cname(n),
                    crate::type_check::darr_by_id(
                        &crate::type_check::darrs(),
                        scope.get(n).map(|v| v.ty).unwrap_or(Ty::I64),
                    )
                    .map(|d| d.clone()),
                ),
                _ => (String::new(), None),
            };
            let de = de.expect("push 目标类型未知（应被 type_check 拦截）");
            let mut vs = String::new();
            if de.elem == Ty::Str {
                // v4.2：str 元素移交所有权（字面量 dup，规范 22.6）
                emit_bind(value, scope, sigs, &mut vs);
            } else {
                emit_expr(value, scope, sigs, &mut vs);
            }
            indent(out, level);
            let helper = if de.elem == Ty::F64 {
                "__t_dpush_f"
            } else if de.elem == Ty::Str {
                "__t_dpush_s"
            } else {
                "__t_dpush_i"
            };
            let _ = writeln!(out, "{} = {}({}, {});", base_name, helper, base_name, vs);
            if de.elem == Ty::Str {
                emit_move_nulls(value, true, scope, sigs, out, level);
            }
        }
        // v4.7：del(m, k) —— 删除键（不存在则静默；不改头部，无需重绑，规范第 25 节）
        Stmt::Expr(Expr::Del { map, key }, _) => {
            let bt = norm(ty_of(map, scope, sigs, &structs()).unwrap_or(Ty::I64));
            let mde = crate::type_check::map_by_id(&crate::type_check::maps(), bt)
                .expect("del 目标应为 map（检查器已拦截）")
                .clone();
            let be = match map.as_ref() {
                Expr::Var(n) => cname(n),
                _ => String::new(),
            };
            let mut ks = String::new();
            emit_expr(key, scope, sigs, &mut ks);
            let key_arg = if map_key_wire(mde.key) == "s" {
                ks
            } else {
                format!("(long long)({})", ks)
            };
            indent(out, level);
            let _ = writeln!(out, "__t_mdel_{}({}, {});", map_wire(&mde), be, key_arg);
        }
        Stmt::Expr(e, _) => {
            let mut x = String::new();
            emit_expr(e, scope, sigs, &mut x);
            indent(out, level);
            let _ = writeln!(out, "{};", x);
            // 语句级调用可能移动了 str 实参
            emit_move_nulls(e, false, scope, sigs, out, level);
        }
        // v3.8：break / continue —— 循环体内跳转；跳过其后语句的 str 释放
        // 与 r/ 早退同类（进程退出回收，规范 11.4.2 边界）
        Stmt::Break(_) => {
            indent(out, level);
            let _ = writeln!(out, "break;");
        }
        Stmt::Continue(_) => {
            indent(out, level);
            let _ = writeln!(out, "continue;");
        }
        // v4.0：panic(msg) / check(cond, msg) —— 运行时同一快速失败出口（规范第 21 节）
        Stmt::Panic(msg, _) => {
            let mut m = String::new();
            emit_bind(msg, scope, sigs, &mut m);
            indent(out, level);
            let _ = writeln!(out, "__t_panic({});", m);
        }
        Stmt::Check(cond, msg, _) => {
            let mut c = String::new();
            emit_expr(cond, scope, sigs, &mut c);
            let mut m = String::new();
            emit_bind(msg, scope, sigs, &mut m);
            indent(out, level);
            let _ = writeln!(out, "if (!({})) {{ __t_panic({}); }}", c, m);
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
        // v3.3/v4.0/v4.7：a[i] 读 —— 定长经 __t_idx，动态经 __t_dget_*，map 经 __t_mget_*（规范 16.3/22/25）
        Expr::Index(base, idx) => {
            let bt = norm(ty_of(base, scope, sigs, &structs()).unwrap_or(Ty::I64));
            let be = match base.as_ref() {
                Expr::Var(n) => cname(n),
                _ => unreachable!("数组下标左侧只能是变量（检查器已拦截）"),
            };
            let mut is = String::new();
            emit_expr(idx, scope, sigs, &mut is);
            if let Some(mde) = crate::type_check::map_by_id(&crate::type_check::maps(), bt) {
                let key_arg = if map_key_wire(mde.key) == "s" {
                    is
                } else {
                    format!("(long long)({})", is)
                };
                out.push_str(&format!(
                    "__t_mget_{}({}, {})",
                    map_wire(&mde.clone()),
                    be,
                    key_arg
                ));
            } else if let Some(de) = crate::type_check::darr_by_id(&crate::type_check::darrs(), bt) {
                let helper = if de.elem == Ty::F64 {
                    "__t_dget_f"
                } else if de.elem == Ty::Str {
                    "__t_dget_s"
                } else {
                    "__t_dget_i"
                };
                out.push_str(&format!("{}({}, {})", helper, be, is));
            } else if bt == Ty::Str {
                // v4.2：s[i] 字节读（规范第 23 节）
                out.push_str(&format!("__t_sget({}, {})", be, is));
            } else {
                let (_, len) = arr_of(bt);
                out.push_str(&format!("{}[__t_idx({}, (long long)({}))]", be, len, is));
            }
        }
        // v3.3/v3.5/v4.0：len —— 定长（编译期）/ str（strlen）/ 动态（__t_dlen）
        Expr::Len(inner) => {
            let bt = norm(ty_of(inner, scope, sigs, &structs()).unwrap_or(Ty::I64));
            if bt == Ty::Str {
                let mut s = String::new();
                emit_expr(inner, scope, sigs, &mut s);
                out.push_str(&format!("(long long)strlen({})", s));
            } else if bt.is_darr() {
                let be = match inner.as_ref() {
                    Expr::Var(n) => cname(n),
                    _ => unreachable!("len 实参只能是变量（检查器已拦截）"),
                };
                out.push_str(&format!("__t_dlen({})", be));
            } else if bt.is_map() {
                // v4.7：len(m) —— 键值对数（规范第 25 节）
                let be = match inner.as_ref() {
                    Expr::Var(n) => cname(n),
                    _ => unreachable!("len 实参只能是变量（检查器已拦截）"),
                };
                out.push_str(&format!("__t_mlen({})", be));
            } else {
                let (_, len) = arr_of(bt);
                out.push_str(&format!("(long long){}", len));
            }
        }
        // 数组字面量只出现在声明/赋值 RHS（已特判），不会走到通用表达式路径
        Expr::ArrLit { .. } => unreachable!("数组字面量位置非法（检查器已拦截）"),
        Expr::DArrLit { .. } => unreachable!("动态数组字面量位置非法（检查器已拦截）"),
        // v4.7：map 字面量只在声明/赋值 RHS（见 emit_map_ctor），不走通用表达式路径
        Expr::MapLit { .. } => unreachable!("map 字面量位置非法（检查器已拦截）"),
        // v4.7：has(m, k) → bool —— __t_mhas_{keywire}（规范第 25 节）
        Expr::Has { map, key } => {
            let bt = norm(ty_of(map, scope, sigs, &structs()).unwrap_or(Ty::I64));
            let mde = crate::type_check::map_by_id(&crate::type_check::maps(), bt)
                .expect("has() 目标应为 map（检查器已拦截）")
                .clone();
            let be = match map.as_ref() {
                Expr::Var(n) => cname(n),
                _ => unreachable!("has 实参只能是变量（检查器已拦截）"),
            };
            let mut ks = String::new();
            emit_expr(key, scope, sigs, &mut ks);
            let key_arg = if map_key_wire(mde.key) == "s" {
                ks
            } else {
                format!("(long long)({})", ks)
            };
            out.push_str(&format!(
                "__t_mhas_{}({}, {})",
                map_key_wire(mde.key),
                be,
                key_arg
            ));
        }
        // v4.7：keys(m) → 新拥有的 []K 键快照（__t_mkeys_{keywire}；规范 25.4）
        Expr::Keys(map) => {
            let bt = norm(ty_of(map, scope, sigs, &structs()).unwrap_or(Ty::I64));
            let mde = crate::type_check::map_by_id(&crate::type_check::maps(), bt)
                .expect("keys() 目标应为 map（检查器已拦截）")
                .clone();
            let be = match map.as_ref() {
                Expr::Var(n) => cname(n),
                _ => unreachable!("keys 实参只能是变量（检查器已拦截）"),
            };
            out.push_str(&format!(
                "__t_mkeys_{}({})",
                map_key_wire(mde.key),
                be
            ));
        }
        // v4.7：values(m) → 新拥有的 []V 值快照（__t_mvalues_{valwire}；规范 25.4）
        Expr::Values(map) => {
            let bt = norm(ty_of(map, scope, sigs, &structs()).unwrap_or(Ty::I64));
            let mde = crate::type_check::map_by_id(&crate::type_check::maps(), bt)
                .expect("values() 目标应为 map（检查器已拦截）")
                .clone();
            let be = match map.as_ref() {
                Expr::Var(n) => cname(n),
                _ => unreachable!("values 实参只能是变量（检查器已拦截）"),
            };
            out.push_str(&format!(
                "__t_mvalues_{}({})",
                map_val_wire(mde.val),
                be
            ));
        }
        // v4.5：元组表达式 → GNU 语句表达式：malloc 块 + 逐元素存储（规范第 24 节）
        Expr::TupExpr { elems, tup } => {
            let td = crate::type_check::tuples()[*tup as usize].clone();
            out.push_str(&format!("({{ void* __t_h = __t_malloc({});", td.elems.len() * 8));
            for (i, e) in elems.iter().enumerate() {
                let mut s = String::new();
                if td.elems[i] == Ty::Str {
                    emit_bind(e, scope, sigs, &mut s);
                } else {
                    emit_expr(e, scope, sigs, &mut s);
                }
                out.push_str(&format!(" ((long long*)__t_h)[{}] = (long long)({});", i, s));
            }
            out.push_str(" return __t_h; })");
        }
        // v4.2：sub(s, start, n) —— 运行时拷贝出新所有权的堆串（规范第 23 节）
        Expr::Sub { s, start, n } => {
            out.push_str("__t_sub(");
            emit_expr(s, scope, sigs, out);
            out.push_str(", (long long)(");
            emit_expr(start, scope, sigs, out);
            out.push_str("), (long long)(");
            emit_expr(n, scope, sigs, out);
            out.push_str("))");
        }
        Expr::Push { .. } => unreachable!("push 只能作为语句（检查器已拦截）"),
        Expr::Pop { .. } => unreachable!("pop 只能作为语句（检查器已拦截）"),
        Expr::Del { .. } => unreachable!("del 只能作为语句（检查器已拦截）"),
        Expr::Sort { .. } => unreachable!("sort 只能作为语句（检查器已拦截）"),
        Expr::DArrLit { .. } => unreachable!("动态数组字面量位置非法（检查器已拦截）"),
        // v3.7：sel(条件, a, b) → C 三元表达式（惰性求值，与原生后端一致，规范第 18 节）
        Expr::Sel { cond, a, b } => {            out.push_str("((");
            emit_expr(cond, scope, sigs, out);
            out.push_str(") ? (");
            emit_expr(a, scope, sigs, out);
            out.push_str(") : (");
            emit_expr(b, scope, sigs, out);
            out.push_str("))");
        }
        // v3.0：结构体字面量 → 构造函数调用；str 字段由 emit_bind 决定 dup/移动（规范 14.2）
        Expr::StructLit { name, fields, .. } => {
            let sd = structs()
                .into_iter()
                .find(|s| s.name == *name)
                .expect("未声明结构体（类型检查已拦截）");
            let mut args = Vec::new();
            for (i, (fname_opt, v)) in fields.iter().enumerate() {
                // 命名式：按字段名查结构体定义中的位置；位置式：用枚举索引
                let (idx, ft) = match fname_opt {
                    Some(nm) => {
                        let pos = sd.fields.iter().position(|f| &f.name == nm)
                            .expect("字段不存在（类型检查已拦截）");
                        (pos, sd.fields[pos].ty)
                    }
                    None => (i, sd.fields[i].ty),
                };
                let mut a = String::new();
                if ft == Ty::Str {
                    emit_bind(v, scope, sigs, &mut a);
                } else {
                    emit_expr(v, scope, sigs, &mut a);
                }
                // 确保参数按结构体字段声明顺序排列
                while args.len() <= idx {
                    args.push(String::new());
                }
                args[idx] = a;
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
            // v3.2：整数除法快速失败——除数为零运行时报错退出（规范 15.1）；
            // f64 除零遵循 IEEE 754（inf/nan），不做检查。
            // i32/i32 → __t_idiv_i，其余整数组合 → __t_idiv_l（类型检查的提升规则保证 C 整数提升安全）
            if *op == BinOp::Div && lt.is_numeric() && rt.is_numeric() && lt != Ty::F64 && rt != Ty::F64 {
                out.push_str(if lt == Ty::I32 && rt == Ty::I32 { "__t_idiv_i(" } else { "__t_idiv_l(" });
                emit_expr(lhs, scope, sigs, out);
                out.push_str(", ");
                emit_expr(rhs, scope, sigs, out);
                out.push(')');
                return;
            }
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
            if name == "tof" {
                // v4.4：tof(s) 运行时转换（规范第 7 节）
                out.push_str("__t_tof(");
                emit_expr(arg, scope, sigs, out);
                out.push(')');
                return;
            }
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
                    Ty::Bool => {
                        // v4.7：tos(bool) → true/false（与原生后端/__t_tos_b 一致）
                        out.push_str("__t_tos_b((int)");
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
        // v3.2：整数除法经守卫助手（除零快速失败，规范 15.1）
        assert!(c.contains("__t_idiv_l(t_a, 2)"));
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
        // @post：返回点内联校验，失败 exit(1)
        assert!(c.contains("@post 契约失败"), "生成的 C：\n{}", c);
        // @example：main 启动自检，str 用 __t_seq 比较
        assert!(c.contains("__t_seq("), "生成的 C：\n{}", c);
        assert!(c.contains("@example 契约失败：wrap"), "生成的 C：\n{}", c);
    }

    #[test]
    fn test_gen_tuple_v45() {
        // v4.5：多返回值（元组）—— r/e1, e2 构造 malloc 块；//a, b = 解构后随即释放
        let src = "\
f/stats(n:i64):(i64, i64){
    r/n, n*2
}
//c, s=stats(5)
`c
`s
";
        let prog = parse(lex_spanned(src).unwrap()).unwrap();
        check(&prog).unwrap();
        let c = generate(&prog);
        // 返回：malloc 8*2 字节块，逐元素写入
        assert!(c.contains("__t_malloc(16)"), "生成的 C：\n{}", c);
        assert!(c.contains("((long long*)t_ret)[0]"), "生成的 C：\n{}", c);
        assert!(c.contains("((long long*)t_ret)[1]"), "生成的 C：\n{}", c);
        // 解构：读槽后释放块
        assert!(c.contains("((long long*)__t_t)[0]"), "生成的 C：\n{}", c);
        assert!(c.contains("__t_free(__t_t)"), "生成的 C：\n{}", c);
        // 标量槽由 prologue 预声明、此处只赋值
        assert!(c.contains("long long t_c;"), "生成的 C：\n{}", c);
    }

    #[test]
    fn test_gen_tuple_str_elem_v45() {
        // v4.5：含 str 元素的元组——str 以 long long 重新解释存入槽，解构转回 const char*
        let src = "\
f/pair(n:i64):(i64, str){
    r/n, \"hello\"
}
//num, label=pair(5)
`num
`label
";
        let prog = parse(lex_spanned(src).unwrap()).unwrap();
        check(&prog).unwrap();
        let c = generate(&prog);
        // str 元素经 __t_dup 取得独立所有权后存入 long long 槽
        assert!(c.contains("((long long*)t_ret)[1] = (long long)(__t_dup(\"hello\"));"), "生成的 C：\n{}", c);
        // 解构：类型转回 const char*
        assert!(c.contains("t_label = (const char*) ((long long*)__t_t)[1];"), "生成的 C：\n{}", c);
        // owned 元素槽由 prologue 预声明为 NULL 指针
        assert!(c.contains("const char* t_label = NULL;"), "生成的 C：\n{}", c);
    }
}
