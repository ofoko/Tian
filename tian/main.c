/* 由 tc（Tian Compiler v0.1）生成 */
#include <stdio.h>

static char __t_sb[8][64]; static int __t_bi = 0;
static const char* __t_tos_ll(long long v){ char* b=__t_sb[__t_bi++&7]; snprintf(b,64,"%lld",v); return b; }
static const char* __t_tos_f(double v){ char* b=__t_sb[__t_bi++&7]; snprintf(b,64,"%g",v); return b; }
static const char* __t_tos_b(int v){ return v ? "true" : "false"; }

static void t_add(long long t_x, long long t_y);

int main(void) {
    long long t_a = 20;
    printf("%lld\n", (long long)((t_a / 2)));
    long long t_cnt = 1;
    while ((t_cnt <= 3)) {
        t_cnt = (t_cnt + 1);
        printf("%lld\n", (long long)((t_cnt / t_a)));
    }
    t_add((long long)10, (long long)90);
    return 0;
}

static void t_add(long long t_x, long long t_y) {
    long long t_res = (t_x + t_y);
    printf("%lld\n", (long long)(t_res));
}

