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

static long long t_greet(const char* t_x);


int main(void) {
    const char* t_s = NULL;
    const char* t_c = NULL;
    __t_free(t_s);
    t_s = __t_dup("天道");
    t_greet(t_s);
    printf("%s\n", (t_s));
    __t_free(t_c);
    t_c = __t_dup(t_s);
    printf("%s\n", (t_c));
    __t_free(t_s); t_s = NULL;
    __t_free(t_c); t_c = NULL;
    return 0;
}

static long long t_greet(const char* t_x) {
    printf("%s\n", (t_x));
    return (long long)(0);
    return 0;
}

