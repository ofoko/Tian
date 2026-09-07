# 天语言（Tian）

> AI 原生的极简编译型语言：取 C 之所长（极简、零运行时、可自由链接 C），
> 取 Rust 之所长（强类型、所有权安全），并为 AI 代理内建了**可执行契约**与**结构化诊断**。

规范唯一标准：[天语言规范.md](./天语言规范.md)。当前版本 **v4.4**。

## 快速开始

```sh
# 编译器（Rust 实现，Cranelift 原生后端 + C 中转后端）
cd tc && cargo build --release

# 运行
./tc/target/release/tc run 示例.t        # C 中转，一步到位
./tc/target/release/tc build 示例.t      # Cranelift 原生机器码
./tc/target/release/tc test 契约.t       # 只跑 @example 语义锚点自检
```

## 一个 30 秒例子

```
# 求和：数组参数是只读视图，len() 是内建
//a=i64[3]{10,20,30}
f/sum(x:i64[3]):i64{
    //i=0
    //s=0
    w/i<len(x){
        s=s+x[i]
        i=i+1
    }
    r/s
}
`sum(a)
```

语言核心特性：
- **五种基本类型**（i32/i64/f64/str/bool）+ **结构体**（v3.0）+ **固定长度数组**（v3.3，参数为只读视图）；
- **天权内存模型**：所有权 + 移动语义 + 只读借用，无 GC、确定性释放（规范第 11 节）；
- **语义锚点** `@pre` `@post` `@example`：函数契约编译为运行时校验与自测（规范第 12 节）；
- **模块系统**：`use 名字` 导入同目录 `名字.t`，支持递归导入、防循环（规范第 17 节）；
- **条件表达式** `sel(条件, 值1, 值2)` 与 **break/continue**（规范第 18/19 节）；
- **动态数组** `[]i64`/`[]str`：push/pop 自动增长、运行时越界守卫、天权移动语义、`&[]T` 只读借用（规范第 22 节）；
- **断言内建** `panic(msg)` / `check(cond, msg)`（规范第 21 节）；
- **快速失败运行时**：除零、数组越界统一 `运行时错误：…` + exit(1)（规范第 15 节）。

## 面向 AI 代理的工具链

| 命令 | 用途 |
|------|------|
| `tc xxx.t --diagnose-json` | 结构化编译诊断（stage/line/message/hint/candidates），供程序化读取与自动修复 |
| `tc test xxx.t` | 只运行 `@example` 自检，改动后快速验证函数级行为不变 |
| `tc fmt xxx.t` | 规范化源码格式（幂等，输出到 stdout）——AI 协作 diff 规范化的基础 |
| `tc lex xxx.t` | 词法分析调试 |

## 开发与测试（改编译器前必读）

```sh
cargo test --release          # 编译器单元测试（40 个）
./tests/run_tests.sh          # 黄金测试：32 个 .t 用例 × 双后端交叉验证
```

- `tests/cases/*.t` 是黄金用例；同名 `.expect` 是期望输出；
  `err_*` 前缀 = 期望编译错误；`rt_*` 前缀 = 期望运行时失败（双后端 stdout/stderr 必须一致）。
- **任何语义改动必须同时通过双后端一致性检查**（C 中转 vs Cranelift 原生）——
  历史上 str 比较、bool 打印、除零行为都曾双后端漂移。
- 语义改动必须同步修订规范并记入修订记录；规范是编译器的唯一标准。

## 吃狗粮示例：demo/

[demo/](./demo/) 是用天语言本身写的多模块程序（最小堆 + 素数筛），是语言能力的第一块试金石：

```sh
./tc/target/release/tc run demo/main.t
```

它实际用到了：use 模块导入、动态数组（push/pop/索引/len）、移动语义的"修改型函数接管并交还"惯用法、@pre/@post 语义锚点、check 断言、sel 与 break/continue。demo/json.t 是 JSON 校验器（递归下降），demo/stats.t 是统计器（字符串感知状态机 + tof）——两者组成第一个端到端流水线：校验 → 计数 → 求和。

## 目录结构

```
天语言规范.md      # 语言规范（唯一标准，含逐版修订记录）
tc/               # 编译器（Rust）
  src/lexer.rs        # 词法（斜杠歧义判定见规范 2.7）
  src/parser.rs       # 语法
  src/type_check.rs   # 类型检查 + 天权所有权检查
  src/gen_c.rs        # C 中转后端
  src/gen_native.rs   # Cranelift 原生后端
  src/tian_rt.c       # 运行时（打印/拼接/panic/契约校验）
tests/            # 黄金测试 + 双后端运行器
docs/             # 决策参考文档
*.t               # 验收程序（main/own/borrow/contract/struct）
demo/             # 吃狗粮多模块示例（最小堆 + 素数筛）
```
