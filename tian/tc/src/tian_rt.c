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

// ── v4.7 关联数组 map[K]V（规范第 25 节）────────────────────────
// 布局：头部复用 darr 的 {len,cap}（各 8 字节），随后桶数组 cap*8 字节。
// 桶数恒为 2 的幂；节点独立 malloc（不入块），扩容 realloc 只搬块与桶数组、节点不动。
// 节点 key/val 为 8 字节槽：str 存指针、f64 以位模式存、i64/bool 直存。
// 所有 K×V 变体节点布局统一；链哈希沿 hash & (cap-1)。
// 键自白名单：i64、str；值 wire 三类：i(long long)/f(double)/s(char*)。
typedef struct __t_mnode { long long hash; struct __t_mnode* next; long long key; long long val; } __t_mnode;
typedef struct { long long len, cap; } __t_map_hdr;
static __t_mnode** __t_mbuckets(void* h) { return (__t_mnode**)((char*)h + 16); }

// str 键 FNV-1a 64；i64 键 splitmix64 扰动
static long long __t_mhash_s(const char* s) {
    long long h = 0xcbf29ce484222325;
    while (*s) h = (h ^ (unsigned char)*s++) * 0x100000001b3;
    return h;
}
static long long __t_mhash_i(long long x) {
    x ^= x >> 33; x *= 0xff51afd7ed558ccd;
    x ^= x >> 33; x *= 0xc4ceb9fe1a85ec53;
    x ^= x >> 33;
    return x;
}

static __t_mnode* __t_mfind_i(void* h, long long k) {
    __t_map_hdr* x = (__t_map_hdr*)h;
    __t_mnode* n = __t_mbuckets(h)[__t_mhash_i(k) & (x->cap - 1)];
    while (n && n->key != k) n = n->next;
    return n;
}
static __t_mnode* __t_mfind_s(void* h, const char* k) {
    __t_map_hdr* x = (__t_map_hdr*)h;
    __t_mnode* n = __t_mbuckets(h)[__t_mhash_s(k) & (x->cap - 1)];
    while (n && strcmp((char*)n->key, k)) n = n->next;
    return n;
}

// 扩容：cap *= 2，重链全部节点（cap 恒为 2 幂 → 每个节点只落回本桶或新一半，不交叉）
static void* __t_mgrow(void* h) {
    __t_map_hdr* x = (__t_map_hdr*)h;
    long long oldcap = x->cap;
    long long newcap = oldcap * 2;
    void* n = realloc(h, 16 + (size_t)newcap * 8);
    if (!n) { fprintf(stderr, "天运行时：内存分配失败\n"); exit(1); }
    ((__t_map_hdr*)n)->cap = newcap;
    __t_mnode** b = __t_mbuckets(n);
    memset(b + oldcap, 0, (size_t)oldcap * 8);   // 新一半清零
    for (long long i = 0; i < oldcap; i++) {
        __t_mnode* cur = b[i];
        b[i] = NULL;                              // 摘下本桶，避免重复重链
        while (cur) {
            __t_mnode* nx = cur->next;
            long long j = cur->hash & (newcap - 1);
            cur->next = b[j];
            b[j] = cur;
            cur = nx;
        }
    }
    return n;
}

static __t_mnode* __t_mnode_new(void) {
    __t_mnode* n = (__t_mnode*)malloc(sizeof(__t_mnode));
    if (!n) { fprintf(stderr, "天运行时：内存分配失败\n"); exit(1); }
    return n;
}

void* __t_mnew(long long cap) {
    if (cap < 8) cap = 8;
    void* h = malloc(16 + (size_t)cap * 8);
    if (!h) { fprintf(stderr, "天运行时：内存分配失败\n"); exit(1); }
    ((__t_map_hdr*)h)->len = 0;
    ((__t_map_hdr*)h)->cap = cap;
    memset(__t_mbuckets(h), 0, (size_t)cap * 8);
    return h;
}
long long __t_mlen(void* h) { return ((__t_map_hdr*)h)->len; }

// has：命中 bool，未命中不 panic
long long __t_mhas_i(void* h, long long k) { return __t_mfind_i(h, k) != NULL; }
long long __t_mhas_s(void* h, const char* k) { return __t_mfind_s(h, k) != NULL; }

