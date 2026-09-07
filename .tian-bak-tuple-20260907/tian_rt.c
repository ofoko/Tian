/* tian_rt.c —— 天语言原生后端运行时（tc build 链接用）
 * tc 生成 .o 后与本文件编译产物链接；cc 仅作链接器与启动器。
 * 约定：编译器总会生成 __tian_top（顶层语句体），本文件 main 调用它。
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int __tian_top(void);

int main(void) { return __tian_top(); }

/* 打印：每种类型一个固定签名函数，规避可变参数 ABI 复杂度 */
void __t_print_ll(long long v) { printf("%lld\n", v); }
void __t_print_f(double v)     { printf("%g\n", v); }
void __t_print_s(const char* v){ printf("%s\n", v ? v : ""); }
void __t_print_b(int v)        { printf("%s\n", v ? "true" : "false"); }

/* tos：数字/bool → 字符串（8 个轮换静态缓冲） */
static char __t_sb[8][64];
static int  __t_bi = 0;
const char* __t_tos_ll(long long v) { char* b = __t_sb[__t_bi++ & 7]; snprintf(b, 64, "%lld", v); return b; }
const char* __t_tos_f(double v)     { char* b = __t_sb[__t_bi++ & 7]; snprintf(b, 64, "%g", v); return b; }
const char* __t_tos_b(int v)        { return v ? "true" : "false"; }

/* 字符串拼接（malloc，进程退出回收——脚本语义可接受） */
char* __t_cat(const char* a, const char* b) {
    size_t la = strlen(a ? a : ""), lb = strlen(b ? b : "");
    char* r = (char*)malloc(la + lb + 1);
    if (!r) { fprintf(stderr, "天运行时：内存分配失败\n"); exit(1); }
    memcpy(r, a ? a : "", la);
    memcpy(r + la, b ? b : "", lb + 1);
    return r;
}

/* 字符串比较 */
int __t_seq(const char* a, const char* b) { return strcmp(a ? a : "", b ? b : "") == 0; }
int __t_sne(const char* a, const char* b) { return strcmp(a ? a : "", b ? b : "") != 0; }

/* 天权 v2.0：显式复制与释放（free(NULL) 天然安全——移动置空后的槽位可无差别释放） */
char* __t_dup(const char* s) {
    size_t n = (s ? strlen(s) : 0) + 1;
    char* r = (char*)malloc(n);
    if (!r) { fprintf(stderr, "天运行时：内存分配失败\n"); exit(1); }
    memcpy(r, s ? s : "", n);
    return r;
}
void __t_free(const char* p) { free((void*)p); }

/* 堆分配（C 与 Cranelift 后端共用；结构体构造 / 未来扩展用） */
void* __t_malloc(long long size) {
    void* p = malloc((size_t)size);
    if (!p) { fprintf(stderr, "天运行时：内存分配失败\n"); exit(1); }
    return p;
}

/* 天权 v2.2 语义锚点：契约断言（@pre/@post/@example 失败即非零退出） */
void __t_check(int ok, const char* what) {
    if (!ok) { fprintf(stderr, "%s\n", what ? what : ""); exit(1); }
}

// v3.2 运行时快速失败：统一错误出口（规范第 15 节）
void __t_panic(const char* what) {
    fprintf(stderr, "运行时错误：%s\n", what ? what : "");
    exit(1);
}
int __t_idiv_i(int a, int b)          { if (b == 0) __t_panic("除数为零"); return a / b; }
long long __t_idiv_l(long long a, long long b) { if (b == 0) __t_panic("除数为零"); return a / b; }

// ── v4.0 动态数组（规范第 22 节）────────────────────────────────
// 布局：头部 {len, cap} 各 8 字节，随后每元素 8 字节槽（i32 符号扩展、bool 零扩展存储，
// 与定长数组的槽格式一致）。元素类型仅 i32/i64/f64/bool。
// push 可能 realloc 移动内存：返回新指针，调用方必须接管（变量重新绑定）。

// v4.4：tof —— str → f64（strtod 全量消费，失败即 panic，规范第 7 节）
double __t_tof(const char* s) {
    char* end = 0;
    double v = strtod(s, &end);
    if (end == s || *end != 0) __t_panic("无效数字");
    return v;
}

