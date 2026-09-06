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
    if (!ok) { fprintf(stderr, "契约失败：%s\n", what ? what : ""); exit(1); }
}