// get：命中返回值；未命中快速失败（对拍数组越界）
long long __t_mget_ii(void* h, long long k) { __t_mnode* n = __t_mfind_i(h, k); if (!n) __t_panic("map 键不存在"); return n->val; }
double __t_mget_if(void* h, long long k) { __t_mnode* n = __t_mfind_i(h, k); if (!n) __t_panic("map 键不存在"); double d; memcpy(&d, &n->val, 8); return d; }
char* __t_mget_is(void* h, long long k) { __t_mnode* n = __t_mfind_i(h, k); if (!n) __t_panic("map 键不存在"); return (char*)n->val; }
long long __t_mget_si(void* h, const char* k) { __t_mnode* n = __t_mfind_s(h, k); if (!n) __t_panic("map 键不存在"); return n->val; }
double __t_mget_sf(void* h, const char* k) { __t_mnode* n = __t_mfind_s(h, k); if (!n) __t_panic("map 键不存在"); double d; memcpy(&d, &n->val, 8); return d; }
char* __t_mget_ss(void* h, const char* k) { __t_mnode* n = __t_mfind_s(h, k); if (!n) __t_panic("map 键不存在"); return (char*)n->val; }

// set：整体移动语义 → 返回新 h，调用方必须重绑（可能 realloc）。
// str 键：调用方交付一份新副本，键已存在则消费（free）该副本、复用旧节点；不存在则接管新开节点。

void* __t_mset_ii(void* h, long long k, long long v) {
    __t_map_hdr* x = (__t_map_hdr*)h;
    __t_mnode* n = __t_mfind_i(h, k);
    if (n) { n->val = v; return h; }
    if ((x->len + 1) * 4 > x->cap * 3) { h = __t_mgrow(h); x = (__t_map_hdr*)h; }
    long long hh = __t_mhash_i(k);
    __t_mnode* nn = __t_mnode_new();
    nn->hash = hh; nn->key = k; nn->val = v;
    nn->next = __t_mbuckets(h)[hh & (x->cap - 1)];
    __t_mbuckets(h)[hh & (x->cap - 1)] = nn;
    x->len++;
    return h;
}
void* __t_mset_if(void* h, long long k, double v) {
    long long bits; memcpy(&bits, &v, 8);
    return __t_mset_ii(h, k, bits);
}
void* __t_mset_is(void* h, long long k, char* v) {
    __t_map_hdr* x = (__t_map_hdr*)h;
    __t_mnode* n = __t_mfind_i(h, k);
    if (n) { free((void*)n->val); n->val = (long long)v; return h; }
    if ((x->len + 1) * 4 > x->cap * 3) { h = __t_mgrow(h); x = (__t_map_hdr*)h; }
    long long hh = __t_mhash_i(k);
    __t_mnode* nn = __t_mnode_new();
    nn->hash = hh; nn->key = k; nn->val = (long long)v;
    nn->next = __t_mbuckets(h)[hh & (x->cap - 1)];
    __t_mbuckets(h)[hh & (x->cap - 1)] = nn;
    x->len++;
    return h;
}
void* __t_mset_si(void* h, char* k, long long v) {
    __t_map_hdr* x = (__t_map_hdr*)h;
    __t_mnode* n = __t_mfind_s(h, k);
    if (n) { free(k); n->val = v; return h; }
    if ((x->len + 1) * 4 > x->cap * 3) { h = __t_mgrow(h); x = (__t_map_hdr*)h; }
    long long hh = __t_mhash_s(k);
    __t_mnode* nn = __t_mnode_new();
    nn->hash = hh; nn->key = (long long)k; nn->val = v;
    nn->next = __t_mbuckets(h)[hh & (x->cap - 1)];
    __t_mbuckets(h)[hh & (x->cap - 1)] = nn;
    x->len++;
    return h;
}
void* __t_mset_sf(void* h, char* k, double v) {
    long long bits; memcpy(&bits, &v, 8);
    return __t_mset_si(h, k, bits);
}
void* __t_mset_ss(void* h, char* k, char* v) {
    __t_map_hdr* x = (__t_map_hdr*)h;
    __t_mnode* n = __t_mfind_s(h, k);
    if (n) { free(k); free((void*)n->val); n->val = (long long)v; return h; }
    if ((x->len + 1) * 4 > x->cap * 3) { h = __t_mgrow(h); x = (__t_map_hdr*)h; }
    long long hh = __t_mhash_s(k);
    __t_mnode* nn = __t_mnode_new();
    nn->hash = hh; nn->key = (long long)k; nn->val = (long long)v;
    nn->next = __t_mbuckets(h)[hh & (x->cap - 1)];
    __t_mbuckets(h)[hh & (x->cap - 1)] = nn;
    x->len++;
    return h;
}