// v4.2：str 字节读 + 子串（规范第 23 节）
long long __t_sget(const char* s, long long i) {
    long long n = (long long)strlen(s);
    if (i < 0 || i >= n) __t_panic("字符串下标越界");
    return (long long)(unsigned char)s[i];
}
char* __t_sub(const char* s, long long start, long long n) {
    long long len = (long long)strlen(s);
    if (start < 0 || n < 0 || start + n > len) __t_panic("子串越界");
    char* r = (char*)malloc((size_t)n + 1);
    if (!r) { fprintf(stderr, "天运行时：内存分配失败\n"); exit(1); }
    memcpy(r, s + start, (size_t)n);
    r[n] = 0;
    return r;
}

typedef struct { long long len, cap; } __t_darr_hdr;
static char* __t_darr_data(void* h) { return (char*)h + 16; }
static void* __t_dgrow(void* h) {
    __t_darr_hdr* x = (__t_darr_hdr*)h;
    x->cap *= 2;
    void* n = realloc(h, 16 + (size_t)x->cap * 8);
    if (!n) { fprintf(stderr, "天运行时：内存分配失败\n"); exit(1); }
    return n;
}

void* __t_dnew(long long cap) {
    if (cap < 4) cap = 4;
    void* h = malloc(16 + (size_t)cap * 8);
    if (!h) { fprintf(stderr, "天运行时：内存分配失败\n"); exit(1); }
    ((__t_darr_hdr*)h)->len = 0;
    ((__t_darr_hdr*)h)->cap = cap;
    return h;
}
long long __t_dlen(void* h) { return ((__t_darr_hdr*)h)->len; }
static void __t_dbound(void* h, long long i) {
    long long len = ((__t_darr_hdr*)h)->len;
    if (i < 0 || i >= len) __t_panic("数组越界");
}
long long __t_dget_i(void* h, long long i) { __t_dbound(h, i); return ((long long*)__t_darr_data(h))[i]; }
double __t_dget_f(void* h, long long i) { __t_dbound(h, i); return ((double*)__t_darr_data(h))[i]; }
void __t_dset_i(void* h, long long i, long long v) { __t_dbound(h, i); ((long long*)__t_darr_data(h))[i] = v; }
void __t_dset_f(void* h, long long i, double v) { __t_dbound(h, i); ((double*)__t_darr_data(h))[i] = v; }
void __t_dpop(void* h) {
    __t_darr_hdr* x = (__t_darr_hdr*)h;
    if (x->len == 0) __t_panic("pop 空数组");
    x->len--;
}
// v4.2：str 元素 —— push 接管所有权（存指针）；容器释放时逐元素深释放
void* __t_dpush_s(void* h, char* v) {
    __t_darr_hdr* x = (__t_darr_hdr*)h;
    if (x->len == x->cap) h = __t_dgrow(h);
    ((char**)__t_darr_data(h))[((__t_darr_hdr*)h)->len] = v;
    ((__t_darr_hdr*)h)->len++;
    return h;
}
char* __t_dget_s(void* h, long long i) {
    __t_dbound(h, i);
    return ((char**)__t_darr_data(h))[i];
}
void __t_dfree_s(void* h) {
    if (!h) return;
    __t_darr_hdr* x = (__t_darr_hdr*)h;
    for (long long i = 0; i < x->len; i++) free(((char**)__t_darr_data(h))[i]);
    free(h);
}
void* __t_dpush_i(void* h, long long v) {
    __t_darr_hdr* x = (__t_darr_hdr*)h;
    if (x->len == x->cap) h = __t_dgrow(h);
    ((long long*)__t_darr_data(h))[((__t_darr_hdr*)h)->len] = v;
    ((__t_darr_hdr*)h)->len++;
    return h;
}
void* __t_dpush_f(void* h, double v) {
    __t_darr_hdr* x = (__t_darr_hdr*)h;
    if (x->len == x->cap) h = __t_dgrow(h);
    ((double*)__t_darr_data(h))[((__t_darr_hdr*)h)->len] = v;
    ((__t_darr_hdr*)h)->len++;
    return h;
}
