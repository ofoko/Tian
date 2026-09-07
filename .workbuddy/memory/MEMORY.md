# Tian（天语言）项目长期笔记

- 位置：/Users/samly/T/tian/（规范：天语言规范.md；编译器：tc/，Rust 实现）
- 唯一标准：规范 v0.1.1。斜杠判定 2.7 = 表达式内 / 恒为除号，其余前缀声明（/a 只读，//b 可变）
- 命令：tc lex xxx.t（调试）/ tc xxx.t --emit-c / tc run xxx.t（C 后端）/ tc build xxx.t（Cranelift 原生机器码，失败回退 C）/ tc fmt（幂等格式化）/ tc test（@example 自检）
- 保留标记字：w/ i/ e/ f/ r/（受表达式上下文门控）；块内末语句可省略换行
- 当前版本 v4.5（2026-09-07）：多返回值元组落地——仅存在于返回边界（非一等公民），Ty::Tuple(id) 走类型表，malloc(N*8) 块每元素 8 字节槽，解构即释放、天权按元素接管 str。双后端一致，45 单测全绿。此前已完成：天权 v2.0 所有权/借用、v2.2 语义锚点、v3.0 结构体、v3.3/v4.0 定长/动态数组、v3.6 模块、v3.7 sel、v3.8 break/continue、v3.9 fmt、v4.2 str 字节访问/[]str、v4.3 &[]T、v4.4 tof
- 代码生成陷阱（详见 2026-09-07 日志）：①C 后端 cast 字符串自带括号不可再套 `(({})…)` 格式，用裸类型名由格式串加括号；②owned 局部槽由 prologue 经 collect_str_decls 统一预声明，MultiDecl 只补标量；③TupExpr 是 r/ 专属，fmt 输出 `r/a, b`（无括号）
- 规划：docs/ROADMAP.md（2026-09-07 拍板）——第 0 步：v4.5 全量入库建基线（仓库仅 1 个 commit，v2.0→v4.5 曾全在工作树！）；阶段一 v4.6 元组收尾 / v4.7 map[K]V / v4.8 enum+match；阶段二 v5.x 泛型/Result/str/IO/模块；阶段三 标准库分层 + 自举前端（只举 lexer/parser/checker，不举后端）+ LSP。否决项：继承/trait/GC/异常/元组一等公民。每版验收门：cargo test + 双后端对拍 + fmt 幂等 + 规范同步
- cargo 在 ~/.cargo/bin/（不在 PATH）