// del：命中则 len--、释放节点与 str 部件；不存在静默（同 darr pop 不报错）
void __t_mdel_ii(void* h, long long k) {
    __t_map_hdr* x = (__t_map_hdr*)h;
    long long idx = __t_mhash_i(k) & (x->cap - 1);
    __t_mnode** pp = &__t_mbuckets(h)[idx];
    while (*pp) {
        if ((*pp)->key == k) { __t_mnode* t = *pp; *pp = t->next; free(t); x->len--; return; }
        pp = &(*pp)->next;
    }
}
void __t_mdel_if(void* h, long long k) { __t_mdel_ii(h, k); }
void __t_mdel_is(void* h, long long k) {
    __t_map_hdr* x = (__t_map_hdr*)h;
    long long idx = __t_mhash_i(k) & (x->cap - 1);
    __t_mnode** pp = &__t_mbuckets(h)[idx];
    while (*pp) {
        if ((*pp)->key == k) { __t_mnode* t = *pp; *pp = t->next; free((void*)t->val); free(t); x->len--; return; }
        pp = &(*pp)->next;
    }
}
void __t_mdel_si(void* h, const char* k) {
    __t_map_hdr* x = (__t_map_hdr*)h;
    long long idx = __t_mhash_s(k) & (x->cap - 1);
    __t_mnode** pp = &__t_mbuckets(h)[idx];
    while (*pp) {
        if (!strcmp((char*)(*pp)->key, k)) { __t_mnode* t = *pp; *pp = t->next; free((void*)t->key); free(t); x->len--; return; }
        pp = &(*pp)->next;
    }
}
void __t_mdel_sf(void* h, const char* k) { __t_mdel_si(h, k); }
void __t_mdel_ss(void* h, const char* k) {
    __t_map_hdr* x = (__t_map_hdr*)h;
    long long idx = __t_mhash_s(k) & (x->cap - 1);
    __t_mnode** pp = &__t_mbuckets(h)[idx];
    while (*pp) {
        if (!strcmp((char*)(*pp)->key, k)) { __t_mnode* t = *pp; *pp = t->next; free((void*)t->key); free((void*)t->val); free(t); x->len--; return; }
        pp = &(*pp)->next;
    }
}

// free：整体深释放（首字母=键、末字母=值，s 表示释放对应 str）；if(!h)return
void __t_mfree_ii(void* h) {
    if (!h) return;
    __t_map_hdr* x = (__t_map_hdr*)h;
    for (long long i = 0; i < x->cap; i++) {
        __t_mnode* n = __t_mbuckets(h)[i];
        while (n) { __t_mnode* nx = n->next; free(n); n = nx; }
    }
    free(h);
}
void __t_mfree_if(void* h) { __t_mfree_ii(h); }
void __t_mfree_is(void* h) {
    if (!h) return;
    __t_map_hdr* x = (__t_map_hdr*)h;
    for (long long i = 0; i < x->cap; i++) {
        __t_mnode* n = __t_mbuckets(h)[i];
        while (n) { __t_mnode* nx = n->next; free((void*)n->val); free(n); n = nx; }
    }
    free(h);
}
void __t_mfree_si(void* h) {
    if (!h) return;
    __t_map_hdr* x = (__t_map_hdr*)h;
    for (long long i = 0; i < x->cap; i++) {
        __t_mnode* n = __t_mbuckets(h)[i];
        while (n) { __t_mnode* nx = n->next; free((void*)n->key); free(n); n = nx; }
    }
    free(h);
}
void __t_mfree_sf(void* h) { __t_mfree_si(h); }
void __t_mfree_ss(void* h) {
    if (!h) return;
    __t_map_hdr* x = (__t_map_hdr*)h;
    for (long long i = 0; i < x->cap; i++) {
        __t_mnode* n = __t_mbuckets(h)[i];
        while (n) { __t_mnode* nx = n->next; free((void*)n->key); free((void*)n->val); free(n); n = nx; }
    }
    free(h);
}
