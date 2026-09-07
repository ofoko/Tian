//! 天语言抽象语法树

/// 类型（规范第 6 节）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ty {
    I32,
    I64,
    F64,
    Str,
    Bool,
    /// v2.1 天权借用：只读借用（仅 str，规范 11.6）；不是所有者
    BorrowStr,
    /// v3.0 结构体：按声明序号索引 Program.structs（规范第 14 节）。
    /// 名义类型：同序号即同类型。保持 Copy 以免全量重构。
    Struct(u32),
    /// v3.3 固定长度数组：索引 Program.arrs（规范第 16 节）。
    /// 纯值类型（不参与天权）；同序号即同类型（元素与长度都一致）。
    Arr(u32),
    /// v4.0 动态数组：索引 Program.darrs（规范第 22 节）。
    /// 堆类型（参与天权：移动所有权、确定性释放）；同序号即同元素类型。
    DArr(u32),
    /// v4.5 元组：索引 Program.tuples（规范第 24 节）。
    /// 仅存在于函数返回边界；堆块（参与天权），解构后元素按类型接管。
    Tuple(u32),
    /// v4.7 关联数组：索引 Program.maps（规范第 25 节）。
    /// 哈希表；键 K ∈ {i64,str}，值 V ∈ {i32,i64,f64,bool,str}；堆类型（参与天权）。
    Map(u32),
}

impl Ty {
    /// 对应的 C 类型（结构体类型请用 gen_c::c_ty，此处不可用）
    pub fn c_name(&self) -> &'static str {
        match self {
            Ty::I32 => "int",
            Ty::I64 => "long long",
            Ty::F64 => "double",
            Ty::Str | Ty::BorrowStr => "const char*",
            Ty::Bool => "int",
            Ty::Struct(_) => "struct",
            Ty::Arr(_) => "array",
            Ty::DArr(_) => "void*",
            Ty::Tuple(_) => "void*",
            Ty::Map(_) => "void*",
        }
    }

    /// 报错信息用名称（结构体需经 structs 表解析，见 type_check::ty_label）
    pub fn label(&self) -> &'static str {
        match self {
            Ty::I32 => "i32",
            Ty::I64 => "i64",
            Ty::F64 => "f64",
            Ty::Str => "str",
            Ty::Bool => "bool",
            Ty::BorrowStr => "&str",
            Ty::Struct(_) => "struct",
            Ty::Arr(_) => "array",
            Ty::DArr(_) => "darr",
            Ty::Tuple(_) => "tuple",
            Ty::Map(_) => "map",
        }
    }

    pub fn is_numeric(&self) -> bool {
        matches!(self, Ty::I32 | Ty::I64 | Ty::F64)
    }

    /// v3.0：结构体类型（堆值，受天权所有权约束）
    pub fn is_struct(&self) -> bool {
        matches!(self, Ty::Struct(_))
    }

    /// v3.3：数组类型（纯值类型，规范第 16 节）
    pub fn is_arr(&self) -> bool {
        matches!(self, Ty::Arr(_))
    }

    /// v4.0：动态数组类型（堆类型，规范第 22 节）
    pub fn is_darr(&self) -> bool {
        matches!(self, Ty::DArr(_))
    }

    /// v4.5：元组类型（仅返回边界，规范第 24 节）
    pub fn is_tuple(&self) -> bool {
        matches!(self, Ty::Tuple(_))
    }

    /// v4.7：关联数组类型（堆类型，规范第 25 节）
    pub fn is_map(&self) -> bool {
        matches!(self, Ty::Map(_))
    }
}

/// v4.5 元组类型定义：元素类型列表（2–4 个，规范第 24 节）
#[derive(Debug, Clone)]
pub struct TupleDef {
    pub elems: Vec<Ty>,
}

/// v4.7 关联数组类型定义：键类型 K + 值类型 V（规范第 25 节）
#[derive(Debug, Clone)]
pub struct MapDef {
    pub key: Ty,
    pub val: Ty,
}

