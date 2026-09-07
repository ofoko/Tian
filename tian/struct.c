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

typedef struct {
    long long t_x;
    long long t_y;
} t_Point;

static t_Point* __t_new_t_Point(long long t0, long long t1) { t_Point* p = (t_Point*)malloc(sizeof(t_Point));
    p->t_x = t0;
    p->t_y = t1;
    return p; }

static void __t_free_t_Point(t_Point* p) { if (!p) return;
    free(p); }

typedef struct {
    const char* t_name;
    long long t_age;
} t_Person;

static t_Person* __t_new_t_Person(const char* t0, long long t1) { t_Person* p = (t_Person*)malloc(sizeof(t_Person));
    p->t_name = t0;
    p->t_age = t1;
    return p; }

static void __t_free_t_Person(t_Person* p) { if (!p) return;
    __t_free(p->t_name); p->t_name = NULL;
    free(p); }

static long long t_getx(t_Point* t_p);
static t_Point* t_origin(void);


int main(void) {
    t_Point* t_p = NULL;
    t_Point* t_q = NULL;
    t_Person* t_who = NULL;
    t_Point* t_o = NULL;
    __t_free_t_Point(t_p);
    t_p = __t_new_t_Point(1, 2);
    printf("%lld\n", (long long)(t_p->t_x));
    printf("%lld\n", (long long)(t_p->t_y));
    __t_free_t_Point(t_q);
    t_q = __t_new_t_Point(20, 10);
    printf("%lld\n", (long long)(t_q->t_x));
    printf("%lld\n", (long long)(t_q->t_y));
    t_p->t_x = 100;
    printf("%lld\n", (long long)(t_p->t_x));
    __t_free_t_Person(t_who);
    t_who = __t_new_t_Person(__t_dup("天"), 1);
    printf("%s\n", (t_who->t_name));
    printf("%lld\n", (long long)(t_who->t_age));
    __t_free(t_who->t_name); t_who->t_name = __t_dup("道");
    printf("%s\n", (t_who->t_name));
    printf("%lld\n", (long long)(t_getx(t_p)));
    t_p = NULL;
    __t_free_t_Point(t_o);
    t_o = t_origin();
    printf("%lld\n", (long long)(t_o->t_x));
    printf("%lld\n", (long long)(t_o->t_y));
    __t_free_t_Point(t_p); t_p = NULL;
    __t_free_t_Point(t_q); t_q = NULL;
    __t_free_t_Person(t_who); t_who = NULL;
    __t_free_t_Point(t_o); t_o = NULL;
    return 0;
}

static long long t_getx(t_Point* t_p) {
    return (long long)(t_p->t_x);
        __t_free_t_Point(t_p); t_p = NULL;
    return 0;
}

static t_Point* t_origin(void) {
    { t_Point* t_ret = __t_new_t_Point(0, 0);
    return t_ret; }
    return NULL;
}

