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
        }
    }

    pub fn is_numeric(&self) -> bool {
        matches!(self, Ty::I32 | Ty::I64 | Ty::F64)
    }

    /// v3.0：结构体类型（堆值，受天权所有权约束）
    pub fn is_struct(&self) -> bool {
        matches!(self, Ty::Struct(_))
    }
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
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    /// 类型标注，None = 默认 i64（规范 5.3）
    pub ty: Option<Ty>,
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
}

#[derive(Debug, Default)]
pub struct Program {
    /// v3.0 结构体声明（顺序即 Ty::Struct 的索引）
    pub structs: Vec<StructDef>,
    pub funcs: Vec<FnDef>,
    pub top: Vec<Stmt>,
}