/// v4.0 动态数组类型定义：元素类型（长度运行时决定，规范第 22 节）
#[derive(Debug, Clone)]
pub struct DArrDef {
    pub elem: Ty,
}

/// v3.3 数组类型定义：元素类型 + 固定长度（规范第 16 节）
#[derive(Debug, Clone)]
pub struct ArrDef {
    pub elem: Ty,
    pub len: u64,
}

/// v3.0 结构体字段声明（规范第 14 节）
#[derive(Debug, Clone)]
pub struct FieldDef {
    pub name: String,
    pub ty: Ty,
    pub line: usize,
}

/// v3.0 结构体定义：值语义的复合堆类型，整体受天权所有权约束
#[derive(Debug, Clone)]
pub struct StructDef {
    pub name: String,
    pub fields: Vec<FieldDef>,
    pub line: usize,
    /// v3.9：来自 use 导入的文件（fmt 跳过，由其所在模块文件自含）
    pub imported: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    Ne,
}

impl BinOp {
    pub fn c_str(&self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Lt => "<",
            BinOp::Gt => ">",
            BinOp::Le => "<=",
            BinOp::Ge => ">=",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
        }
    }
}

#[derive(Debug, Clone)]
pub enum Expr {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Var(String),
    Neg(Box<Expr>),
    Bin {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    /// 函数调用（v0.1 无返回值，仅语句级）
    Call {
        name: String,
        args: Vec<Expr>,
    },
    /// toi / tos 转换
    Convert {
        name: String,
        arg: Box<Expr>,
    },
    /// v2.1 天权借用：&s（只读借用，规范 11.6；仅限 &str 形参实参位置）
    Borrow(Box<Expr>),
    /// v3.0 结构体字面量：Point{1,2} 或 Point{x:1,y:2}（规范第 14 节）
    StructLit {
        name: String,
        fields: Vec<(Option<String>, Expr)>,
        line: usize,
    },
    /// v3.0 字段访问：p.x（规范第 14 节）
    Field(Box<Expr>, String),
    /// v3.3 数组字面量：i64[3]{1,2,3}（规范第 16 节）；arr 为 Program.arrs 索引
    ArrLit { arr: u32, elems: Vec<Expr> },
    /// v3.3 数组下标：a[i]（读，规范第 16 节）；v4.0 起也用于动态数组
    Index(Box<Expr>, Box<Expr>),
    /// v4.0 动态数组字面量：[]i64{1,2}（规范第 22 节）
    DArrLit { darr: u32, elems: Vec<Expr> },
    /// v4.0 追加元素（语句级）：push(a, v)，可能 realloc 更换指针（规范第 22 节）
    Push { arr: Box<Expr>, value: Box<Expr> },
    /// v4.1 移除末元素（语句级）：pop(a)，长度必须 > 0（规范第 22 节）
    Pop { arr: Box<Expr> },
    /// v4.7 map 字面量：map[K]V{ k1: v1, k2: v2 }（可为空 {}，规范第 25 节）
    MapLit {
        map: u32,
        entries: Vec<(Expr, Expr)>,
    },
    /// v4.7 键存在性：has(m, k) → bool（表达式，规范第 25 节）
    Has { map: Box<Expr>, key: Box<Expr> },
    /// v4.7 删除键：del(m, k)（语句级，规范第 25 节）
    Del { map: Box<Expr>, key: Box<Expr> },
    /// v4.2 子串：sub(s, start, n)，产生新所有权的堆串（规范第 23 节）
    Sub {
        s: Box<Expr>,
        start: Box<Expr>,
        n: Box<Expr>,
    },
    /// v4.5 元组表达式：r/e1, e2（仅返回位置合法，规范第 24 节）
    TupExpr { elems: Vec<Expr>, tup: u32 },
    /// v3.3 数组长度：len(a)（编译期常量，规范第 16 节）
    Len(Box<Expr>),
    /// v3.7 条件表达式：sel(条件, a, b)，惰性求值（规范第 18 节）
    Sel {
        cond: Box<Expr>,
        a: Box<Expr>,
        b: Box<Expr>,
    },
}

/// 语句（line 用于报错定位）
#[derive(Debug, Clone)]
pub enum Stmt {
    Decl {
        mutable: bool,
        name: String,
        ty: Option<Ty>,
        value: Expr,
        line: usize,
    },
    /// v4.5：多声明解构 //a, b = f(...)（规范第 24 节；无逐名类型标注）
    MultiDecl {
        mutable: bool,
        names: Vec<String>,
        value: Expr,
        line: usize,
    },
    /// 赋值（v3.0 起左侧可为字段：p.x = e，规范 14.3）
    Assign {
        target: Expr,
        value: Expr,
        line: usize,
    },
    Print(Expr, usize),
    While {
        cond: Expr,
        body: Vec<Stmt>,
        line: usize,
    },
    If {
        cond: Expr,
        body: Vec<Stmt>,
        else_body: Option<Vec<Stmt>>,
        line: usize,
    },
    /// r/ 返回（v0.2 固定 i64；None = r/ 裸返回 0）
    Return(Option<Expr>, usize),
    /// 语句级函数调用
    Expr(Expr, usize),
    /// v3.8 break（仅 w/ 循环体内，规范第 19 节）
    Break(usize),
    /// v3.8 continue（仅 w/ 循环体内，规范第 19 节）
    Continue(usize),
    /// v4.0 panic(msg)：快速失败（规范第 21 节）
    Panic(Box<Expr>, usize),
    /// v4.0 check(cond, msg)：断言失败即 panic（规范第 21 节）
    Check(Box<Expr>, Box<Expr>, usize),
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    /// 类型标注，None = 默认 i64（规范 5.3）
    pub ty: Option<Ty>,
    /// v4.3：动态数组借用形参（&[]T，只读视图不移动，规范 22.8）
    pub borrow: bool,
}

/// v2.2 语义锚点（规范第 12 节）：嵌入函数的机器可验证契约。
/// 对 AI 是生成时的思维链约束，对编译器是运行时断言与自测样例。
#[derive(Debug, Clone)]
pub enum Contract {
    /// @pre: <bool 表达式>——函数入口必须成立的假设
    Pre(Expr, usize),
    /// @post: <bool 表达式>——函数出口必须成立的保证（ret 指返回值）
    Post(Expr, usize),
    /// @example: f(args) -> expected——启动自检样例
    Example { call: Expr, expected: Expr, line: usize },
}

#[derive(Debug, Clone)]
pub struct FnDef {
    pub name: String,
    pub params: Vec<Param>,
    /// 返回类型标注，None = 默认 i64
    pub ret: Option<Ty>,
    pub body: Vec<Stmt>,
    /// 语义锚点（v2.2）：仅允许出现在函数体首部
    pub contracts: Vec<Contract>,
    /// v3.9：来自 use 导入的文件（fmt 跳过）
    pub imported: bool,
}

#[derive(Debug, Default)]
pub struct Program {
    /// v3.0 结构体声明（顺序即 Ty::Struct 的索引）
    pub structs: Vec<StructDef>,
    /// v3.3 数组类型表（顺序即 Ty::Arr 的索引）
    pub arrs: Vec<ArrDef>,
    /// v4.0 动态数组类型表（顺序即 Ty::DArr 的索引）
    pub darrs: Vec<DArrDef>,
    /// v4.5 元组类型表（顺序即 Ty::Tuple 的索引）
    pub tuples: Vec<TupleDef>,
    /// v4.7 关联数组类型表（顺序即 Ty::Map 的索引）
    pub maps: Vec<MapDef>,
    /// v3.6 use 导入的模块名（按出现顺序，供 fmt 重建源码）
    pub uses: Vec<String>,
    pub funcs: Vec<FnDef>,
    pub top: Vec<Stmt>,
}
