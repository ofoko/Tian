/* 由 tc（Tian Compiler v2.0，天权）生成 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static char __t_sb[8][64]; static int __t_bi = 0;
static const char* __t_tos_ll(long long v){ char* b=__t_sb[__t_bi++&7]; snprintf(b,64,"%lld",v); return b; }
static const char* __t_tos_f(double v){ char* b=__t_sb[__t_bi++&7]; snprintf(b,64,"%g",v); return b; }
static const char* __t_tos_b(int v){ return v ? "true" : "false"; }
static char* __t_cat(const char* a, const char* b){ size_t la=strlen(a),lb=strlen(b); char* r=(char*)malloc(la+lb+1); memcpy(r,a,la); memcpy(r+la,b,lb+1); return r; }
static int __t_seq(const char* a, const char* b){ return strcmp(a,b)==0; }
static int __t_sne(const char* a, const char* b){ return strcmp(a,b)!=0; }
static char* __t_dup(const char* s){ size_t n=strlen(s?s:"")+1; char* r=(char*)malloc(n); memcpy(r,s?s:"",n); return r; }
static void __t_free(const char* p){ free((void*)p); }
static void __t_panic(const char* w){ fprintf(stderr, "运行时错误：%s\n", w?w:""); exit(1); }
static long long __t_idx(long long len, long long i){ if(i<0||i>=len) __t_panic("数组越界"); return i; }
typedef struct { long long len, cap; } __t_darr_hdr;
static char* __t_darr_data(void* h){ return (char*)h + 16; }
static void* __t_dgrow(void* h){ __t_darr_hdr* x=(__t_darr_hdr*)h; x->cap*=2; void* n=realloc(h,16+(size_t)x->cap*8); if(!n){fprintf(stderr,"天运行时：内存分配失败\n");exit(1);} return n; }
static void* __t_dnew(long long cap){ if(cap<4)cap=4; void* h=malloc(16+(size_t)cap*8); if(!h){fprintf(stderr,"天运行时：内存分配失败\n");exit(1);} ((__t_darr_hdr*)h)->len=0; ((__t_darr_hdr*)h)->cap=cap; return h; }
static long long __t_dlen(void* h){ return ((__t_darr_hdr*)h)->len; }
static void __t_dbound(void* h, long long i){ long long len=((__t_darr_hdr*)h)->len; if(i<0||i>=len) __t_panic("数组越界"); }
static long long __t_dget_i(void* h, long long i){ __t_dbound(h,i); return ((long long*)__t_darr_data(h))[i]; }
static double __t_dget_f(void* h, long long i){ __t_dbound(h,i); return ((double*)__t_darr_data(h))[i]; }
static void __t_dset_i(void* h, long long i, long long v){ __t_dbound(h,i); ((long long*)__t_darr_data(h))[i]=v; }
static void __t_dset_f(void* h, long long i, double v){ __t_dbound(h,i); ((double*)__t_darr_data(h))[i]=v; }
static void* __t_dpush_i(void* h, long long v){ __t_darr_hdr* x=(__t_darr_hdr*)h; if(x->len==x->cap) h=__t_dgrow(h); ((long long*)__t_darr_data(h))[((__t_darr_hdr*)h)->len]=v; ((__t_darr_hdr*)h)->len++; return h; }
static void* __t_dpush_f(void* h, double v){ __t_darr_hdr* x=(__t_darr_hdr*)h; if(x->len==x->cap) h=__t_dgrow(h); ((double*)__t_darr_data(h))[((__t_darr_hdr*)h)->len]=v; ((__t_darr_hdr*)h)->len++; return h; }
static void __t_dpop(void* h){ __t_darr_hdr* x=(__t_darr_hdr*)h; if(x->len==0) __t_panic("pop 空数组"); x->len--; }
static long long __t_sget(const char* s, long long i){ long long n=(long long)strlen(s); if(i<0||i>=n) __t_panic("字符串下标越界"); return (long long)(unsigned char)s[i]; }
static void* __t_dpush_s(void* h, char* v){ __t_darr_hdr* x=(__t_darr_hdr*)h; if(x->len==x->cap) h=__t_dgrow(h); ((char**)__t_darr_data(h))[((__t_darr_hdr*)h)->len]=v; ((__t_darr_hdr*)h)->len++; return h; }
static char* __t_dget_s(void* h, long long i){ __t_dbound(h,i); return ((char**)__t_darr_data(h))[i]; }
static void __t_dfree_s(void* h){ __t_darr_hdr* x=(__t_darr_hdr*)h; for(long long i=0;i<x->len;i++) free(((char**)__t_darr_data(h))[i]); free(h); }
static char* __t_sub(const char* s, long long start, long long n){ long long len=(long long)strlen(s); if(start<0||n<0||start+n>len) __t_panic("子串越界"); char* r=(char*)malloc((size_t)n+1); if(!r){fprintf(stderr,"天运行时：内存分配失败\n");exit(1);} memcpy(r,s+start,(size_t)n); r[n]=0; return r; }
static int __t_idiv_i(int a, int b){ if(b==0) __t_panic("除数为零"); return a/b; }
static long long __t_idiv_l(long long a, long long b){ if(b==0) __t_panic("除数为零"); return a/b; }

static const char* t_join(void* t_a);


int main(void) {
    void* t_names = NULL;
    const char* t_first = NULL;
    void* t_moved = NULL;
    { void* __t_h = __t_dnew(2);
      __t_dpush_s(__t_h, __t_dup("天"));
      __t_dpush_s(__t_h, __t_dup("道"));
      t_names = __t_h; }
    t_names = __t_dpush_s(t_names, __t_dup("无穷"));
    printf("%lld\n", (long long)(__t_dlen(t_names)));
    printf("%s\n", (__t_dget_s(t_names, 0)));
    printf("%s\n", (__t_dget_s(t_names, 2)));
    __t_free(t_first);
    t_first = __t_dup(__t_dget_s(t_names, 0));
    printf("%s\n", (t_first));
    t_moved = t_names;
    t_moved = __t_dpush_s(t_moved, __t_dup("终"));
    printf("%lld\n", (long long)(__t_dlen(t_moved)));
    printf("%s\n", (__t_dget_s(t_moved, 3)));
    printf("%s\n", (t_join(t_moved)));
    t_moved = NULL;
    __t_dfree_s(t_names); t_names = NULL;
    __t_free(t_first); t_first = NULL;
    __t_dfree_s(t_moved); t_moved = NULL;
    return 0;
}

static const char* t_join(void* t_a) {
    const char* t_r = NULL;
    long long t_i = 1;
    __t_free(t_r);
    t_r = __t_dup(__t_dget_s(t_a, 0));
    while ((t_i < __t_dlen(t_a))) {
        { const char* __t_tmp = __t_cat(__t_cat(t_r, "-"), __t_dget_s(t_a, t_i));
        __t_free(t_r);
t_r = __t_tmp; }
        t_i = (t_i + 1);
    }
    { const char* t_ret = t_r;
    t_r = NULL;
    return t_ret; }
        __t_free(t_r); t_r = NULL;
        __t_dfree_s(t_a); t_a = NULL;
    return __t_dup("");
}

