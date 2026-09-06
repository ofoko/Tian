# Tian（天语言）项目长期笔记

- 位置：/Users/samly/T/tian/（规范：天语言规范.md；编译器：tc/，Rust 实现）
- 唯一标准：规范 v0.1.1。斜杠判定 2.7 = 表达式内 / 恒为除号，其余前缀声明（/a 只读，//b 可变）
- 命令：tc lex xxx.t（调试）/ tc xxx.t --emit-c / tc run xxx.t（C 后端）/ tc build xxx.t（Cranelift 原生机器码，失败回退 C）
- 版本 v1.1（2026-09-06）：Cranelift 原生后端落地，双后端输出一致，26 单测全绿；cargo 在 ~/.cargo/bin/（不在 PATH）
- 保留标记字：w/ i/ e/ f/ r/（受表达式上下文门控）；块内末语句可省略换行
- 规划：天权 v2.0（所有权/借用）→ struct/数组 → 标准库
