# 天语言路线图（v4.5 → v6+）

> 决策准绳（历史证明有效，继续坚守）：
> 1. **狗粮驱动**——每个版本由真实程序暴露的别扭拍板（json.t → 动态数组/借用/str 能力包；stats.t → tof/元组）。
> 2. **最小机制**——元组"仅存在于返回边界"是成功范例：不进表达式世界，不新增运行时表示。
> 3. **快速失败 + 诚实语义**——统一 `__t_panic` 出口；str 按字节诚实处理。
> 4. **双后端一致验收**——C 与 Cranelift 对拍；机制复用：类型表（STRUCTS/ARRS/DARRS/TUPLES → 未来 MAPS/ENUMS）+ 8 字节槽物理布局。
>
> 每版验收门：`cargo test` 全绿 · 双后端 golden 对拍 · `tc fmt` 幂等且可重编译 · 规范章节与修订记录同步。

---

## 第 0 步：工程卫生（立即，不是特性）

当前仓库只有一个 Initial commit，**v2.0～v4.5 全部成果都在未提交的工作树中**——一次误操作即全灭。

1. `git commit` 全量工作树，打 tag `v4.5`，建立基线。
2. `.bak-tuple-20260907-0912/` 移出仓库（基线建立后备份失去意义）。
3. 落地一键验收脚本 `run_tests.sh`：cargo test → 双后端对拍 demo 套件（json/stats/heap/sieve/tuple）→ fmt 幂等检查。此后每版只跑这一个脚本。

## 阶段一：数据结构补全（v4.6 – v4.8）✅ **已收官（2026-09-08，v4.9 枚举与 match）**

### v4.6 元组生态收尾 + 验收工程化（小步快跑）

- ~~**首要：修复 3 个被 C 回退长期掩盖的 Cranelift Verifier bug**~~ ✅ **已完成（2026-09-07），回退归零**。二分定位后确认实为 **9 个独立 bug**（远超登记的 3 个），全部被 C 回退掩盖多个版本：
  1. `If` 臂发射块终止符后无条件补 `jump(merge)` → Verifier 报错（v3.8 break 引入）
  2. `h=h` 自移动赋值仍执行置空 → darr 变量被清成 NULL → 段错误
  3. `While`/`If` 汇合新块未复位 `block_terminated` → 循环漏发回边
  4. `[]bool` 动态数组 4 条路径（字面量 push×2/赋值 push/dset/dget）缺 Bool↔I64 桥接
  5. `sel` 同宽 `uextend(I8→I8)` 非法——**sel 原生后端从未真正编译过**
  6. `check()` 同款同宽 uextend
  7. `emit_ty_of` Index 臂不认识 BorrowStr（`@pre: s[i]==34` 内部错误）
  8. `sub`/`tof` 借用实参缺"只读位视同 str"归一（coerce 无 BorrowStr→Str）
  9. `emit_ty_of` Convert 兜底把 tof 推成 I64 → f64 声明槽类型错配
- `@example` 支持元组**结构比较**：编译期解包逐元素比对（对拍 C/原生），**不引入元素访问语法**——坚守规范 24.1"仅返回边界"。
- `@post` 对元组返回给出明确报错文案（当前静默跳过）。
- 检查器补漏：元组作 `sel` 分支/字段/数组元素/打印的拦截用例补进单测。

### v4.7 关联数组 `map[K]V`（哈希表）

- 语法：`map[str]i64` 声明、`m[k]` 读写、内建 `has/ del/ len`；K 首批限 `i64 | str`，V 限标量/str/结构体指针。
- 物理：走类型表 `MAPS`；malloc 块（拉链或开放寻址，首版选**拉链**——删除与扩容简单，与 darr 头部复用 `{len, cap, buckets}` 布局）。
- 天权：整体移动语义；str 键/值随容器深释放（`__t_map_free_s`，复用 `dfree_s` 先例）；借用 `&map[K]V` 只读视图随 v4.3 先例。
- 狗粮：stats.t 从"单遍计数求和"升级为**按类目聚合**——这正是 v4.5 元组拍板时"消除两遍扫描"的下一站。

