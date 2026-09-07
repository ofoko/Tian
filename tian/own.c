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

static const char* t_shout(const char* t_x);
static const char* t_id(const char* t_x);


int main(void) {
    const char* t_s = NULL;
    const char* t_c = NULL;
    const char* t_t = NULL;
    const char* t_m = NULL;
    const char* t_e = NULL;
    __t_free(t_s);
    t_s = __t_dup("天道");
    printf("%s\n", (__t_dup(t_s)));
    __t_free(t_c);
    t_c = __t_cat(t_s, "无穷");
    printf("%s\n", (t_s));
    __t_free(t_t);
    t_t = t_s;
    t_s = NULL;
    printf("%s\n", (t_t));
    { const char* __t_tmp = __t_dup("重生");
    __t_free(t_s);
t_s = __t_tmp; }
    printf("%s\n", (t_s));
    long long t_i = 0;
    while ((t_i < 3)) {
        __t_free(t_m);
        t_m = __t_cat("轮", __t_tos_ll((long long)t_i));
        printf("%s\n", (t_m));
        t_i = (t_i + 1);
    }
    printf("%s\n", (t_shout(__t_dup(t_c))));
    printf("%s\n", (t_id("直传")));
    __t_free(t_e);
    t_e = t_shout(t_c);
    t_c = NULL;
    printf("%s\n", (t_e));
    { const char* __t_tmp = t_id(t_e);
    t_e = NULL;
    t_e = __t_tmp; }
    printf("%s\n", (t_e));
    __t_free(t_s); t_s = NULL;
    __t_free(t_c); t_c = NULL;
    __t_free(t_t); t_t = NULL;
    __t_free(t_m); t_m = NULL;
    __t_free(t_e); t_e = NULL;
    return 0;
}

static const char* t_shout(const char* t_x) {
    { const char* t_ret = __t_cat(t_x, "!");
    return t_ret; }
        __t_free(t_x); t_x = NULL;
    return __t_dup("");
}

static const char* t_id(const char* t_x) {
    { const char* t_ret = t_x;
    t_x = NULL;
    return t_ret; }
        __t_free(t_x); t_x = NULL;
    return __t_dup("");
}

