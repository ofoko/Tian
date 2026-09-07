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

static long long t_add(long long t_x, long long t_y);
static const char* t_make(const char* t_s);


int main(void) {
    if (!((t_add(1, 2)) == (3))) { fprintf(stderr, "@example 契约失败：add（第 5 行）\n"); exit(1); }
    if (!(__t_seq(t_make("天"), "天道"))) { fprintf(stderr, "@example 契约失败：make（第 11 行）\n"); exit(1); }
    printf("%lld\n", (long long)(t_add(1, 2)));
    printf("%s\n", (t_make("天")));
    return 0;
}

static long long t_add(long long t_x, long long t_y) {
    if (!((t_x > 0))) { fprintf(stderr, "@pre 契约失败：add（第 3 行）\n"); exit(1); }
    { long long t_ret = (t_x + t_y);
    if (!((t_ret > t_x))) { fprintf(stderr, "@post 契约失败（第 4 行）\n"); exit(1); }
    return t_ret; }
    { long long t_ret = 0;
    if (!((t_ret > t_x))) { fprintf(stderr, "@post 契约失败：add（第 4 行）\n"); exit(1); } return t_ret; }
}

static const char* t_make(const char* t_s) {
    if (!(__t_sne(t_s, ""))) { fprintf(stderr, "@pre 契约失败：make（第 9 行）\n"); exit(1); }
    { const char* t_ret = __t_cat(t_s, "道");
    if (!(__t_seq(t_ret, "天道"))) { fprintf(stderr, "@post 契约失败（第 10 行）\n"); exit(1); }
    return t_ret; }
        __t_free(t_s); t_s = NULL;
    { const char* t_ret = __t_dup("");
    if (!(__t_seq(t_ret, "天道"))) { fprintf(stderr, "@post 契约失败：make（第 10 行）\n"); exit(1); } return t_ret; }
}