### v4.8 枚举与 match（和类型）——阶段一压轴 ✅ **已完成（2026-09-08）**

> 落地要点：`enum` 声明 + `Name::Variant(...)` 构造 + `match` 穷尽匹配（位图覆盖/多模式 `|`/`_` 通配）；递归自引用占位注册；枚举作 `[]Enum` 元素与 `map[str|i64]Enum` 值（`__t_dpush_e`/`__t_dget_e`/`__t_dfree_e`、`__t_mset_se/ie`±efree 回调/`__t_mget_se/ie`/`__t_mfree_se/ie`），原生 efree 注册前置于函数编译；借用枚举 match 不释放包装句柄防双重释放。狗娘 `demo/json.t` → 真 AST（golden json_ast.t / enumfull.t），双后端 84/84 全绿。

- 语法：`enum Json { Null, Bool(bool), Num(f64), Str(str), Arr([]Json), Obj(map[str]Json) }`；`match e { Num(n) => ..., _ => ... }`，**穷尽性检查**。
- 物理：tag + payload 指针（间接存储，递归类型必需）；tag 走类型表 `ENUMS`；payload 槽复用 8 字节槽协议。
- 天权：payload 含 str/容器时深释放（struct 深释放已有先例）；match 绑定按元素接管。
- match 惰性求值复用 sel 的块参数机制（规范 18 节）。
- 狗粮：json.t（152 行字符串状态机）重写为**真 AST**——递归下降产出 `Json` 值，校验与统计共用一个类型。这是天语言从"脚本级"跨入"建模级"的门槛版本。

## 阶段二：语言成熟度（v5.x）

| 版本 | 主题 | 要点 |
|---|---|---|
| v5.0 | **泛型**（单态化） | `Map<K,V>`、`Result<T,E>` 泛型化；`map[str]i64` 降级为语法糖。8 字节槽统一布局延后单态化压力（指针/标量同宽，首版可少生成特化） |
| v5.1 | **错误处理** | 内建 `Result(T, E)` + `?` 传播；**不引入异常**（与确定性释放冲突）。快速失败（panic/check）与可恢复错误（Result）双轨 |
| v5.2 | **字符串与格式化** | fmt 插值、`chars()` 码点迭代、科学计数法（v4.4 遗留边界：数字字符类不含 e/E） |
| v5.3 | **文件 IO** | `read_file/write_file/args` 内建，快速失败。狗粮从"内嵌字面量"升级为读真实数据文件 |
| v5.4 | **模块升级** | 嵌套路径、`pub`/私有、重导出；use 表复用现有跨文件共享机制 |

## 阶段三：生态与自举（v6+）

1. **标准库分层**：tcore（str/内存）→ tcoll（容器）→ tio；全部用天自身编写（吃最深的狗粮）。
2. **自举**：用天重写 lexer + parser（与 Rust 前端逐 token 对拍），后端保留 C/Cranelift。自举顺序：lexer → parser → checker，**不自举后端**（生成代码正确性是生命线）。
3. **工具链**：LSP（`--diagnose-json` 的 stage/line/hint/candidates 已是 LSP 诊断的直译）、`tc doc`、最小包管理（use 路径即包路径）。
4. **发布工程**：release 通道、Cranelift 版本跟进、交叉编译评估。

## 明确不做（设计否决项）

- **继承 / trait / GC / 异常 / 隐式数值宽转换**——分别与最小机制、确定性释放、快速失败哲学冲突。
- 元组升为一等公民——24.1 边界是被狗粮验证过的正确取舍。

## 风险登记

| 风险 | 缓解 |
|---|---|
| v4.8 递归 enum 的深释放协议复杂 | struct 深释放先例 + 逐元素天权接管已有完整机制 |
| map 扩容与天权移动语义交织 | push/realloc 语义照抄 darr（v4.0 已验证） |
| 泛型单态化膨胀 | 首版"统一 8 字节槽"少特化，实测膨胀再收紧 |
| 工作树长期不提交（已发生） | 第 0 步强制基线 + 每版一 tag |
